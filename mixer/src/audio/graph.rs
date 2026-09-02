use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::abi::{OverlayDesc, UnitState, is_scene, mixing_unit_from_source};
use crate::upload::{AUDIO_FIFO_FRAMES, SampleRing, UploadStore};

use super::AUDIO_PRIME_FRAMES;
use super::AudioDelay;
use super::DeviceKey;

pub const MASTER_BUS: u64 = 1;
pub const HEADPHONE_BUS: u64 = 2;
pub const ROLE_MASTER: u32 = 0;
pub const ROLE_HEADPHONE: u32 = 1;
pub const ROLE_AUX: u32 = 2;
pub const DEVICE_NONE: u32 = 0;
pub const DEVICE_WASAPI: u32 = 1;
pub const DEVICE_ASIO: u32 = 2;
pub const DEVICE_COREAUDIO: u32 = 3;
pub const LINK_FOLLOW: u32 = 0;
#[allow(dead_code)]
pub const LINK_INDEPENDENT: u32 = 1;

pub struct BusRing {
    pcm: Mutex<SampleRing>,
    primed: AtomicBool,
    last: Mutex<(f32, f32)>,
}

impl BusRing {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            pcm: Mutex::new(SampleRing::new(AUDIO_FIFO_FRAMES * 2)),
            primed: AtomicBool::new(false),
            last: Mutex::new((0.0, 0.0)),
        })
    }

    pub fn push(&self, interleaved: &[f32]) {
        let mut pcm = self.pcm.lock().expect("bus ring");
        pcm.extend(interleaved.iter().copied());
        if pcm.len() >= AUDIO_PRIME_FRAMES * 2 {
            self.primed.store(true, Ordering::Relaxed);
        }
    }

    pub fn pop_stereo(&self) -> (f32, f32) {
        if !self.primed.load(Ordering::Relaxed) {
            return (0.0, 0.0);
        }
        let mut pcm = self.pcm.lock().expect("bus ring");
        if pcm.len() >= 2 {
            let left = pcm.pop_front().unwrap_or(0.0);
            let right = pcm.pop_front().unwrap_or(left);
            *self.last.lock().expect("bus last") = (left, right);
            (left, right)
        } else {
            *self.last.lock().expect("bus last")
        }
    }

    pub fn skip_frames(&self, frames: usize) {
        if frames == 0 {
            return;
        }
        let mut pcm = self.pcm.lock().expect("bus ring");
        let n = frames.saturating_mul(2).min(pcm.len());
        if n > 0 {
            pcm.drain_front(n);
        }
    }

    pub fn pop_interleaved(&self, frames: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames * 2);
        if !self.primed.load(Ordering::Relaxed) {
            out.resize(frames * 2, 0.0);
            return out;
        }
        let mut pcm = self.pcm.lock().expect("bus ring");
        pcm.pop_into(frames * 2, &mut out);
        let hold = *self.last.lock().expect("bus last");
        while out.len() < frames * 2 {
            out.push(hold.0);
            out.push(hold.1);
        }
        if out.len() >= 2 {
            *self.last.lock().expect("bus last") = (out[out.len() - 2], out[out.len() - 1]);
        }
        out
    }
}

pub struct AudioBus {
    pub id: u64,
    pub name: String,
    pub role: u32,
    pub bit: u32,
    pub device_kind: u32,
    pub device_id: String,
    pub map_left: i32,
    pub map_right: i32,
    pub exclusive: bool,
    pub gain: f32,
    pub mute: bool,
    pub peak: (f32, f32),
    pub ring: Arc<BusRing>,
}

#[derive(Clone, Copy)]
pub struct InputAudio {
    pub bus_mask: u32,
    pub gain: f32,
    pub mute: bool,
}

#[derive(Clone, Copy)]
pub struct UnitLink {
    pub bus_id: u64,
    pub mode: u32,
}

pub struct AudioGraph {
    pub buses: Vec<AudioBus>,
    pub inputs: HashMap<u64, InputAudio>,
    pub unit_links: HashMap<u64, UnitLink>,
    pub headphone_cue_unit: u64,
    pub headphone_copy_master: bool,
    pub master_peak: (f32, f32),
    scratch_master: Vec<f32>,
    scratch_mixed: Vec<f32>,
    popped: HashMap<u64, Vec<f32>>,
}

