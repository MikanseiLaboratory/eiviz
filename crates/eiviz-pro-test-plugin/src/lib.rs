//! Fake Pro module for loader and lifecycle tests.
//!
//! Capture callbacks are serial. `capture_destroy` returns only after an
//! in-flight callback finishes. A second destroy is a no-op. `shutdown`
//! returns busy while a capture or output handle is still alive.

use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use eiviz_pro_api::{
    ABI_MAJOR, Entitlements, FEATURE_DECKLINK, FEATURE_LICENSE, FEATURE_RTMP, LAYOUT_BGRA,
    LAYOUT_UYVY, PRO_BUSY, PRO_DEVICE, PRO_INVALID, PRO_OK, PRO_UNAVAILABLE, ProApi, ProAudioView,
    ProCaptureCallbacks, ProCaptureStatsAbi, ProEntitlementsAbi, ProLicenseStatusAbi,
    ProOutputStatsAbi, ProPlayoutConfig, ProReplayConfig, ProReplayDecoded, ProReplayEstimateAbi,
    ProReplayPacket, ProReplayStatsAbi, ProStatus, ProStreamConfig, ProVideoView,
    REPLAY_CODEC_VMX1, abi_hash, copy_to_buffer,
};

const VERSION: &str = "0.3.0";

static LAST_ERROR: Mutex<String> = Mutex::new(String::new());
static HANDLES: AtomicUsize = AtomicUsize::new(0);
static CALLBACK_DEPTH: AtomicUsize = AtomicUsize::new(0);
static HOLD_CALLBACK: AtomicBool = AtomicBool::new(false);
static CALLBACK_ENTERED: AtomicBool = AtomicBool::new(false);
static HOLD_DESTROY: AtomicBool = AtomicBool::new(false);
static CAPTURE: Mutex<Option<CaptureState>> = Mutex::new(None);
static LICENSE_OK: AtomicBool = AtomicBool::new(false);
/// Frames stored through the passthrough path, so a test can see them arrive.
static ENCODED_FRAMES: AtomicUsize = AtomicUsize::new(0);

struct CaptureState {
    callbacks: ProCaptureCallbacks,
    frames: u64,
}

unsafe impl Send for CaptureState {}

fn error(message: &str) -> ProStatus {
    *LAST_ERROR
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = message.to_string();
    ProStatus::new(PRO_INVALID)
}

fn guard(body: impl FnOnce() -> ProStatus + std::panic::UnwindSafe) -> ProStatus {
    match std::panic::catch_unwind(body) {
        Ok(status) => status,
        Err(_) => {
            *LAST_ERROR
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = "panic".into();
            ProStatus::new(PRO_DEVICE)
        }
    }
}

fn module_ptr() -> *mut c_void {
    1 as *mut c_void
}

unsafe extern "C" fn create() -> *mut c_void {
    module_ptr()
}

unsafe extern "C" fn shutdown(module: *mut c_void) -> ProStatus {
    guard(|| {
        if module != module_ptr() {
            return error("bad module");
        }
        if HANDLES.load(Ordering::SeqCst) > 0 {
            return ProStatus::new(PRO_BUSY);
        }
        ProStatus::ok()
    })
}

unsafe extern "C" fn destroy(_module: *mut c_void) {}

unsafe extern "C" fn copy_name(
    module: *mut c_void,
    dst: *mut u8,
    cap: usize,
    written: *mut usize,
) -> ProStatus {
    guard(|| {
        if module != module_ptr() {
            return error("bad module");
        }
        copy_to_buffer(b"eiviz-pro-test", dst, cap, written)
    })
}

unsafe extern "C" fn copy_version(
    module: *mut c_void,
    dst: *mut u8,
    cap: usize,
    written: *mut usize,
) -> ProStatus {
    guard(|| {
        if module != module_ptr() {
            return error("bad module");
        }
        copy_to_buffer(VERSION.as_bytes(), dst, cap, written)
    })
}

unsafe extern "C" fn query_entitlements(
    module: *mut c_void,
    out: *mut ProEntitlementsAbi,
) -> ProStatus {
    guard(|| {
        if module != module_ptr() || out.is_null() {
            return error("bad entitlements query");
        }
        let raw =
            Entitlements::professional().to_abi(FEATURE_DECKLINK | FEATURE_RTMP | FEATURE_LICENSE);
        unsafe { *out = raw };
        ProStatus::ok()
    })
}

unsafe extern "C" fn copy_last_error(
    _module: *mut c_void,
    dst: *mut u8,
    cap: usize,
    written: *mut usize,
) -> ProStatus {
    let message = LAST_ERROR
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    copy_to_buffer(message.as_bytes(), dst, cap, written)
}

