//! Small OS services that are not part of the compositor abstraction.

/// Puts the calling thread on a QoS class that keeps it off the efficiency cores
/// when the machine is busy. macOS only; other platforms have no equivalent here.
#[cfg(target_os = "macos")]
pub fn set_thread_qos(qos: u32) {
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
    }
    let _ = unsafe { pthread_set_qos_class_self_np(qos, 0) };
}

pub const QOS_USER_INTERACTIVE: u32 = 0x21;
pub const QOS_USER_INITIATED: u32 = 0x19;
pub const QOS_UTILITY: u32 = 0x11;

#[cfg(not(target_os = "macos"))]
pub fn set_thread_qos(_qos: u32) {}

/// Gives the audio scheduler a Mach time-constraint policy. QoS alone still lets a
/// busy render/upload thread delay the block by tens of milliseconds, and the
/// scheduler then has to skip slots (audible gaps). The policy asks the kernel to
/// run this thread for `computation` every `period`.
#[cfg(target_os = "macos")]
pub fn set_thread_time_constraint(period_ms: u32, computation_ms: u32) {
    unsafe extern "C" {
        fn mach_thread_self() -> u32;
        fn mach_timebase_info(info: *mut u32) -> i32;
        fn thread_policy_set(thread: u32, flavor: u32, policy: *const u32, count: u32) -> i32;
    }
    const THREAD_TIME_CONSTRAINT_POLICY: u32 = 2;
    let mut timebase = [0u32, 0u32];
    if unsafe { mach_timebase_info(timebase.as_mut_ptr()) } != 0 || timebase[0] == 0 {
        return;
    }
    let ticks = |ms: u32| -> u32 {
        let ns = u64::from(ms) * 1_000_000;
        ((u128::from(ns) * u128::from(timebase[1]) / u128::from(timebase[0])) as u64)
            .min(u64::from(u32::MAX)) as u32
    };
    let policy = [
        ticks(period_ms),
        ticks(computation_ms),
        ticks(period_ms),
        1, // preemptible
    ];
    let status = unsafe {
        thread_policy_set(
            mach_thread_self(),
            THREAD_TIME_CONSTRAINT_POLICY,
            policy.as_ptr(),
            4,
        )
    };
    if status != 0 {
        crate::diag::warn(&format!("audio time-constraint policy refused: {status}"));
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_thread_time_constraint(_period_ms: u32, _computation_ms: u32) {}

/// What the platform actually uses for each media stage. Logged once at startup so a
/// performance report does not have to guess which path a build selected.
pub fn accel_profile() -> &'static str {
    #[cfg(windows)]
    {
        "decode=media-foundation-dxva upload=rebar/vulkan ingest=rebar/vulkan export=media-foundation live=openh264 replay=vmx1-cpu decklink=yes"
    }
    #[cfg(target_os = "macos")]
    {
        "decode=videotoolbox-nv12-bgra-zero-copy upload=uma ingest=uma export=videotoolbox live=videotoolbox-h264 replay=vmx1-cpu decklink=no"
    }
    #[cfg(target_os = "linux")]
    {
        "decode=vulkan-video upload=vulkan/staging ingest=vulkan export=vmx1 live=openh264 replay=vmx1-cpu decklink=yes"
    }
}
