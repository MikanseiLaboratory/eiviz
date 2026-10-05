//! Boundary between the public mixer and the Pro module.
//!
//! The mixer does not link the Pro crate. A signed module is loaded at
//! startup through [`ffi::ProApi`]. Without that module the mixer uses
//! [`Entitlements::free`] and reports DeckLink, RTMP, and licensing as
//! unlinked. A required Pro package must fail startup when the module is
//! missing, unsigned, or built for a different ABI. It must not fall back
//! to the Free plan.
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
mod plan;
mod stream;

pub use decklink::{
    CaptureHandle, CaptureSink, CaptureStats, DeckLinkBackend, DeckLinkCaptureConfig,
    DeckLinkDevice, DeckLinkMode, DeckLinkPlayoutConfig, SignalStatus,
};
pub use error::{ProError, ProErrorKind, ProResult};
pub use ffi::{
    ABI_MAJOR, FEATURE_DECKLINK, FEATURE_LICENSE, FEATURE_RTMP, GET_API_SYMBOL, GetApiFn,
    LAYOUT_BGRA, LAYOUT_UYVY, PRO_BUSY, PRO_DEVICE, PRO_INVALID, PRO_IO, PRO_NOT_SUPPORTED, PRO_OK,
    PRO_UNAVAILABLE, ProApi, ProAudioView, ProBytes, ProCaptureCallbacks, ProCaptureStatsAbi,
    ProEntitlementsAbi, ProLicenseStatusAbi, ProOutputStatsAbi, ProPlayoutConfig, ProRtmpConfig,
    ProStatus, ProVideoView, SIGNAL_NONE, SIGNAL_PRESENT, abi_hash, copy_to_buffer, module_target,
};
pub use license::{LicenseBackend, LicenseCondition, LicenseStatus};
pub use manifest::ModuleManifest;
pub use media::{AudioFrame, MediaOutput, OutputStats, PixelLayout, VideoFrame};
pub use plan::{Entitlements, Plan, Quota, VideoLimit};
pub use stream::{AudioEncodeConfig, RtmpConfig, StreamBackend, VideoEncoderKind};

/// Entry point implemented by the Free stand-in and by the private Pro crate.
pub trait ProModule: Send + Sync {
    /// Short identifier shown in About / diagnostics (`"stub"`, `"eiviz-pro"`).
    fn module_name(&self) -> &'static str;

    /// Plan currently in force. Cheap; the mixer calls it on every gated call.
    fn entitlements(&self) -> Entitlements;

    /// `None` when this build links no DeckLink implementation.
    fn decklink(&self) -> Option<&dyn DeckLinkBackend>;

    /// `None` when this build links no RTMP implementation.
    fn streaming(&self) -> Option<&dyn StreamBackend>;

    /// `None` on the Free stand-in. Official builds verify a signed ticket here.
    fn license(&self) -> Option<&dyn LicenseBackend> {
        None
    }
}
