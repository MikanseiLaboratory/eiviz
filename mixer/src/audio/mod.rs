#[cfg(windows)]
mod asio;
#[cfg(target_os = "macos")]
mod coreaudio;
#[cfg(windows)]
mod device;
mod graph;
mod info;
#[cfg(windows)]
mod wasapi;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::abi::OverlayDesc;
use crate::upload::{AUDIO_RATE, UploadStore};

#[cfg_attr(not(windows), allow(unused_imports))]
pub use graph::{
    AudioGraph, BusRing, DEVICE_ASIO, DEVICE_COREAUDIO, DEVICE_NONE, DEVICE_WASAPI, LINK_FOLLOW,
    MASTER_BUS, MixedAudio,
};
pub use info::{AudioBusInfo, AudioDeviceInfo};

pub(crate) const AUDIO_PRIME_FRAMES: usize = AUDIO_RATE as usize / 50;

pub struct AudioDelay {
    delay_frames: usize,
    fifos: HashMap<u64, VecDeque<f32>>,
    last: HashMap<u64, (f32, f32)>,
}

impl AudioDelay {
    pub fn new() -> Self {
        Self {
            delay_frames: 0,
            fifos: HashMap::new(),
            last: HashMap::new(),
        }
    }

    pub fn set_delay_frames(&mut self, frames: usize) {
        self.delay_frames = frames;
        let cap = frames
            .saturating_mul(2)
            .saturating_add((AUDIO_RATE as usize / 5) * 2);
        for fifo in self.fifos.values_mut() {
            while fifo.len() > cap {
                fifo.pop_front();
            }
        }
    }

    pub fn push(&mut self, id: u64, pcm: &[f32]) {
        let fifo = self.fifos.entry(id).or_default();
        fifo.extend(pcm.iter().copied());
        let cap = self
            .delay_frames
            .saturating_mul(2)
            .saturating_add((AUDIO_RATE as usize / 5) * 2);
        while fifo.len() > cap {
            fifo.pop_front();
        }
    }

    pub fn pop(&mut self, id: u64, frames: usize, drain: bool) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames.saturating_mul(2));
        let delay = self.delay_frames;
        let fifo = self.fifos.entry(id).or_default();
        for _ in 0..frames {
            let queued = fifo.len() / 2;
            let keep = if drain { 0 } else { delay };
            if queued > keep && fifo.len() >= 2 {
                let left = fifo.pop_front().unwrap_or(0.0);
                let right = fifo.pop_front().unwrap_or(left);
                self.last.insert(id, (left, right));
                out.push(left);
                out.push(right);
            } else {
                let (left, right) = self.last.get(&id).copied().unwrap_or((0.0, 0.0));
                out.push(left);
                out.push(right);
            }
        }
        out
    }

    pub fn skip_frames(&mut self, frames: usize) {
        let n = frames.saturating_mul(2);
        for fifo in self.fifos.values_mut() {
            let drop = n.min(fifo.len());
            if drop > 0 {
                fifo.drain(..drop);
            }
        }
    }
}

#[derive(Clone)]
pub struct AudioEngine {
    graph: Arc<Mutex<AudioGraph>>,
    outputs: Arc<Mutex<Vec<DeviceOutput>>>,
    delay: Arc<Mutex<AudioDelay>>,
}

struct DeviceOutput {
    key: DeviceKey,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct DeviceKey {
    pub kind: u32,
    pub id: String,
    pub exclusive: bool,
}

impl AudioEngine {
    pub fn new() -> Self {
        let engine = Self {
            graph: Arc::new(Mutex::new(AudioGraph::with_defaults())),
            outputs: Arc::new(Mutex::new(Vec::new())),
            delay: Arc::new(Mutex::new(AudioDelay::new())),
        };
        engine.sync_outputs();
        engine
    }

    pub fn graph(&self) -> Arc<Mutex<AudioGraph>> {
        Arc::clone(&self.graph)
    }

    pub fn shutdown(&self) {
        let joins = {
            let mut outputs = self.outputs.lock().expect("audio outputs");
            let mut joins = Vec::new();
            for output in outputs.iter_mut() {
                output.stop.store(true, Ordering::Relaxed);
                if let Some(join) = output.join.take() {
                    joins.push(join);
                }
            }
            outputs.clear();
            joins
        };
        // Detach joins. HAL Start/Stop can block forever on some machines;
        // freezing mixer_destroy (and the AppKit main thread) is worse.
        for join in joins {
            let _ = std::thread::Builder::new()
                .name("eiviz-audio-join".into())
                .spawn(move || {
                    let _ = join.join();
                });
        }
    }

