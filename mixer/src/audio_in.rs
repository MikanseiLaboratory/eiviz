//! Conversion of incoming audio packets (any rate, any channel count, planar) into the
//! mixer's 48 kHz interleaved stereo.
//!
//! The resampler keeps its position across packets, so packet boundaries add no error and
//! the output sample count follows the exact rate ratio over time instead of being rounded
//! per packet.

use crate::upload::AUDIO_RATE;

const MIX_RATE: u32 = AUDIO_RATE as u32;
const MAX_SOURCE_RATE: i32 = 768_000;
const MAX_SOURCE_CHANNELS: i32 = 64;
const CENTER_AND_SURROUND: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Why a packet was not accepted.
#[derive(Debug, PartialEq, Eq)]
pub enum PacketError {
    SampleRate(i32),
    Channels(i32),
    Empty,
}

impl std::fmt::Display for PacketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SampleRate(rate) => write!(f, "unsupported sample rate {rate}"),
            Self::Channels(count) => write!(f, "unsupported channel count {count}"),
            Self::Empty => write!(f, "packet has no samples"),
        }
    }
}

/// Downmixes planar audio (`channels` planes of `frames` samples) to interleaved stereo.
///
/// 1 channel is duplicated, 2 pass through, 6 is treated as 5.1 (L R C LFE Ls Rs) and 8 as
/// 7.1 (L R C LFE Lb Rb Ls Rs) with the ITU-R BS.775 coefficients normalised so a full-scale
/// signal cannot clip; the LFE channel is dropped. Any other count uses the first two.
/// Non-finite samples become silence.
pub fn planar_to_stereo(planar: &[f32], frames: usize, channels: usize, out: &mut Vec<f32>) {
    out.clear();
    out.reserve(frames * 2);
    let plane = |channel: usize, i: usize| -> f32 {
        let value = planar.get(channel * frames + i).copied().unwrap_or(0.0);
        if value.is_finite() { value } else { 0.0 }
    };
    match channels {
        0 => out.resize(frames * 2, 0.0),
        1 => {
            for i in 0..frames {
                let mono = plane(0, i);
                out.push(mono);
                out.push(mono);
            }
        }
        6 | 8 => {
            let surround_pairs: &[(usize, usize)] = if channels == 6 {
                &[(4, 5)]
            } else {
                &[(4, 5), (6, 7)]
            };
            let norm = 1.0 / (1.0 + CENTER_AND_SURROUND * (1.0 + surround_pairs.len() as f32));
            for i in 0..frames {
                let center = plane(2, i) * CENTER_AND_SURROUND;
                let mut left = plane(0, i) + center;
                let mut right = plane(1, i) + center;
                for &(l, r) in surround_pairs {
                    left += plane(l, i) * CENTER_AND_SURROUND;
                    right += plane(r, i) * CENTER_AND_SURROUND;
                }
                out.push(left * norm);
                out.push(right * norm);
            }
        }
        _ => {
            for i in 0..frames {
                out.push(plane(0, i));
                out.push(plane(1, i));
            }
        }
    }
}

/// Frames over which a starved stream ramps down to silence (about 2 ms at 48 kHz).
const UNDERFLOW_FADE_FRAMES: usize = 96;

/// Fills the missing tail of an interleaved stereo block after a buffer underrun.
///
/// Holding the last sample would leave a DC offset and a click when audio resumes, so the
/// tail ramps from `last` to zero and stays silent.
pub fn fill_underrun(tail: &mut [f32], last: (f32, f32)) {
    for (i, frame) in tail.chunks_exact_mut(2).enumerate() {
        let gain = 1.0 - ((i + 1) as f32 / UNDERFLOW_FADE_FRAMES as f32).min(1.0);
        frame[0] = last.0 * gain;
        frame[1] = last.1 * gain;
    }
}

/// Streaming linear-interpolation resampler from an arbitrary rate to 48 kHz stereo.
#[derive(Default)]
pub struct StreamResampler {
    prev: (f32, f32),
    /// Position of the next output sample, in source frames after `prev`.
    phase: f64,
    started: bool,
    rate: u32,
}

impl StreamResampler {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Resamples interleaved stereo `input` at `src_rate` into `out` (appended).
    pub fn process(&mut self, input: &[f32], src_rate: u32, out: &mut Vec<f32>) {
        let frames = input.len() / 2;
        if frames == 0 {
            return;
        }
        if src_rate == MIX_RATE {
            out.extend_from_slice(&input[..frames * 2]);
            self.reset();
            return;
        }
        if self.rate != src_rate {
            self.rate = src_rate;
            self.phase = 0.0;
        }
        if !self.started {
            self.prev = (input[0], input[1]);
            self.started = true;
        }
        let step = f64::from(src_rate) / f64::from(MIX_RATE);
        let limit = frames as f64;
        let mut pos = self.phase;
        out.reserve(((limit - pos).max(0.0) / step) as usize * 2 + 2);
        while pos < limit {
            let index = pos.floor() as usize;
            let t = (pos - index as f64) as f32;
            let a = if index == 0 {
                self.prev
            } else {
                (input[(index - 1) * 2], input[(index - 1) * 2 + 1])
            };
            let b = (input[index * 2], input[index * 2 + 1]);
            out.push(a.0 + (b.0 - a.0) * t);
            out.push(a.1 + (b.1 - a.1) * t);
            pos += step;
        }
        self.phase = pos - limit;
        self.prev = (input[frames * 2 - 2], input[frames * 2 - 1]);
    }
}

