//! Replay store boundary. The Pro module encodes on the GPU, stores, and decodes.
//! The mixer owns transport, health, and playout upload.

use std::sync::Arc;

use crate::error::{ProError, ProResult};
use crate::media::VideoFrame;

pub const PROFILE_HQ: u32 = 0;
pub const PROFILE_SQ: u32 = 1;
pub const PROFILE_LQ: u32 = 2;

/// A stored packet the mixer can decode on its own device.
pub const REPLAY_CODEC_VMX1: u32 = 0;
/// A bitstream this mixer did not decode: a sender's original frames, or frames the
/// mixer encoded on the GPU. Stored as they arrive.
pub const REPLAY_CODEC_PASSTHROUGH: u32 = 1;

pub const OVERFLOW_DROP: u32 = 0;
pub const OVERFLOW_STOP: u32 = 1;

pub const RETENTION_STOP: u32 = 0;
pub const RETENTION_OVERWRITE: u32 = 1;
pub const RETENTION_PROTECT: u32 = 2;

pub const STATE_IDLE: u32 = 0;
pub const STATE_RECORDING: u32 = 1;
pub const STATE_STOPPED: u32 = 2;

pub const STOP_NONE: u32 = 0;
pub const STOP_USER: u32 = 1;
pub const STOP_STORAGE_FULL: u32 = 2;
pub const STOP_BUFFER_FULL: u32 = 3;
pub const STOP_IO: u32 = 4;
pub const STOP_FPS: u32 = 5;

pub const ISSUE_CPU_ENCODE: u32 = 1 << 0;
pub const ISSUE_STORAGE_WRITE: u32 = 1 << 1;
pub const ISSUE_GPU_READBACK: u32 = 1 << 2;
pub const ISSUE_STORAGE_LOW: u32 = 1 << 3;
pub const ISSUE_STORAGE_FULL: u32 = 1 << 4;
pub const ISSUE_FRAMES_DROPPED: u32 = 1 << 5;
pub const ISSUE_STOPPED_BY_POLICY: u32 = 1 << 6;
pub const ISSUE_PRO_UNAVAILABLE: u32 = 1 << 7;

pub const SEVERITY_OK: u32 = 0;
pub const SEVERITY_WARNING: u32 = 1;
pub const SEVERITY_CRITICAL: u32 = 2;

pub fn profile_code(profile: u32) -> i32 {
    match profile {
        PROFILE_HQ => 99,
        PROFILE_LQ => 33,
        _ => 66,
    }
}

/// Megabits per second at 60 fps, mirroring the `VMX_BITRATE_TABLE` rows that
/// vmx-rs applies in `lookup_bitrate`. The table is copied here because vmx-rs
/// keeps `bitrate` private, so the codec crate exposes no way to read the
/// configured target rate. Keep it in sync with `vmx-rs/src/tables.rs`.
const BITRATE_TARGETS: [[i32; 3]; 36] = [
    // profile, minimum height, target Mbps at 60 fps
    [99, 4320, 1320],
    [199, 4320, 1200],
    [66, 4320, 660],
    [166, 4320, 600],
    [33, 4320, 440],
    [133, 4320, 400],
    [99, 2160, 800],
    [199, 2160, 600],
    [66, 2160, 400],
    [166, 2160, 300],
    [33, 2160, 266],
    [133, 2160, 200],
    [99, 1440, 504],
    [199, 1440, 450],
    [66, 1440, 252],
    [166, 1440, 300],
    [33, 1440, 168],
    [133, 1440, 120],
    [99, 1080, 260],
    [199, 1080, 260],
    [66, 1080, 130],
    [166, 1080, 200],
    [33, 1080, 86],
    [133, 1080, 86],
    [99, 720, 136],
    [199, 720, 136],
    [66, 720, 68],
    [166, 720, 68],
    [33, 720, 45],
    [133, 720, 45],
    [99, 0, 72],
    [199, 0, 72],
    [66, 0, 36],
    [166, 0, 36],
    [33, 0, 24],
    [133, 0, 24],
];

/// Target rate of the configured profile at `height`, in Mbps.
fn bitrate_target_mbps(profile: i32, height: i32) -> i32 {
    for row in BITRATE_TARGETS {
        if row[0] == profile && height >= row[1] {
            return row[2];
        }
    }
    72
}

