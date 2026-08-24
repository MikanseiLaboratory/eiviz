use eiviz_media::{Capability, VideoFrame};
use parking_lot::Mutex;

/// NDI is feature/SDK gated. Default builds report Unavailable and stay running.
pub fn probe() -> Capability {
    Capability {
        id: "ndi".into(),
        available: cfg!(feature = "ndi-sdk"),
        detail: if cfg!(feature = "ndi-sdk") {
            "grafton-ndi adapter compiled".into()
        } else {
            "NDI SDK not linked; enable feature ndi-sdk on a licensed build".into()
        },
    }
}

/// Software loopback used in CI when the native SDK is absent.
#[derive(Default)]
pub struct SimulatedNdi {
    last: Mutex<Option<VideoFrame>>,
}

impl SimulatedNdi {
    pub fn send(&self, frame: &VideoFrame) {
        *self.last.lock() = Some(frame.clone());
    }

    pub fn receive(&self) -> Option<VideoFrame> {
        self.last.lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_time::MediaTime;

    #[test]
    fn default_is_unavailable() {
        let c = super::probe();
        assert!(!c.available);
        assert!(c.detail.contains("NDI"));
    }

    #[test]
    fn simulated_loopback_roundtrips_pixels() {
        let bus = SimulatedNdi::default();
        let frame = VideoFrame::rgba_solid(1, MediaTime::ZERO, 2, 2, [1, 2, 3, 255]);
        bus.send(&frame);
        let got = bus.receive().unwrap();
        assert_eq!(got.pixel(0, 0), [1, 2, 3, 255]);
    }
}
