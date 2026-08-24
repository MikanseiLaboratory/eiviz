use eiviz_core::{AsrcProfile, AsrcQuality};
use std::collections::VecDeque;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AsrcError {
    #[error("ASRC sample rates and channel count must be non-zero")]
    InvalidFormat,
    #[error(
        "ASRC input format changed from {expected_rate} Hz/{expected_channels} ch to {actual_rate} Hz/{actual_channels} ch"
    )]
    FormatChanged {
        expected_rate: u32,
        expected_channels: u16,
        actual_rate: u32,
        actual_channels: u16,
    },
    #[error("ASRC input planes are inconsistent with the declared channel count")]
    InvalidPlanes,
}

#[derive(Clone, Debug, Default)]
pub struct AsrcDiagnostics {
    pub input_rate: u32,
    pub output_rate: u32,
    /// Current input frames consumed per output frame.
    pub ratio: f64,
    pub drift_ppm: f64,
    pub buffered_frames: usize,
    pub buffer_capacity_frames: usize,
    pub input_frames: u64,
    pub output_frames: u64,
    pub queue_overflows: u64,
    pub queue_underflows: u64,
    pub discontinuities: u64,
}

/// Stateful, bounded, bandlimited asynchronous sample-rate converter.
///
/// The converter preserves every input channel. A Blackman-windowed sinc
/// provides anti-alias filtering, while source timestamps and queue occupancy
/// steer a bounded clock-drift servo. It is intended for non-callback media
/// threads; audio callbacks only consume/produce bounded rings.
pub struct StreamingAsrc {
    input_rate: u32,
    output_rate: u32,
    channels: u16,
    profile: AsrcProfile,
    taps: usize,
    planes: Vec<VecDeque<f32>>,
    phase: f64,
    expected_input_sample: Option<u64>,
    previous_clock: Option<(u64, u64)>,
    measured_drift_ppm: f64,
    output_fraction: f64,
    output_sample_index: u64,
    input_frames: u64,
    output_frames: u64,
    queue_overflows: u64,
    queue_underflows: u64,
    discontinuities: u64,
    reset_pending: bool,
}

impl StreamingAsrc {
    pub fn new(
        input_rate: u32,
        output_rate: u32,
        channels: u16,
        profile: AsrcProfile,
    ) -> Result<Self, AsrcError> {
        if input_rate == 0 || output_rate == 0 || channels == 0 {
            return Err(AsrcError::InvalidFormat);
        }
        let taps = match profile.quality {
            AsrcQuality::Broadcast => 32,
            AsrcQuality::Mastering => 64,
        };
        let mut converter = Self {
            input_rate,
            output_rate,
            channels,
            profile,
            taps,
            planes: vec![VecDeque::new(); channels as usize],
            phase: 0.0,
            expected_input_sample: None,
            previous_clock: None,
            measured_drift_ppm: 0.0,
            output_fraction: 0.0,
            output_sample_index: 0,
            input_frames: 0,
            output_frames: 0,
            queue_overflows: 0,
            queue_underflows: 0,
            discontinuities: 0,
            reset_pending: false,
        };
        converter.clear_filter_history();
        Ok(converter)
    }

    pub const fn input_rate(&self) -> u32 {
        self.input_rate
    }

    pub const fn output_rate(&self) -> u32 {
        self.output_rate
    }

    pub const fn channels(&self) -> u16 {
        self.channels
    }

    pub fn push(&mut self, input: &crate::AudioBuffer) -> Result<(), AsrcError> {
        if input.sample_rate != self.input_rate || input.channels != self.channels {
            return Err(AsrcError::FormatChanged {
                expected_rate: self.input_rate,
                expected_channels: self.channels,
                actual_rate: input.sample_rate,
                actual_channels: input.channels,
            });
        }
        let frames = input.planes.first().map_or(0, Vec::len);
        if input.planes.len() != self.channels as usize
            || input.planes.iter().any(|plane| plane.len() != frames)
        {
            return Err(AsrcError::InvalidPlanes);
        }
        let sample_discontinuity = self
            .expected_input_sample
            .is_some_and(|expected| expected != input.sample_index);
        if input.discontinuity || sample_discontinuity {
            self.reset_stream();
        }
        self.update_clock_estimate(input);
        self.expected_input_sample = Some(input.sample_index.saturating_add(frames as u64));
        self.input_frames = self.input_frames.saturating_add(frames as u64);
        for (queue, plane) in self.planes.iter_mut().zip(&input.planes) {
            queue.extend(plane.iter().copied());
        }
        self.enforce_bound();
        Ok(())
    }

