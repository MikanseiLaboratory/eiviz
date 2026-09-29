//! Device-side audio pump: pulls bus rings at the device rate.
//!
//! This is the body of every output callback. It is allocation-free in steady state (the
//! scratch buffers only grow when the driver raises its buffer size), lock-free and keeps
//! per-route resampler state across callbacks so that block boundaries are inaudible and
//! the small clock difference between the mixer and the device is absorbed by a slow
//! ring-fill controller instead of glitches.

use std::sync::Arc;

use crate::upload::AUDIO_RATE;

const MIX_RATE: u32 = AUDIO_RATE as u32;

use super::AUDIO_PRIME_FRAMES;
use super::graph::BusRing;
use super::rt::{Published, PublishedReader};

/// A bus routed to two device channels.
pub type Route = (Arc<BusRing>, i32, i32);

/// Ring fill (in mixer frames) the drift controller steers towards.
const TARGET_FILL_FRAMES: f64 = (AUDIO_PRIME_FRAMES * 2) as f64;
/// Above this fill the consumer drops audio instead of carrying the latency forever.
const MAX_FILL_FRAMES: usize = AUDIO_PRIME_FRAMES * 3;
/// Largest speed correction applied by the controller (0.1 %).
const MAX_DRIFT: f64 = 0.001;
const FILL_SMOOTHING: f64 = 0.02;
const KP: f64 = 2.0e-7;
const KI: f64 = 2.0e-10;

/// Streaming linear-interpolation resampler for one bus with fill-level drift control.
pub struct RouteResampler {
    a: (f32, f32),
    b: (f32, f32),
    frac: f64,
    fill_avg: f64,
    integral: f64,
    seeded: bool,
}

impl RouteResampler {
    pub fn new() -> Self {
        Self {
            a: (0.0, 0.0),
            b: (0.0, 0.0),
            frac: 0.0,
            fill_avg: TARGET_FILL_FRAMES,
            integral: 0.0,
            seeded: false,
        }
    }

    fn drift(&mut self, ring: &BusRing) -> f64 {
        if !ring.is_primed() {
            self.seeded = false;
            return 0.0;
        }
        if !self.seeded {
            // A ring that filled up while nobody was listening carries stale audio.
            ring.trim_to(TARGET_FILL_FRAMES as usize);
            self.fill_avg = ring.fill_frames() as f64;
            self.integral = 0.0;
            self.seeded = true;
        }
        if ring.fill_frames() > MAX_FILL_FRAMES {
            ring.trim_to(TARGET_FILL_FRAMES as usize);
        }
        let fill = ring.fill_frames() as f64;
        self.fill_avg += FILL_SMOOTHING * (fill - self.fill_avg);
        let error = self.fill_avg - TARGET_FILL_FRAMES;
        self.integral = (self.integral + KI * error).clamp(-MAX_DRIFT / 2.0, MAX_DRIFT / 2.0);
        (KP * error + self.integral).clamp(-MAX_DRIFT, MAX_DRIFT)
    }

    /// Renders `out.len() / 2` device frames of interleaved stereo.
    pub fn render(&mut self, ring: &BusRing, dst_rate: u32, src: &mut Vec<f32>, out: &mut [f32]) {
        let dst_frames = out.len() / 2;
        if dst_frames == 0 {
            return;
        }
        let step = f64::from(MIX_RATE) / f64::from(dst_rate.max(1)) * (1.0 + self.drift(ring));

        let mut probe = self.frac;
        let mut needed = 0usize;
        for _ in 0..dst_frames {
            probe += step;
            while probe >= 1.0 {
                probe -= 1.0;
                needed += 1;
            }
        }
        if src.len() < needed * 2 {
            src.resize(needed * 2, 0.0);
        }
        ring.pop_into(&mut src[..needed * 2]);

        let mut next = 0usize;
        for frame in out[..dst_frames * 2].chunks_exact_mut(2) {
            self.frac += step;
            while self.frac >= 1.0 {
                self.frac -= 1.0;
                self.a = self.b;
                self.b = (src[next * 2], src[next * 2 + 1]);
                next += 1;
            }
            let t = self.frac as f32;
            frame[0] = self.a.0 + (self.b.0 - self.a.0) * t;
            frame[1] = self.a.1 + (self.b.1 - self.a.1) * t;
        }
    }
}

struct RouteState {
    key: usize,
    resampler: RouteResampler,
}

/// Owns everything an output callback needs; created per stream.
pub struct OutputPump {
    routes: PublishedReader<Vec<Route>>,
    states: Vec<RouteState>,
    src: Vec<f32>,
    stereo: Vec<f32>,
}

