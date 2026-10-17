//! Pitch-preserving time stretch for replay slow motion.
//!
//! At anything but normal speed the resampler moves the pitch along with the
//! speed. This is WSOLA: successive windows of the source are overlap-added at a
//! fixed synthesis hop, while the analysis pointer moves at the playback speed,
//! and each window is nudged to the position that best continues what was just
//! emitted. The result is longer or shorter audio that keeps its pitch.

use super::audio::AudioReader;

/// Analysis window. 21 ms at 48 kHz is long enough to cover a low voice and short
/// enough that a cut between windows is not audible.
const FRAME: usize = 1024;
/// How far the window may be nudged to find a match, in frames.
const SEARCH: usize = 512;
/// A window must repeat at this distance to overlap correctly.
const HOP: usize = FRAME / 2;

pub struct TimeStretch {
    channels: usize,
    /// Output waiting to be handed to the caller.
    pending: Vec<f32>,
    /// Read position of the next analysis window, fractional.
    cursor: f64,
    /// Natural continuation of the previous window, used for the match search.
    tail: Vec<f32>,
    source: Vec<f32>,
    /// Whether a window has been emitted, so the first one starts at the cursor.
    started: bool,
    ended: bool,
}

impl TimeStretch {
    pub fn new(channels: usize) -> Self {
        let channels = channels.max(1);
        Self {
            channels,
            // Start with silence so the first window overlaps cleanly.
            pending: vec![0.0; (FRAME - HOP) * channels],
            cursor: 0.0,
            tail: vec![0.0; (FRAME - HOP) * channels],
            source: Vec::new(),
            started: false,
            ended: false,
        }
    }

    /// Points the analysis pointer at a source frame and drops queued output.
    pub fn reset(&mut self, frame: f64) {
        self.cursor = if frame.is_finite() { frame.max(0.0) } else { 0.0 };
        self.pending.clear();
        self.pending.resize((FRAME - HOP) * self.channels, 0.0);
        self.tail.fill(0.0);
        self.started = false;
        self.ended = false;
    }

    pub fn cursor(&self) -> f64 {
        self.cursor
    }

    /// Produces `frames` output frames at `speed` while keeping the pitch. A
    /// negative speed is refused: reversing is the resampler's job.
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
        let speed = if speed.is_finite() && speed > 0.0 {
            speed
        } else {
            1.0
        };
        let mut written = 0;
        while written < available {
            while self.pending.len() / channels < available - written + (FRAME - HOP) {
                if !self.step(speed, read) {
                    break;
                }
            }
            // The last window is only half overlap-added, so it stays in the
            // buffer until the next one completes it.
            let ready = (self.pending.len() / channels).saturating_sub(FRAME - HOP);
            let have = ready.min(available - written);
            if have == 0 {
                // Source exhausted: the rest of the block is silence.
                break;
            }
            out[written * channels..(written + have) * channels]
                .copy_from_slice(&self.pending[..have * channels]);
            self.pending.drain(..have * channels);
            written += have;
        }
        out[written * channels..available * channels].fill(0.0);
        written
    }

    /// Emits one synthesis window. Returns false when the source is finished.
    fn step(&mut self, speed: f64, read: &mut AudioReader<'_>) -> bool {
        if self.ended {
            return false;
        }
        let channels = self.channels;
        let overlap = FRAME - HOP;
        let base = self.cursor.floor().max(0.0) as u64;
        self.source.clear();
        self.source.resize((FRAME + SEARCH) * channels, 0.0);
        let got = read(base, FRAME + SEARCH, &mut self.source);
        if got < FRAME {
            self.ended = true;
            return false;
        }
        let offset = if self.started {
            best_offset(&self.source, &self.tail, overlap, channels, got - FRAME)
        } else {
            0
        };
        // Remember the window's natural continuation for the next search.
        self.tail.clear();
        for index in HOP..FRAME {
            let from = (offset + index) * channels;
            self.tail
                .extend_from_slice(&self.source[from..from + channels]);
        }
        if self.started {
            // The first `overlap` samples land on the previous window's tail.
            let start = self.pending.len() - overlap * channels;
            for index in 0..overlap {
                let weight = hann(index, FRAME);
                for channel in 0..channels {
                    let from = (offset + index) * channels + channel;
                    self.pending[start + index * channels + channel] +=
                        self.source[from] * weight;
                }
            }
        }
        for index in overlap..FRAME {
            let weight = hann(index, FRAME);
            for channel in 0..channels {
                let from = (offset + index) * channels + channel;
                self.pending.push(self.source[from] * weight);
            }
        }
        self.started = true;
        // The analysis pointer moves at the playback speed, so slow motion reads
        // less source per output frame and the pitch stays put.
        self.cursor += HOP as f64 * speed;
        true
    }
}

fn hann(index: usize, frame: usize) -> f32 {
    0.5 - 0.5 * (std::f64::consts::TAU * index as f64 / frame as f64).cos() as f32
}

