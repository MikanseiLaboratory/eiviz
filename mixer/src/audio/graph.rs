use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::abi::{MixInputSpec, OverlayDesc, is_scene, mixing_unit_from_source};
use crate::upload::{AUDIO_FIFO_FRAMES, AUDIO_RATE, AudioInputStore};

use super::AUDIO_PRIME_FRAMES;
use super::AudioDelay;
use super::DeviceKey;
use super::rt::SpscF32;

/// Gain and peak id for the headphone bus. It is not a Mixing Unit id.
pub const HEADPHONE_BUS: u64 = u64::MAX;
pub const LISTEN_OFF: u32 = 0;
pub const LISTEN_UNIT: u32 = 1;
pub const LISTEN_INPUT: u32 = 2;
pub const DEVICE_NONE: u32 = 0;
pub const DEVICE_WASAPI: u32 = 1;
pub const DEVICE_ASIO: u32 = 2;
pub const DEVICE_COREAUDIO: u32 = 3;

pub struct MixedAudio {
    /// PCM of the cued Mixing Unit. Local monitors play this.
    pub monitor: Vec<f32>,
    pub by_unit: HashMap<u64, Vec<f32>>,
}

impl Default for MixedAudio {
    fn default() -> Self {
        Self {
            monitor: Vec::new(),
            by_unit: HashMap::new(),
        }
    }
}

impl MixedAudio {
    pub fn for_unit(&self, unit_id: u64) -> &[f32] {
        if unit_id == 0 {
            return &[];
        }
        self.by_unit.get(&unit_id).map(Vec::as_slice).unwrap_or(&[])
    }
}

pub const LINK_FOLLOW: u32 = 0;
#[allow(dead_code)]
pub const LINK_INDEPENDENT: u32 = 1;

/// Mixer-thread to device-callback hand-off for one bus. Lock-free: the mixer thread is the
/// only producer and the device callback the only consumer.
pub struct BusRing {
    pcm: SpscF32,
    primed: AtomicBool,
    last: [AtomicU32; 2],
    skip_request: AtomicUsize,
    overruns: AtomicU64,
}

impl BusRing {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            pcm: SpscF32::new(AUDIO_FIFO_FRAMES * 2),
            primed: AtomicBool::new(false),
            last: [AtomicU32::new(0), AtomicU32::new(0)],
            skip_request: AtomicUsize::new(0),
            overruns: AtomicU64::new(0),
        })
    }

    /// Drops queued audio and leaves the ring unprimed. A bus with no output device
    /// calls this instead of [`Self::push`], so a device attached later does not play
    /// the samples that piled up while nothing was listening.
    pub fn release(&self) {
        loop {
            let queued = self.pcm.len();
            if queued == 0 || self.pcm.discard(queued) == 0 {
                break;
            }
        }
        self.primed.store(false, Ordering::Relaxed);
    }

    /// Producer side. Samples that do not fit are dropped (counted in `overruns`).
    pub fn push(&self, interleaved: &[f32]) {
        let even = interleaved.len() & !1;
        let stored = self.pcm.push_slice(&interleaved[..even]) & !1;
        if stored < even {
            let count = self.overruns.fetch_add(1, Ordering::Relaxed) + 1;
            if count.is_power_of_two() {
                crate::diag::warn(&format!(
                    "audio bus ring overrun (consumer stalled), {count} dropped blocks so far"
                ));
            }
        }
        if self.pcm.len() >= AUDIO_PRIME_FRAMES * 2 {
            self.primed.store(true, Ordering::Relaxed);
        }
    }

    /// Asks the consumer to drop the oldest `frames`; applied on its next pop.
    #[allow(dead_code)]
    pub fn skip_frames(&self, frames: usize) {
        if frames > 0 {
            self.skip_request.fetch_add(frames, Ordering::Relaxed);
        }
    }

    pub fn fill_frames(&self) -> usize {
        self.pcm.len() / 2
    }

    pub fn is_primed(&self) -> bool {
        self.primed.load(Ordering::Relaxed)
    }

    /// Consumer side: drops queued audio beyond `keep_frames`, newest kept.
    pub fn trim_to(&self, keep_frames: usize) {
        let excess = self.fill_frames().saturating_sub(keep_frames);
        if excess > 0 {
            self.pcm.discard(excess * 2);
        }
    }

    /// Consumer side. Fills `out` (interleaved stereo) without allocating. Before the ring
    /// has primed it yields silence; if the ring runs dry it fades out and the ring re-primes.
    pub fn pop_into(&self, out: &mut [f32]) {
        let out_len = out.len() & !1;
        let (out, _) = out.split_at_mut(out_len);
        let skip = self.skip_request.swap(0, Ordering::Relaxed);
        if skip > 0 {
            self.pcm.discard(skip.saturating_mul(2));
        }
        if !self.primed.load(Ordering::Relaxed) {
            out.fill(0.0);
            return;
        }
        let got = self.pcm.pop_slice(out) & !1;
        let last = if got >= 2 {
            (out[got - 2], out[got - 1])
        } else {
            (
                f32::from_bits(self.last[0].load(Ordering::Relaxed)),
                f32::from_bits(self.last[1].load(Ordering::Relaxed)),
            )
        };
        if got < out.len() {
            // Ran dry: ramp down and wait for the ring to refill to the priming level so
            // playback restarts from a safe cushion instead of stuttering.
            crate::audio_in::fill_underrun(&mut out[got..], last);
            self.primed.store(false, Ordering::Relaxed);
            self.last[0].store(0, Ordering::Relaxed);
            self.last[1].store(0, Ordering::Relaxed);
        } else {
            self.last[0].store(last.0.to_bits(), Ordering::Relaxed);
            self.last[1].store(last.1.to_bits(), Ordering::Relaxed);
        }
    }
}