/// Megabits per second at 60 fps for the replay profile code.
pub fn estimate_bitrate_mbps(profile: u32, height: u32) -> u32 {
    let target = bitrate_target_mbps(profile_code(profile), i32::try_from(height).unwrap_or(0));
    u32::try_from(target.max(0)).unwrap_or(0)
}

/// Bytes per second across `angles`, scaled from the 1920x1080x60 table row.
pub fn estimate_rate_bps(
    profile: u32,
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    angles: u32,
) -> u64 {
    if width == 0 || height == 0 || fps_num == 0 || fps_den == 0 || angles == 0 {
        return 0;
    }
    let mbps = estimate_bitrate_mbps(profile, height) as f64;
    let fps = f64::from(fps_num) / f64::from(fps_den);
    let scale = (f64::from(width) * f64::from(height) * fps) / (1920.0 * 1080.0 * 60.0);
    (mbps * 1_000_000.0 / 8.0 * scale * f64::from(angles)) as u64
}

#[derive(Clone, Debug)]
pub struct ReplayStorageTarget {
    pub path: String,
}

#[derive(Clone, Debug)]
pub struct ReplayAngleTarget {
    pub angle_id: u32,
    pub storage_index: u32,
}

#[derive(Clone, Debug)]
pub struct ReplayStoreConfig {
    pub recording_uuid: String,
    pub storages: Vec<ReplayStorageTarget>,
    pub angles: Vec<ReplayAngleTarget>,
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
    /// Frames at or after this one are still referenced by a clip or a protected
    /// event, so `RETENTION_PROTECT` must not reclaim segments that hold them.
    /// Zero means nothing is protected.
    pub protect_from_frame: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOutcome {
    Accepted,
    Dropped,
}

#[derive(Clone, Debug, Default)]
pub struct ReplayStats {
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
    /// Frames stored as the sender encoded them, without a second encoder.
    pub encoded_frames: u64,
    pub angles: Vec<ReplayAngleStats>,
}

/// One frame as it arrived from the sender, still encoded.
///
/// The store keeps this instead of a decoded picture, so the
/// recording holds what the camera sent rather than a re-encode of it.
#[derive(Clone, Debug, Default)]
pub struct EncodedFrame {
    /// The bitstream, exactly as it arrived.
    pub payload: Arc<Vec<u8>>,
    /// Picture width the sender declared.
    pub width: u32,
    /// Picture height the sender declared.
    pub height: u32,
    /// Timestamp the sender stamped, in 100 ns units.
    pub pts_100ns: i64,
    /// Frame rate numerator the sender declared.
    pub fps_num: u32,
    /// Frame rate denominator the sender declared.
    pub fps_den: u32,
    /// Frames the receiver's capture replaced before this one. A gap in the record
    /// the operator should be able to see rather than guess.
    pub dropped_before: u64,
}

#[derive(Clone, Debug, Default)]
pub struct ReplayAngleStats {
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

#[derive(Clone, Debug)]
pub struct DecodedFrame {
    pub actual_frame_index: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pts_100ns: i64,
    pub substituted: bool,
}

/// One encoded replay frame handed to the mixer for GPU decode.
#[derive(Clone, Debug, Default)]
pub struct ReplayPacket {
    pub actual_frame_index: u64,
    pub width: u32,
    pub height: u32,
    pub pts_100ns: i64,
    pub substituted: bool,
    /// [`REPLAY_CODEC_VMX1`] or [`REPLAY_CODEC_PASSTHROUGH`].
    pub codec: u32,
}

#[derive(Clone, Debug)]
pub struct ReplayEstimate {
    pub free_bytes: u64,
    pub estimate_rate_bps: u64,
    pub remaining_secs: u32,
    pub path_writable: bool,
}

/// One run of the recording timeline that goes into an exported file.
#[derive(Clone, Debug)]
pub struct ReplayExportSegment {
    pub angle_id: u32,
    /// Inclusive, matching how the session stores clip ranges.
    pub from_frame: u64,
    pub to_frame: u64,
}

#[derive(Clone, Debug)]
pub struct ReplayExportSpec {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub bitrate: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub with_audio: bool,
    pub segments: Vec<ReplayExportSegment>,
    /// 0 is H.264. 1 is HEVC. The writer uses the one named here and does not
    /// switch to the other when it is missing.
    pub codec: u32,
}

/// `state` values, matching the module's `EXPORT_*`.
pub const EXPORT_IDLE: u32 = 0;
pub const EXPORT_RUNNING: u32 = 1;
pub const EXPORT_DONE: u32 = 2;
pub const EXPORT_FAILED: u32 = 3;
pub const EXPORT_CANCELLED: u32 = 4;

#[derive(Clone, Debug, Default)]
pub struct ReplayExportProgress {
    pub state: u32,
    pub done: u64,
    pub total: u64,
    pub skipped: u64,
    pub bytes: u64,
    /// Why it failed, when it did.
    pub message: String,
}

pub trait ReplayStore: Send {
    fn write_video(
        &mut self,
        angle_id: u32,
        frame_index: u64,
        frame: &VideoFrame,
    ) -> ProResult<WriteOutcome>;
    fn decode(&mut self, angle_id: u32, frame_index: u64, dst: &mut [u8]) -> ProResult<DecodedFrame>;