unsafe extern "C" fn decklink_enum_devices(
    module: *mut c_void,
    dst: *mut u8,
    cap: usize,
    written: *mut usize,
) -> ProStatus {
    guard(|| {
        if module != module_ptr() {
            return error("bad module");
        }
        copy_to_buffer(b"[]", dst, cap, written)
    })
}

unsafe extern "C" fn decklink_enum_modes(
    module: *mut c_void,
    _device: *const u8,
    _device_len: usize,
    dst: *mut u8,
    cap: usize,
    written: *mut usize,
) -> ProStatus {
    guard(|| {
        if module != module_ptr() {
            return error("bad module");
        }
        copy_to_buffer(b"[]", dst, cap, written)
    })
}

unsafe extern "C" fn capture_start(
    module: *mut c_void,
    _device: *const u8,
    _device_len: usize,
    _mode: *const u8,
    _mode_len: usize,
    callbacks: ProCaptureCallbacks,
    out: *mut *mut c_void,
) -> ProStatus {
    guard(|| {
        if module != module_ptr() || out.is_null() {
            return error("bad capture start");
        }
        if callbacks.struct_size < std::mem::size_of::<ProCaptureCallbacks>() as u32 {
            return error("capture callbacks are too short");
        }
        *CAPTURE.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(CaptureState {
            callbacks,
            frames: 0,
        });
        HANDLES.fetch_add(1, Ordering::SeqCst);
        unsafe { *out = 2 as *mut c_void };
        ProStatus::ok()
    })
}

unsafe extern "C" fn capture_stats(
    handle: *mut c_void,
    out: *mut ProCaptureStatsAbi,
    error_buf: *mut u8,
    error_cap: usize,
) -> ProStatus {
    guard(|| {
        if handle != 2 as *mut c_void || out.is_null() {
            return error("bad capture");
        }
        let frames = CAPTURE
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
            .map(|capture| capture.frames)
            .unwrap_or(0);
        unsafe {
            *out = ProCaptureStatsAbi {
                struct_size: std::mem::size_of::<ProCaptureStatsAbi>() as u32,
                frames,
                no_signal: 0,
                error_len: 0,
            };
        }
        if !error_buf.is_null() && error_cap > 0 {
            unsafe { *error_buf = 0 };
        }
        ProStatus::ok()
    })
}

fn finish_capture() {
    while CALLBACK_DEPTH.load(Ordering::SeqCst) > 0 || CALLBACK_ENTERED.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let mut slot = CAPTURE.lock().unwrap_or_else(|poison| poison.into_inner());
    if slot.take().is_some() {
        HANDLES.fetch_sub(1, Ordering::SeqCst);
    }
}

unsafe extern "C" fn capture_destroy(handle: *mut c_void) -> ProStatus {
    guard(|| {
        if handle != 2 as *mut c_void {
            return error("bad capture");
        }
        finish_capture();
        ProStatus::ok()
    })
}

unsafe extern "C" fn playout_start(
    module: *mut c_void,
    config: *const ProPlayoutConfig,
    out: *mut *mut c_void,
) -> ProStatus {
    open_output(module, !config.is_null(), out)
}

unsafe extern "C" fn stream_start(
    module: *mut c_void,
    config: *const ProStreamConfig,
    out: *mut *mut c_void,
) -> ProStatus {
    open_output(module, !config.is_null(), out)
}

unsafe extern "C" fn stream_add_destination(
    handle: *mut c_void,
    _id: u64,
    url: *const u8,
    url_len: usize,
) -> ProStatus {
    guard(|| {
        if handle != 3 as *mut c_void || url.is_null() && url_len != 0 {
            return error("bad destination");
        }
        ProStatus::ok()
    })
}

unsafe extern "C" fn stream_remove_destination(handle: *mut c_void, _id: u64) -> ProStatus {
    guard(|| {
        if handle != 3 as *mut c_void {
            return error("bad destination");
        }
        ProStatus::ok()
    })
}

unsafe extern "C" fn stream_destination_stats(
    handle: *mut c_void,
    _id: u64,
    out: *mut ProOutputStatsAbi,
    message: *mut u8,
    message_cap: usize,
) -> ProStatus {
    unsafe { output_stats(handle, out, message, message_cap) }
}

fn open_output(module: *mut c_void, ok: bool, out: *mut *mut c_void) -> ProStatus {
    if module != module_ptr() || out.is_null() || !ok {
        return error("bad output start");
    }
    HANDLES.fetch_add(1, Ordering::SeqCst);
    unsafe { *out = 3 as *mut c_void };
    ProStatus::ok()
}

