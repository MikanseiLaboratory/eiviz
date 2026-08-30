/// Raw pointer that may hop to the AppKit main thread.
///
/// `run_on_main` either runs inline or blocks the caller until the closure
/// returns, so the pointee is not used concurrently.
pub struct SendPtr<T>(*mut T);

impl<T> SendPtr<T> {
    pub fn from_const(ptr: *const T) -> Self {
        Self(ptr as *mut T)
    }

    pub fn from_mut(ptr: *mut T) -> Self {
        Self(ptr)
    }

    pub unsafe fn as_ref(&self) -> &T {
        unsafe { &*self.0 }
    }

    pub unsafe fn as_mut(&self) -> &mut T {
        unsafe { &mut *self.0 }
    }
}

unsafe impl<T> Send for SendPtr<T> {}

/// Runs `f` on the AppKit main thread.
///
/// If the caller is already on the main thread this executes inline, so it is
/// safe from `mixer_unit_attach_native` while that call blocks waiting for the
/// render thread. `dispatch_sync` onto main from main would deadlock.
pub fn run_on_main<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    dispatch2::run_on_main(|_| f())
}