    /// Decodes the jog proxy. A store without one returns an error and does not
    /// decode the full-size picture in its place.
    fn decode_proxy(
        &mut self,
        _angle_id: u32,
        _frame_index: u64,
        _dst: &mut [u8],
    ) -> ProResult<DecodedFrame> {
        Err(ProError::invalid("proxy is missing"))
    }

    /// Reads the stored packet without decoding it, so the mixer can decode on its
    /// own GPU and never copy a decoded picture back to the CPU.
    ///
    /// A store that cannot hand out packets says so; the caller then keeps using
    /// [`Self::decode`]. The buffer is reused by the caller and may be swapped with
    /// the store's own scratch buffer.
    fn read_packet(
        &mut self,
        _angle_id: u32,
        _frame_index: u64,
        _out: &mut Vec<u8>,
    ) -> ProResult<ReplayPacket> {
        Err(ProError::unavailable(
            "this store cannot hand out replay packets",
        ))
    }
    fn range(&self, angle_id: u32) -> ProResult<Option<(u64, u64)>>;
    fn stats(&self) -> ReplayStats;
    fn close(&mut self) -> ProResult<()>;

    /// Stores a frame the sender already encoded, without decoding or re-encoding it.
    ///
    /// GPU-encoded angles record this way: the clip plays back the bitstream the
    /// camera sent, so the picture never passes through a second encoder. A store
    /// that cannot keep a bitstream says so instead of quietly re-encoding.
    fn write_encoded(
        &mut self,
        _angle_id: u32,
        _frame_index: u64,
        _frame: &EncodedFrame,
    ) -> ProResult<WriteOutcome> {
        Err(ProError::unavailable(
            "this store cannot record an already-encoded frame",
        ))
    }

    /// Appends interleaved PCM for the video frame it belongs to. The samples
    /// also feed the waveform buckets, so the Event List never reads PCM back.
    fn write_audio(
        &mut self,
        _angle_id: u32,
        _frame_index: u64,
        _sample_rate: u32,
        _channels: u32,
        _pcm: &[f32],
    ) -> ProResult<WriteOutcome> {
        Err(ProError::unavailable("replay audio is not implemented"))
    }

    /// Reads `frames` audio frames starting at `from_frame`'s position on the
    /// recording timeline. Returns how many were written.
    fn read_audio(
        &mut self,
        _angle_id: u32,
        _from_frame: u64,
        _frames: usize,
        _out: &mut [f32],
    ) -> ProResult<usize> {
        Err(ProError::unavailable("replay audio is not implemented"))
    }

    /// Waveform buckets of an angle: how many there are, how many were dropped
    /// from the front, and the values themselves.
    fn peak_count(&mut self, _angle_id: u32) -> ProResult<(u32, u32)> {
        Ok((0, 0))
    }

    fn copy_peaks(
        &mut self,
        _angle_id: u32,
        _from: u32,
        _min_out: &mut [f32],
        _max_out: &mut [f32],
    ) -> ProResult<u32> {
        Ok(0)
    }

    /// Starts writing a recording to MP4. The job runs on the module's own
    /// thread, so this returns as soon as it is under way.
    fn export_start(&mut self, _spec: ReplayExportSpec) -> ProResult<()> {
        Err(ProError::unavailable("replay export is not implemented"))
    }

    fn export_progress(&mut self) -> ProResult<ReplayExportProgress> {
        Ok(ReplayExportProgress::default())
    }