    /// Produces exactly `frames`, inserting counted silence if source data is
    /// unavailable. This fixed-size contract matches a project media boundary.
    pub fn render(&mut self, sample_index: u64, frames: usize) -> crate::AudioBuffer {
        self.render_with_correction(sample_index, frames, 0.0)
    }

    pub fn process(
        &mut self,
        input: &crate::AudioBuffer,
        sample_index: u64,
        frames: usize,
    ) -> Result<crate::AudioBuffer, AsrcError> {
        self.push(input)?;
        Ok(self.render(sample_index, frames))
    }

    /// Converts one complete input chunk. `output_fill` is the current and
    /// target destination-ring occupancy. It allows an output adapter to track
    /// an independent hardware clock without touching its callback.
    pub fn process_chunk(
        &mut self,
        input: &crate::AudioBuffer,
        output_fill: Option<(usize, usize)>,
    ) -> Result<crate::AudioBuffer, AsrcError> {
        let input_frames = input.planes.first().map_or(0, Vec::len);
        self.push(input)?;
        let correction_ppm = output_fill.map_or(0.0, |(fill, target)| {
            if target == 0 {
                0.0
            } else {
                ((fill as f64 - target as f64) / target as f64 * self.profile.max_drift_ppm as f64)
                    .clamp(
                        -(self.profile.max_drift_ppm as f64),
                        self.profile.max_drift_ppm as f64,
                    )
            }
        });
        let output_per_input = self.output_rate as f64 / self.input_rate as f64;
        self.output_fraction +=
            input_frames as f64 * output_per_input * (1.0 - correction_ppm / 1_000_000.0);
        let frames = self.output_fraction.floor() as usize;
        self.output_fraction -= frames as f64;
        let sample_index = self.output_sample_index;
        self.output_sample_index = self.output_sample_index.saturating_add(frames as u64);
        Ok(self.render_with_correction(sample_index, frames, correction_ppm))
    }

    pub fn reset(&mut self) {
        self.reset_stream();
    }

    pub fn diagnostics(&self) -> AsrcDiagnostics {
        AsrcDiagnostics {
            input_rate: self.input_rate,
            output_rate: self.output_rate,
            ratio: self.current_ratio(0.0),
            drift_ppm: self.measured_drift_ppm,
            buffered_frames: self.buffered_frames(),
            buffer_capacity_frames: self.capacity_frames(),
            input_frames: self.input_frames,
            output_frames: self.output_frames,
            queue_overflows: self.queue_overflows,
            queue_underflows: self.queue_underflows,
            discontinuities: self.discontinuities,
        }
    }

    fn render_with_correction(
        &mut self,
        sample_index: u64,
        frames: usize,
        external_correction_ppm: f64,
    ) -> crate::AudioBuffer {
        let mut output =
            crate::AudioBuffer::silence(sample_index, self.output_rate, self.channels, frames);
        output.discontinuity = std::mem::take(&mut self.reset_pending);
        if frames == 0 {
            return output;
        }
        let step = self.current_ratio(external_correction_ppm);
        let half = self.taps / 2;
        let mut produced = 0;
        while produced < frames {
            let center = self.phase.floor() as usize;
            if center < half || center.saturating_add(half) >= self.buffered_frames() {
                self.queue_underflows = self.queue_underflows.saturating_add(1);
                break;
            }
            let cutoff = (self.output_rate as f64 / self.input_rate as f64).min(1.0) * 0.94;
            for channel in 0..self.channels as usize {
                output.planes[channel][produced] =
                    windowed_sinc(&self.planes[channel], self.phase, self.taps, cutoff);
            }
            self.phase += step;
            produced += 1;
        }
        self.output_frames = self.output_frames.saturating_add(frames as u64);
        self.discard_consumed();
        output
    }

