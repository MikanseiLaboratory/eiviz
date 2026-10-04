/// Frame-indexed progress on the master compose cursor.
///
/// `start_frame` is the last composed frame when the animation was armed.
/// The next compose tick is the first step, and the animation completes on
/// the tick where `frame_i - start_frame == duration_frames`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AnimClock {
    pub start_frame: u64,
    pub duration_frames: u32,
}

impl AnimClock {
    pub fn new(start_frame: u64, duration_frames: u32) -> Self {
        Self {
            start_frame,
            duration_frames: duration_frames.max(1),
        }
    }

    pub fn elapsed(&self, frame_i: u64) -> u64 {
        frame_i.saturating_sub(self.start_frame)
    }

    pub fn finished(&self, frame_i: u64) -> bool {
        self.elapsed(frame_i) >= u64::from(self.duration_frames.max(1))
    }

    /// Linear 0..1. Callers apply [`super::Curve`] on top.
    pub fn progress(&self, frame_i: u64) -> f32 {
        let dur = self.duration_frames.max(1) as f32;
        (self.elapsed(frame_i) as f32 / dur).clamp(0.0, 1.0)
    }
}