unsafe extern "C" fn output_submit_video(
    handle: *mut c_void,
    _frame: *const ProVideoView,
) -> ProStatus {
    guard(|| {
        if handle != 3 as *mut c_void {
            return error("bad output");
        }
        ProStatus::new(PRO_OK)
    })
}

unsafe extern "C" fn output_submit_audio(
    handle: *mut c_void,
    _frame: *const ProAudioView,
) -> ProStatus {
    guard(|| {
        if handle != 3 as *mut c_void {
            return error("bad output");
        }
        ProStatus::ok()
    })
}

unsafe extern "C" fn output_stats(
    handle: *mut c_void,
    out: *mut ProOutputStatsAbi,
    message: *mut u8,
    message_cap: usize,
) -> ProStatus {
    guard(|| {
        if handle != 3 as *mut c_void || out.is_null() {
            return error("bad output");
        }
        unsafe {
            *out = ProOutputStatsAbi {
                struct_size: std::mem::size_of::<ProOutputStatsAbi>() as u32,
                connected: 1,
                video_frames: 1,
                repeated: 0,
                dropped: 0,
                bitrate: 0,
                encoder_threads: 0,
                media_lag_ms: 0,
                sent_fps_milli: 0,
            };
        }
        if !message.is_null() && message_cap > 0 {
            unsafe { *message = 0 };
        }
        ProStatus::ok()
    })
}

unsafe extern "C" fn output_destroy(handle: *mut c_void) -> ProStatus {
    guard(|| {
        if handle != 3 as *mut c_void {
            return error("bad output");
        }
        while HOLD_DESTROY.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if HANDLES.load(Ordering::SeqCst) > 0 {
            HANDLES.fetch_sub(1, Ordering::SeqCst);
        }
        ProStatus::ok()
    })
}

unsafe extern "C" fn license_install(module: *mut c_void, ticket: *const c_char) -> ProStatus {
    guard(|| {
        if module != module_ptr() || ticket.is_null() {
            return error("bad ticket");
        }
        let text = unsafe { std::ffi::CStr::from_ptr(ticket) };
        LICENSE_OK.store(text.to_bytes() == b"good-ticket", Ordering::SeqCst);
        if LICENSE_OK.load(Ordering::SeqCst) {
            ProStatus::ok()
        } else {
            error("bad signature")
        }
    })
}

unsafe extern "C" fn license_status(
    module: *mut c_void,
    out: *mut ProLicenseStatusAbi,
    ticket_id: *mut u8,
    cap: usize,
    written: *mut usize,
) -> ProStatus {
    guard(|| {
        if module != module_ptr() || out.is_null() {
            return error("bad license status");
        }
        let valid = LICENSE_OK.load(Ordering::SeqCst);
        unsafe {
            *out = ProLicenseStatusAbi {
                struct_size: std::mem::size_of::<ProLicenseStatusAbi>() as u32,
                state: if valid { 1 } else { 0 },
                plan: if valid { 1 } else { 0 },
                expires_at: if valid { 9_999_999_999 } else { 0 },
            };
        }
        copy_to_buffer(
            if valid { b"test-ticket" } else { b"" },
            ticket_id,
            cap,
            written,
        )
    })
}

unsafe extern "C" fn license_clear(module: *mut c_void) -> ProStatus {
    guard(|| {
        if module != module_ptr() {
            return error("bad module");
        }
        LICENSE_OK.store(false, Ordering::SeqCst);
        ProStatus::ok()
    })
}

unsafe extern "C" fn license_fingerprint(
    module: *mut c_void,
    dst: *mut u8,
    cap: usize,
    written: *mut usize,
) -> ProStatus {
    guard(|| {
        if module != module_ptr() {
            return error("bad module");
        }
        copy_to_buffer(b"[]", dst, cap, written)
    })
}

unsafe extern "C" fn license_email(
    module: *mut c_void,
    dst: *mut u8,
    cap: usize,
    written: *mut usize,
) -> ProStatus {
    guard(|| {
        if module != module_ptr() {
            return error("bad module");
        }
        let valid = LICENSE_OK.load(Ordering::SeqCst);
        copy_to_buffer(
            if valid { b"test@example.com" } else { b"" },
            dst,
            cap,
            written,
        )
    })
}