impl OutputPump {
    pub fn new() -> Self {
        Self {
            routes: PublishedReader::new(),
            states: Vec::new(),
            src: Vec::new(),
            stereo: Vec::new(),
        }
    }

    /// Mixes all routes into `dest` (interleaved, `channels` wide) at `rate`.
    pub fn render(
        &mut self,
        shared: &Published<Vec<Route>>,
        dest: &mut [f32],
        channels: usize,
        rate: u32,
    ) {
        dest.fill(0.0);
        let channels = channels.max(1);
        let (routes, refreshed) = self.routes.get(shared);
        if refreshed {
            let mut states = Vec::with_capacity(routes.len());
            for (ring, ..) in routes {
                let key = Arc::as_ptr(ring) as usize;
                if states.iter().any(|state: &RouteState| state.key == key) {
                    continue;
                }
                let state = match self.states.iter().position(|state| state.key == key) {
                    Some(index) => self.states.swap_remove(index),
                    None => RouteState {
                        key,
                        resampler: RouteResampler::new(),
                    },
                };
                states.push(state);
            }
            self.states = states;
        }
        let frames = dest.len() / channels;
        if self.stereo.len() < frames * 2 {
            self.stereo.resize(frames * 2, 0.0);
        }
        for (ring, left, right) in routes {
            let key = Arc::as_ptr(ring) as usize;
            let Some(state) = self.states.iter_mut().find(|state| state.key == key) else {
                continue;
            };
            state
                .resampler
                .render(ring, rate, &mut self.src, &mut self.stereo[..frames * 2]);
            let map_left = (*left).max(0) as usize;
            let map_right = (*right).max(0) as usize;
            for (i, pair) in self.stereo[..frames * 2].chunks_exact(2).enumerate() {
                let base = i * channels;
                if map_left < channels {
                    dest[base + map_left] += finite(pair[0]);
                }
                if map_right != map_left && map_right < channels {
                    dest[base + map_right] += finite(pair[1]);
                }
            }
        }
        for sample in dest.iter_mut() {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frames: usize, phase: &mut f64, rate: f64) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames * 2);
        for _ in 0..frames {
            let v = (*phase * std::f64::consts::TAU).sin() as f32 * 0.5;
            out.push(v);
            out.push(v);
            *phase = (*phase + 1000.0 / rate).fract();
        }
        out
    }

    fn prime(ring: &BusRing, phase: &mut f64) {
        ring.push(&sine(AUDIO_PRIME_FRAMES * 2, phase, f64::from(MIX_RATE)));
    }

    #[test]
    fn same_rate_passes_signal_without_gaps() {
        let ring = BusRing::new();
        let mut phase = 0.0;
        prime(&ring, &mut phase);
        let mut resampler = RouteResampler::new();
        let mut scratch = Vec::new();
        let mut out = vec![0.0f32; 480 * 2];
        let mut previous = None::<f32>;
        let mut worst = 0.0f32;
        for _ in 0..40 {
            ring.push(&sine(480, &mut phase, f64::from(MIX_RATE)));
            resampler.render(&ring, MIX_RATE, &mut scratch, &mut out);
            for frame in out.chunks_exact(2) {
                if let Some(prev) = previous {
                    worst = worst.max((frame[0] - prev).abs());
                }
                previous = Some(frame[0]);
            }
        }
        // 1 kHz at 0.5 amplitude never moves more than ~0.07 per sample at 48 kHz.
        assert!(worst < 0.1, "discontinuity {worst}");
    }

    #[test]
    fn block_boundaries_are_seamless_when_resampling() {
        let ring = BusRing::new();
        let mut phase = 0.0;
        prime(&ring, &mut phase);
        let mut resampler = RouteResampler::new();
        let mut scratch = Vec::new();
        let mut previous = None::<f32>;
        let mut worst = 0.0f32;
        // 44.1 kHz callbacks of an odd size so the fractional position matters.
        for _ in 0..200 {
            ring.push(&sine(480, &mut phase, f64::from(MIX_RATE)));
            let mut out = vec![0.0f32; 441 * 2];
            resampler.render(&ring, 44_100, &mut scratch, &mut out);
            for frame in out.chunks_exact(2) {
                if let Some(prev) = previous {
                    worst = worst.max((frame[0] - prev).abs());
                }
                previous = Some(frame[0]);
            }
        }
        assert!(worst < 0.1, "discontinuity {worst}");
    }

    #[test]
    fn drift_controller_drains_an_overfull_ring() {
        let ring = BusRing::new();
        let mut phase = 0.0;
        ring.push(&sine(AUDIO_PRIME_FRAMES * 2 + 700, &mut phase, 48_000.0));
        let mut resampler = RouteResampler::new();
        let mut scratch = Vec::new();
        let mut out = vec![0.0f32; 480 * 2];
        // Producer runs 0.05 % slow relative to the device: fill must not run away or dry up.
        let mut residue = 0.0f64;
        let mut min_fill = usize::MAX;
        for round in 0..4000 {
            residue += 480.0 * 0.9995;
            let frames = residue as usize;
            residue -= frames as f64;
            ring.push(&sine(frames, &mut phase, 48_000.0));
            resampler.render(&ring, MIX_RATE, &mut scratch, &mut out);
            if round > 2000 {
                min_fill = min_fill.min(ring.fill_frames());
            }
        }
        assert!(min_fill > 0, "ring ran dry");
        assert!(ring.fill_frames() < MAX_FILL_FRAMES);
    }

    #[test]
    fn stale_ring_is_trimmed_on_first_pull() {
        let ring = BusRing::new();
        let mut phase = 0.0;
        ring.push(&sine(3500, &mut phase, 48_000.0));
        let mut resampler = RouteResampler::new();
        let mut scratch = Vec::new();
        let mut out = vec![0.0f32; 480 * 2];
        resampler.render(&ring, MIX_RATE, &mut scratch, &mut out);
        assert!(ring.fill_frames() <= TARGET_FILL_FRAMES as usize);
    }

    #[test]
    fn underrun_fades_to_silence_and_reprimes() {
        let ring = BusRing::new();
        ring.push(&vec![0.5f32; AUDIO_PRIME_FRAMES * 2]);
        let mut out = vec![9.0f32; AUDIO_PRIME_FRAMES * 4];
        ring.pop_into(&mut out);
        let starved = AUDIO_PRIME_FRAMES;
        assert!((out[starved * 2 - 2] - 0.5).abs() < 1e-6);
        assert!(out[starved * 2] < 0.5, "tail must ramp down");
        assert_eq!(out[out.len() - 1], 0.0);
        assert!(!ring.is_primed());
        // Below the priming level the ring stays silent.
        ring.push(&vec![0.5f32; 100 * 2]);
        let mut quiet = vec![9.0f32; 64];
        ring.pop_into(&mut quiet);
        assert!(quiet.iter().all(|v| *v == 0.0));
        ring.push(&vec![0.5f32; AUDIO_PRIME_FRAMES * 2]);
        assert!(ring.is_primed());
    }

    #[test]
    fn pump_maps_routes_to_device_channels() {
        let ring = BusRing::new();
        let mut phase = 0.0;
        ring.push(&vec![0.25f32; AUDIO_PRIME_FRAMES * 4]);
        let _ = &mut phase;
        let shared = Published::new(vec![(Arc::clone(&ring), 2, 3)]);
        let mut pump = OutputPump::new();
        let mut dest = vec![0.0f32; 64 * 4];
        pump.render(&shared, &mut dest, 4, MIX_RATE);
        let tail = &dest[32 * 4..];
        for frame in tail.chunks_exact(4) {
            assert_eq!(frame[0], 0.0);
            assert_eq!(frame[1], 0.0);
            assert!((frame[2] - 0.25).abs() < 1e-3);
            assert!((frame[3] - 0.25).abs() < 1e-3);
        }
    }

    #[test]
    fn pump_follows_route_updates() {
        let a = BusRing::new();
        let b = BusRing::new();
        a.push(&vec![0.5f32; AUDIO_PRIME_FRAMES * 4]);
        b.push(&vec![-0.5f32; AUDIO_PRIME_FRAMES * 4]);
        let shared = Published::new(vec![(Arc::clone(&a), 0, 1)]);
        let mut pump = OutputPump::new();
        let mut dest = vec![0.0f32; 64 * 2];
        pump.render(&shared, &mut dest, 2, MIX_RATE);
        assert!(dest[100] > 0.4);
        shared.set(vec![(Arc::clone(&b), 0, 1)]);
        pump.render(&shared, &mut dest, 2, MIX_RATE);
        assert!(dest[100] < -0.4);
    }

    #[test]
    fn non_finite_samples_are_silenced() {
        let ring = BusRing::new();
        ring.push(&vec![f32::NAN; AUDIO_PRIME_FRAMES * 4]);
        let shared = Published::new(vec![(Arc::clone(&ring), 0, 1)]);
        let mut pump = OutputPump::new();
        let mut dest = vec![1.0f32; 32 * 2];
        pump.render(&shared, &mut dest, 2, MIX_RATE);
        assert!(dest.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    }
}
