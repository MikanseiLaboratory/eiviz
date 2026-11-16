//! Replay playback audio.
//!
//! The channel that is playing reads PCM recorded alongside the video and hands
//! it to the mixer as if it came from a live input. Speed changes resample the
//! source, so slow motion lowers the pitch the way a tape would.

use super::stretch::TimeStretch;

/// Reads interleaved source PCM: `want` frames starting at `from`. Returns how
/// many frames were actually available.
pub type AudioReader<'a> = dyn FnMut(u64, usize, &mut [f32]) -> usize + 'a;

/// How far playback may drift from the stretcher before it resyncs. A seek or a
/// clip change jumps much further than this; normal play drifts by well under a
/// frame.
const RESYNC_FRAMES: f64 = 12_000.0;

pub struct PlaybackAudio {
    /// Next source frame to read, fractional so slow motion stays smooth.
    cursor: f64,
    channels: usize,
    scratch: Vec<f32>,
    interleaved: Vec<f32>,
    stretch: TimeStretch,
}

impl PlaybackAudio {
    pub fn new(channels: usize) -> Self {
        Self {
            cursor: 0.0,
            channels: channels.max(1),
            scratch: Vec::new(),
            interleaved: Vec::new(),
            stretch: TimeStretch::new(channels.max(1)),
        }
    }

    /// Points the cursor at a source frame. Used on cue, seek and clip changes.
    pub fn reset(&mut self, frame: f64) {
        self.cursor = if frame.is_finite() {
            frame.max(0.0)
        } else {
            0.0
        };
    }

    /// Where the cursor sits in the source. The transport owns the position, so
    /// this is only read back by the tests that pin the resampling.
    #[cfg(test)]
    pub fn cursor(&self) -> f64 {
        self.cursor
    }

    /// Produces `frames` output frames for the given speed. A negative speed
    /// walks the source backwards. Anything the reader cannot supply is silence,
    /// so a clip that runs out of audio never repeats the tail.
    pub fn fill(
        &mut self,
        frames: usize,
        speed: f64,
        read: &mut AudioReader<'_>,
        out: &mut [f32],
    ) -> usize {
        let channels = self.channels;
        let available = frames.min(out.len() / channels);
        if available == 0 {
            return 0;
        }
        let speed = if speed.is_finite() { speed } else { 1.0 };
        if speed == 0.0 {
            out[..available * channels].fill(0.0);
            return 0;
        }
        let span = ((available as f64) * speed.abs()).ceil() as usize + 2;
        self.scratch.clear();
        self.scratch.resize(span * channels, 0.0);
        let first = if speed >= 0.0 {
            self.cursor.floor().max(0.0) as u64
        } else {
            // Reading backwards needs a block that ends at the cursor.
            (self.cursor.floor() - span as f64 + 2.0).max(0.0) as u64
        };
        let got = read(first, span, &mut self.scratch);
        if got == 0 {
            out[..available * channels].fill(0.0);
            return 0;
        }
        let mut written = 0;
        for index in 0..available {
            let position = self.cursor - first as f64;
            let base = position.floor();
            if base < 0.0 || base as usize >= got {
                break;
            }
            let frac = (position - base) as f32;
            let next = ((base as usize) + 1).min(got - 1);
            for channel in 0..channels {
                let a = self.scratch[base as usize * channels + channel];
                let b = self.scratch[next * channels + channel];
                out[index * channels + channel] = a + (b - a) * frac;
            }
            self.cursor += speed;
            written += 1;
        }
        out[written * channels..available * channels].fill(0.0);
        written
    }

    /// Same as [`Self::fill`], but writes the planar layout the mixer's inputs
    /// use. Channel buffers are reused between calls.
    pub fn fill_planar(
        &mut self,
        frames: usize,
        speed: f64,
        read: &mut AudioReader<'_>,
        planar_out: &mut [f32],
    ) -> usize {
        let channels = self.channels;
        let capacity = frames.min(planar_out.len() / channels);
        if capacity == 0 {
            return 0;
        }
        self.interleaved.clear();
        self.interleaved.resize(capacity * channels, 0.0);
        let mut interleaved = std::mem::take(&mut self.interleaved);
        let written = self.fill(capacity, speed, read, &mut interleaved);
        for frame in 0..capacity {
            for channel in 0..channels {
                planar_out[channel * capacity + frame] = if frame < written {
                    interleaved[frame * channels + channel]
                } else {
                    0.0
                };
            }
        }
        self.interleaved = interleaved;
        written
    }