fn bus_output_armed(bus: &AudioBus) -> bool {
    bus.device_kind != DEVICE_NONE
}

pub struct AudioBus {
    pub id: u64,
    pub device_kind: u32,
    pub device_id: String,
    pub map_left: i32,
    pub map_right: i32,
    pub gain: f32,
    pub mute: bool,
    pub peak: (f32, f32),
    pub ring: Arc<BusRing>,
}

impl AudioBus {
    fn silent(id: u64) -> Self {
        Self {
            id,
            device_kind: DEVICE_NONE,
            device_id: String::new(),
            map_left: 0,
            map_right: 1,
            gain: 1.0,
            mute: false,
            peak: (0.0, 0.0),
            ring: BusRing::new(),
        }
    }
}

pub struct InputAudio {
    pub units: Vec<u64>,
    pub gain: f32,
    pub mute: bool,
}

#[derive(Clone, Copy)]
pub struct UnitLink {
    pub mode: u32,
}

pub struct AudioGraph {
    pub unit_buses: HashMap<u64, AudioBus>,
    pub headphone: AudioBus,
    pub inputs: HashMap<u64, InputAudio>,
    pub unit_links: HashMap<u64, UnitLink>,
    pub headphone_cue_unit: u64,
    pub headphone_copy_monitor: bool,
    pub headphone_listen_kind: u32,
    pub headphone_listen_id: u64,
    pub monitor_peak: (f32, f32),
    scratch_master: Vec<f32>,
    scratch_mixed: Vec<f32>,
    popped: HashMap<u64, Vec<f32>>,
    mix_fifos: HashMap<u64, VecDeque<f32>>,
    mix_last: HashMap<u64, (f32, f32)>,
    mix_peaks: HashMap<u64, (f32, f32)>,
}

impl AudioGraph {
    pub fn with_defaults() -> Self {
        Self {
            unit_buses: HashMap::new(),
            headphone: AudioBus::silent(HEADPHONE_BUS),
            inputs: HashMap::new(),
            unit_links: HashMap::new(),
            headphone_cue_unit: 1,
            headphone_copy_monitor: false,
            headphone_listen_kind: LISTEN_OFF,
            headphone_listen_id: 0,
            monitor_peak: (0.0, 0.0),
            scratch_master: Vec::new(),
            scratch_mixed: Vec::new(),
            popped: HashMap::new(),
            mix_fifos: HashMap::new(),
            mix_last: HashMap::new(),
            mix_peaks: HashMap::new(),
        }
    }

    pub fn ensure_unit(&mut self, unit_id: u64) {
        self.unit_buses
            .entry(unit_id)
            .or_insert_with(|| AudioBus::silent(unit_id));
        self.unit_links
            .entry(unit_id)
            .or_insert(UnitLink { mode: LINK_FOLLOW });
    }