fn base_api() -> ProApi {
    ProApi {
        struct_size: std::mem::size_of::<ProApi>() as u32,
        abi_major: ABI_MAJOR,
        abi_hash: abi_hash(),
        create: Some(create),
        shutdown: Some(shutdown),
        destroy: Some(destroy),
        copy_name: Some(copy_name),
        copy_version: Some(copy_version),
        query_entitlements: Some(query_entitlements),
        copy_last_error: Some(copy_last_error),
        decklink_enum_devices: Some(decklink_enum_devices),
        decklink_enum_modes: Some(decklink_enum_modes),
        capture_start: Some(capture_start),
        capture_stats: Some(capture_stats),
        capture_destroy: Some(capture_destroy),
        playout_start: Some(playout_start),
        stream_start: Some(stream_start),
        stream_add_destination: Some(stream_add_destination),
        stream_remove_destination: Some(stream_remove_destination),
        stream_destination_stats: Some(stream_destination_stats),
        output_submit_video: Some(output_submit_video),
        output_submit_audio: Some(output_submit_audio),
        output_stats: Some(output_stats),
        output_destroy: Some(output_destroy),
        license_install: Some(license_install),
        license_status: Some(license_status),
        license_clear: Some(license_clear),
        license_fingerprint: Some(license_fingerprint),
        license_email: Some(license_email),
        replay_open: Some(replay_open),
        // The test plugin has no recording to attach to.
        replay_attach: None,
        replay_close: Some(replay_close),
        replay_write_video: Some(replay_write_video),
        replay_decode: Some(replay_decode),
        replay_range: Some(replay_range),
        replay_stats: Some(replay_stats),
        replay_estimate: Some(replay_estimate),
        replay_write_audio: Some(replay_write_audio),
        replay_write_encoded: Some(replay_write_encoded),
        replay_read_audio: Some(replay_read_audio),
        replay_copy_peaks: Some(replay_copy_peaks),
        replay_export_start: Some(replay_export_start),
        replay_export_progress: Some(replay_export_progress),
        replay_export_cancel: Some(replay_export_cancel),
        // The test store stores raw pictures; it encodes one into a VMX1 packet
        // on demand so the mixer's GPU playback path is exercised too.
        replay_read_packet: Some(replay_read_packet),
    }
}

unsafe extern "C" fn replay_export_start(
    handle: *mut c_void,
    _request: *const eiviz_pro_api::ProReplayExportRequest,
) -> ProStatus {
    guard(|| {
        if handle != 9 as *mut c_void {
            return error("bad replay export start");
        }
        ProStatus::ok()
    })
}

unsafe extern "C" fn replay_export_progress(
    handle: *mut c_void,
    out: *mut eiviz_pro_api::ProReplayExportProgress,
    _message: *mut u8,
    _cap: usize,
    written: *mut usize,
) -> ProStatus {
    guard(|| {
        if handle != 9 as *mut c_void || out.is_null() {
            return error("bad replay export progress");
        }
        unsafe {
            *out = eiviz_pro_api::ProReplayExportProgress {
                struct_size: std::mem::size_of::<eiviz_pro_api::ProReplayExportProgress>() as u32,
                ..Default::default()
            };
        }
        if !written.is_null() {
            unsafe { *written = 0 };
        }
        ProStatus::ok()
    })
}

unsafe extern "C" fn replay_export_cancel(handle: *mut c_void) -> ProStatus {
    guard(|| {
        if handle != 9 as *mut c_void {
            return error("bad replay export cancel");
        }
        ProStatus::ok()
    })
}
unsafe extern "C" fn replay_write_audio(
    handle: *mut c_void,
    angle_id: u32,
    frame_index: u64,
    view: *const eiviz_pro_api::ProReplayAudioView,
) -> ProStatus {
    guard(|| {
        if handle != 9 as *mut c_void || view.is_null() {
            return error("bad replay audio write");
        }
        let view = unsafe { &*view };
        if view.channels == 0 || view.data.is_null() {
            return error("bad replay audio write");
        }
        let samples = unsafe {
            std::slice::from_raw_parts(view.data, view.data_len / std::mem::size_of::<f32>())
        };
        let channels = view.channels as usize;
        let mut store = REPLAY_AUDIO
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(store) = store.as_mut() else {
            return ProStatus::new(PRO_UNAVAILABLE);
        };
        // The mixer asks for a frame's worth of audio at the frame it belongs to,
        // so the recording keeps one timeline for picture and sound.
        let track = store.entry(angle_id).or_default();
        let offset = frame_index as usize * channels;
        let end = offset.saturating_add(samples.len());
        if track.len() < end {
            track.resize(end, 0.0);
        }
        track[offset..end].copy_from_slice(samples);
        ProStatus::ok()
    })
}

