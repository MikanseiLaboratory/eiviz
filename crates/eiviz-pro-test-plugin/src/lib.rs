//! Fake Pro module for loader and lifecycle tests.
//!
//! Capture callbacks are serial. `capture_destroy` returns only after an
//! in-flight callback finishes. A second destroy is a no-op. `shutdown`
//! returns busy while a capture or output handle is still alive.

use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use eiviz_pro_api::{
    ABI_MAJOR, Entitlements, FEATURE_DECKLINK, FEATURE_LICENSE, FEATURE_RTMP, LAYOUT_UYVY,
    PRO_BUSY, PRO_DEVICE, PRO_INVALID, PRO_OK, ProApi, ProAudioView, ProCaptureCallbacks,
    ProCaptureStatsAbi, ProEntitlementsAbi, ProLicenseStatusAbi, ProOutputStatsAbi,
    ProPlayoutConfig, ProRtmpConfig, ProStatus, ProVideoView, abi_hash, copy_to_buffer,
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

unsafe extern "C" fn rtmp_start(
    module: *mut c_void,
    config: *const ProRtmpConfig,
    out: *mut *mut c_void,
) -> ProStatus {
    open_output(module, !config.is_null(), out)
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
        rtmp_start: Some(rtmp_start),
        output_submit_video: Some(output_submit_video),
        output_submit_audio: Some(output_submit_audio),
        output_stats: Some(output_stats),
        output_destroy: Some(output_destroy),
        license_install: Some(license_install),
        license_status: Some(license_status),
        license_clear: Some(license_clear),
        license_fingerprint: Some(license_fingerprint),
    }
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