    pub fn remove_unit(&mut self, unit_id: u64) {
        self.unit_buses.remove(&unit_id);
        self.unit_links.remove(&unit_id);
        for input in self.inputs.values_mut() {
            input.units.retain(|id| *id != unit_id);
        }
        if self.headphone_cue_unit == unit_id {
            self.headphone_cue_unit = self.unit_buses.keys().copied().next().unwrap_or(0);
        }
        if self.headphone_listen_kind == LISTEN_UNIT && self.headphone_listen_id == unit_id {
            self.headphone_listen_kind = LISTEN_OFF;
            self.headphone_listen_id = 0;
        }
    }

    pub fn set_unit_device(
        &mut self,
        unit_id: u64,
        device_kind: u32,
        device_id: &str,
        map_left: i32,
        map_right: i32,
    ) {
        self.ensure_unit(unit_id);
        if let Some(bus) = self.unit_buses.get_mut(&unit_id) {
            bus.device_kind = device_kind;
            bus.device_id = device_id.to_string();
            bus.map_left = map_left;
            bus.map_right = map_right;
        }
    }

    pub fn set_headphone_device(
        &mut self,
        device_kind: u32,
        device_id: &str,
        map_left: i32,
        map_right: i32,
    ) {
        self.headphone.device_kind = device_kind;
        self.headphone.device_id = device_id.to_string();
        self.headphone.map_left = map_left;
        self.headphone.map_right = map_right;
    }

    pub fn set_input(&mut self, id: u64, units: &[u64], gain: f32, mute: bool) {
        let mut units = units.to_vec();
        units.sort_unstable();
        units.dedup();
        self.inputs.insert(id, InputAudio { units, gain, mute });
    }

    pub fn mix_input_peaks(&self) -> Vec<(u64, f32, f32)> {
        self.mix_peaks
            .iter()
            .map(|(id, peak)| (*id, peak.0, peak.1))
            .collect()
    }

    pub fn set_bus_gain(&mut self, id: u64, gain: f32, mute: bool) {
        let gain = gain.max(0.0);
        if id == HEADPHONE_BUS {
            self.headphone.gain = gain;
            self.headphone.mute = mute;
            return;
        }
        if let Some(bus) = self.unit_buses.get_mut(&id) {
            bus.gain = gain;
            bus.mute = mute;
        }
    }

    pub fn set_unit_link(&mut self, unit_id: u64, mode: u32) {
        self.ensure_unit(unit_id);
        self.unit_links.insert(unit_id, UnitLink { mode });
    }

    pub fn device_groups(&self) -> Vec<(super::DeviceKey, Vec<(Arc<BusRing>, i32, i32)>)> {
        let mut groups: HashMap<DeviceKey, Vec<(Arc<BusRing>, i32, i32)>> = HashMap::new();
        let push = |bus: &AudioBus, groups: &mut HashMap<DeviceKey, Vec<(Arc<BusRing>, i32, i32)>>| {
            if !bus_output_armed(bus) {
                return;
            }
            let key = DeviceKey {
                kind: bus.device_kind,
                id: bus.device_id.clone(),
            };
            groups.entry(key).or_default().push((
                Arc::clone(&bus.ring),
                bus.map_left,
                bus.map_right,
            ));
        };
        for bus in self.unit_buses.values() {
            push(bus, &mut groups);
        }
        push(&self.headphone, &mut groups);
        groups.into_iter().collect()
    }

