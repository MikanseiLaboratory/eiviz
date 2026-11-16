//! Output-device loopback and application capture through Core Audio process taps.
//!
//! The tap callback runs on a serial dispatch queue. It maps the tapped channels
//! to stereo and pushes them into the lock-free [`CaptureFeed`]; the capture
//! thread drains that feed into the shared input store.

use std::cell::UnsafeCell;
use std::ffi::{CString, c_char};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::capture::{AudioCaptureSpec, send_ready};
use super::feed::CaptureFeed;
use crate::upload::AudioInputStore;

const ERROR_CAP: usize = 1024;

enum EivizAudioTap {}

type TapCallback = unsafe extern "C" fn(
    ctx: *mut std::ffi::c_void,
    interleaved: *const f32,
    frames: u32,
    channels: u32,
    rate: f64,
);

unsafe extern "C" {
    fn eiviz_audio_tap_start(
        mode: u32,
        device_id: *const c_char,
        process_exe: *const c_char,
        process_aumid: *const c_char,
        callback: TapCallback,
        ctx: *mut std::ffi::c_void,
        err: *mut c_char,
        err_cap: usize,
    ) -> *mut EivizAudioTap;
    fn eiviz_audio_tap_stop(tap: *mut EivizAudioTap);
}

struct TapSink {
    feed: Arc<CaptureFeed>,
    map_left: usize,
    map_right: usize,
    scratch: UnsafeCell<Vec<f32>>,
}

// The callback is serialized on the tap's dispatch queue, and the sink is not
// touched from another thread until the tap has stopped.
unsafe impl Sync for TapSink {}

unsafe extern "C" fn tap_callback(
    ctx: *mut std::ffi::c_void,
    interleaved: *const f32,
    frames: u32,
    channels: u32,
    rate: f64,
) {
    if ctx.is_null() || interleaved.is_null() || frames == 0 || channels == 0 {
        return;
    }
    let sink = unsafe { &*(ctx as *const TapSink) };
    let count = frames as usize * channels as usize;
    let samples = unsafe { std::slice::from_raw_parts(interleaved, count) };
    sink.feed
        .set_rate(rate.round().clamp(1.0, 192_000.0) as u32);
    let scratch = unsafe { &mut *sink.scratch.get() };
    sink.feed.push_mapped(
        samples,
        channels as usize,
        sink.map_left,
        sink.map_right,
        scratch,
    );
}

fn take_error(buffer: &[c_char]) -> String {
    let end = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    let bytes: Vec<u8> = buffer[..end].iter().map(|value| *value as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

pub fn run(
    spec: &AudioCaptureSpec,
    uploads: &Arc<Mutex<AudioInputStore>>,
    stop: &AtomicBool,
    ready: Option<&std::sync::mpsc::Sender<Result<(), String>>>,
    signaled: &AtomicBool,
) -> Result<(), String> {
    let feed = Arc::new(CaptureFeed::new(48_000));
    let sink = Box::new(TapSink {
        feed: Arc::clone(&feed),
        map_left: spec.map_left.max(0) as usize,
        map_right: spec.map_right.max(0) as usize,
        scratch: UnsafeCell::new(Vec::with_capacity(16_384)),
    });
    let device = CString::new(spec.device_id.clone()).unwrap_or_default();
    let exe = CString::new(spec.process_exe.clone()).unwrap_or_default();
    let aumid = CString::new(spec.process_aumid.clone()).unwrap_or_default();
    let mut error = [0 as c_char; ERROR_CAP];
    let tap = unsafe {
        eiviz_audio_tap_start(
            spec.mode,
            device.as_ptr(),
            exe.as_ptr(),
            aumid.as_ptr(),
            tap_callback,
            (&*sink as *const TapSink).cast_mut().cast(),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    if tap.is_null() {
        let message = take_error(&error);
        send_ready(ready, signaled, Err(message.clone()))?;
        return Err(message);
    }
    send_ready(ready, signaled, Ok(()))?;

    let mut pts = 0i64;
    let mut last = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        feed.drain(uploads, spec.id, &mut pts);
        std::thread::sleep(Duration::from_millis(10));
        if last.elapsed() > Duration::from_secs(2) {
            last = Instant::now();
        }
    }
    unsafe { eiviz_audio_tap_stop(tap) };
    feed.drain(uploads, spec.id, &mut pts);
    Ok(())
}
