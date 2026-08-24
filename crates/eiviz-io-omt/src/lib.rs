//! Open Media Transport adapter.
//!
//! Production capture uses the pure-Rust `openmediatransport-rs` protocol stack.
//! No native libomt fallback or simulated production path exists.

mod native;

pub use native::{
    OmtColorProfile, OmtError, OmtOutputConfig, OmtOutputPixelFormat, OmtSink, OmtSource,
    discover_sources, validate_video_format,
};

use eiviz_media::Capability;

pub fn probe() -> Capability {
    Capability {
        id: "omt".into(),
        available: true,
        detail: "pure Rust openmediatransport-rs + vmx-rs".into(),
    }
}

/// Test-only loopback. Runtime does not instantiate this type for OMT inputs.
#[cfg(test)]
#[derive(Default)]
struct SimulatedOmt {
    last: parking_lot::Mutex<Option<eiviz_media::VideoFrame>>,
}

#[cfg(test)]
impl SimulatedOmt {
    fn send(&self, frame: &eiviz_media::VideoFrame) {
        *self.last.lock() = Some(frame.clone());
    }

    fn receive(&self) -> Option<eiviz_media::VideoFrame> {
        self.last.lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_media::VideoFrame;
    use eiviz_time::MediaTime;

    #[test]
    fn reports_capability() {
        assert_eq!(super::probe().id, "omt");
    }

    #[test]
    fn simulated_loopback_roundtrips_pixels() {
        let bus = SimulatedOmt::default();
        let frame = VideoFrame::rgba_solid(1, MediaTime::ZERO, 2, 2, [9, 8, 7, 255]);
        bus.send(&frame);
        assert_eq!(bus.receive().unwrap().pixel(0, 0), [9, 8, 7, 255]);
    }
}