    pub fn mix(
        &mut self,
        uploads: &mut AudioInputStore,
        snapshot: &[crate::abi::UnitSnap],
        scenes: &[(u64, u32, u32, Arc<[OverlayDesc]>, crate::MvLabelStyle)],
        frames: usize,
        delay: &mut AudioDelay,
        produce: bool,
        mix_inputs: &HashMap<u64, MixInputSpec>,
        fps_num: u32,
        fps_den: u32,
    ) -> MixedAudio {
        if frames == 0 {
            return MixedAudio::default();
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
            self.popped
                .retain(|id, _| ids.contains(id) || mix_inputs.contains_key(id));
            for id in ids {
                if mix_inputs.contains_key(&id) {
                    continue;
                }
                let slot = self.popped.entry(id).or_default();
                uploads.pop_frames_into(id, frames, slot);
            }
            let spec_map: HashMap<u64, &[OverlayDesc]> = scenes
                .iter()
                .map(|spec| (spec.0, spec.3.as_ref()))
                .collect();
            self.pop_mix_inputs(mix_inputs, frames, fps_num, fps_den);
            let unit_ids: Vec<u64> = self.unit_buses.keys().copied().collect();
            let mut listened_unit = Vec::new();
            for unit_id in unit_ids {
                let fader = {
                    let Some(bus) = self.unit_buses.get(&unit_id) else {
                        continue;
                    };
                    if bus.mute { 0.0 } else { bus.gain.max(0.0) }
                };
                self.render_bus(unit_id, false, fader, snapshot, &spec_map, mix_inputs, frames);
                if unit_id == self.headphone_cue_unit {
                    self.scratch_master.clear();
                    self.scratch_master.extend_from_slice(&self.scratch_mixed);
                    self.monitor_peak = crate::simd::peak_interleaved(&self.scratch_mixed);
                }
                if let Some(bus) = self.unit_buses.get_mut(&unit_id) {
                    bus.peak = crate::simd::peak_interleaved(&self.scratch_mixed);
                }
                if self.headphone_listen_kind == LISTEN_UNIT
                    && unit_id == self.headphone_listen_id
                {
                    listened_unit.clear();
                    listened_unit.extend_from_slice(&self.scratch_mixed);
                }
                delay.push(unit_id, &self.scratch_mixed);
            }
            let hp_fader = if self.headphone.mute {
                0.0
            } else {
                self.headphone.gain.max(0.0)
            };
            self.scratch_mixed.clear();
            self.scratch_mixed.resize(frames * 2, 0.0);
            if self.headphone_listen_kind == LISTEN_UNIT && listened_unit.len() == frames * 2 {
                self.scratch_mixed.copy_from_slice(&listened_unit);
            } else if self.headphone_listen_kind == LISTEN_INPUT {
                let gain = self
                    .inputs
                    .get(&self.headphone_listen_id)
                    .map(|input| if input.mute { 0.0 } else { input.gain.max(0.0) })
                    .unwrap_or(0.0);
                if let Some(samples) = self.popped.get(&self.headphone_listen_id) {
                    crate::simd::mix_stereo_gain(&mut self.scratch_mixed, samples, gain);
                }
            }
            crate::simd::scale_f32(&mut self.scratch_mixed, hp_fader);
            self.headphone.peak = crate::simd::peak_interleaved(&self.scratch_mixed);
            delay.push(HEADPHONE_BUS, &self.scratch_mixed);
        }
        let mut by_unit = HashMap::new();
        let buses: Vec<(u64, bool, Arc<BusRing>)> = self
            .unit_buses
            .iter()
            .map(|(id, bus)| (*id, bus_output_armed(bus), Arc::clone(&bus.ring)))
            .collect();
        for (unit_id, armed, ring) in buses {
            let delayed = delay.pop(unit_id, frames, !produce);
            if armed {
                ring.push(&delayed);
            } else {
                ring.release();
            }
            by_unit.insert(unit_id, delayed);
        }
        let hp_armed = bus_output_armed(&self.headphone);
        let hp_ring = Arc::clone(&self.headphone.ring);
        let headphone = delay.pop(HEADPHONE_BUS, frames, !produce);
        if hp_armed {
            hp_ring.push(&headphone);
        } else {
            hp_ring.release();
        }
        let monitor = by_unit
            .get(&self.headphone_cue_unit)
            .cloned()
            .unwrap_or_else(|| vec![0.0; frames * 2]);
        MixedAudio { monitor, by_unit }
    }

    fn render_bus(
        &mut self,
        unit_id: u64,
        headphone: bool,
        fader: f32,
        snapshot: &[crate::abi::UnitSnap],
        spec_map: &HashMap<u64, &[OverlayDesc]>,
        mix_inputs: &HashMap<u64, MixInputSpec>,
        frames: usize,
    ) {
        let gains = self.gains_for_unit(unit_id, headphone, snapshot, spec_map, mix_inputs);
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
        if !headphone {
            self.push_mix_from_unit(unit_id, mix_inputs);
            self.mix_self_copies(unit_id, snapshot, spec_map, mix_inputs);
        }
    }