    pub fn upsert_bus(
        &self,
        id: u64,
        name: &str,
        role: u32,
        device_kind: u32,
        device_id: &str,
        map_left: i32,
        map_right: i32,
        exclusive: u32,
    ) {
        self.graph.lock().expect("audio").upsert_bus(
            id,
            name,
            role,
            device_kind,
            device_id,
            map_left,
            map_right,
            exclusive != 0,
        );
        self.sync_outputs();
    }

    pub fn remove_bus(&self, id: u64) {
        self.graph.lock().expect("audio").remove_bus(id);
        self.sync_outputs();
    }

    pub fn set_input(&self, id: u64, bus_mask: u32, gain: f32, mute: u32) {
        self.graph
            .lock()
            .expect("audio")
            .set_input(id, bus_mask, gain, mute != 0);
    }

    pub fn set_bus_gain(&self, id: u64, gain: f32, mute: u32) {
        self.graph
            .lock()
            .expect("audio")
            .set_bus_gain(id, gain, mute != 0);
    }

    pub fn set_unit_link(&self, unit_id: u64, bus_id: u64, mode: u32) {
        self.graph
            .lock()
            .expect("audio")
            .set_unit_link(unit_id, bus_id, mode);
    }

    pub fn set_headphone_cue(&self, unit_id: u64) {
        self.graph.lock().expect("audio").headphone_cue_unit = unit_id;
    }

    pub fn set_headphone_copy_master(&self, enabled: u32) {
        self.graph.lock().expect("audio").headphone_copy_master = enabled != 0;
    }

    pub fn set_video_delay(&self, buffer_frames: u32, fps_num: u32, fps_den: u32) {
        let samples =
            (AUDIO_RATE as u64 * u64::from(buffer_frames.max(1)) * u64::from(fps_den.max(1))
                / u64::from(fps_num.max(1))) as usize;
        self.delay
            .lock()
            .expect("audio delay")
            .set_delay_frames(samples);
    }

    pub fn mix(
        &self,
        uploads: &mut UploadStore,
        snapshot: &[crate::abi::UnitSnap],
        scenes: &[(u64, u32, u32, Arc<[OverlayDesc]>, crate::MvLabelStyle)],
        frames: usize,
        produce: bool,
        mix_inputs: &std::collections::HashMap<u64, crate::abi::MixInputSpec>,
        fps_num: u32,
        fps_den: u32,
    ) -> MixedAudio {
        let mut graph = self.graph.lock().expect("audio");
        let mut delay = self.delay.lock().expect("audio delay");
        graph.mix(
            uploads, snapshot, scenes, frames, &mut delay, produce, mix_inputs, fps_num, fps_den,
        )
    }

    pub fn master_peak(&self) -> (f32, f32) {
        self.graph.lock().expect("audio").master_peak
    }

    pub fn bus_peaks(&self) -> Vec<(u64, f32, f32)> {
        self.graph
            .lock()
            .expect("audio")
            .buses
            .iter()
            .map(|bus| (bus.id, bus.peak.0, bus.peak.1))
            .collect()
    }

    pub fn mix_input_peaks(&self) -> Vec<(u64, f32, f32)> {
        self.graph.lock().expect("audio").mix_input_peaks()
    }

    pub fn skip_bus_frames(&self, frames: usize) {
        if frames == 0 {
            return;
        }
        self.delay.lock().expect("audio delay").skip_frames(frames);
        let graph = self.graph.lock().expect("audio");
        for bus in &graph.buses {
            bus.ring.skip_frames(frames);
        }
    }

