//! Versioned C ABI between the public mixer and a Pro module.
//!
//! Rust traits, `String`, `Vec`, `Arc`, and `Result` stay on each side of the
//! boundary. The only shared contract is the `#[repr(C)]` table returned by
//! `eiviz_pro_get_api`.
//!
//! # Ownership
//!
//! The module creates opaque handles. The caller destroys them. `module_shutdown`
//! returns [`PRO_BUSY`] while any capture or output handle is still alive.
//! Frame and audio pointers are valid only for the duration of the call that
//! receives them. Capture callbacks are serial and non-reentrant; their
//! pointers are valid only until the callback returns, and the mixer copies
//! before returning.
//!
//! # Failure isolation
//!
//! Both sides catch panics at the boundary and turn them into a status code.
//! An access violation is a process failure and is not isolated.

use std::os::raw::{c_char, c_void};

use crate::plan::{Entitlements, Plan, Quota, VideoLimit};

/// First published table. A different major is rejected.
pub const ABI_MAJOR: u32 = 1;

pub const PRO_OK: u32 = 0;
pub const PRO_INVALID: u32 = 1;
pub const PRO_IO: u32 = 2;
pub const PRO_DEVICE: u32 = 3;
pub const PRO_UNAVAILABLE: u32 = 4;
pub const PRO_BUSY: u32 = 5;
pub const PRO_NOT_SUPPORTED: u32 = 6;

pub const FEATURE_DECKLINK: u32 = 1 << 0;
pub const FEATURE_RTMP: u32 = 1 << 1;
pub const FEATURE_LICENSE: u32 = 1 << 2;
pub const FEATURE_REPLAY: u32 = 1 << 3;

pub const LAYOUT_UYVY: u32 = 0;
pub const LAYOUT_BGRA: u32 = 1;

pub const SIGNAL_PRESENT: u32 = 0;
pub const SIGNAL_NONE: u32 = 1;

/// Single symbol the mixer loads. Arguments are the host ABI major, the host
/// ABI hash, and `size_of::<ProApi>()`.
pub const GET_API_SYMBOL: &[u8] = b"eiviz_pro_get_api\0";

pub type GetApiFn =
    unsafe extern "C" fn(abi_major: u32, abi_hash: u64, struct_size: u32) -> *const ProApi;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProStatus {
    pub code: u32,
}

impl ProStatus {
    pub const fn new(code: u32) -> Self {
        Self { code }
    }

    pub const fn ok() -> Self {
        Self { code: PRO_OK }
    }