    fn gains_for_unit(
        &self,
        unit_id: u64,
        headphone: bool,
        snapshot: &[crate::abi::UnitSnap],
        spec_map: &HashMap<u64, &[OverlayDesc]>,
        mix_inputs: &HashMap<u64, MixInputSpec>,
    ) -> Vec<(u64, f32)> {
        let mut gains = HashMap::<u64, f32>::new();
        let Some(snap) = snapshot.iter().find(|item| item.id == unit_id) else {
            return Vec::new();
        };
        let follow = headphone
            || self
                .unit_links
                .get(&unit_id)
                .is_none_or(|link| link.mode == LINK_FOLLOW);
        if !follow {
            self.add_routed(unit_id, mix_inputs, &mut gains);
            return gains.into_iter().filter(|(_, gain)| *gain > 1e-4).collect();
        }
        let mix = snap.state.mix.clamp(0.0, 1.0);
        let prv_gain = if headphone { 0.35 } else { mix };
        let pgm_gain = if headphone { 1.0 } else { 1.0 - mix };
        add_source(
            snap.state.program_source,
            pgm_gain,
            spec_map,
            &self.inputs,
            mix_inputs,
            unit_id,
            &mut gains,
        );
        add_source(
            snap.state.mix_incoming(),
            prv_gain,
            spec_map,
            &self.inputs,
            mix_inputs,
            unit_id,
            &mut gains,
        );
        for overlay in snap.overlays.iter() {
            if overlay.audio_follow == 0 {
                continue;
            }
            add_source(
                overlay.source_id,
                overlay.opacity.max(0.0),
                spec_map,
                &self.inputs,
                mix_inputs,
                unit_id,
                &mut gains,
            );
        }
        gains.into_iter().filter(|(_, gain)| *gain > 1e-4).collect()
    }

    fn pop_mix_inputs(
        &mut self,
        mix_inputs: &HashMap<u64, MixInputSpec>,
        frames: usize,
        fps_num: u32,
        fps_den: u32,
    ) {
        self.mix_fifos.retain(|id, _| mix_inputs.contains_key(id));
        self.mix_last.retain(|id, _| mix_inputs.contains_key(id));
        self.mix_peaks.retain(|id, _| mix_inputs.contains_key(id));
        for (mix_id, spec) in mix_inputs {
            let slot = self.popped.entry(*mix_id).or_default();
            slot.clear();
            if spec.audio_unit() == 0 {
                slot.resize(frames * 2, 0.0);
                self.mix_peaks.insert(*mix_id, (0.0, 0.0));
                continue;
            }
            let delay_samples = mix_delay_samples(spec.delay, fps_num, fps_den);
            let fifo = self.mix_fifos.entry(*mix_id).or_default();
            slot.reserve(frames * 2);
            for _ in 0..frames {
                let queued = fifo.len() / 2;
                if queued > delay_samples && fifo.len() >= 2 {
                    let left = fifo.pop_front().unwrap_or(0.0);
                    let right = fifo.pop_front().unwrap_or(left);
                    self.mix_last.insert(*mix_id, (left, right));
                    slot.push(left);
                    slot.push(right);
                } else {
                    let (left, right) = self.mix_last.get(mix_id).copied().unwrap_or((0.0, 0.0));
                    slot.push(left);
                    slot.push(right);
                }
            }
            self.mix_peaks
                .insert(*mix_id, crate::simd::peak_interleaved(slot));
        }
    }

    fn mix_self_copies(
        &mut self,
        unit_id: u64,
        snapshot: &[crate::abi::UnitSnap],
        spec_map: &HashMap<u64, &[OverlayDesc]>,
        mix_inputs: &HashMap<u64, MixInputSpec>,
    ) {
        let extras = self.self_mix_gains(unit_id, snapshot, spec_map, mix_inputs);
        for (id, gain) in extras {
            if gain.abs() < 1e-6 {
                continue;
            }
            let Some(samples) = self.popped.get(&id) else {
                continue;
            };
            crate::simd::mix_stereo_gain(&mut self.scratch_mixed, samples, gain);
        }
    }