    /// Like [`Self::fill_planar`], but keeps the pitch while the speed changes.
    /// `position` is where the picture is on the source timeline, so the stretcher
    /// can resynchronise after a seek without a click on every block.
    pub fn fill_planar_pitched(
        &mut self,
        frames: usize,
        speed: f64,
        position: f64,
        read: &mut AudioReader<'_>,
        planar_out: &mut [f32],
    ) -> usize {
        let channels = self.channels;
        let capacity = frames.min(planar_out.len() / channels);
        if capacity == 0 {
            return 0;
        }
        // Reverse and a stopped cursor are the resampler's job; WSOLA only runs
        // forwards at a speed that actually needs stretching.
        if speed <= 0.0 || (speed - 1.0).abs() < 1e-3 {
            self.reset(position);
            return self.fill_planar(frames, speed, read, planar_out);
        }
        if (position - self.stretch.cursor()).abs() > RESYNC_FRAMES {
            self.stretch.reset(position);
        }
        self.interleaved.clear();
        self.interleaved.resize(capacity * channels, 0.0);
        let mut interleaved = std::mem::take(&mut self.interleaved);
        let written = self.stretch.fill(capacity, speed, read, &mut interleaved);
        for frame in 0..capacity {
            for channel in 0..channels {
                planar_out[channel * capacity + frame] = interleaved[frame * channels + channel];
            }
        }
        self.interleaved = interleaved;
        self.cursor = self.stretch.cursor();
        written
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Source PCM that counts up, so a resampling mistake is visible in the value.
    struct Ramp {
        frames: usize,
    }

    impl Ramp {
        fn read(&self) -> impl FnMut(u64, usize, &mut [f32]) -> usize + '_ {
            move |from, want, out| {
                let mut written = 0;
                for index in 0..want {
                    let frame = from as usize + index;
                    if frame >= self.frames {
                        break;
                    }
                    out[index * 2] = frame as f32;
                    out[index * 2 + 1] = frame as f32;
                    written += 1;
                }
                written
            }
        }
    }

    #[test]
    fn speed_one_returns_the_source_untouched() {
        let ramp = Ramp { frames: 100 };
        let mut reader = ramp.read();
        let mut audio = PlaybackAudio::new(2);
        audio.reset(10.0);
        let mut out = vec![0.0f32; 8 * 2];
        assert_eq!(audio.fill(8, 1.0, &mut reader, &mut out), 8);
        for index in 0..8 {
            assert!((out[index * 2] - (10 + index) as f32).abs() < 1e-6);
        }
        assert!((audio.cursor() - 18.0).abs() < 1e-9);
    }

    #[test]
    fn double_speed_consumes_two_source_frames_per_output_frame() {
        let ramp = Ramp { frames: 100 };
        let mut reader = ramp.read();
        let mut audio = PlaybackAudio::new(2);
        audio.reset(0.0);
        let mut out = vec![0.0f32; 4 * 2];
        assert_eq!(audio.fill(4, 2.0, &mut reader, &mut out), 4);
        let values: Vec<f32> = (0..4).map(|index| out[index * 2]).collect();
        assert_eq!(values, vec![0.0, 2.0, 4.0, 6.0]);
        assert!((audio.cursor() - 8.0).abs() < 1e-9);
    }

    #[test]
    fn slow_motion_interpolates_between_source_frames() {
        let ramp = Ramp { frames: 100 };
        let mut reader = ramp.read();
        let mut audio = PlaybackAudio::new(2);
        audio.reset(0.0);
        let mut out = vec![0.0f32; 4 * 2];
        assert_eq!(audio.fill(4, 0.5, &mut reader, &mut out), 4);
        let values: Vec<f32> = (0..4).map(|index| out[index * 2]).collect();
        assert!((values[0] - 0.0).abs() < 1e-6);
        assert!((values[1] - 0.5).abs() < 1e-6);
        assert!((values[2] - 1.0).abs() < 1e-6);
        assert!((values[3] - 1.5).abs() < 1e-6);
    }

    #[test]
    fn a_negative_speed_walks_the_source_backwards() {
        let ramp = Ramp { frames: 100 };
        let mut reader = ramp.read();
        let mut audio = PlaybackAudio::new(2);
        audio.reset(10.0);
        let mut out = vec![0.0f32; 4 * 2];
        assert_eq!(audio.fill(4, -1.0, &mut reader, &mut out), 4);
        let values: Vec<f32> = (0..4).map(|index| out[index * 2]).collect();
        assert_eq!(values, vec![10.0, 9.0, 8.0, 7.0]);
        assert!((audio.cursor() - 6.0).abs() < 1e-9);
    }

