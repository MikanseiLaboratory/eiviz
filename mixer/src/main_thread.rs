/// Runs `f` on the AppKit main thread.
///
/// If the caller is already on the main thread this executes inline, so it is
/// safe from `mixer_unit_attach_native` while that call blocks waiting for the
/// render thread. `dispatch_sync` onto main from main would deadlock.
pub fn run_on_main<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    dispatch2::run_on_main(|_| f())
}