unsafe extern "C" fn replay_read_audio(
    handle: *mut c_void,
    angle_id: u32,
    from_frame: u64,
    frames: usize,
    out: *mut f32,
    written: *mut u32,
) -> ProStatus {
    guard(|| {
        if handle != 9 as *mut c_void || out.is_null() {
            return error("bad replay audio read");
        }
        if !written.is_null() {
            unsafe { *written = 0 };
        }
        let store = REPLAY_AUDIO
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(track) = store.as_ref().and_then(|store| store.get(&angle_id)) else {
            return ProStatus::ok();
        };
        // The mixer reads interleaved stereo, which is what it records.
        let channels = 2usize;
        let offset = from_frame as usize * channels;
        let available = track.len().saturating_sub(offset) / channels;
        let count = frames.min(available);
        if count == 0 {
            return ProStatus::ok();
        }
        unsafe {
            std::ptr::copy_nonoverlapping(track.as_ptr().add(offset), out, count * channels);
            if !written.is_null() {
                *written = count as u32;
            }
        }
        ProStatus::ok()
    })
}

unsafe extern "C" fn replay_copy_peaks(
    handle: *mut c_void,
    _angle_id: u32,
    _from: u32,
    _min_out: *mut f32,
    _max_out: *mut f32,
    _cap: u32,
    copied: *mut u32,
    dropped: *mut u32,
) -> ProStatus {
    guard(|| {
        if handle != 9 as *mut c_void {
            return error("bad replay peaks");
        }
        if !copied.is_null() {
            unsafe { *copied = 0 };
        }
        if !dropped.is_null() {
            unsafe { *dropped = 0 };
        }
        ProStatus::ok()
    })
}

struct ReplayFrame {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    stride: u32,
    layout: u32,
    pts_100ns: i64,
}

static REPLAY: Mutex<Option<std::collections::HashMap<(u32, u64), ReplayFrame>>> = Mutex::new(None);

/// One codec per angle, used only to turn a stored raw picture into a VMX1 packet
/// for the mixer's GPU playback path.
static REPLAY_ENCODERS: Mutex<Option<std::collections::HashMap<u32, vmx::Codec>>> =
    Mutex::new(None);

/// Recorded replay audio, interleaved stereo at 48 kHz, indexed by frame.
static REPLAY_AUDIO: Mutex<Option<std::collections::HashMap<u32, Vec<f32>>>> = Mutex::new(None);

unsafe extern "C" fn replay_open(
    module: *mut c_void,
    _config: *const ProReplayConfig,
    out: *mut *mut c_void,
) -> ProStatus {
    guard(|| {
        if module != module_ptr() || out.is_null() {
            return error("bad replay open");
        }
        *REPLAY.lock().unwrap_or_else(|poison| poison.into_inner()) =
            Some(std::collections::HashMap::new());
        *REPLAY_AUDIO
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(std::collections::HashMap::new());
        *REPLAY_ENCODERS
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(std::collections::HashMap::new());
        HANDLES.fetch_add(1, Ordering::SeqCst);
        unsafe { *out = 9 as *mut c_void };
        ProStatus::ok()
    })
}

unsafe extern "C" fn replay_close(handle: *mut c_void) -> ProStatus {
    guard(|| {
        if handle != 9 as *mut c_void {
            return error("bad replay handle");
        }
        *REPLAY.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
        *REPLAY_AUDIO
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
        *REPLAY_ENCODERS
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
        HANDLES.fetch_sub(1, Ordering::SeqCst);
        ProStatus::ok()
    })
}

unsafe extern "C" fn replay_write_video(
    handle: *mut c_void,
    angle_id: u32,
    frame_index: u64,
    view: *const ProVideoView,
) -> ProStatus {
    guard(|| {
        if handle != 9 as *mut c_void || view.is_null() {
            return error("bad replay write");
        }
        if std::env::var("EIVIZ_TEST_REPLAY_BUSY").ok().as_deref() == Some("1") {
            return ProStatus::new(PRO_BUSY);
        }
        let view = unsafe { &*view };
        let bytes = unsafe { std::slice::from_raw_parts(view.data, view.data_len) }.to_vec();
        REPLAY
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_mut()
            .ok_or(())
            .map(|map| {
                map.insert(
                    (angle_id, frame_index),
                    ReplayFrame {
                        bytes,
                        width: view.width,
                        height: view.height,
                        stride: view.stride,
                        layout: view.layout,
                        pts_100ns: view.pts_100ns,
                    },
                );
            })
            .map_err(|_| ())
            .ok();
        ProStatus::ok()
    })
}