    #[test]
    fn a_paused_channel_is_silent_and_does_not_move() {
        let ramp = Ramp { frames: 100 };
        let mut reader = ramp.read();
        let mut audio = PlaybackAudio::new(2);
        audio.reset(5.0);
        let mut out = vec![1.0f32; 4 * 2];
        assert_eq!(audio.fill(4, 0.0, &mut reader, &mut out), 0);
        assert!(out.iter().all(|sample| *sample == 0.0));
        assert!((audio.cursor() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn planar_output_matches_the_interleaved_values() {
        let ramp = Ramp { frames: 100 };
        let mut reader = ramp.read();
        let mut audio = PlaybackAudio::new(2);
        audio.reset(4.0);
        let mut planar = vec![0.0f32; 4 * 2];
        assert_eq!(audio.fill_planar(4, 1.0, &mut reader, &mut planar), 4);
        // Planar: left channel then right channel.
        assert_eq!(&planar[0..4], &[4.0, 5.0, 6.0, 7.0]);
        assert_eq!(&planar[4..8], &[4.0, 5.0, 6.0, 7.0]);
    }

    #[test]
    fn the_pitched_path_holds_the_pitch_and_the_plain_path_does_not() {
        // A sine at 440 Hz, played at half speed.
        let frames = 96_000usize;
        let source = |from: u64, want: usize, out: &mut [f32]| -> usize {
            let mut written = 0;
            for index in 0..want {
                let frame = from as usize + index;
                if frame >= frames {
                    break;
                }
                let value = (std::f64::consts::TAU * 440.0 * frame as f64 / 48_000.0).sin() as f32;
                out[index * 2] = value;
                out[index * 2 + 1] = value;
                written += 1;
            }
            written
        };
        // Mono samples: the planar left channel is contiguous.
        let magnitude = |pcm: &[f32], hz: f64| -> f64 {
            let count = pcm.len();
            let mut re = 0.0;
            let mut im = 0.0;
            for (index, sample) in pcm.iter().enumerate() {
                let phase = std::f64::consts::TAU * hz * index as f64 / 48_000.0;
                let sample = f64::from(*sample);
                re += sample * phase.cos();
                im += sample * phase.sin();
            }
            (re * re + im * im).sqrt() / count as f64
        };

        let mut pitched = PlaybackAudio::new(2);
        let mut reader = source;
        let mut planar = vec![0.0f32; 8192 * 2];
        let mut collected = Vec::new();
        let mut position = 0.0f64;
        for _ in 0..4 {
            pitched.fill_planar_pitched(8192, 0.5, position, &mut reader, &mut planar);
            // Planar: the left channel of this block is the first `capacity` samples.
            collected.extend_from_slice(&planar[..8192]);
            position += 8192.0 * 0.5;
        }
        let middle = &collected[8192..collected.len() - 8192];
        let held = magnitude(middle, 440.0);
        let lowered = magnitude(middle, 220.0);
        assert!(
            held > lowered * 4.0,
            "pitch must stay at 440 Hz: {held} vs {lowered}"
        );

        // The plain resampler is what the flag turns off, and it moves the pitch.
        let mut plain = PlaybackAudio::new(2);
        let mut reader = source;
        let mut planar = vec![0.0f32; 8192 * 2];
        plain.reset(0.0);
        plain.fill_planar(8192, 0.5, &mut reader, &mut planar);
        let plain_held = magnitude(&planar[..8192], 440.0);
        let plain_lowered = magnitude(&planar[..8192], 220.0);
        assert!(
            plain_lowered > plain_held * 4.0,
            "resampling halves the pitch: {plain_held} vs {plain_lowered}"
        );
    }

    #[test]
    fn running_past_the_end_gives_silence_rather_than_a_repeat() {
        let ramp = Ramp { frames: 4 };
        let mut reader = ramp.read();
        let mut audio = PlaybackAudio::new(2);
        audio.reset(0.0);
        let mut out = vec![1.0f32; 8 * 2];
        // Only four source frames exist, so only four outputs carry audio.
        assert_eq!(audio.fill(8, 1.0, &mut reader, &mut out), 4);
        assert!((out[3 * 2] - 3.0).abs() < 1e-6);
        assert!(out[4 * 2..].iter().all(|sample| *sample == 0.0));
    }
}
