//! Panic and lock-poison containment for the C ABI.
//!
//! A panic that unwinds out of an `extern "C"` function aborts the whole host
//! process, and a poisoned `Mutex` would turn every later call into such a
//! panic. Every exported function therefore runs inside [`ffi_guard`], and
//! shared state is locked through [`LockExt`] so one failed frame cannot
//! disable the rest of the mixer.

use std::any::Any;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Mutex, MutexGuard, PoisonError};

pub(crate) trait LockExt<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

/// Runs an exported function body, converting a panic into `on_panic` and a
/// recorded error message instead of unwinding across the C ABI.
pub(crate) fn ffi_guard<R>(name: &'static str, on_panic: R, f: impl FnOnce() -> R) -> R {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(payload) => {
            let message = format!("{name} panicked: {}", panic_message(payload.as_ref()));
            crate::diag::error(&message);
            let _ = panic::catch_unwind(AssertUnwindSafe(|| {
                crate::report_session_error(message);
            }));
            on_panic
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_returns_fallback_on_panic() {
        let value = ffi_guard("test", 7_i32, || -> i32 { panic!("boom") });
        assert_eq!(value, 7);
    }

    #[test]
    fn guard_passes_value_through() {
        assert_eq!(ffi_guard("test", 7_i32, || 3), 3);
    }

    #[test]
    fn mixer_slot_survives_poison() {
        let _ = std::thread::spawn(|| {
            let _guard = crate::lifecycle::mixer_slot().lock().unwrap();
            panic!("poison the slot");
        })
        .join();
        let _ = crate::lifecycle::with_mixer(|_| ());
        let _ = crate::lifecycle::with_mixer(|_| ());
    }

    #[test]
    fn poisoned_mutex_is_recovered() {
        let lock = std::sync::Arc::new(Mutex::new(1_u32));
        let clone = std::sync::Arc::clone(&lock);
        let _ = std::thread::spawn(move || {
            let _guard = clone.lock().unwrap();
            panic!("poison");
        })
        .join();
        assert!(lock.is_poisoned());
        assert_eq!(*lock.lock_or_recover(), 1);
    }
}
