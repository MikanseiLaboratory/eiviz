//! CPU-side animation. Sampling stays on scalars; shaders receive the result.

mod clock;
mod curve;
mod scene;

pub(crate) use scene::SceneRuntime;

pub(crate) use clock::AnimClock;
pub(crate) use curve::Curve;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thirty_frames_finish_on_the_thirtieth_tick() {
        let clock = AnimClock::new(100, 30);
        let mut ticks = 0u32;
        let mut frame = 101u64;
        loop {
            ticks += 1;
            if clock.finished(frame) {
                break;
            }
            frame += 1;
            assert!(ticks < 30, "transition ran past 30 ticks");
        }
        assert_eq!(ticks, 30);
        assert_eq!(frame, 130);
        assert!(!clock.finished(129));
        assert!((clock.progress(115) - 15.0 / 30.0).abs() < 1e-5);
    }
}