    fn current_ratio(&self, external_correction_ppm: f64) -> f64 {
        let target = self.target_frames().max(1) as f64;
        let queue_error = (self.buffered_frames() as f64 - target) / target;
        let queue_ppm = (queue_error * self.profile.max_drift_ppm as f64).clamp(
            -(self.profile.max_drift_ppm as f64),
            self.profile.max_drift_ppm as f64,
        );
        let correction = (self.measured_drift_ppm + queue_ppm + external_correction_ppm).clamp(
            -(self.profile.max_drift_ppm as f64),
            self.profile.max_drift_ppm as f64,
        );
        self.input_rate as f64 / self.output_rate as f64 * (1.0 + correction / 1_000_000.0)
    }

    fn update_clock_estimate(&mut self, input: &crate::AudioBuffer) {
        let Some(timestamp) = input.capture_timestamp else {
            return;
        };
        if let Some((previous_sample, previous_nanos)) = self.previous_clock {
            let sample_delta = timestamp
                .device_sample_index
                .saturating_sub(previous_sample);
            let nanos_delta = timestamp.capture_nanos.saturating_sub(previous_nanos);
            if sample_delta > 0 && nanos_delta > 0 {
                let measured_rate = sample_delta as f64 * 1_000_000_000.0 / nanos_delta as f64;
                let ppm = ((measured_rate / self.input_rate as f64) - 1.0) * 1_000_000.0;
                let limit = self.profile.max_drift_ppm as f64;
                let bounded = ppm.clamp(-limit, limit);
                self.measured_drift_ppm = self.measured_drift_ppm.mul_add(0.98, bounded * 0.02);
            }
        }
        self.previous_clock = Some((timestamp.device_sample_index, timestamp.capture_nanos));
    }

    fn enforce_bound(&mut self) {
        let overflow = self
            .buffered_frames()
            .saturating_sub(self.capacity_frames());
        if overflow == 0 {
            return;
        }
        for plane in &mut self.planes {
            plane.drain(..overflow);
        }
        self.phase = (self.phase - overflow as f64).max(self.taps as f64 / 2.0);
        self.queue_overflows = self.queue_overflows.saturating_add(1);
        self.discontinuities = self.discontinuities.saturating_add(1);
        self.reset_pending = true;
    }

    fn discard_consumed(&mut self) {
        let keep = self.taps / 2;
        let discard = (self.phase.floor() as usize).saturating_sub(keep);
        if discard == 0 {
            return;
        }
        for plane in &mut self.planes {
            plane.drain(..discard.min(plane.len()));
        }
        self.phase -= discard as f64;
    }

    fn reset_stream(&mut self) {
        self.discontinuities = self.discontinuities.saturating_add(1);
        self.expected_input_sample = None;
        self.previous_clock = None;
        self.measured_drift_ppm = 0.0;
        self.reset_pending = true;
        self.clear_filter_history();
    }

    fn clear_filter_history(&mut self) {
        let half = self.taps / 2;
        for plane in &mut self.planes {
            plane.clear();
            plane.resize(half, 0.0);
        }
        self.phase = half as f64;
    }

    fn buffered_frames(&self) -> usize {
        self.planes.first().map_or(0, VecDeque::len)
    }

    fn target_frames(&self) -> usize {
        (self.input_rate as u64 * self.profile.target_latency_ms as u64 / 1_000) as usize
    }

    fn capacity_frames(&self) -> usize {
        (self.input_rate as u64 * self.profile.max_buffer_ms as u64 / 1_000) as usize + self.taps
    }
}

