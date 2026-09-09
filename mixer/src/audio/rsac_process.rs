use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rsac::{AudioCapture, AudioCaptureBuilder, CaptureTarget, ProcessId};

use crate::upload::AudioInputStore;

use super::capture::{AudioCaptureSpec, send_ready};
use super::pcm::interleaved_f32_packet;

/// Process-loopback via rsac. We only use `AudioCapture` + `read_buffer` (the
/// crate's `WindowsApplicationCapture::start_capture` callback is RT-unsafe).
/// rsac initializes the virtual client as f32 / 48 kHz / stereo with
/// `autoconvert: false` — process-loopback `GetMixFormat` is E_NOTIMPL and
/// `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM` must not be combined with loopback.
pub fn run(
    spec: &AudioCaptureSpec,
    uploads: &Arc<Mutex<AudioInputStore>>,
    stop: &AtomicBool,
    ready: Option<&mpsc::Sender<Result<(), String>>>,
    signaled: &AtomicBool,
) -> Result<(), String> {
    if spec.process_exe.trim().is_empty() && spec.process_aumid.trim().is_empty() {
        return Err("WASAPI process loopback needs a process exe or AUMID".into());
    }

    let _ = unsafe {
        windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        )
    };

    let map_left = spec.map_left.max(0) as usize;
    let map_right = spec.map_right.max(0) as usize;
    let mut first = true;
    let mut pts = 0i64;
    while !stop.load(Ordering::Relaxed) {
        let Some(pid) = super::process::resolve_pid(&spec.process_exe, &spec.process_aumid) else {
            if first {
                return Err(format!(
                    "process not running exe='{}' aumid='{}'",
                    spec.process_exe, spec.process_aumid
                ));
            }
            thread::sleep(Duration::from_millis(250));
            continue;
        };

        let mut capture = match open_capture(pid) {
            Ok(capture) => capture,
            Err(error) => {
                if first {
                    return Err(error);
                }
                crate::diag::error(&format!("audio capture {}: {error}", spec.id));
                thread::sleep(Duration::from_millis(250));
                continue;
            }
        };
        send_ready(ready, signaled, Ok(()))?;
        first = false;

        let mut follow_check = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            match capture.read_buffer() {
                Ok(Some(buffer)) => {
                    let channels = buffer.channels() as usize;
                    let rate = buffer.sample_rate().max(1);
                    let frames = buffer.num_frames() as i64;
                    let packet = interleaved_f32_packet(
                        pts,
                        rate as i32,
                        buffer.data(),
                        channels,
                        map_left,
                        map_right,
                    );
                    uploads.lock().expect("audio").ingest_audio(spec.id, packet);
                    pts = pts.saturating_add(frames * 10_000_000 / i64::from(rate));
                }
                Ok(None) => thread::sleep(Duration::from_millis(5)),
                Err(error) => {
                    crate::diag::error(&format!("audio capture {}: rsac read: {error}", spec.id));
                    break;
                }
            }
            if follow_check.elapsed() >= Duration::from_millis(250) {
                follow_check = Instant::now();
                if !super::process::process_alive(pid) {
                    break;
                }
            }
        }
        let _ = capture.stop();
    }
    Ok(())
}

fn open_capture(pid: u32) -> Result<AudioCapture, String> {
    let mut capture = AudioCaptureBuilder::new()
        .with_target(CaptureTarget::ProcessTree(ProcessId(pid)))
        .sample_rate(48_000)
        .channels(2)
        .build()
        .map_err(|error| format!("rsac process build: {error}"))?;
    capture
        .start()
        .map_err(|error| format!("rsac process start: {error}"))?;
    Ok(capture)
}
