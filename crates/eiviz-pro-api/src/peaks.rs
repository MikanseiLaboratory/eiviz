//! Recorded-audio peaks.
//!
//! The replay Event List draws a waveform from these, so choosing a quiet In/Out
//! point never has to read PCM back from disk. Both the mixer (live display) and
//! the Pro module (persistence) use this one implementation.

use std::collections::VecDeque;

/// Buckets per second the waveform is stored at. Twenty keeps 50 ms of detail,
/// which is fine enough to spot a quiet gap between words.
pub const PEAK_BUCKETS_PER_SEC: u32 = 20;

/// Ten minutes of buckets is 12k entries of two `f32` per angle.
const MAX_BUCKETS: usize = (PEAK_BUCKETS_PER_SEC as usize) * 60 * 10;

pub struct PeakAccumulator {
    bucket_frames: usize,
    filled: usize,
    min: f32,
    max: f32,
    buckets: VecDeque<(f32, f32)>,
    /// Buckets dropped from the front once the ring is full. The host adds this
    /// to an index to recover the absolute bucket number.
    dropped: u32,
}

impl PeakAccumulator {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            bucket_frames: (sample_rate as usize / PEAK_BUCKETS_PER_SEC as usize).max(1),
            filled: 0,
            min: f32::MAX,
            max: f32::MIN,
            buckets: VecDeque::new(),
            dropped: 0,
        }
    }

    /// Adds interleaved samples. Channels are averaged into one lane so the
    /// waveform shows what the mix sounds like, not one side of it.
    pub fn push(&mut self, pcm: &[f32], channels: usize) {
        let channels = channels.max(1);
        let mut index = 0;
        while index + channels <= pcm.len() {
            let mut sum = 0.0;
            for lane in 0..channels {
                sum += pcm[index + lane];
            }
            let value = sum / channels as f32;
            index += channels;
            self.min = self.min.min(value);
            self.max = self.max.max(value);
            self.filled += 1;
            if self.filled >= self.bucket_frames {
                self.close_bucket();
            }
        }
    }

    /// Flushes the partly filled bucket. Used when recording stops so the last
    /// fractions of a second are still visible.
    pub fn flush(&mut self) {
        if self.filled > 0 {
            self.close_bucket();
        }
    }

    fn close_bucket(&mut self) {
        let min = if self.min == f32::MAX { 0.0 } else { self.min };
        let max = if self.max == f32::MIN { 0.0 } else { self.max };
        self.buckets.push_back((min, max));
        while self.buckets.len() > MAX_BUCKETS {
            self.buckets.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.filled = 0;
        self.min = f32::MAX;
        self.max = f32::MIN;
    }

    pub fn bucket_count(&self) -> u32 {
        self.buckets.len() as u32
    }

    /// Buckets evicted from the front because the ring filled up.
    pub fn dropped_buckets(&self) -> u32 {
        self.dropped
    }

    /// Copies buckets starting at `from` into the caller's arrays. Returns how
    /// many were written.
    pub fn copy(&self, from: u32, min_out: &mut [f32], max_out: &mut [f32]) -> usize {
        let capacity = min_out.len().min(max_out.len());
        let mut copied = 0;
        for (min, max) in self.buckets.iter().skip(from as usize) {
            if copied >= capacity {
                break;
            }
            min_out[copied] = *min;
            max_out[copied] = *max;
            copied += 1;
        }
        copied
    }

    /// Every bucket already closed, oldest first. Used when writing the sidecar.
    pub fn buckets(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.buckets.iter().copied()
    }

    /// Replaces the ring from a sidecar, so a reopened recording shows the same
    /// waveform it did while it was recorded.
    pub fn load(&mut self, dropped: u32, buckets: impl Iterator<Item = (f32, f32)>) {
        self.buckets.clear();
        self.dropped = dropped;
        for bucket in buckets {
            self.buckets.push_back(bucket);
        }
        while self.buckets.len() > MAX_BUCKETS {
            self.buckets.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.filled = 0;
        self.min = f32::MAX;
        self.max = f32::MIN;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// One bucket worth of interleaved stereo samples.
    fn bucket_samples(value: f32) -> Vec<f32> {
        let frames = RATE as usize / PEAK_BUCKETS_PER_SEC as usize;
        let mut samples = Vec::with_capacity(frames * 2);
        for _ in 0..frames {
            samples.push(value);
            samples.push(value);
        }
        samples
    }

    #[test]
    fn buckets_track_min_and_max_and_flush_the_last_fraction() {
        let mut peaks = PeakAccumulator::new(RATE);
        peaks.push(&bucket_samples(0.5), 2);
        assert_eq!(peaks.bucket_count(), 1);
        // A half bucket is only visible after the flush.
        let half = bucket_samples(0.25);
        peaks.push(&half[..half.len() / 2], 2);
        assert_eq!(peaks.bucket_count(), 1);
        peaks.flush();
        assert_eq!(peaks.bucket_count(), 2);

        let mut min = [0.0f32; 4];
        let mut max = [0.0f32; 4];
        assert_eq!(peaks.copy(0, &mut min, &mut max), 2);
        assert!((max[0] - 0.5).abs() < 1e-6);
        assert!((max[1] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn a_partial_block_does_not_shift_the_bucket_grid() {
        let mut peaks = PeakAccumulator::new(RATE);
        let block = bucket_samples(0.8);
        let mut offset = 0;
        for size in [37usize, 101, 512, 2048] {
            let end = (offset + size).min(block.len());
            peaks.push(&block[offset..end], 2);
            offset = end;
        }
        peaks.push(&block[offset..], 2);
        peaks.flush();
        assert_eq!(
            peaks.bucket_count(),
            1,
            "one bucket of samples is one bucket"
        );
        let mut min = [0.0f32; 1];
        let mut max = [0.0f32; 1];
        assert_eq!(peaks.copy(0, &mut min, &mut max), 1);
        assert!((max[0] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn the_ring_keeps_the_newest_buckets_and_counts_what_it_dropped() {
        let mut peaks = PeakAccumulator::new(RATE);
        for index in 0..(MAX_BUCKETS + 5) {
            let value = if index % 2 == 0 { 0.1 } else { 0.2 };
            peaks.push(&bucket_samples(value), 2);
        }
        peaks.flush();
        assert_eq!(peaks.bucket_count() as usize, MAX_BUCKETS);
        assert_eq!(peaks.dropped_buckets(), 5);
    }

    #[test]
    fn silence_reads_as_zero_not_as_an_empty_bucket() {
        let mut peaks = PeakAccumulator::new(RATE);
        peaks.push(&bucket_samples(0.0), 2);
        let mut min = [1.0f32; 1];
        let mut max = [1.0f32; 1];
        assert_eq!(peaks.copy(0, &mut min, &mut max), 1);
        assert_eq!(min[0], 0.0);
        assert_eq!(max[0], 0.0);
    }

    #[test]
    fn a_sidecar_reload_shows_the_same_buckets() {
        let mut peaks = PeakAccumulator::new(RATE);
        peaks.push(&bucket_samples(0.4), 2);
        peaks.push(&bucket_samples(0.9), 2);
        let saved: Vec<(f32, f32)> = peaks.buckets().collect();
        assert_eq!(saved.len(), 2);

        let mut restored = PeakAccumulator::new(RATE);
        restored.load(7, saved.iter().copied());
        assert_eq!(restored.bucket_count(), 2);
        assert_eq!(restored.dropped_buckets(), 7);
        let mut min = [0.0f32; 2];
        let mut max = [0.0f32; 2];
        assert_eq!(restored.copy(0, &mut min, &mut max), 2);
        assert!((max[0] - 0.4).abs() < 1e-6);
        assert!((max[1] - 0.9).abs() < 1e-6);
    }

    #[test]
    fn mono_and_stereo_agree_on_the_loudest_sample() {
        let mut stereo = PeakAccumulator::new(RATE);
        let mut mono = PeakAccumulator::new(RATE);
        let frames = RATE as usize / PEAK_BUCKETS_PER_SEC as usize;
        let mut interleaved = Vec::with_capacity(frames * 2);
        let mut single = Vec::with_capacity(frames);
        for index in 0..frames {
            let value = if index % 3 == 0 { 0.75 } else { -0.25 };
            interleaved.push(value);
            interleaved.push(value);
            single.push(value);
        }
        stereo.push(&interleaved, 2);
        mono.push(&single, 1);
        let mut min_a = [0.0f32; 1];
        let mut max_a = [0.0f32; 1];
        let mut min_b = [0.0f32; 1];
        let mut max_b = [0.0f32; 1];
        stereo.copy(0, &mut min_a, &mut max_a);
        mono.copy(0, &mut min_b, &mut max_b);
        assert!((max_a[0] - max_b[0]).abs() < 1e-6);
        assert!((min_a[0] - min_b[0]).abs() < 1e-6);
    }
}