    pub fn is_ok(self) -> bool {
        self.code == PRO_OK
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProBytes {
    pub ptr: *const u8,
    pub len: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProEntitlementsAbi {
    pub struct_size: u32,
    pub plan: u32,
    pub mixing_units: u32,
    pub decklink_inputs: u32,
    pub decklink_outputs: u32,
    pub rtmp_max_width: u32,
    pub rtmp_max_height: u32,
    pub rtmp_max_fps_num: u32,
    pub rtmp_max_fps_den: u32,
    pub recording: u32,
    pub srt: u32,
    pub hardware_encode: u32,
    pub replay_angles: u32,
    pub replay_channels: u32,
    pub features: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProVideoView {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub layout: u32,
    pub pts_100ns: i64,
    pub fps_num: u32,
    pub fps_den: u32,
    pub data: *const u8,
    pub data_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProAudioView {
    pub sample_rate: u32,
    pub channels: u32,
    pub frames: u32,
    pub pts_100ns: i64,
    pub planar: *const f32,
    pub sample_count: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProCaptureCallbacks {
    pub struct_size: u32,
    pub user_data: *mut c_void,
    pub video: Option<unsafe extern "C" fn(*mut c_void, *const ProVideoView)>,
    pub audio: Option<unsafe extern "C" fn(*mut c_void, *const ProAudioView)>,
    pub signal: Option<unsafe extern "C" fn(*mut c_void, u32)>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProPlayoutConfig {
    pub struct_size: u32,
    pub device: *const u8,
    pub device_len: usize,
    pub mode: *const u8,
    pub mode_len: usize,
    pub external_key: u32,
    pub fps_num: u32,
    pub fps_den: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProStreamConfig {
    pub struct_size: u32,
    pub video_bitrate: u32,
    pub audio_bitrate: u32,
    pub keyint: u32,
    pub video_only: u32,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    /// `0` means automatic.
    pub encoder_threads: u32,
    /// `0` sends immediately. Otherwise hold this many milliseconds, then release at 1x.
    pub buffer_ms: u32,
    /// `0` is OpenH264. Any other value is rejected.
    pub encoder: u32,
    /// `0` low, `1` medium, `2` high.
    pub complexity: u32,
    pub ref_frames: u32,
    pub cabac: u32,
    pub deblock: u32,
    pub background_detection: u32,
    pub adaptive_quant: u32,
    pub scene_change: u32,
    pub audio_delay_ms: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProLicenseStatusAbi {
    pub struct_size: u32,
    pub state: u32,
    pub plan: u32,
    pub expires_at: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProCaptureStatsAbi {
    pub struct_size: u32,
    pub frames: u64,
    pub no_signal: u32,
    pub error_len: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProOutputStatsAbi {
    pub struct_size: u32,
    pub connected: u32,
    pub video_frames: u64,
    pub repeated: u64,
    pub dropped: u64,
    pub bitrate: u32,
    pub encoder_threads: u32,
    pub media_lag_ms: u32,
    pub sent_fps_milli: u32,
}

/// Function table. `struct_size` is the plugin's view of this struct.
/// `abi_hash` must equal [`abi_hash`].
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ProApi {
    pub struct_size: u32,
    pub abi_major: u32,
    pub abi_hash: u64,
    pub create: Option<unsafe extern "C" fn() -> *mut c_void>,
    pub shutdown: Option<unsafe extern "C" fn(*mut c_void) -> ProStatus>,
    pub destroy: Option<unsafe extern "C" fn(*mut c_void)>,
    pub copy_name:
        Option<unsafe extern "C" fn(*mut c_void, *mut u8, usize, *mut usize) -> ProStatus>,
    pub copy_version:
        Option<unsafe extern "C" fn(*mut c_void, *mut u8, usize, *mut usize) -> ProStatus>,
    pub query_entitlements:
        Option<unsafe extern "C" fn(*mut c_void, *mut ProEntitlementsAbi) -> ProStatus>,
    pub copy_last_error:
        Option<unsafe extern "C" fn(*mut c_void, *mut u8, usize, *mut usize) -> ProStatus>,
    pub decklink_enum_devices:
        Option<unsafe extern "C" fn(*mut c_void, *mut u8, usize, *mut usize) -> ProStatus>,
    pub decklink_enum_modes: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *const u8,
            usize,
            *mut u8,
            usize,
            *mut usize,
        ) -> ProStatus,
    >,
    pub capture_start: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *const u8,
            usize,
            *const u8,
            usize,
            ProCaptureCallbacks,
            *mut *mut c_void,
        ) -> ProStatus,
    >,
    pub capture_stats: Option<
        unsafe extern "C" fn(*mut c_void, *mut ProCaptureStatsAbi, *mut u8, usize) -> ProStatus,
    >,
    pub capture_destroy: Option<unsafe extern "C" fn(*mut c_void) -> ProStatus>,
    pub playout_start: Option<
        unsafe extern "C" fn(*mut c_void, *const ProPlayoutConfig, *mut *mut c_void) -> ProStatus,
    >,
    pub stream_start: Option<
        unsafe extern "C" fn(*mut c_void, *const ProStreamConfig, *mut *mut c_void) -> ProStatus,
    >,
    pub stream_add_destination:
        Option<unsafe extern "C" fn(*mut c_void, u64, *const u8, usize) -> ProStatus>,
    pub stream_remove_destination: Option<unsafe extern "C" fn(*mut c_void, u64) -> ProStatus>,
    pub stream_destination_stats: Option<
        unsafe extern "C" fn(*mut c_void, u64, *mut ProOutputStatsAbi, *mut u8, usize) -> ProStatus,
    >,
    pub output_submit_video:
        Option<unsafe extern "C" fn(*mut c_void, *const ProVideoView) -> ProStatus>,
    pub output_submit_audio:
        Option<unsafe extern "C" fn(*mut c_void, *const ProAudioView) -> ProStatus>,
    pub output_stats: Option<
        unsafe extern "C" fn(*mut c_void, *mut ProOutputStatsAbi, *mut u8, usize) -> ProStatus,
    >,
    pub output_destroy: Option<unsafe extern "C" fn(*mut c_void) -> ProStatus>,
    pub license_install: Option<unsafe extern "C" fn(*mut c_void, *const c_char) -> ProStatus>,
    pub license_status: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *mut ProLicenseStatusAbi,
            *mut u8,
            usize,
            *mut usize,
        ) -> ProStatus,
    >,
    pub license_clear: Option<unsafe extern "C" fn(*mut c_void) -> ProStatus>,
    pub license_fingerprint:
        Option<unsafe extern "C" fn(*mut c_void, *mut u8, usize, *mut usize) -> ProStatus>,
    pub license_email:
        Option<unsafe extern "C" fn(*mut c_void, *mut u8, usize, *mut usize) -> ProStatus>,
    pub replay_open: Option<
        unsafe extern "C" fn(*mut c_void, *const ProReplayConfig, *mut *mut c_void) -> ProStatus,
    >,
    pub replay_close: Option<unsafe extern "C" fn(*mut c_void) -> ProStatus>,
    pub replay_write_video: Option<
        unsafe extern "C" fn(*mut c_void, u32, u64, *const ProVideoView) -> ProStatus,
    >,
    pub replay_decode: Option<
        unsafe extern "C" fn(
            *mut c_void,
            u32,
            u64,
            *mut u8,
            usize,
            *mut ProReplayDecoded,
        ) -> ProStatus,
    >,
    pub replay_range: Option<
        unsafe extern "C" fn(*mut c_void, u32, *mut u64, *mut u64) -> ProStatus,
    >,
    /// Added after the first layout. A module that does not export it has no
    /// replay audio, and a caller that passes a smaller `struct_size` never set
    /// it.
    pub replay_write_audio: Option<
        unsafe extern "C" fn(*mut c_void, u32, u64, *const ProReplayAudioView) -> ProStatus,
    >,
    pub replay_read_audio: Option<
        unsafe extern "C" fn(*mut c_void, u32, u64, usize, *mut f32, *mut u32) -> ProStatus,
    >,
    pub replay_copy_peaks: Option<
        unsafe extern "C" fn(
            *mut c_void,
            u32,
            u32,
            *mut f32,
            *mut f32,
            u32,
            *mut u32,
            *mut u32,
        ) -> ProStatus,
    >,
    pub replay_stats: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *mut ProReplayStatsAbi,
            *mut ProReplayAngleStatsAbi,
            u32,
            *mut u32,
        ) -> ProStatus,
    >,
    pub replay_estimate: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *const u8,
            usize,
            u32,
            u32,
            u32,
            u32,
            u32,
            u32,
            u64,
            *mut ProReplayEstimateAbi,
        ) -> ProStatus,
    >,
    /// Export a recording to MP4. The job runs on its own thread, so the host can
    /// show progress and cancel.
    pub replay_export_start: Option<
        unsafe extern "C" fn(*mut c_void, *const ProReplayExportRequest) -> ProStatus,
    >,
    pub replay_export_progress: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *mut ProReplayExportProgress,
            *mut u8,
            usize,
            *mut usize,
        ) -> ProStatus,
    >,
    pub replay_export_cancel: Option<unsafe extern "C" fn(*mut c_void) -> ProStatus>,
}

/// One run of the recording timeline that goes into the exported file.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProReplayExportSegment {
    pub angle_id: u32,
    /// Inclusive, matching how the session stores clip ranges.
    pub from_frame: u64,
    pub to_frame: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProReplayExportRequest {
    pub struct_size: u32,
    pub path: *const u8,
    pub path_len: usize,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub bitrate: u32,
    pub sample_rate: u32,
    pub channels: u32,
    /// Non-zero to include the recorded audio.
    pub with_audio: u32,
    pub segments: *const ProReplayExportSegment,
    pub segment_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ProReplayExportProgress {
    pub struct_size: u32,
    /// `EXPORT_*` from the module.
    pub state: u32,
    pub done: u64,
    pub total: u64,
    pub skipped: u64,
    pub bytes: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProReplayStorage {
    pub path: *const u8,
    pub path_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProReplayAngle {
    pub angle_id: u32,
    pub storage_index: u32,
}

/// Interleaved `f32` audio for one video frame of the recording timeline.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProReplayAudioView {
    pub struct_size: u32,
    pub sample_rate: u32,
    pub channels: u32,
    /// Frames per channel in `data`.
    pub frames: u32,
    pub data: *const f32,
    pub data_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProReplayConfig {
    pub struct_size: u32,
    pub recording_uuid: *const u8,
    pub recording_uuid_len: usize,
    pub storages: *const ProReplayStorage,
    pub storage_count: u32,
    pub angles: *const ProReplayAngle,
    pub angle_count: u32,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub profile: u32,
    pub segment_frames: u32,
    pub encode_queue_frames: u32,
    pub write_buffer_bytes: u64,
    pub overflow_policy: u32,
    pub reserve_bytes: u64,
    pub retention: u32,
    /// Appended after the original layout. A caller that passes a smaller
    /// `struct_size` cannot have set it, and the module must read zero instead.
    pub protect_from_frame: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ProReplayStatsAbi {
    pub struct_size: u32,
    pub state: u32,
    pub stop_reason: u32,
    pub issues: u32,
    pub first_frame: u64,
    pub last_frame: u64,
    pub free_bytes: u64,
    pub reserve_bytes: u64,
    pub write_rate_bps: u64,
    pub write_rate_instant_bps: u64,
    pub frames_written: u64,
    pub frames_dropped: u64,
    pub frames_dropped_recent: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ProReplayAngleStatsAbi {
    pub struct_size: u32,
    pub angle_id: u32,
    pub encode_ms_avg_milli: u32,
    pub encode_ms_max_milli: u32,
    pub encode_queue_len: u32,
    pub encode_queue_cap: u32,
    pub write_buffer_bytes: u64,
    pub write_buffer_cap: u64,
    pub write_latency_p95_ms: u32,
    pub frames_written: u64,
    pub dropped_encode: u64,
    pub dropped_write: u64,
    pub bytes_written: u64,
    pub segments: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ProReplayDecoded {
    pub struct_size: u32,
    pub actual_frame_index: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pts_100ns: i64,
    pub substituted: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ProReplayEstimateAbi {
    pub struct_size: u32,
    pub free_bytes: u64,
    pub estimate_rate_bps: u64,
    pub remaining_secs: u32,
    pub path_writable: u32,
}

/// FNV-1a over the sizes of every struct that crosses the boundary.
/// Mixer and plugin match only when they were built from the same API crate.
pub fn abi_hash() -> u64 {
    let sizes = [
        std::mem::size_of::<ProApi>(),
        std::mem::size_of::<ProStatus>(),
        std::mem::size_of::<ProBytes>(),
        std::mem::size_of::<ProEntitlementsAbi>(),
        std::mem::size_of::<ProVideoView>(),
        std::mem::size_of::<ProAudioView>(),
        std::mem::size_of::<ProCaptureCallbacks>(),
        std::mem::size_of::<ProPlayoutConfig>(),
        std::mem::size_of::<ProStreamConfig>(),
        std::mem::size_of::<ProLicenseStatusAbi>(),
        std::mem::size_of::<ProCaptureStatsAbi>(),
        std::mem::size_of::<ProOutputStatsAbi>(),
        std::mem::size_of::<ProReplayConfig>(),
        std::mem::size_of::<ProReplayStatsAbi>(),
        std::mem::size_of::<ProReplayAngleStatsAbi>(),
        std::mem::size_of::<ProReplayDecoded>(),
        std::mem::size_of::<ProReplayEstimateAbi>(),
        ABI_MAJOR as usize,
    ];
    let mut hash = 0xcbf29ce484222325u64;
    for size in sizes {
        hash ^= size as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// `arch-os`, for example `x86_64-windows` or `aarch64-macos`.
pub fn module_target() -> String {
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

pub fn copy_to_buffer(src: &[u8], dst: *mut u8, cap: usize, written: *mut usize) -> ProStatus {
    if dst.is_null() || written.is_null() {
        return ProStatus::new(PRO_INVALID);
    }
    if cap < src.len() {
        unsafe { *written = src.len() };
        return ProStatus::new(PRO_INVALID);
    }
    unsafe {
        if !src.is_empty() {
            std::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
        }
        *written = src.len();
    }
    ProStatus::ok()
}

fn quota_count<T: Copy + Into<u32>>(quota: Quota<T>) -> u32 {
    quota.abi_count()
}

impl Entitlements {
    pub fn to_abi(self, features: u32) -> ProEntitlementsAbi {
        let (width, height, fps_num, fps_den) = match self.rtmp {
            Quota::Denied => (0, 0, 0, 0),
            Quota::Unlimited => (u32::MAX, u32::MAX, u32::MAX, 1),
            Quota::Limited(limit) => (limit.width, limit.height, limit.fps_num, limit.fps_den),
        };
        ProEntitlementsAbi {
            struct_size: std::mem::size_of::<ProEntitlementsAbi>() as u32,
            plan: self.plan.abi(),
            mixing_units: quota_count(self.mixing_units),
            decklink_inputs: quota_count(self.decklink_inputs),
            decklink_outputs: quota_count(self.decklink_outputs),
            rtmp_max_width: width,
            rtmp_max_height: height,
            rtmp_max_fps_num: fps_num,
            rtmp_max_fps_den: fps_den,
            recording: u32::from(self.recording),
            srt: u32::from(self.srt),
            hardware_encode: u32::from(self.hardware_encode),
            replay_angles: quota_count(self.replay_angles),
            replay_channels: quota_count(self.replay_channels),
            features,
        }
    }

    pub fn from_abi(raw: ProEntitlementsAbi) -> Self {
        let plan = match raw.plan {
            1 => Plan::Professional,
            2 => Plan::Enterprise,
            _ => Plan::Community,
        };
        Self {
            plan,
            mixing_units: quota_from_count(raw.mixing_units),
            decklink_inputs: quota_from_count(raw.decklink_inputs),
            decklink_outputs: quota_from_count(raw.decklink_outputs),
            rtmp: rtmp_from_abi(raw),
            recording: raw.recording != 0,
            srt: raw.srt != 0,
            hardware_encode: raw.hardware_encode != 0,
            replay_angles: quota_from_count(raw.replay_angles),
            replay_channels: quota_from_count(raw.replay_channels),
        }
    }
}

fn quota_from_count(value: u32) -> Quota<u32> {
    match value {
        0 => Quota::Denied,
        u32::MAX => Quota::Unlimited,
        limited => Quota::Limited(limited),
    }
}

fn rtmp_from_abi(raw: ProEntitlementsAbi) -> Quota<VideoLimit> {
    if raw.rtmp_max_width == 0 || raw.rtmp_max_height == 0 || raw.rtmp_max_fps_den == 0 {
        return Quota::Denied;
    }
    if raw.rtmp_max_width == u32::MAX {
        return Quota::Unlimited;
    }
    Quota::Limited(VideoLimit::new(
        raw.rtmp_max_width,
        raw.rtmp_max_height,
        raw.rtmp_max_fps_num,
        raw.rtmp_max_fps_den,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_hash_is_stable_for_this_layout() {
        assert_ne!(abi_hash(), 0);
        assert_eq!(abi_hash(), abi_hash());
        assert!(std::mem::size_of::<ProApi>() > 64);
        assert_eq!(
            std::mem::size_of::<ProEntitlementsAbi>(),
            std::mem::align_of::<ProEntitlementsAbi>()
                * (std::mem::size_of::<ProEntitlementsAbi>()
                    / std::mem::align_of::<ProEntitlementsAbi>())
        );
    }

    #[test]
    fn entitlements_round_trip_keeps_sentinels() {
        let community = Entitlements::community().to_abi(0);
        let back = Entitlements::from_abi(community);
        assert_eq!(back, Entitlements::community());
        let enterprise = Entitlements::enterprise().to_abi(FEATURE_DECKLINK | FEATURE_RTMP);
        let back = Entitlements::from_abi(enterprise);
        assert_eq!(back.plan, Plan::Enterprise);
        assert_eq!(back.mixing_units, Quota::Unlimited);
        assert_eq!(enterprise.features, FEATURE_DECKLINK | FEATURE_RTMP);
    }
}


