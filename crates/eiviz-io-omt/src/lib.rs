use eiviz_media::{Capability, VideoFrame};
use parking_lot::Mutex;

pub fn probe() -> Capability {
    Capability {
        id: "omt".into(),
        available: false,
        detail: "OMT adapter compiled; native libomt not linked in this build".into(),
    }
}

#[derive(Default)]
pub struct SimulatedOmt {
    last: Mutex<Option<VideoFrame>>,
}

impl SimulatedOmt {
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
    fn reports_capability() {
        assert_eq!(super::probe().id, "omt");
        assert!(!super::probe().available);
    }

    #[test]
    fn simulated_loopback_roundtrips_pixels() {
        let bus = SimulatedOmt::default();
        let frame = VideoFrame::rgba_solid(1, MediaTime::ZERO, 2, 2, [9, 8, 7, 255]);
        bus.send(&frame);
        assert_eq!(bus.receive().unwrap().pixel(0, 0), [9, 8, 7, 255]);
    }
}
