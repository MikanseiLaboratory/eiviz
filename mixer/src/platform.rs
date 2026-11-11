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

/// What the platform actually uses for each media stage. Logged once at startup so a
/// performance report does not have to guess which path a build selected.
pub fn accel_profile() -> &'static str {
    #[cfg(windows)]
    {
        "decode=media-foundation-dxva upload=rebar/vulkan ingest=rebar/vulkan export=media-foundation live=openh264 replay=vmx1-cpu decklink=yes"
    }
    #[cfg(target_os = "macos")]
    {
        "decode=videotoolbox-nv12-bgra-zero-copy upload=uma ingest=uma export=videotoolbox live=openh264 replay=vmx1-cpu decklink=no"
    }
    #[cfg(target_os = "linux")]
    {
        "decode=vulkan-video upload=vulkan/staging ingest=vulkan export=vmx1 live=openh264 replay=vmx1-cpu decklink=yes"
    }
}
