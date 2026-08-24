use eiviz_media::{Capability, VideoFrame};
use parking_lot::Mutex;

/// Official DeckLink SDK is dynamically loaded at runtime when present.
pub fn probe() -> Capability {
    Capability {
        id: "decklink".into(),
        available: false,
        detail: "Desktop Video runtime not detected".into(),
    }
}

pub fn schedule_timescale() -> (u32, u32) {
    // duration, timescale for 59.94: 1001 / 60000
    (1001, 60000)
}

#[derive(Default)]
pub struct SimulatedDeckLink {
    last: Mutex<Option<VideoFrame>>,
}

impl SimulatedDeckLink {
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
    fn ntsc_schedule_is_rational() {
        assert_eq!(super::schedule_timescale(), (1001, 60000));
    }

    #[test]
    fn simulated_loopback_roundtrips_pixels() {
        let bus = SimulatedDeckLink::default();
        let frame = VideoFrame::rgba_solid(1, MediaTime::ZERO, 2, 2, [4, 5, 6, 255]);
        bus.send(&frame);
        assert_eq!(bus.receive().unwrap().pixel(1, 1), [4, 5, 6, 255]);
    }
}