/// Stores a frame the sender already encoded. The test module keeps no bitstream
/// (the real store does), but it counts what arrived so a test can prove the
/// passthrough path is wired, and it refuses an empty payload like a real store.
unsafe extern "C" fn replay_write_encoded(
    handle: *mut c_void,
    _angle_id: u32,
    _frame_index: u64,
    view: *const eiviz_pro_api::ProReplayEncodedView,
) -> ProStatus {
    guard(|| {
        if handle != 9 as *mut c_void || view.is_null() {
            return error("bad encoded replay write");
        }
        let view = unsafe { &*view };
        if view.data_len == 0 || view.data.is_null() || view.width == 0 || view.height == 0 {
            return error("an encoded frame needs a bitstream and a size");
        }
        ENCODED_FRAMES.fetch_add(1, Ordering::Relaxed);
        ProStatus::ok()
    })
}

unsafe extern "C" fn replay_decode(
    handle: *mut c_void,
    angle_id: u32,
    frame_index: u64,
    dst: *mut u8,
    dst_cap: usize,
    out: *mut ProReplayDecoded,
) -> ProStatus {
    guard(|| {
        if handle != 9 as *mut c_void || dst.is_null() || out.is_null() {
            return error("bad replay decode");
        }
        let map = REPLAY.lock().unwrap_or_else(|poison| poison.into_inner());
        let Some(map) = map.as_ref() else {
            return ProStatus::new(PRO_UNAVAILABLE);
        };
        let Some(frame) = map.get(&(angle_id, frame_index)) else {
            return ProStatus::new(PRO_UNAVAILABLE);
        };
        let uyvy_len = frame.width as usize * frame.height as usize * 2;
        if dst_cap < uyvy_len {
            return error("replay decode buffer is short");
        }
        unsafe {
            if frame.layout == LAYOUT_UYVY {
                std::ptr::copy_nonoverlapping(
                    frame.bytes.as_ptr(),
                    dst,
                    uyvy_len.min(frame.bytes.len()),
                );
            } else if frame.layout == LAYOUT_BGRA {
                for y in 0..frame.height as usize {
                    for x in (0..frame.width as usize).step_by(2) {
                        let source = y * frame.stride as usize + x * 4;
                        let target = y * frame.width as usize * 2 + x * 2;
                        let b0 = frame.bytes.get(source).copied().unwrap_or(0) as i32;
                        let g0 = frame.bytes.get(source + 1).copied().unwrap_or(0) as i32;
                        let r0 = frame.bytes.get(source + 2).copied().unwrap_or(0) as i32;
                        let b1 = frame.bytes.get(source + 4).copied().unwrap_or(b0 as u8) as i32;
                        let g1 = frame.bytes.get(source + 5).copied().unwrap_or(g0 as u8) as i32;
                        let r1 = frame.bytes.get(source + 6).copied().unwrap_or(r0 as u8) as i32;
                        let y0 = ((77 * r0 + 150 * g0 + 29 * b0) >> 8) as u8;
                        let y1 = ((77 * r1 + 150 * g1 + 29 * b1) >> 8) as u8;
                        *dst.add(target) = ((b0 - y0 as i32) / 2 + 128).clamp(0, 255) as u8;
                        *dst.add(target + 1) = y0;
                        *dst.add(target + 2) = ((r1 - y1 as i32) / 2 + 128).clamp(0, 255) as u8;
                        *dst.add(target + 3) = y1;
                    }
                }
            } else {
                return error("unsupported replay pixel layout");
            }
            *out = ProReplayDecoded {
                struct_size: std::mem::size_of::<ProReplayDecoded>() as u32,
                actual_frame_index: frame_index,
                width: frame.width,
                height: frame.height,
                stride: frame.width * 2,
                pts_100ns: frame.pts_100ns,
                substituted: 0,
            };
        }
        ProStatus::ok()
    })
}

fn encode_replay_packet(angle_id: u32, frame: &ReplayFrame) -> Option<Vec<u8>> {
    let mut guard = REPLAY_ENCODERS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let cache = guard.as_mut()?;
    if !cache.contains_key(&angle_id) {
        let codec = vmx::Codec::new(vmx::Config {
            width: frame.width as i32,
            height: frame.height as i32,
            profile: vmx::Profile::Sq,
            color_space: vmx::ColorSpace::Bt709,
        })
        .ok()?;
        cache.insert(angle_id, codec);
    }
    let codec = cache.get_mut(&angle_id)?;
    let encoded = match frame.layout {
        LAYOUT_UYVY => codec
            .encode_uyvy(&frame.bytes, frame.stride as usize)
            .is_ok(),
        LAYOUT_BGRA => codec
            .encode_bgrx(&frame.bytes, frame.stride as usize)
            .is_ok(),
        _ => false,
    };
    if !encoded {
        return None;
    }
    let mut packet = vec![0u8; 1024 * 1024];
    loop {
        match codec.save_to(&mut packet) {
            Ok(size) => {
                packet.truncate(size);
                return Some(packet);
            }
            Err(_) => packet.resize(packet.len().saturating_mul(2).max(1024), 0),
        }
    }
}

