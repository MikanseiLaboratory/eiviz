//! Boundary between the public mixer and the Pro module.
//!
//! The mixer does not link the Pro crate. A signed module is loaded at
//! startup through [`ffi::ProApi`]. Without that module the mixer uses
//! [`Entitlements::community`] and reports DeckLink, RTMP, and licensing as
//! unlinked. A required Pro package must fail startup when the module is
//! missing, unsigned, or built for a different ABI. It must not fall back
//! to the Community plan.
//!
//! The traits in this crate stay on one side of the DLL. The private module
//! implements them and exports [`ffi::GET_API_SYMBOL`]. The mixer adapts the
//! function table back into those traits.

mod decklink;
mod error;
mod ffi;
mod license;
mod manifest;
mod media;
mod peaks;
mod plan;
mod replay;
mod stream;

pub use decklink::{
    CaptureHandle, CaptureSink, CaptureStats, DeckLinkBackend, DeckLinkCaptureConfig,
    DeckLinkDevice, DeckLinkMode, DeckLinkPlayoutConfig, SignalStatus,
};
pub use error::{ProError, ProErrorKind, ProResult};
pub use ffi::{
    ABI_MAJOR, EXPORT_AUDIO_SILENT, FEATURE_DECKLINK, FEATURE_LICENSE, FEATURE_REPLAY,
    FEATURE_RTMP, GET_API_SYMBOL, GetApiFn, LAYOUT_BGRA, LAYOUT_UYVY, PRO_BUSY, PRO_DEVICE,
    PRO_INVALID, PRO_IO, PRO_NOT_SUPPORTED, PRO_OK, PRO_UNAVAILABLE, ProApi, ProAudioView,
    ProBytes, ProCaptureCallbacks, ProCaptureStatsAbi, ProEntitlementsAbi, ProLicenseStatusAbi,
    ProOutputStatsAbi, ProPlayoutConfig, ProReplayAngle, ProReplayAngleStatsAbi,
    ProReplayAudioView, ProReplayConfig, ProReplayDecoded, ProReplayEncodedView,
    ProReplayEstimateAbi, ProReplayExportClip, ProReplayExportProgress, ProReplayExportRequest,
    ProReplayExportSegment, ProReplayPacket, ProReplayStatsAbi, ProReplayStorage, ProStatus,
    ProStreamConfig, ProVideoView, SIGNAL_NONE, SIGNAL_PRESENT, abi_hash, copy_to_buffer,
    module_target,
};
pub use license::{LicenseBackend, LicenseCondition, LicenseStatus};
pub use manifest::ModuleManifest;
pub use media::{AudioFrame, MediaOutput, OutputStats, PixelLayout, VideoFrame};
pub use peaks::{PEAK_BUCKETS_PER_SEC, PeakAccumulator};
pub use plan::{Entitlements, Plan, Quota, VideoLimit};
pub use replay::{
    DecodedFrame, EXPORT_CANCELLED, EXPORT_DONE, EXPORT_FAILED, EXPORT_IDLE, EXPORT_RUNNING,
    EXPORT_TIMING_ADDITIVE, EXPORT_TIMING_OVERWRAP, EncodedFrame, ISSUE_CPU_ENCODE,
    ISSUE_FRAMES_DROPPED, ISSUE_GPU_READBACK, ISSUE_PRO_UNAVAILABLE, ISSUE_STOPPED_BY_POLICY,
    ISSUE_STORAGE_FULL, ISSUE_STORAGE_LOW, ISSUE_STORAGE_WRITE, OVERFLOW_DROP, OVERFLOW_STOP,
    PROFILE_HQ, PROFILE_LQ, PROFILE_SQ, REPLAY_CODEC_PASSTHROUGH, REPLAY_CODEC_VMX1,
    RETENTION_OVERWRITE, RETENTION_PROTECT, RETENTION_STOP, ReplayAngleStats, ReplayAngleTarget,
    ReplayBackend, ReplayEstimate, ReplayEstimateQuery, ReplayExportClip, ReplayExportProgress,
    ReplayExportSegment, ReplayExportSpec, ReplayExportTransition, ReplayPacket, ReplayStats,
    ReplayStorageTarget, ReplayStore, ReplayStoreConfig, SEVERITY_CRITICAL, SEVERITY_OK,
    SEVERITY_WARNING, STATE_IDLE, STATE_RECORDING, STATE_STOPPED, STOP_BUFFER_FULL, STOP_FPS,
    STOP_IO, STOP_NONE, STOP_STORAGE_FULL, STOP_USER, WriteOutcome, estimate_bitrate_mbps,
    estimate_rate_bps, free_bytes, free_bytes_at, profile_code, remaining_secs,
};
pub use stream::{
    AudioEncodeConfig, StreamBackend, StreamEncodeConfig, StreamOutput, VideoEncoderKind,
};

/// Entry point implemented by the Community stand-in and by the private Pro crate.
pub trait ProModule: Send + Sync {
    /// Short identifier shown in About / diagnostics (`"stub"`, `"eiviz-pro"`).
    fn module_name(&self) -> &'static str;

    /// Plan currently in force. Cheap; the mixer calls it on every gated call.
    fn entitlements(&self) -> Entitlements;

    /// `None` when this build links no DeckLink implementation.
    fn decklink(&self) -> Option<&dyn DeckLinkBackend>;

    /// `None` when this build links no RTMP implementation.
    fn streaming(&self) -> Option<&dyn StreamBackend>;

    /// `None` on the Community stand-in. Official builds verify a signed ticket here.
    fn license(&self) -> Option<&dyn LicenseBackend> {
        None
    }

    /// `None` when this build links no replay store.
    fn replay(&self) -> Option<&dyn ReplayBackend> {
        None
    }
}