    fn self_mix_gains(
        &self,
        unit_id: u64,
        snapshot: &[crate::abi::UnitSnap],
        spec_map: &HashMap<u64, &[OverlayDesc]>,
        mix_inputs: &HashMap<u64, MixInputSpec>,
    ) -> Vec<(u64, f32)> {
        if !mix_inputs.values().any(|spec| spec.audio_unit() == unit_id) {
            return Vec::new();
        }
        let Some(snap) = snapshot.iter().find(|item| item.id == unit_id) else {
            return Vec::new();
        };
        if self
            .unit_links
            .get(&unit_id)
            .is_some_and(|link| link.mode != LINK_FOLLOW)
        {
            return Vec::new();
        }
        let mut gains = HashMap::<u64, f32>::new();
        let mix = snap.state.mix.clamp(0.0, 1.0);
        add_self_mix(
            snap.state.program_source,
            1.0 - mix,
            spec_map,
            mix_inputs,
            unit_id,
            &mut gains,
        );
        add_self_mix(
            snap.state.mix_incoming(),
            mix,
            spec_map,
            mix_inputs,
            unit_id,
            &mut gains,
        );
        for overlay in snap.overlays.iter() {
            if overlay.audio_follow == 0 {
                continue;
            }
            add_self_mix(
                overlay.source_id,
                overlay.opacity.max(0.0),
                spec_map,
                mix_inputs,
                unit_id,
                &mut gains,
            );
        }
        gains.into_iter().filter(|(_, gain)| *gain > 1e-4).collect()
    }

    fn push_mix_from_unit(&mut self, unit_id: u64, mix_inputs: &HashMap<u64, MixInputSpec>) {
        for (mix_id, spec) in mix_inputs {
            if spec.audio_unit() != unit_id {
                continue;
            }
            let fifo = self.mix_fifos.entry(*mix_id).or_default();
            fifo.extend(self.scratch_mixed.iter().copied());
            while fifo.len() > (AUDIO_RATE as usize) * 2 {
                fifo.pop_front();
            }
        }
    }

    fn add_routed(
        &self,
        unit_id: u64,
        mix_inputs: &HashMap<u64, MixInputSpec>,
        gains: &mut HashMap<u64, f32>,
    ) {
        for (id, input) in &self.inputs {
            if mix_inputs.contains_key(id) || input.mute || !input.units.contains(&unit_id) {
                continue;
            }
            *gains.entry(*id).or_insert(0.0) += input.gain.max(0.0);
        }
    }
}

fn mix_delay_samples(delay: u32, fps_num: u32, fps_den: u32) -> usize {
    (AUDIO_RATE as u64 * u64::from(delay.max(1)) * u64::from(fps_den.max(1))
        / u64::from(fps_num.max(1))) as usize
}

fn add_source(
    id: u64,
    gain: f32,
    spec_map: &HashMap<u64, &[OverlayDesc]>,
    inputs: &HashMap<u64, InputAudio>,
    mix_inputs: &HashMap<u64, MixInputSpec>,
    unit_id: u64,
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
                    mix_inputs,
                    unit_id,
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
    if let Some(spec) = mix_inputs.get(&id) {
        if spec.audio_unit() == 0 || spec.audio_unit() == unit_id {
            return;
        }
        let level = gain
            * inputs
                .get(&id)
                .map(|input| input.gain.max(0.0))
                .unwrap_or(1.0);
        gains
            .entry(id)
            .and_modify(|current| *current = (*current).max(level))
            .or_insert(level);
        return;
    }
    let Some(input) = inputs.get(&id) else {
        return;
    };
    if input.mute {
        return;
    }
    let level = input.gain.max(0.0);
    let level = gain * level;
    gains
        .entry(id)
        .and_modify(|current| *current = (*current).max(level))
        .or_insert(level);
}

