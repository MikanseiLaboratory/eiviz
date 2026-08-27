mod asio;
mod device;
mod graph;
mod wasapi;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::abi::{OverlayDesc, UnitState};
use crate::upload::{UploadStore, AUDIO_RATE};

pub use device::{AudioBusInfo, AudioDeviceInfo};
pub use graph::{AudioGraph, BusRing, DEVICE_ASIO, DEVICE_NONE, DEVICE_WASAPI, LINK_FOLLOW, MASTER_BUS};

pub(crate) const AUDIO_PRIME_FRAMES: usize = AUDIO_RATE as usize / 50;

#[derive(Clone)]
pub struct AudioEngine {
    graph: Arc<Mutex<AudioGraph>>,
    outputs: Arc<Mutex<Vec<DeviceOutput>>>,
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
        };
        engine.sync_outputs();
        engine
    }

    pub fn graph(&self) -> Arc<Mutex<AudioGraph>> {
        Arc::clone(&self.graph)
    }

    pub fn shutdown(&self) {
        let mut outputs = self.outputs.lock().expect("audio outputs");
        for output in outputs.iter_mut() {
            output.stop.store(true, Ordering::Relaxed);
            if let Some(join) = output.join.take() {
                let _ = join.join();
            }
        }
        outputs.clear();
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
        self.graph.lock().expect("audio").set_bus_gain(id, gain, mute != 0);
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

    pub fn mix(
        &self,
        uploads: &mut UploadStore,
        snapshot: &[(u64, u32, u32, u32, u32, UnitState)],
        scenes: &[(u64, u32, u32, Arc<[OverlayDesc]>)],
        frames: usize,
    ) -> Vec<f32> {
        self.graph
            .lock()
            .expect("audio")
            .mix(uploads, snapshot, scenes, frames)
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

    pub fn skip_bus_frames(&self, frames: usize) {
        if frames == 0 {
            return;
        }
        let graph = self.graph.lock().expect("audio");
        for bus in &graph.buses {
            bus.ring.skip_frames(frames);
        }
    }

    fn sync_outputs(&self) {
        let desired = self.graph.lock().expect("audio").device_groups();
        let mut outputs = self.outputs.lock().expect("audio outputs");
        let mut keep = Vec::new();
        for mut output in outputs.drain(..) {
            if desired.iter().any(|(key, _)| *key == output.key) {
                keep.push(output);
            } else {
                output.stop.store(true, Ordering::Relaxed);
                if let Some(join) = output.join.take() {
                    let _ = join.join();
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
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_device(
    key: DeviceKey,
    maps: Vec<(Arc<BusRing>, i32, i32)>,
    stop: Arc<AtomicBool>,
) {
    match key.kind {
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

pub fn resample_stereo(src: &[f32], src_rate: u32, dst_frames: usize, dst_rate: u32) -> Vec<f32> {
    let src_frames = src.len() / 2;
    if dst_frames == 0 {
        return Vec::new();
    }
    if src_frames == 0 {
        return vec![0.0; dst_frames * 2];
    }
    if src_rate == dst_rate && src_frames == dst_frames {
        return src.to_vec();
    }
    let mut out = vec![0.0f32; dst_frames * 2];
    let last = src_frames.saturating_sub(1);
    for i in 0..dst_frames {
        let src_pos = i as f64 * f64::from(src_rate) / f64::from(dst_rate.max(1));
        let idx = (src_pos.floor() as usize).min(last);
        let frac = (src_pos - idx as f64) as f32;
        let nxt = (idx + 1).min(last);
        let l = src[idx * 2] * (1.0 - frac) + src[nxt * 2] * frac;
        let r = src[idx * 2 + 1] * (1.0 - frac) + src[nxt * 2 + 1] * frac;
        out[i * 2] = l;
        out[i * 2 + 1] = r;
    }
    out
}

pub fn pop_stereo_rate(maps: &[(Arc<BusRing>, i32, i32)], dst_frames: usize, dst_rate: u32) -> HashMap<(i32, i32), Vec<(f32, f32)>> {
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
    device::enumerate(kind, dest)
}

pub fn device_channels(kind: u32, device_id: &str) -> i32 {
    device::channel_count(kind, device_id)
}