impl AudioGraph {
    pub fn with_defaults() -> Self {
        let mut graph = Self {
            buses: Vec::new(),
            inputs: HashMap::new(),
            unit_links: HashMap::new(),
            headphone_cue_unit: 1,
            headphone_copy_master: false,
            master_peak: (0.0, 0.0),
            scratch_master: Vec::new(),
            scratch_mixed: Vec::new(),
            popped: HashMap::new(),
        };
        // Tests must not open the machine's output. HAL Start can block forever
        // on CI runners, and AudioEngine used to join that thread from Drop.
        let master_kind = if cfg!(test) {
            DEVICE_NONE
        } else if cfg!(windows) {
            DEVICE_WASAPI
        } else if cfg!(target_os = "macos") {
            DEVICE_COREAUDIO
        } else {
            DEVICE_NONE
        };
        graph.upsert_bus(
            MASTER_BUS,
            "Master",
            ROLE_MASTER,
            master_kind,
            "",
            0,
            1,
            false,
        );
        graph.upsert_bus(
            HEADPHONE_BUS,
            "Headphone",
            ROLE_HEADPHONE,
            DEVICE_NONE,
            "",
            0,
            1,
            false,
        );
        graph
    }

    pub fn upsert_bus(
        &mut self,
        id: u64,
        name: &str,
        role: u32,
        device_kind: u32,
        device_id: &str,
        map_left: i32,
        map_right: i32,
        exclusive: bool,
    ) {
        if let Some(bus) = self.buses.iter_mut().find(|bus| bus.id == id) {
            bus.name = name.to_string();
            bus.role = role;
            bus.device_kind = device_kind;
            bus.device_id = device_id.to_string();
            bus.map_left = map_left;
            bus.map_right = map_right;
            bus.exclusive = exclusive;
            return;
        }
        let bit = if role == ROLE_MASTER {
            0
        } else if role == ROLE_HEADPHONE {
            1
        } else {
            (2..32)
                .find(|bit| self.buses.iter().all(|bus| bus.bit != *bit))
                .unwrap_or(31)
        };
        self.buses.push(AudioBus {
            id,
            name: name.to_string(),
            role,
            bit,
            device_kind,
            device_id: device_id.to_string(),
            map_left,
            map_right,
            exclusive,
            gain: 1.0,
            mute: false,
            peak: (0.0, 0.0),
            ring: BusRing::new(),
        });
    }

    pub fn remove_bus(&mut self, id: u64) {
        self.buses
            .retain(|bus| !(bus.id == id && bus.role == ROLE_AUX));
        for link in self.unit_links.values_mut() {
            if link.bus_id == id {
                link.bus_id = MASTER_BUS;
            }
        }
    }

    pub fn set_input(&mut self, id: u64, bus_mask: u32, gain: f32, mute: bool) {
        self.inputs.insert(
            id,
            InputAudio {
                bus_mask,
                gain,
                mute,
            },
        );
    }

    pub fn set_bus_gain(&mut self, id: u64, gain: f32, mute: bool) {
        if let Some(bus) = self.buses.iter_mut().find(|bus| bus.id == id) {
            bus.gain = gain.max(0.0);
            bus.mute = mute;
        }
    }

    pub fn set_unit_link(&mut self, unit_id: u64, bus_id: u64, mode: u32) {
        self.unit_links.insert(unit_id, UnitLink { bus_id, mode });
    }

    pub fn device_groups(&self) -> Vec<(super::DeviceKey, Vec<(Arc<BusRing>, i32, i32)>)> {
        let mut groups: HashMap<DeviceKey, Vec<(Arc<BusRing>, i32, i32)>> = HashMap::new();
        for bus in &self.buses {
            if bus.device_kind == DEVICE_NONE {
                continue;
            }
            if bus.role != ROLE_MASTER
                && bus.device_id.is_empty()
                && bus.device_kind == DEVICE_WASAPI
            {
                continue;
            }
            let key = DeviceKey {
                kind: bus.device_kind,
                id: bus.device_id.clone(),
                exclusive: bus.exclusive,
            };
            groups.entry(key).or_default().push((
                Arc::clone(&bus.ring),
                bus.map_left,
                bus.map_right,
            ));
        }
        groups.into_iter().collect()
    }