    fn export_cancel(&mut self) -> ProResult<()> {
        Err(ProError::unavailable("replay export is not implemented"))
    }
}

pub trait ReplayBackend: Send + Sync {
    fn open(&self, config: &ReplayStoreConfig) -> ProResult<Box<dyn ReplayStore>>;
    /// Free space and a bitrate model for a plan of `angles` streams. The argument
    /// list is the module ABI: it travels through `ProApi` field for field.
    #[allow(clippy::too_many_arguments)]
    fn estimate(
        &self,
        path: &str,
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
        profile: u32,
        angles: u32,
        reserve_bytes: u64,
    ) -> ProResult<ReplayEstimate>;
}

/// Usable bytes at `path`. `None` when the path cannot be queried.
pub fn free_bytes(path: &str) -> Option<u64> {
    free_bytes_at(std::path::Path::new(path))
}

pub fn free_bytes_at(path: &std::path::Path) -> Option<u64> {
    let probe = if path.exists() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    #[cfg(windows)]
    let answer = free_bytes_windows(&probe);
    #[cfg(unix)]
    let answer = free_bytes_unix(&probe);
    #[cfg(not(any(windows, unix)))]
    let answer = {
        let _ = probe;
        None::<u64>
    };
    answer
}

#[cfg(windows)]
fn free_bytes_windows(path: &std::path::Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let mut free = 0u64;
    let mut total = 0u64;
    let mut total_free = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            &mut total,
            &mut total_free,
        )
    };
    if ok == 0 { None } else { Some(free) }
}

#[cfg(windows)]
unsafe extern "system" {
    fn GetDiskFreeSpaceExW(
        directory: *const u16,
        free_available: *mut u64,
        total: *mut u64,
        total_free: *mut u64,
    ) -> i32;
}

#[cfg(all(unix, not(target_os = "macos")))]
fn free_bytes_unix(path: &std::path::Path) -> Option<u64> {
    let mut buf = LinuxStatvfs::default();
    let c = std::ffi::CString::new(path.to_string_lossy().as_bytes()).ok()?;
    let rc = unsafe { statvfs(c.as_ptr(), &mut buf) };
    if rc != 0 {
        return None;
    }
    Some(buf.f_bavail.saturating_mul(buf.f_frsize))
}

#[cfg(all(unix, not(target_os = "macos")))]
#[repr(C)]
#[derive(Default)]
struct LinuxStatvfs {
    f_bsize: u64,
    f_frsize: u64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_favail: u64,
    f_fsid: u64,
    f_flag: u64,
    f_namemax: u64,
}

#[cfg(all(unix, not(target_os = "macos")))]
unsafe extern "C" {
    fn statvfs(path: *const i8, buf: *mut LinuxStatvfs) -> i32;
}

#[cfg(target_os = "macos")]
fn free_bytes_unix(path: &std::path::Path) -> Option<u64> {
    let mut buf = MacStatvfs::default();
    let c = std::ffi::CString::new(path.to_string_lossy().as_bytes()).ok()?;
    let rc = unsafe { statvfs(c.as_ptr(), &mut buf) };
    if rc != 0 {
        return None;
    }
    Some(u64::from(buf.f_bavail).saturating_mul(buf.f_frsize))
}

/// Apple `statvfs`: `fsblkcnt_t` is `unsigned int`, then pad to `unsigned long`.
#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Default)]
struct MacStatvfs {
    f_bsize: u64,
    f_frsize: u64,
    f_blocks: u32,
    f_bfree: u32,
    f_bavail: u32,
    f_files: u32,
    f_ffree: u32,
    f_favail: u32,
    _pad: u32,
    f_fsid: u64,
    f_flag: u64,
    f_namemax: u64,
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn statvfs(path: *const i8, buf: *mut MacStatvfs) -> i32;
}

pub fn remaining_secs(usable: u64, rate_bps: u64) -> u32 {
    if rate_bps == 0 {
        return u32::MAX;
    }
    (usable / rate_bps).min(u64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hq_1080p60_four_angles_is_about_130_mib() {
        let bps = estimate_rate_bps(PROFILE_HQ, 1920, 1080, 60, 1, 4);
        let mib = bps as f64 / (1024.0 * 1024.0);
        assert!((mib - 124.0).abs() < 8.0, "got {mib} MiB/s ({bps} B/s)");
    }

    #[test]
    fn sq_is_half_of_hq_at_1080() {
        assert_eq!(estimate_bitrate_mbps(PROFILE_HQ, 1080), 260);
        assert_eq!(estimate_bitrate_mbps(PROFILE_SQ, 1080), 130);
        assert_eq!(estimate_bitrate_mbps(PROFILE_LQ, 1080), 86);
    }
}
