use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
const SLOW_LOCK_MS: u128 = 20;

static INIT: OnceLock<()> = OnceLock::new();
static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn init() {
    INIT.get_or_init(|| {
        install_panic_hook();
        #[cfg(windows)]
        install_unhandled_exception_filter();
        if let Ok(mut slot) = LOG.lock() {
            *slot = open_log();
        }
        write("INFO", "diag init");
    });
}

pub fn info(message: &str) {
    init();
    write("INFO", message);
}

pub fn warn(message: &str) {
    init();
    write("WARN", message);
}

pub fn error(message: &str) {
    init();
    write("ERROR", message);
}

pub fn log_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("LOCALAPPDATA") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("eiviz");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let mac = PathBuf::from(&home)
            .join("Library")
            .join("Logs")
            .join("eiviz");
        if cfg!(target_os = "macos") || mac.parent().is_some_and(|p| p.exists()) {
            return mac;
        }
        return PathBuf::from(home).join(".eiviz");
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn join_timeout(handle: JoinHandle<()>, timeout: Duration, name: &str) {
    let (tx, rx) = mpsc::channel();
    let join_name = format!("eiviz-join-{name}");
    if thread::Builder::new()
        .name(join_name)
        .spawn(move || {
            let _ = handle.join();
            let _ = tx.send(());
        })
        .is_err()
    {
        warn(&format!("{name} join helper failed to spawn"));
        return;
    }
    if rx.recv_timeout(timeout).is_err() {
        warn(&format!(
            "{name} join timed out after {}ms",
            timeout.as_millis()
        ));
    }
}

pub fn lock_held<T>(name: &str, start: Instant, result: T) -> T {
    let ms = start.elapsed().as_millis();
    if ms >= SLOW_LOCK_MS {
        warn(&format!("{name} held {ms}ms"));
    }
    result
}

fn write(level: &str, message: &str) {
    let line = format!("{} {level} {message}\n", timestamp());
    eprint!("{line}");
    let Ok(mut slot) = LOG.lock() else {
        return;
    };
    if slot.is_none() {
        *slot = open_log();
    }
    if let Some(file) = slot.as_mut() {
        let _ = file.write_all(line.as_bytes());
        let _ = file.flush();
    }
}

fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn open_log() -> Option<std::fs::File> {
    let dir = log_dir();
    fs::create_dir_all(&dir).ok()?;
    let path = dir.join("eiviz-mixer.log");
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() >= MAX_LOG_BYTES {
            let old = dir.join("eiviz-mixer.log.old");
            let _ = fs::remove_file(&old);
            let _ = fs::rename(&path, old);
        }
    }
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("panic");
        let location = info
            .location()
            .map(|loc| format!("{}:{}", loc.file(), loc.line()))
            .unwrap_or_else(|| "unknown".into());
        write("ERROR", &format!("panic at {location}: {payload}"));
        write_crash(&format!("panic at {location}: {payload}"));
        previous(info);
    }));
}

fn write_crash(message: &str) {
    let dir = log_dir();
    let _ = fs::create_dir_all(&dir);
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("eiviz-crash.log"))
    {
        let _ = writeln!(file, "{} ERROR {message}", timestamp());
    }
}

#[cfg(windows)]
fn install_unhandled_exception_filter() {
    unsafe extern "system" {
        fn SetUnhandledExceptionFilter(
            handler: Option<unsafe extern "system" fn(*mut core::ffi::c_void) -> i32>,
        ) -> Option<unsafe extern "system" fn(*mut core::ffi::c_void) -> i32>;
    }
    unsafe {
        SetUnhandledExceptionFilter(Some(on_unhandled_exception));
    }
}

#[cfg(windows)]
unsafe extern "system" fn on_unhandled_exception(_info: *mut core::ffi::c_void) -> i32 {
    write_crash("native unhandled exception");
    0
}

pub static GPU_FAULT: AtomicBool = AtomicBool::new(false);
static SURFACE_LOST: AtomicU64 = AtomicU64::new(0);
static FATAL: AtomicBool = AtomicBool::new(false);
static FATAL_TAKEN: AtomicBool = AtomicBool::new(false);
static FATAL_MSG: Mutex<String> = Mutex::new(String::new());

pub fn mark_gpu_fault(message: &str) {
    GPU_FAULT.store(true, Ordering::Release);
    error(message);
}

pub fn take_gpu_fault() -> bool {
    GPU_FAULT.swap(false, Ordering::AcqRel)
}

pub fn note_surface_lost() {
    SURFACE_LOST.fetch_add(1, Ordering::Relaxed);
}

pub fn surface_lost() -> u64 {
    SURFACE_LOST.load(Ordering::Relaxed)
}

pub fn mark_fatal(message: impl Into<String>) {
    let message = message.into();
    error(&message);
    write_crash(&message);
    if let Ok(mut slot) = FATAL_MSG.lock() {
        if slot.is_empty() {
            *slot = message;
        }
    }
    FATAL.store(true, Ordering::Release);
}

pub fn is_fatal() -> bool {
    FATAL.load(Ordering::Acquire)
}

/// First caller receives the message. Later callers get `None` so the host
/// shows the fatal dialog only once. The fatal flag itself stays set.
pub fn take_fatal() -> Option<String> {
    if !FATAL.load(Ordering::Acquire) {
        return None;
    }
    if FATAL_TAKEN.swap(true, Ordering::AcqRel) {
        return None;
    }
    FATAL_MSG
        .lock()
        .ok()
        .map(|slot| slot.clone())
        .filter(|msg| !msg.is_empty())
}