    pub fn mix(
        &mut self,
        uploads: &mut UploadStore,
        snapshot: &[crate::abi::UnitSnap],
        scenes: &[(u64, u32, u32, Arc<[OverlayDesc]>, crate::MvLabelStyle)],
        frames: usize,
        delay: &mut AudioDelay,
        produce: bool,
    ) -> Vec<f32> {
        if frames == 0 {
            return Vec::new();
        }
        self.scratch_master.clear();
        self.scratch_master.resize(frames * 2, 0.0);
        if produce {
            let mut ids: Vec<u64> = uploads.primed_ids();
            for id in self.inputs.keys() {
                if !ids.contains(id) {
                    ids.push(*id);
                }
            }
            self.popped.retain(|id, _| ids.contains(id));
            for id in ids {
                let slot = self.popped.entry(id).or_default();
                uploads.pop_frames_into(id, frames, slot);
            }
            let spec_map: HashMap<u64, &[OverlayDesc]> = scenes
                .iter()
                .map(|spec| (spec.0, spec.3.as_ref()))
                .collect();
            let bus_ids: Vec<u64> = self.buses.iter().map(|bus| bus.id).collect();
            for bus_id in bus_ids {
                let (role, bit, copy_master, fader) = {
                    let Some(bus) = self.buses.iter().find(|bus| bus.id == bus_id) else {
                        continue;
                    };
                    let fader = if bus.mute { 0.0 } else { bus.gain.max(0.0) };
                    (
                        bus.role,
                        bus.bit,
                        self.headphone_copy_master && bus.role == ROLE_HEADPHONE,
                        fader,
                    )
                };
                if copy_master {
                    self.scratch_mixed.clear();
                    self.scratch_mixed.extend_from_slice(&self.scratch_master);
                    crate::simd::scale_f32(&mut self.scratch_mixed, fader);
                    if let Some(bus) = self.buses.iter_mut().find(|bus| bus.id == bus_id) {
                        bus.peak = crate::simd::peak_interleaved(&self.scratch_mixed);
                    }
                    delay.push(bus_id, &self.scratch_mixed);
                    continue;
                }
                let gains = self.gains_for_bus(bus_id, role, bit, snapshot, &spec_map);
                self.scratch_mixed.clear();
                self.scratch_mixed.resize(frames * 2, 0.0);
                for (id, gain) in gains {
                    if gain.abs() < 1e-6 {
                        continue;
                    }
                    let Some(samples) = self.popped.get(&id) else {
                        continue;
                    };
                    crate::simd::mix_stereo_gain(&mut self.scratch_mixed, samples, gain);
                }
                crate::simd::scale_f32(&mut self.scratch_mixed, fader);
                if role == ROLE_MASTER {
                    self.scratch_master.copy_from_slice(&self.scratch_mixed);
                    self.master_peak = crate::simd::peak_interleaved(&self.scratch_mixed);
                }
                if let Some(bus) = self.buses.iter_mut().find(|bus| bus.id == bus_id) {
                    bus.peak = crate::simd::peak_interleaved(&self.scratch_mixed);
                }
                delay.push(bus_id, &self.scratch_mixed);
            }
        }
        let mut master = vec![0.0f32; frames * 2];
        let bus_ids: Vec<(u64, u32, Arc<BusRing>)> = self
            .buses
            .iter()
            .map(|bus| (bus.id, bus.role, Arc::clone(&bus.ring)))
            .collect();
        for (bus_id, role, ring) in bus_ids {
            let delayed = delay.pop(bus_id, frames, !produce);
            if role == ROLE_MASTER {
                master.copy_from_slice(&delayed);
            }
            ring.push(&delayed);
        }
        master
    }