/// Validates a packet's header fields and returns `(frames, channels, rate)`.
pub fn validate(
    sample_rate: i32,
    channels: i32,
    samples_per_channel: i32,
    total_samples: usize,
) -> Result<(usize, usize, u32), PacketError> {
    if sample_rate <= 0 || sample_rate > MAX_SOURCE_RATE {
        return Err(PacketError::SampleRate(sample_rate));
    }
    if channels <= 0 || channels > MAX_SOURCE_CHANNELS {
        return Err(PacketError::Channels(channels));
    }
    let channels = channels as usize;
    let available = total_samples / channels;
    let frames = if samples_per_channel > 0 {
        (samples_per_channel as usize).min(available)
    } else {
        available
    };
    if frames == 0 {
        return Err(PacketError::Empty);
    }
    Ok((frames, channels, sample_rate as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(frames: usize, rate: f64, phase: &mut f64) -> Vec<f32> {
        (0..frames)
            .flat_map(|_| {
                let v = (*phase * std::f64::consts::TAU).sin() as f32 * 0.5;
                *phase = (*phase + 1000.0 / rate).fract();
                [v, v]
            })
            .collect()
    }

    #[test]
    fn output_count_follows_the_rate_ratio_across_packets() {
        let mut resampler = StreamResampler::default();
        let mut out = Vec::new();
        let mut phase = 0.0;
        // 44.1 kHz in 10 ms packets for ten seconds.
        for _ in 0..1000 {
            resampler.process(&ramp(441, 44_100.0, &mut phase), 44_100, &mut out);
        }
        let frames = out.len() / 2;
        assert!(
            (frames as i64 - 480_000).abs() <= 2,
            "expected ~480000 frames, got {frames}"
        );
    }

    #[test]
    fn packet_boundaries_are_continuous() {
        let mut resampler = StreamResampler::default();
        let mut phase = 0.0;
        let mut out = Vec::new();
        for _ in 0..50 {
            resampler.process(&ramp(320, 32_000.0, &mut phase), 32_000, &mut out);
        }
        let worst = out
            .chunks_exact(2)
            .zip(out.chunks_exact(2).skip(1))
            .map(|(a, b)| (a[0] - b[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 0.1, "discontinuity {worst}");
    }

    #[test]
    fn native_rate_is_passed_through() {
        let mut resampler = StreamResampler::default();
        let mut out = Vec::new();
        resampler.process(&[0.1, 0.2, 0.3, 0.4], 48_000, &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn underrun_ramps_to_silence() {
        let mut tail = vec![9.0f32; 400];
        fill_underrun(&mut tail, (1.0, -1.0));
        assert!(tail[0] > 0.9 && tail[1] < -0.9);
        for pair in tail.chunks_exact(2).skip(UNDERFLOW_FADE_FRAMES) {
            assert_eq!(pair, [0.0, 0.0]);
        }
        let mut previous = 1.0f32;
        for pair in tail.chunks_exact(2).take(UNDERFLOW_FADE_FRAMES) {
            assert!(pair[0] <= previous);
            previous = pair[0];
        }
    }

    #[test]
    fn mono_is_duplicated() {
        let mut out = Vec::new();
        planar_to_stereo(&[0.5, 0.25], 2, 1, &mut out);
        assert_eq!(out, vec![0.5, 0.5, 0.25, 0.25]);
    }

    #[test]
    fn five_one_folds_center_and_surrounds_without_clipping() {
        let frames = 1;
        // L R C LFE Ls Rs, all at full scale.
        let planar = vec![1.0f32; 6];
        let mut out = Vec::new();
        planar_to_stereo(&planar, frames, 6, &mut out);
        assert!((out[0] - 1.0).abs() < 1e-5, "left {}", out[0]);
        assert!((out[1] - 1.0).abs() < 1e-5, "right {}", out[1]);
        // LFE alone is dropped.
        let mut lfe = vec![0.0f32; 6];
        lfe[3] = 1.0;
        planar_to_stereo(&lfe, frames, 6, &mut out);
        assert_eq!(out, vec![0.0, 0.0]);
    }

    #[test]
    fn five_one_keeps_left_and_right_apart() {
        let mut planar = vec![0.0f32; 6];
        planar[0] = 1.0;
        let mut out = Vec::new();
        planar_to_stereo(&planar, 1, 6, &mut out);
        assert!(out[0] > 0.3);
        assert_eq!(out[1], 0.0);
    }

    #[test]
    fn non_finite_samples_become_silence() {
        let mut out = Vec::new();
        planar_to_stereo(&[f32::NAN, f32::INFINITY, 0.5, 0.5], 2, 2, &mut out);
        assert_eq!(out, vec![0.0, 0.5, 0.0, 0.5]);
    }

    #[test]
    fn validate_rejects_bad_headers() {
        assert_eq!(validate(0, 2, 10, 20), Err(PacketError::SampleRate(0)));
        assert_eq!(validate(48_000, 0, 10, 20), Err(PacketError::Channels(0)));
        assert_eq!(validate(48_000, 2, 10, 0), Err(PacketError::Empty));
        assert_eq!(validate(48_000, 2, 100, 20), Ok((10, 2, 48_000)));
        assert_eq!(validate(48_000, 2, 0, 20), Ok((10, 2, 48_000)));
    }
}