unsafe extern "C" fn replay_read_packet(
    handle: *mut c_void,
    angle_id: u32,
    frame_index: u64,
    data: *mut u8,
    data_cap: usize,
    required: *mut usize,
    out: *mut ProReplayPacket,
) -> ProStatus {
    guard(|| {
        if handle != 9 as *mut c_void || required.is_null() || out.is_null() {
            return error("bad replay packet");
        }
        let (width, height, pts, packet) = {
            let map = REPLAY.lock().unwrap_or_else(|poison| poison.into_inner());
            let Some(map) = map.as_ref() else {
                return ProStatus::new(PRO_UNAVAILABLE);
            };
            let Some(frame) = map.get(&(angle_id, frame_index)) else {
                return ProStatus::new(PRO_UNAVAILABLE);
            };
            let Some(packet) = encode_replay_packet(angle_id, frame) else {
                return error("replay packet encode failed");
            };
            (frame.width, frame.height, frame.pts_100ns, packet)
        };
        unsafe { *required = packet.len() };
        if data.is_null() || data_cap < packet.len() {
            return ProStatus::new(PRO_BUSY);
        }
        unsafe {
            std::ptr::copy_nonoverlapping(packet.as_ptr(), data, packet.len());
            *out = ProReplayPacket {
                struct_size: std::mem::size_of::<ProReplayPacket>() as u32,
                actual_frame_index: frame_index,
                width,
                height,
                pts_100ns: pts,
                substituted: 0,
                codec: REPLAY_CODEC_VMX1,
                required_len: packet.len(),
            };
        }
        ProStatus::ok()
    })
}

unsafe extern "C" fn replay_range(
    handle: *mut c_void,
    angle_id: u32,
    first: *mut u64,
    last: *mut u64,
) -> ProStatus {
    guard(|| {
        if handle != 9 as *mut c_void || first.is_null() || last.is_null() {
            return error("bad replay range");
        }
        let map = REPLAY.lock().unwrap_or_else(|poison| poison.into_inner());
        let Some(map) = map.as_ref() else {
            return ProStatus::new(PRO_UNAVAILABLE);
        };
        let mut found: Option<(u64, u64)> = None;
        for ((angle, frame), _) in map {
            if *angle != angle_id {
                continue;
            }
            found = Some(match found {
                None => (*frame, *frame),
                Some((start, end)) => (start.min(*frame), end.max(*frame)),
            });
        }
        let Some((start, end)) = found else {
            return ProStatus::new(PRO_UNAVAILABLE);
        };
        unsafe {
            *first = start;
            *last = end;
        }
        ProStatus::ok()
    })
}

unsafe extern "C" fn replay_stats(
    handle: *mut c_void,
    stats: *mut ProReplayStatsAbi,
    _angles: *mut eiviz_pro_api::ProReplayAngleStatsAbi,
    _cap: u32,
    written: *mut u32,
) -> ProStatus {
    guard(|| {
        if handle != 9 as *mut c_void || stats.is_null() || written.is_null() {
            return error("bad replay stats");
        }
        unsafe {
            *stats = ProReplayStatsAbi {
                struct_size: std::mem::size_of::<ProReplayStatsAbi>() as u32,
                state: 1,
                encoded_frames: ENCODED_FRAMES.load(Ordering::Relaxed) as u64,
                ..ProReplayStatsAbi::default()
            };
            *written = 0;
        }
        ProStatus::ok()
    })
}

unsafe extern "C" fn replay_estimate(
    module: *mut c_void,
    _path: *const u8,
    _path_len: usize,
    _width: u32,
    _height: u32,
    _fps_num: u32,
    _fps_den: u32,
    _profile: u32,
    _angles: u32,
    _reserve: u64,
    out: *mut ProReplayEstimateAbi,
) -> ProStatus {
    guard(|| {
        if module != module_ptr() || out.is_null() {
            return error("bad replay estimate");
        }
        unsafe {
            *out = ProReplayEstimateAbi {
                struct_size: std::mem::size_of::<ProReplayEstimateAbi>() as u32,
                free_bytes: 1 << 40,
                estimate_rate_bps: 1,
                remaining_secs: 60,
                path_writable: 1,
            };
        }
        ProStatus::ok()
    })
}