    fn gains_for_bus(
        &self,
        bus_id: u64,
        role: u32,
        bit: u32,
        snapshot: &[crate::abi::UnitSnap],
        spec_map: &HashMap<u64, &[OverlayDesc]>,
    ) -> Vec<(u64, f32)> {
        let mut gains = HashMap::<u64, f32>::new();
        let follow_units: Vec<(u64, UnitState, bool)> = if role == ROLE_HEADPHONE
            && !self.headphone_copy_master
        {
            snapshot
                .iter()
                .filter(|(id, ..)| *id == self.headphone_cue_unit || self.headphone_cue_unit == 0)
                .map(|(id, _, _, _, _, state, _, _)| (*id, *state, true))
                .collect()
        } else {
            snapshot
                .iter()
                .filter_map(|(id, _, _, _, _, state, _, _)| {
                    let link = self.unit_links.get(id).copied().unwrap_or(UnitLink {
                        bus_id: MASTER_BUS,
                        mode: LINK_FOLLOW,
                    });
                    if link.bus_id != bus_id {
                        return None;
                    }
                    Some((*id, *state, link.mode == LINK_FOLLOW))
                })
                .collect()
        };
        let any_independent = follow_units.iter().any(|(_, _, follow)| !*follow)
            || (role != ROLE_HEADPHONE
                && snapshot.iter().all(|(id, ..)| {
                    self.unit_links
                        .get(id)
                        .map(|link| link.bus_id != bus_id)
                        .unwrap_or(true)
                })
                && follow_units.is_empty());
        if any_independent && follow_units.iter().all(|(_, _, follow)| !*follow) {
            self.add_independent(bit, &mut gains);
            return gains.into_iter().filter(|(_, gain)| *gain > 1e-4).collect();
        }
        if follow_units.is_empty() && role != ROLE_HEADPHONE {
            self.add_independent(bit, &mut gains);
            return gains.into_iter().filter(|(_, gain)| *gain > 1e-4).collect();
        }
        for (_, state, follow) in &follow_units {
            if *follow {
                let mix = state.mix.clamp(0.0, 1.0);
                let prv_gain = if role == ROLE_HEADPHONE { 0.35 } else { mix };
                let pgm_gain = if role == ROLE_HEADPHONE {
                    1.0
                } else {
                    1.0 - mix
                };
                add_source(
                    state.program_source,
                    pgm_gain,
                    spec_map,
                    &self.inputs,
                    bit,
                    &mut gains,
                );
                add_source(
                    state.mix_incoming(),
                    prv_gain,
                    spec_map,
                    &self.inputs,
                    bit,
                    &mut gains,
                );
                for overlay in state.overlays.iter().take(state.overlay_count as usize) {
                    if overlay.audio_follow == 0 {
                        continue;
                    }
                    add_source(
                        overlay.source_id,
                        overlay.opacity.max(0.0),
                        spec_map,
                        &self.inputs,
                        bit,
                        &mut gains,
                    );
                }
            } else {
                self.add_independent(bit, &mut gains);
            }
        }
        gains.into_iter().filter(|(_, gain)| *gain > 1e-4).collect()
    }

    fn add_independent(&self, bit: u32, gains: &mut HashMap<u64, f32>) {
        let mask = 1u32 << bit;
        for (id, input) in &self.inputs {
            if input.mute || input.bus_mask & mask == 0 {
                continue;
            }
            *gains.entry(*id).or_insert(0.0) += input.gain.max(0.0);
        }
    }
}

fn add_source(
    id: u64,
    gain: f32,
    spec_map: &HashMap<u64, &[OverlayDesc]>,
    inputs: &HashMap<u64, InputAudio>,
    bit: u32,
    gains: &mut HashMap<u64, f32>,
) {
    if gain.abs() < 1e-4 {
        return;
    }
    if is_scene(id) {
        if let Some(layers) = spec_map.get(&id) {
            for layer in *layers {
                if layer.audio_follow == 0 {
                    continue;
                }
                add_source(
                    layer.source_id,
                    gain * layer.opacity.max(0.0),
                    spec_map,
                    inputs,
                    bit,
                    gains,
                );
            }
        }
        return;
    }
    if mixing_unit_from_source(id).is_some() {
        return;
    }
    if id == 0 {
        return;
    }
    let mask = 1u32 << bit;
    let (routed, level, mute) = match inputs.get(&id) {
        Some(input) => (input.bus_mask & mask != 0, input.gain.max(0.0), input.mute),
        None => (bit == 0, 1.0, false),
    };
    if mute || !routed {
        return;
    }
    let level = gain * level;
    gains
        .entry(id)
        .and_modify(|current| *current = (*current).max(level))
        .or_insert(level);
}

