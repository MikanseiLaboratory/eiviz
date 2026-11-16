//! Capture hand-off from a device callback to the mixer's input store.
//!
//! The callback maps the requested device channels to stereo and pushes them into a
//! lock-free ring. A regular (non-real-time) thread drains the ring into the shared
//! `AudioInputStore`, which is where locks and allocation are acceptable.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::guard::LockExt;
use crate::upload::AudioInputStore;

use super::pcm::interleaved_f32_packet;
use super::rt::SpscF32;

/// Roughly one second of 192 kHz stereo.
const FEED_SAMPLES: usize = 192_000 * 2;

pub struct CaptureFeed {
    ring: SpscF32,
    rate: AtomicU32,
    dropped: AtomicU64,
}

impl CaptureFeed {
    pub fn new(rate: u32) -> Self {
        Self {
            ring: SpscF32::new(FEED_SAMPLES),
            rate: AtomicU32::new(rate.max(1)),
            dropped: AtomicU64::new(0),
        }
    }

    pub fn set_rate(&self, rate: u32) {
        self.rate.store(rate.max(1), Ordering::Relaxed);
    }

    /// Callback side: extracts `map_left`/`map_right` from `interleaved` (`channels` wide)
    /// into `scratch` and queues the stereo pairs. Allocation-free once `scratch` has grown.
    pub fn push_mapped(
        &self,
        interleaved: &[f32],
        channels: usize,
        map_left: usize,
        map_right: usize,
        scratch: &mut Vec<f32>,
    ) {
        let channels = channels.max(1);
        let frames = interleaved.len() / channels;
        if frames == 0 {
            return;
        }
        scratch.clear();
        scratch.reserve(frames * 2);
        for frame in interleaved.chunks_exact(channels) {
            scratch.push(frame.get(map_left).copied().unwrap_or(0.0));
            scratch.push(frame.get(map_right).copied().unwrap_or(0.0));
        }
        if self.ring.push_slice(scratch) < scratch.len() {
            let count = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if count.is_power_of_two() {
                crate::diag::warn(&format!(
                    "audio capture feed overrun (drain thread stalled), {count} dropped blocks so far"
                ));
            }
        }
    }

    /// Consumer side: moves queued audio into the input store. `pts` is advanced by the
    /// duration of what was moved.
    pub fn drain(&self, uploads: &Mutex<AudioInputStore>, id: u64, pts: &mut i64) {
        let queued = self.ring.len() & !1;
        if queued == 0 {
            return;
        }
        let mut stereo = vec![0.0f32; queued];
        let got = self.ring.pop_slice(&mut stereo) & !1;
        stereo.truncate(got);
        if got == 0 {
            return;
        }
        let rate = self.rate.load(Ordering::Relaxed);
        let packet = interleaved_f32_packet(*pts, rate as i32, &stereo, 2, 0, 1);
        let frames = i64::from(packet.samples_per_channel.max(0));
        uploads.lock_or_recover().ingest_audio(id, packet);
        *pts = pts.saturating_add(frames * 10_000_000 / i64::from(rate.max(1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_requested_channels_to_stereo() {
        let feed = CaptureFeed::new(48_000);
        let mut scratch = Vec::new();
        // Two frames of 4 channels: pick channels 3 and 1.
        feed.push_mapped(
            &[0.0, 0.1, 0.2, 0.3, 1.0, 1.1, 1.2, 1.3],
            4,
            3,
            1,
            &mut scratch,
        );
        let mut out = [0.0f32; 4];
        assert_eq!(feed.ring.pop_slice(&mut out), 4);
        assert_eq!(out, [0.3, 0.1, 1.3, 1.1]);
    }

    #[test]
    fn out_of_range_channels_are_silent() {
        let feed = CaptureFeed::new(48_000);
        let mut scratch = Vec::new();
        feed.push_mapped(&[0.5, 0.5], 2, 5, 0, &mut scratch);
        let mut out = [9.0f32; 2];
        feed.ring.pop_slice(&mut out);
        assert_eq!(out, [0.0, 0.5]);
    }
}