    fn sync_outputs(&self) {
        let desired = self.graph.lock().expect("audio").device_groups();
        let mut stale = Vec::new();
        {
            let mut outputs = self.outputs.lock().expect("audio outputs");
            let mut keep = Vec::new();
            for mut output in outputs.drain(..) {
                if desired.iter().any(|(key, _)| *key == output.key) {
                    keep.push(output);
                } else {
                    output.stop.store(true, Ordering::Relaxed);
                    if let Some(join) = output.join.take() {
                        stale.push(join);
                    }
                }
            }
            for (key, maps) in desired {
                if keep.iter().any(|output| output.key == key) {
                    continue;
                }
                if key.kind == DEVICE_NONE || maps.is_empty() {
                    continue;
                }
                let stop = Arc::new(AtomicBool::new(false));
                let stop_t = Arc::clone(&stop);
                let key_t = key.clone();
                let join = std::thread::Builder::new()
                    .name(format!("eiviz-audio-{}", key.kind))
                    .spawn(move || run_device(key_t, maps, stop_t))
                    .ok();
                if let Some(join) = join {
                    keep.push(DeviceOutput {
                        key,
                        stop,
                        join: Some(join),
                    });
                }
            }
            *outputs = keep;
        }
        for join in stale {
            let _ = std::thread::Builder::new()
                .name("eiviz-audio-join".into())
                .spawn(move || {
                    let _ = join.join();
                });
        }
    }
}

fn run_device(key: DeviceKey, maps: Vec<(Arc<BusRing>, i32, i32)>, stop: Arc<AtomicBool>) {
    #[cfg(windows)]
    {
        let kind = if key.kind == DEVICE_COREAUDIO {
            DEVICE_WASAPI
        } else {
            key.kind
        };
        match kind {
            DEVICE_WASAPI => {
                if let Err(error) = wasapi::run(&key.id, key.exclusive, &maps, &stop) {
                    eprintln!("eiviz wasapi: {error}");
                }
            }
            DEVICE_ASIO => {
                if let Err(error) = asio::run(&key.id, &maps, &stop) {
                    eprintln!("eiviz asio: {error}");
                }
            }
            _ => {}
        }
    }
    #[cfg(target_os = "macos")]
    {
        if key.kind == DEVICE_ASIO || key.kind == DEVICE_NONE {
            return;
        }
        if let Err(error) = coreaudio::run(&key.id, &maps, &stop) {
            eprintln!("eiviz coreaudio: {error}");
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (key, maps, stop);
    }
}

pub fn resample_stereo(src: &[f32], src_rate: u32, dst_frames: usize, dst_rate: u32) -> Vec<f32> {
    if dst_frames == 0 {
        return Vec::new();
    }
    let mut out = vec![0.0f32; dst_frames * 2];
    crate::simd::resample_stereo(src, src_rate, dst_frames, dst_rate, &mut out);
    out
}

pub fn pop_stereo_rate(
    maps: &[(Arc<BusRing>, i32, i32)],
    dst_frames: usize,
    dst_rate: u32,
) -> HashMap<(i32, i32), Vec<(f32, f32)>> {
    let src_frames = ((dst_frames as u64 * AUDIO_RATE as u64 + u64::from(dst_rate.max(1)) / 2)
        / u64::from(dst_rate.max(1))) as usize;
    let mut by_map = HashMap::new();
    for (ring, left, right) in maps {
        let interleaved = ring.pop_interleaved(src_frames.max(1));
        let resampled = resample_stereo(&interleaved, AUDIO_RATE as u32, dst_frames, dst_rate);
        let stereo: Vec<(f32, f32)> = resampled
            .chunks_exact(2)
            .map(|chunk| (chunk[0], chunk[1]))
            .collect();
        by_map
            .entry((*left, *right))
            .and_modify(|acc: &mut Vec<(f32, f32)>| {
                for (i, sample) in stereo.iter().enumerate() {
                    if let Some(slot) = acc.get_mut(i) {
                        slot.0 += sample.0;
                        slot.1 += sample.1;
                    }
                }
            })
            .or_insert(stereo);
    }
    by_map
}

pub fn enumerate_devices(kind: u32, dest: &mut [AudioDeviceInfo]) -> usize {
    #[cfg(windows)]
    {
        device::enumerate(kind, dest)
    }
    #[cfg(target_os = "macos")]
    {
        if kind == 0 || kind == DEVICE_COREAUDIO || kind == DEVICE_WASAPI {
            coreaudio::enumerate(dest)
        } else {
            0
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (kind, dest);
        0
    }
}

pub fn device_channels(kind: u32, device_id: &str) -> i32 {
    #[cfg(windows)]
    {
        device::channel_count(kind, device_id)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = kind;
        coreaudio::channel_count(device_id)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (kind, device_id);
        0
    }
}