fn windowed_sinc(samples: &VecDeque<f32>, position: f64, taps: usize, cutoff: f64) -> f32 {
    let start = position.floor() as isize - taps as isize / 2 + 1;
    let mut weighted = 0.0;
    let mut weight_sum = 0.0;
    for tap in 0..taps {
        let index = start + tap as isize;
        if index < 0 {
            continue;
        }
        let distance = index as f64 - position;
        let x = std::f64::consts::PI * distance * cutoff;
        let sinc = if x.abs() < 1.0e-12 { 1.0 } else { x.sin() / x };
        let phase = 2.0 * std::f64::consts::PI * tap as f64 / (taps - 1) as f64;
        let window = 0.42 - 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos();
        let weight = sinc * window * cutoff;
        weighted += samples.get(index as usize).copied().unwrap_or(0.0) as f64 * weight;
        weight_sum += weight;
    }
    if weight_sum.abs() < 1.0e-12 {
        0.0
    } else {
        (weighted / weight_sum) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioBuffer, AudioCaptureTimestamp};

    fn tone(rate: u32, start: u64, frames: usize, channels: u16) -> AudioBuffer {
        let mut audio = AudioBuffer::silence(start, rate, channels, frames);
        for frame in 0..frames {
            let phase =
                2.0 * std::f64::consts::PI * 997.0 * (start + frame as u64) as f64 / rate as f64;
            audio.planes[0][frame] = (phase.sin() * 0.7) as f32;
            for channel in 1..channels as usize {
                audio.planes[channel][frame] = (phase.sin() * (0.7 / (channel + 1) as f64)) as f32;
            }
        }
        audio
    }

    fn convert_one_second(input_rate: u32, output_rate: u32) -> Vec<f32> {
        let mut converter =
            StreamingAsrc::new(input_rate, output_rate, 2, AsrcProfile::mastering()).unwrap();
        let mut output = Vec::new();
        let chunk = 100;
        let mut start = 0;
        while start < input_rate as usize {
            let frames = chunk.min(input_rate as usize - start);
            let converted = converter
                .process_chunk(&tone(input_rate, start as u64, frames, 2), None)
                .unwrap();
            assert_eq!(converted.channels, 2);
            assert_eq!(converted.planes.len(), 2);
            output.extend_from_slice(&converted.planes[0]);
            start += frames;
        }
        assert_eq!(converter.diagnostics().output_frames, output_rate as u64);
        output
    }

    #[test]
    fn deterministic_44k1_and_48k_duration_and_tone_quality() {
        for (input_rate, output_rate) in [(44_100, 48_000), (48_000, 44_100)] {
            let output = convert_one_second(input_rate, output_rate);
            assert_eq!(output.len(), output_rate as usize);
            let trim = 256.min(output.len() / 8);
            let body = &output[trim..output.len() - trim];
            let rms = (body
                .iter()
                .map(|sample| f64::from(*sample).powi(2))
                .sum::<f64>()
                / body.len() as f64)
                .sqrt();
            assert!((0.42..0.56).contains(&rms), "unexpected RMS {rms}");
            let rising_crossings = body
                .windows(2)
                .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
                .count();
            let duration = body.len() as f64 / output_rate as f64;
            let measured_hz = rising_crossings as f64 / duration;
            assert!(
                (measured_hz - 997.0).abs() < 3.0,
                "unexpected tone frequency {measured_hz}"
            );
        }
    }

    #[test]
    fn discontinuity_resets_filter_and_clock_state() {
        let mut converter =
            StreamingAsrc::new(44_100, 48_000, 2, AsrcProfile::broadcast()).unwrap();
        let mut first = tone(44_100, 0, 1_000, 2);
        first.capture_timestamp = Some(AudioCaptureTimestamp {
            device_sample_index: 0,
            callback_nanos: 0,
            capture_nanos: 0,
        });
        converter.push(&first).unwrap();
        let _ = converter.render(0, 500);
        let mut jumped = tone(44_100, 9_000, 1_000, 2);
        jumped.discontinuity = true;
        jumped.capture_timestamp = Some(AudioCaptureTimestamp {
            device_sample_index: 9_000,
            callback_nanos: 1_000_000,
            capture_nanos: 1_000_000,
        });
        let output = converter.process(&jumped, 500, 500).unwrap();
        assert!(output.discontinuity);
        assert!(converter.diagnostics().discontinuities >= 1);
        assert_eq!(converter.diagnostics().drift_ppm, 0.0);
    }

    #[test]
    fn buffer_is_bounded_and_overflow_is_visible() {
        let profile = AsrcProfile {
            max_buffer_ms: 50,
            target_latency_ms: 10,
            ..AsrcProfile::broadcast()
        };
        let mut converter = StreamingAsrc::new(48_000, 44_100, 1, profile).unwrap();
        converter.push(&tone(48_000, 0, 10_000, 1)).unwrap();
        let diagnostics = converter.diagnostics();
        assert!(diagnostics.queue_overflows > 0);
        assert!(diagnostics.buffered_frames <= diagnostics.buffer_capacity_frames);
    }
}
