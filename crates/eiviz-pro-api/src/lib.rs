//! Boundary between the public mixer and the Pro module.
//!
//! The public repository ships a Free-plan stand-in at `pro/eiviz_pro`.
//! Official builds replace that directory with a checkout of the private
//! `eiviz-pro` repository. `mixer/Cargo.toml` always path-depends on
//! `pro/eiviz_pro`, so the manifest does not change between the two builds.
//!
//! There is one `eiviz_pro::module()` entry. The mixer reads
//! [`ProModule::entitlements`] and returns `ERR_NOT_SUPPORTED_PLAN` before
//! touching a backend. A missing backend is `ERR_IO`, not a plan error.

mod decklink;
mod error;
mod license;
mod media;
mod plan;
mod stream;

pub use decklink::{
    CaptureHandle, CaptureSink, CaptureStats, DeckLinkBackend, DeckLinkCaptureConfig,
    DeckLinkDevice, DeckLinkMode, DeckLinkPlayoutConfig, SignalStatus,
};
pub use error::{ProError, ProErrorKind, ProResult};
pub use license::{LicenseBackend, LicenseCondition, LicenseStatus};
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