fn add_self_mix(
    id: u64,
    gain: f32,
    spec_map: &HashMap<u64, &[OverlayDesc]>,
    mix_inputs: &HashMap<u64, MixInputSpec>,
    unit_id: u64,
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
                add_self_mix(
                    layer.source_id,
                    gain * layer.opacity.max(0.0),
                    spec_map,
                    mix_inputs,
                    unit_id,
                    gains,
                );
            }
        }
        return;
    }
    let Some(spec) = mix_inputs.get(&id) else {
        return;
    };
    if spec.audio_unit() != unit_id {
        return;
    }
    gains
        .entry(id)
        .and_modify(|current| *current = (*current).max(gain))
        .or_insert(gain);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::UnitState;
    use crate::audio::AudioDelay;
    use crate::upload::AudioInputStore;

    #[test]
    fn idle_bus_does_not_fill_its_ring() {
        let mut graph = AudioGraph::with_defaults();
        let mut uploads = AudioInputStore::default();
        let mut delay = AudioDelay::new();
        let mixed = graph.mix(
            &mut uploads,
            &[],
            &[],
            64,
            &mut delay,
            true,
            &HashMap::new(),
            60,
            1,
        );
        assert_eq!(mixed.monitor.len(), 128);
        assert_eq!(graph.headphone.ring.fill_frames(), 0);
        assert!(!graph.headphone.ring.is_primed());
    }

    #[test]
    fn armed_bus_queues_mixed_audio() {
        let mut graph = AudioGraph::with_defaults();
        graph.ensure_unit(1);
        graph.set_unit_device(1, DEVICE_WASAPI, "", 0, 1);
        let mut uploads = AudioInputStore::default();
        let mut delay = AudioDelay::new();
        graph.mix(
            &mut uploads,
            &[],
            &[],
            64,
            &mut delay,
            true,
            &HashMap::new(),
            60,
            1,
        );
        let master = graph.unit_buses.get(&1).unwrap();
        assert_eq!(master.ring.fill_frames(), 64);
        assert_eq!(graph.headphone.ring.fill_frames(), 0);
    }

    #[test]
    fn many_units_each_get_a_bus() {
        let mut graph = AudioGraph::with_defaults();
        for id in 1..40u64 {
            graph.ensure_unit(id);
        }
        assert_eq!(graph.unit_buses.len(), 39);
    }

    #[test]
    fn for_unit_does_not_alias_unknown_to_monitor() {
        let mixed = MixedAudio {
            monitor: vec![0.5, -0.5],
            by_unit: HashMap::from([(3, vec![0.25, 0.25])]),
        };
        assert!(mixed.for_unit(0).is_empty());
        assert_eq!(mixed.for_unit(3), &[0.25, 0.25]);
        assert!(mixed.for_unit(9).is_empty());
    }

    #[test]
    fn mix_keeps_sample_count_for_broadcast_clocks() {
        let mut graph = AudioGraph::with_defaults();
        let mut uploads = AudioInputStore::default();
        let mut delay = AudioDelay::new();
        graph.ensure_unit(1);
        graph.set_input(10, &[1], 1.0, false);
        uploads.ingest_audio(
            10,
            crate::upload::AudioPacket {
                timestamp: 0,
                sample_rate: AUDIO_RATE,
                channels: 2,
                samples_per_channel: AUDIO_RATE,
                pcm_planar_f32: vec![0.25; AUDIO_RATE as usize * 2],
            },
        );
        let snapshot = [crate::abi::UnitSnap::bare(
            1,
            UnitState {
                program_source: 10,
                preview_source: 10,
                ..UnitState::default()
            },
        )];
        for (fps_num, fps_den, frames) in [(60_000u32, 1_001u32, 300u32), (30_000, 1_001, 150)] {
            let mut carry = 0u64;
            let mut total = 0usize;
            for _ in 0..frames {
                carry += AUDIO_RATE as u64 * u64::from(fps_den);
                let audio_frames = (carry / u64::from(fps_num)) as usize;
                carry %= u64::from(fps_num);
                if audio_frames == 0 {
                    continue;
                }
                let mixed = graph.mix(
                    &mut uploads,
                    &snapshot,
                    &[],
                    audio_frames,
                    &mut delay,
                    true,
                    &HashMap::new(),
                    fps_num,
                    fps_den,
                );
                assert_eq!(mixed.monitor.len(), audio_frames * 2);
                total += audio_frames;
            }
            let expected = (AUDIO_RATE as u64 * u64::from(fps_den) * u64::from(frames)
                / u64::from(fps_num)) as usize;
            assert_eq!(total, expected, "fps {fps_num}/{fps_den}");
        }
    }
}
