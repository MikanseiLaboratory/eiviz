//! Blackmagic DeckLink adapter boundary.
//!
//! `decklink-sdk` compiles a C ABI shim against an explicitly installed
//! Desktop Video SDK 16. No simulator, generated frames, or alternate backend
//! exists in this crate.

use eiviz_media::Capability;
#[cfg(any(feature = "decklink-sdk", test))]
use eiviz_time::{MediaTime, Rational};
#[cfg(any(feature = "decklink-sdk", test))]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "decklink-sdk")]
mod native;

#[cfg(feature = "decklink-sdk")]
pub use native::{
    DeckLinkConfig, DeckLinkError, DeckLinkPlaybackDiagnostics, DeckLinkSink, DeckLinkSource,
    enumerate_devices, probe,
};

#[cfg(not(feature = "decklink-sdk"))]
pub fn probe() -> Capability {
    Capability {
        id: "decklink".into(),
        available: false,
        detail: "not compiled; install Desktop Video SDK 16 and enable the `decklink-sdk` feature"
            .into(),
    }
}

pub const SDK_ABI_VERSION: u32 = 1;
pub const VIDEO_TIME_SCALE: i64 = 60_000;
pub const VIDEO_FRAME_DURATION: i64 = 1_001;
pub const AUDIO_TIME_SCALE: i64 = 48_000;
pub const VIDEO_WIDTH: u32 = 1_920;
pub const VIDEO_HEIGHT: u32 = 1_080;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceDirection {
    Capture,
    Playback,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub persistent_id: String,
    pub display_name: String,
    pub supports_capture: bool,
    pub supports_playback: bool,
}