fn published() -> &'static Mutex<ProApi> {
    static TABLE: OnceLock<Mutex<ProApi>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(base_api()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn eiviz_pro_get_api(
    major: u32,
    hash: u64,
    struct_size: u32,
) -> *const ProApi {
    let fault = std::env::var("EIVIZ_PRO_TEST_FAULT").unwrap_or_default();
    if fault == "reject"
        || major != ABI_MAJOR
        || hash != abi_hash()
        || struct_size != std::mem::size_of::<ProApi>() as u32
    {
        return std::ptr::null();
    }
    let mut table = published()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    *table = base_api();
    if fault == "short" {
        table.struct_size = 4;
    }
    if fault == "hash" {
        table.abi_hash = 1;
    }
    if fault == "incomplete" {
        table.license_fingerprint = None;
    }
    std::ptr::from_ref(&*table)
}

fn emit_video() {
    let mut slot = CAPTURE.lock().unwrap_or_else(|poison| poison.into_inner());
    let Some(capture) = slot.as_mut() else {
        return;
    };
    let callback = capture.callbacks.video;
    let user = capture.callbacks.user_data;
    capture.frames += 1;
    drop(slot);
    let Some(callback) = callback else {
        return;
    };
    CALLBACK_DEPTH.fetch_add(1, Ordering::SeqCst);
    static PIXELS: [u8; 4] = [0; 4];
    let view = ProVideoView {
        width: 16,
        height: 16,
        stride: 32,
        layout: LAYOUT_UYVY,
        pts_100ns: 1,
        fps_num: 30,
        fps_den: 1,
        data: PIXELS.as_ptr(),
        data_len: PIXELS.len(),
    };
    unsafe { callback(user, &view) };
    CALLBACK_DEPTH.fetch_sub(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
pub extern "C" fn eiviz_pro_test_emit_video() {
    emit_video();
}

#[unsafe(no_mangle)]
pub extern "C" fn eiviz_pro_test_arm_hold() {
    HOLD_CALLBACK.store(true, Ordering::SeqCst);
    CALLBACK_ENTERED.store(false, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
pub extern "C" fn eiviz_pro_test_fire_held() {
    std::thread::spawn(|| {
        CALLBACK_ENTERED.store(true, Ordering::SeqCst);
        while HOLD_CALLBACK.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        emit_video();
        CALLBACK_ENTERED.store(false, Ordering::SeqCst);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn eiviz_pro_test_callback_entered() -> u32 {
    u32::from(CALLBACK_ENTERED.load(Ordering::SeqCst))
}

#[unsafe(no_mangle)]
pub extern "C" fn eiviz_pro_test_release_hold() {
    HOLD_CALLBACK.store(false, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
pub extern "C" fn eiviz_pro_test_handles() -> u32 {
    HANDLES.load(Ordering::SeqCst) as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn eiviz_pro_test_arm_destroy_hold() {
    HOLD_DESTROY.store(true, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
pub extern "C" fn eiviz_pro_test_release_destroy_hold() {
    HOLD_DESTROY.store(false, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn eiviz_pro_test_double_destroy() -> u32 {
    let first = unsafe { capture_destroy(2 as *mut c_void) };
    let second = unsafe { capture_destroy(2 as *mut c_void) };
    u32::from(first.is_ok() && second.is_ok() && HANDLES.load(Ordering::SeqCst) == 0)
}

#[cfg(test)]
mod contract {
    use super::*;

    #[test]
    fn destroy_waits_for_callback_and_shutdown_reports_busy() {
        let callbacks = ProCaptureCallbacks {
            struct_size: std::mem::size_of::<ProCaptureCallbacks>() as u32,
            user_data: std::ptr::null_mut(),
            video: None,
            audio: None,
            signal: None,
        };
        let mut handle = std::ptr::null_mut();
        let status = unsafe {
            capture_start(
                module_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null(),
                0,
                callbacks,
                &mut handle,
            )
        };
        assert!(status.is_ok());
        assert_eq!(unsafe { shutdown(module_ptr()) }.code, PRO_BUSY);
        eiviz_pro_test_arm_hold();
        eiviz_pro_test_fire_held();
        while eiviz_pro_test_callback_entered() == 0 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let handle_bits = handle as usize;
        let worker = std::thread::spawn(move || {
            let status = unsafe { capture_destroy(handle_bits as *mut c_void) };
            let _ = tx.send(status.is_ok());
        });
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(rx.try_recv().is_err());
        eiviz_pro_test_release_hold();
        assert!(worker.join().is_ok());
        assert!(rx.recv().unwrap());
        assert_eq!(unsafe { eiviz_pro_test_double_destroy() }, 1);
        assert!(unsafe { shutdown(module_ptr()) }.is_ok());
    }
}