/// Offset within `search` frames whose window best continues `tail`. A shift has
/// to beat the natural continuation clearly, otherwise a periodic signal would
/// wander a whole period and shift its phase for no reason.
fn best_offset(source: &[f32], tail: &[f32], overlap: usize, channels: usize, search: usize) -> usize {
    let score_at = |offset: usize| -> f32 {
        let mut score = 0.0f32;
        let mut energy = 1e-6f32;
        for index in 0..overlap {
            let mut left = 0.0f32;
            let mut right = 0.0f32;
            for channel in 0..channels {
                left += source[(offset + index) * channels + channel];
                right += tail[index * channels + channel];
            }
            score += left * right;
            energy += left * left;
        }
        // Normalizing keeps a loud window from winning on volume alone.
        score / energy.sqrt()
    };
    let natural = score_at(0);
    let mut best = 0usize;
    let mut best_score = natural;
    for offset in 1..=search {
        let score = score_at(offset);
        if score > best_score {
            best_score = score;
            best = offset;
        }
    }
    if best_score > natural * 1.02 {
        best
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sine source at `hz`, as interleaved stereo.
    struct Sine {
        frames: usize,
    }

    impl Sine {
        fn read(&self, hz: f64) -> impl FnMut(u64, usize, &mut [f32]) -> usize + '_ {
            move |from, want, out| {
                let mut written = 0;
                for index in 0..want {
                    let frame = from as usize + index;
                    if frame >= self.frames {
                        break;
                    }
                    let value =
                        (std::f64::consts::TAU * hz * frame as f64 / 48_000.0).sin() as f32;
                    out[index * 2] = value;
                    out[index * 2 + 1] = value;
                    written += 1;
                }
                written
            }
        }
    }

    /// Magnitude of `hz` in `pcm`, by direct correlation. Enough to tell whether
    /// the pitch survived.
    fn magnitude_at(pcm: &[f32], hz: f64) -> f64 {
        let frames = pcm.len() / 2;
        let mut re = 0.0;
        let mut im = 0.0;
        for index in 0..frames {
            let phase = std::f64::consts::TAU * hz * index as f64 / 48_000.0;
            let sample = f64::from(pcm[index * 2]);
            re += sample * phase.cos();
            im += sample * phase.sin();
        }
        (re * re + im * im).sqrt() / frames as f64
    }

    #[test]
    fn normal_speed_returns_the_source_unchanged() {
        let sine = Sine { frames: 48_000 };
        let mut reader = sine.read(440.0);
        let mut stretch = TimeStretch::new(2);
        let mut out = vec![0.0f32; 8192 * 2];
        let written = stretch.fill(8192, 1.0, &mut reader, &mut out);
        assert_eq!(written, 8192);
        // Overlap-add of a Hann window at half-frame hops sums to one, so the
        // samples come back as they went in.
        for index in 2048..written {
            let expected =
                (std::f64::consts::TAU * 440.0 * index as f64 / 48_000.0).sin() as f32;
            assert!(
                (out[index * 2] - expected).abs() < 0.02,
                "frame {index}: {} vs {expected}",
                out[index * 2]
            );
        }
    }

    #[test]
    fn half_speed_doubles_the_length_and_keeps_the_pitch() {
        let sine = Sine { frames: 48_000 };
        let mut reader = sine.read(440.0);
        let mut stretch = TimeStretch::new(2);
        let mut out = vec![0.0f32; 8192 * 2];
        let written = stretch.fill(8192, 0.5, &mut reader, &mut out);
        assert_eq!(written, 8192);
        // Half speed must consume half the source for the same output: otherwise
        // the "stretch" is really a resample and the pitch check below is empty.
        assert!(
            (stretch.cursor() - 4096.0).abs() < 64.0,
            "source advanced {} for 8192 output frames",
            stretch.cursor()
        );
        // The analysis pointer moved 8192 * 0.5 frames, so the same source would
        // have lasted twice as long; the pitch must not have moved with it.
        let middle = &out[2048 * 2..(written - 2048) * 2];
        let at_440 = magnitude_at(middle, 440.0);
        let at_220 = magnitude_at(middle, 220.0);
        assert!(
            at_440 > at_220 * 4.0,
            "440 Hz should dominate: {at_440} vs {at_220}"
        );
    }

    #[test]
    fn double_speed_keeps_the_pitch_too() {
        let sine = Sine { frames: 96_000 };
        let mut reader = sine.read(1000.0);
        let mut stretch = TimeStretch::new(2);
        let mut out = vec![0.0f32; 8192 * 2];
        let written = stretch.fill(8192, 2.0, &mut reader, &mut out);
        assert_eq!(written, 8192);
        let middle = &out[2048 * 2..(written - 2048) * 2];
        let at_1000 = magnitude_at(middle, 1000.0);
        let at_2000 = magnitude_at(middle, 2000.0);
        assert!(
            at_1000 > at_2000 * 4.0,
            "1000 Hz should dominate: {at_1000} vs {at_2000}"
        );
    }

    #[test]
    fn running_out_of_source_gives_silence_and_stops() {
        let sine = Sine { frames: 4096 };
        let mut reader = sine.read(440.0);
        let mut stretch = TimeStretch::new(2);
        let mut out = vec![1.0f32; 8192 * 2];
        let written = stretch.fill(8192, 1.0, &mut reader, &mut out);
        assert!(written < 8192);
        assert!(out[written * 2..].iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn reset_clears_queued_audio() {
        let sine = Sine { frames: 96_000 };
        let mut reader = sine.read(440.0);
        let mut stretch = TimeStretch::new(2);
        let mut out = vec![0.0f32; 1024 * 2];
        assert_eq!(stretch.fill(1024, 1.0, &mut reader, &mut out), 1024);
        stretch.reset(20_000.0);
        assert!((stretch.cursor() - 20_000.0).abs() < 1e-9);
        let mut out = vec![0.0f32; 1024 * 2];
        assert_eq!(stretch.fill(1024, 1.0, &mut reader, &mut out), 1024);
    }
}