impl DeviceInfo {
    pub fn supports(&self, direction: DeviceDirection) -> bool {
        match direction {
            DeviceDirection::Capture => self.supports_capture,
            DeviceDirection::Playback => self.supports_playback,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BindingResolutionError {
    #[error("binding kind must be `decklink`, got `{0}`")]
    WrongKind(String),
    #[error("bound DeckLink hardware `{0}` is not present")]
    HardwareMissing(String),
    #[error("DeckLink hardware `{0}` does not support {1:?}")]
    UnsupportedDirection(String, DeviceDirection),
    #[error("no DeckLink named `{0}` supports {1:?}")]
    LogicalNameMissing(String, DeviceDirection),
    #[error("DeckLink logical name `{0}` is ambiguous for {1:?}")]
    AmbiguousLogicalName(String, DeviceDirection),
}

/// Resolves a project binding without silently switching to another physical card.
///
/// A remembered persistent ID is authoritative. Logical-name matching is used
/// only for a new binding and must produce exactly one direction-capable device.
pub fn resolve_binding<'a>(
    binding: &eiviz_core::DeviceBinding,
    direction: DeviceDirection,
    devices: &'a [DeviceInfo],
) -> Result<&'a DeviceInfo, BindingResolutionError> {
    if binding.kind != "decklink" {
        return Err(BindingResolutionError::WrongKind(binding.kind.clone()));
    }
    if let Some(id) = &binding.last_seen_hardware_id {
        let device = devices
            .iter()
            .find(|device| device.persistent_id == *id)
            .ok_or_else(|| BindingResolutionError::HardwareMissing(id.clone()))?;
        if !device.supports(direction) {
            return Err(BindingResolutionError::UnsupportedDirection(
                id.clone(),
                direction,
            ));
        }
        return Ok(device);
    }
    let mut matches = devices
        .iter()
        .filter(|device| device.display_name == binding.logical_name && device.supports(direction));
    let first = matches.next().ok_or_else(|| {
        BindingResolutionError::LogicalNameMissing(binding.logical_name.clone(), direction)
    })?;
    if matches.next().is_some() {
        return Err(BindingResolutionError::AmbiguousLogicalName(
            binding.logical_name.clone(),
            direction,
        ));
    }
    Ok(first)
}

pub const fn schedule_timescale() -> (u32, u32) {
    (VIDEO_FRAME_DURATION as u32, VIDEO_TIME_SCALE as u32)
}

#[cfg(any(feature = "decklink-sdk", test))]
fn decklink_ticks_to_media_time(ticks: i64, time_scale: i64) -> Option<MediaTime> {
    if time_scale <= 0 {
        return None;
    }
    Some(MediaTime::new(ticks, Rational::new(1, time_scale).ok()?))
}

#[cfg(any(feature = "decklink-sdk", test))]
fn media_time_to_decklink_ticks(time: MediaTime, time_scale: i64) -> Option<i64> {
    if time_scale <= 0 {
        return None;
    }
    let ticks = (time.ticks() as i128)
        .checked_mul(time.timebase().numerator() as i128)?
        .checked_mul(time_scale as i128)?
        / time.timebase().denominator() as i128;
    i64::try_from(ticks).ok()
}

#[cfg(any(feature = "decklink-sdk", test))]
fn decklink_audio_time_to_sample_index(ticks: i64, time_scale: i64, sample_rate: u32) -> u64 {
    if ticks <= 0 || time_scale <= 0 || sample_rate == 0 {
        return 0;
    }
    let samples = ticks as u128 * sample_rate as u128 / time_scale as u128;
    samples.min(u64::MAX as u128) as u64
}

#[cfg(any(feature = "decklink-sdk", test))]
fn sample_index_to_decklink_ticks(
    sample_index: u64,
    sample_rate: u32,
    time_scale: i64,
) -> Option<i64> {
    if sample_rate == 0 || time_scale <= 0 {
        return None;
    }
    let ticks = sample_index as u128 * time_scale as u128 / sample_rate as u128;
    i64::try_from(ticks).ok()
}

#[cfg(any(feature = "decklink-sdk", test))]
fn push_latest<T>(
    tx: &crossbeam_channel::Sender<T>,
    drop_rx: &crossbeam_channel::Receiver<T>,
    value: T,
    dropped: &AtomicU64,
) {
    match tx.try_send(value) {
        Ok(()) => {}
        Err(crossbeam_channel::TrySendError::Full(value)) => {
            let _ = drop_rx.try_recv();
            if tx.try_send(value).is_ok() {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        Err(crossbeam_channel::TrySendError::Disconnected(_)) => {}
    }
}

#[cfg(any(feature = "decklink-sdk", test))]
mod ffi {
    use std::ffi::c_char;
    #[cfg(feature = "decklink-sdk")]
    use std::ffi::c_void;

    #[cfg(feature = "decklink-sdk")]
    pub const DEVICE_CAPTURE: u32 = 1;
    #[cfg(feature = "decklink-sdk")]
    pub const DEVICE_PLAYBACK: u32 = 2;
    #[cfg(feature = "decklink-sdk")]
    pub const FRAME_NO_INPUT: u32 = 1;

    #[repr(C)]
    pub struct Device {
        pub persistent_id: *const c_char,
        pub display_name: *const c_char,
        pub capabilities: u32,
    }

    #[repr(C)]
    pub struct VideoFrame {
        pub data: *const u8,
        pub data_len: usize,
        pub width: u32,
        pub height: u32,
        pub row_bytes: u32,
        pub flags: u32,
        pub stream_time: i64,
        pub duration: i64,
        pub time_scale: i64,
    }

    #[repr(C)]
    pub struct AudioPacket {
        pub samples: *const i16,
        pub sample_count: usize,
        pub frame_count: u32,
        pub channels: u32,
        pub sample_rate: u32,
        pub packet_time: i64,
        pub time_scale: i64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct PlaybackDiagnostics {
        pub scheduled_video: u64,
        pub completed_video: u64,
        pub late_video: u64,
        pub dropped_video: u64,
        pub flushed_video: u64,
        pub buffered_video: u32,
        pub buffered_audio_frames: u32,
        pub reference_locked: i32,
    }

    #[cfg(feature = "decklink-sdk")]
    unsafe extern "C" {
        pub fn eiviz_decklink_abi_version() -> u32;
        pub fn eiviz_decklink_enumerate(
            callback: unsafe extern "C" fn(*mut c_void, *const Device),
            context: *mut c_void,
            error: *mut c_char,
            error_capacity: usize,
        ) -> i32;
        pub fn eiviz_decklink_capture_open(
            persistent_id: *const c_char,
            audio_channels: u32,
            video_callback: unsafe extern "C" fn(*mut c_void, *const VideoFrame),
            audio_callback: unsafe extern "C" fn(*mut c_void, *const AudioPacket),
            context: *mut c_void,
            capture: *mut *mut c_void,
            error: *mut c_char,
            error_capacity: usize,
        ) -> i32;
        pub fn eiviz_decklink_capture_close(capture: *mut c_void);
        pub fn eiviz_decklink_playback_open(
            persistent_id: *const c_char,
            audio_channels: u32,
            playback: *mut *mut c_void,
            error: *mut c_char,
            error_capacity: usize,
        ) -> i32;
        pub fn eiviz_decklink_playback_schedule_video(
            playback: *mut c_void,
            bgra: *const u8,
            data_len: usize,
            row_bytes: u32,
            display_time: i64,
            duration: i64,
            time_scale: i64,
            error: *mut c_char,
            error_capacity: usize,
        ) -> i32;
        pub fn eiviz_decklink_playback_schedule_audio(
            playback: *mut c_void,
            samples: *const i16,
            frame_count: u32,
            stream_time: i64,
            time_scale: i64,
            error: *mut c_char,
            error_capacity: usize,
        ) -> i32;
        pub fn eiviz_decklink_playback_start(
            playback: *mut c_void,
            start_time: i64,
            time_scale: i64,
            error: *mut c_char,
            error_capacity: usize,
        ) -> i32;
        pub fn eiviz_decklink_playback_get_diagnostics(
            playback: *mut c_void,
            diagnostics: *mut PlaybackDiagnostics,
            error: *mut c_char,
            error_capacity: usize,
        ) -> i32;
        pub fn eiviz_decklink_playback_close(playback: *mut c_void);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_time::{MediaTime, NTSC_5994};

    #[test]
    fn ntsc_schedule_is_rational() {
        assert_eq!(super::schedule_timescale(), (1001, 60000));
    }

    #[test]
    fn ffi_struct_layout_matches_64_bit_c_abi() {
        assert_eq!(std::mem::size_of::<ffi::Device>(), 24);
        assert_eq!(std::mem::size_of::<ffi::VideoFrame>(), 56);
        assert_eq!(std::mem::size_of::<ffi::AudioPacket>(), 48);
        assert_eq!(std::mem::size_of::<ffi::PlaybackDiagnostics>(), 56);
        assert_eq!(std::mem::align_of::<ffi::VideoFrame>(), 8);
    }

    #[test]
    fn millionth_frame_converts_without_accumulated_rounding() {
        let time = MediaTime::from_frame_index(1_000_000, NTSC_5994).unwrap();
        assert_eq!(
            media_time_to_decklink_ticks(time, VIDEO_TIME_SCALE),
            Some(1_000_000 * VIDEO_FRAME_DURATION)
        );
        let converted =
            decklink_ticks_to_media_time(1_000_000 * VIDEO_FRAME_DURATION, VIDEO_TIME_SCALE)
                .unwrap();
        assert_eq!(converted.frame_index(NTSC_5994).unwrap(), 1_000_000);
    }

    #[test]
    fn audio_timestamps_use_absolute_sample_index() {
        assert_eq!(
            sample_index_to_decklink_ticks(48_000, 48_000, AUDIO_TIME_SCALE),
            Some(48_000)
        );
        assert_eq!(
            decklink_audio_time_to_sample_index(48_000, AUDIO_TIME_SCALE, 48_000),
            48_000
        );
        assert_eq!(
            decklink_audio_time_to_sample_index(-1, AUDIO_TIME_SCALE, 48_000),
            0
        );
    }

    #[test]
    fn bounded_capture_queue_keeps_latest_frame() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let dropped = AtomicU64::new(0);
        push_latest(&tx, &rx, 1, &dropped);
        push_latest(&tx, &rx, 2, &dropped);
        assert_eq!(rx.try_recv().unwrap(), 2);
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn default_profile_never_claims_runtime_availability() {
        #[cfg(not(feature = "decklink-sdk"))]
        {
            let capability = probe();
            assert!(!capability.available);
            assert!(capability.detail.contains("not compiled"));
        }
    }

    #[test]
    fn remembered_hardware_id_never_falls_back_to_same_named_device() {
        let binding = eiviz_core::DeviceBinding {
            id: eiviz_core::DeviceBindingId::new(),
            kind: "decklink".into(),
            logical_name: "DeckLink 8K Pro".into(),
            last_seen_hardware_id: Some("persistent:old".into()),
        };
        let devices = vec![DeviceInfo {
            persistent_id: "persistent:new".into(),
            display_name: "DeckLink 8K Pro".into(),
            supports_capture: true,
            supports_playback: true,
        }];
        assert_eq!(
            resolve_binding(&binding, DeviceDirection::Capture, &devices),
            Err(BindingResolutionError::HardwareMissing(
                "persistent:old".into()
            ))
        );
    }

    #[test]
    fn new_logical_binding_must_be_unambiguous() {
        let binding = eiviz_core::DeviceBinding {
            id: eiviz_core::DeviceBindingId::new(),
            kind: "decklink".into(),
            logical_name: "DeckLink Duo 2".into(),
            last_seen_hardware_id: None,
        };
        let device = DeviceInfo {
            persistent_id: "persistent:a".into(),
            display_name: binding.logical_name.clone(),
            supports_capture: true,
            supports_playback: false,
        };
        assert!(matches!(
            resolve_binding(
                &binding,
                DeviceDirection::Capture,
                &[
                    device.clone(),
                    DeviceInfo {
                        persistent_id: "persistent:b".into(),
                        ..device
                    }
                ]
            ),
            Err(BindingResolutionError::AmbiguousLogicalName(_, _))
        ));
    }
}
