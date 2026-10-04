#[cfg(windows)]
mod asio;
mod capture;
#[cfg(target_os = "macos")]
mod coreaudio;
#[cfg(any(windows, target_os = "macos"))]
mod cpal_io;
#[cfg(windows)]
mod device;
mod feed;
mod graph;
mod info;
mod pcm;
mod process;
mod pump;
#[cfg(windows)]
mod rsac_process;
mod rt;
mod scheduler;

use crate::guard::LockExt;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::abi::OverlayDesc;
use crate::upload::{AUDIO_RATE, AudioInputStore};

pub use capture::{AudioCaptureSpec, AudioCaptureStore};
#[cfg_attr(not(windows), allow(unused_imports))]
pub use graph::{
    AudioGraph, DEVICE_ASIO, DEVICE_COREAUDIO, DEVICE_NONE, DEVICE_WASAPI, HEADPHONE_BUS, LINK_FOLLOW,
    MixedAudio,
};
pub use info::{AudioBusInfo, AudioDeviceInfo};
pub use process::processes_json;

pub use scheduler::{AudioMixSnapshot, AudioOutputRoute, AudioScheduler};

#[derive(Clone)]
pub struct AudioMonitor {
    pub pcm: Arc<Mutex<VecDeque<f32>>>,
    pub primed: Arc<AtomicBool>,
}

impl Default for AudioMonitor {
    fn default() -> Self {
        Self {
            pcm: Arc::new(Mutex::new(VecDeque::new())),
            primed: Arc::new(AtomicBool::new(false)),
        }
    }
}

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

    /// Changes the delay. Queued audio is shifted to match immediately: a longer delay
    /// inserts silence at the front, a shorter one drops the oldest audio, so the output
    /// follows the new video delay in one step instead of drifting towards it.
    pub fn set_delay_frames(&mut self, frames: usize) {
        let previous = self.delay_frames;
        self.delay_frames = frames;
        let cap = frames
            .saturating_mul(2)
            .saturating_add((AUDIO_RATE as usize / 5) * 2);
        for fifo in self.fifos.values_mut() {
            if frames > previous {
                for _ in 0..(frames - previous) * 2 {
                    fifo.push_front(0.0);
                }
            } else if frames < previous {
                let drop = ((previous - frames) * 2).min(fifo.len());
                fifo.drain(..drop);
            }
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

    #[allow(dead_code)]
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
    routes: Arc<rt::Published<Vec<pump::Route>>>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct DeviceKey {
    pub kind: u32,
    pub id: String,
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
            let mut outputs = self.outputs.lock_or_recover();
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
        #[cfg(windows)]
        asio::shutdown();
    }

    pub fn ensure_unit(&self, unit_id: u64) {
        self.graph.lock_or_recover().ensure_unit(unit_id);
        self.sync_outputs();
    }

    pub fn remove_unit(&self, unit_id: u64) {
        self.graph.lock_or_recover().remove_unit(unit_id);
        self.sync_outputs();
    }

    pub fn set_unit_device(
        &self,
        unit_id: u64,
        device_kind: u32,
        device_id: &str,
        map_left: i32,
        map_right: i32,
    ) {
        self.graph.lock_or_recover().set_unit_device(
            unit_id,
            device_kind,
            device_id,
            map_left,
            map_right,
        );
        self.sync_outputs();
    }

    pub fn set_headphone_device(
        &self,
        device_kind: u32,
        device_id: &str,
        map_left: i32,
        map_right: i32,
    ) {
        self.graph.lock_or_recover().set_headphone_device(
            device_kind,
            device_id,
            map_left,
            map_right,
        );
        self.sync_outputs();
    }

    pub fn set_input(&self, id: u64, units: &[u64], gain: f32, mute: u32) {
        self.graph
            .lock_or_recover()
            .set_input(id, units, gain, mute != 0);
    }

    pub fn set_bus_gain(&self, id: u64, gain: f32, mute: u32) {
        self.graph
            .lock_or_recover()
            .set_bus_gain(id, gain, mute != 0);
    }

    pub fn set_unit_link(&self, unit_id: u64, mode: u32) {
        self.graph.lock_or_recover().set_unit_link(unit_id, mode);
    }

    pub fn set_headphone_cue(&self, unit_id: u64) {
        self.graph.lock_or_recover().headphone_cue_unit = unit_id;
    }

    pub fn set_headphone_copy_monitor(&self, enabled: u32) {
        self.graph.lock_or_recover().headphone_copy_monitor = enabled != 0;
    }

    pub fn set_video_delay(&self, buffer_frames: u32, fps_num: u32, fps_den: u32) {
        let samples =
            (AUDIO_RATE as u64 * u64::from(buffer_frames.max(1)) * u64::from(fps_den.max(1))
                / u64::from(fps_num.max(1))) as usize;
        self.delay.lock_or_recover().set_delay_frames(samples);
    }

    pub fn mix(
        &self,
        uploads: &mut AudioInputStore,
        snapshot: &[crate::abi::UnitSnap],
        scenes: &[(u64, u32, u32, Arc<[OverlayDesc]>, crate::MvLabelStyle)],
        frames: usize,
        produce: bool,
        mix_inputs: &std::collections::HashMap<u64, crate::abi::MixInputSpec>,
        fps_num: u32,
        fps_den: u32,
    ) -> MixedAudio {
        let mut graph = self.graph.lock_or_recover();
        let mut delay = self.delay.lock_or_recover();
        graph.mix(
            uploads, snapshot, scenes, frames, &mut delay, produce, mix_inputs, fps_num, fps_den,
        )
    }

    pub fn monitor_peak(&self) -> (f32, f32) {
        self.graph.lock_or_recover().monitor_peak
    }

    pub fn bus_peaks(&self) -> Vec<(u64, f32, f32)> {
        let graph = self.graph.lock_or_recover();
        let mut peaks: Vec<(u64, f32, f32)> = graph
            .unit_buses
            .iter()
            .map(|(id, bus)| (*id, bus.peak.0, bus.peak.1))
            .collect();
        peaks.push((
            HEADPHONE_BUS,
            graph.headphone.peak.0,
            graph.headphone.peak.1,
        ));
        peaks
    }

    pub fn mix_input_peaks(&self) -> Vec<(u64, f32, f32)> {
        self.graph.lock_or_recover().mix_input_peaks()
    }

    #[allow(dead_code)]
    pub fn skip_bus_frames(&self, frames: usize) {
        if frames == 0 {
            return;
        }
        self.delay.lock_or_recover().skip_frames(frames);
        let graph = self.graph.lock_or_recover();
        for bus in graph.unit_buses.values() {
            bus.ring.skip_frames(frames);
        }
        graph.headphone.ring.skip_frames(frames);
    }

    fn sync_outputs(&self) {
        let desired = self.graph.lock_or_recover().device_groups();
        let mut stale = Vec::new();
        {
            let mut outputs = self.outputs.lock_or_recover();
            let mut keep = Vec::new();
            for mut output in outputs.drain(..) {
                if let Some((_, maps)) = desired.iter().find(|(key, _)| *key == output.key) {
                    output.routes.set(maps.clone());
                    keep.push(output);
                } else {
                    output.stop.store(true, Ordering::Relaxed);
                    if let Some(join) = output.join.take() {
                        stale.push(join);
                    }
                }
            }
            #[cfg(windows)]
            {
                let mut asio_keep = std::collections::HashSet::new();
                for (key, maps) in &desired {
                    if key.kind != DEVICE_ASIO {
                        continue;
                    }
                    asio::set_outputs(&key.id, maps.clone());
                    asio_keep.insert(key.id.clone());
                }
                asio::retain_outputs(&asio_keep);
            }
            for (key, maps) in desired {
                #[cfg(windows)]
                if key.kind == DEVICE_ASIO {
                    continue;
                }
                if keep.iter().any(|output| output.key == key) {
                    continue;
                }
                if key.kind == DEVICE_NONE || maps.is_empty() {
                    continue;
                }
                let stop = Arc::new(AtomicBool::new(false));
                let stop_t = Arc::clone(&stop);
                let key_t = key.clone();
                let routes = rt::Published::new(maps);
                let routes_t = Arc::clone(&routes);
                let join = std::thread::Builder::new()
                    .name(format!("eiviz-audio-{}", key.kind))
                    .spawn(move || run_device(key_t, routes_t, stop_t))
                    .ok();
                if let Some(join) = join {
                    keep.push(DeviceOutput {
                        key,
                        routes,
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

fn run_device(key: DeviceKey, routes: Arc<rt::Published<Vec<pump::Route>>>, stop: Arc<AtomicBool>) {
    #[cfg(windows)]
    {
        match key.kind {
            DEVICE_WASAPI => {
                if let Err(error) = cpal_io::run_output(&key.id, &routes, &stop) {
                    crate::diag::error(&format!("eiviz cpal output: {error}"));
                }
            }
            DEVICE_ASIO => {
                let _ = (routes, stop);
            }
            DEVICE_COREAUDIO => {
                crate::diag::error("Core Audio output is only available on macOS");
            }
            _ => {}
        }
    }
    #[cfg(target_os = "macos")]
    {
        match key.kind {
            DEVICE_COREAUDIO => {
                if let Err(error) = cpal_io::run_output(&key.id, &routes, &stop) {
                    crate::diag::error(&format!("eiviz cpal output: {error}"));
                }
            }
            DEVICE_WASAPI => {
                crate::diag::error("WASAPI output is only available on Windows");
            }
            DEVICE_ASIO | DEVICE_NONE => {}
            _ => {}
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (key, routes, stop);
    }
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
    device_io_channels(kind, device_id).0
}

pub fn device_io_channels(kind: u32, device_id: &str) -> (i32, i32) {
    #[cfg(windows)]
    {
        device::io_channels(kind, device_id)
    }
    #[cfg(target_os = "macos")]
    {
        let n = if kind == DEVICE_ASIO {
            0
        } else {
            coreaudio::channel_count(device_id)
        };
        (n, n)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (kind, device_id);
        (0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(delay: &mut AudioDelay, blocks: usize, value: f32, frames: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for _ in 0..blocks {
            delay.push(1, &vec![value; frames * 2]);
            out.extend(delay.pop(1, frames, false));
        }
        out
    }

    #[test]
    fn delay_increase_inserts_silence_and_keeps_length() {
        let mut delay = AudioDelay::new();
        delay.set_delay_frames(480);
        run(&mut delay, 10, 0.5, 480);
        delay.set_delay_frames(960);
        let out = run(&mut delay, 3, 0.5, 480);
        // The extra 480 frames of delay show up as one block of silence.
        assert!(out[..480 * 2].iter().all(|v| *v == 0.0));
        assert!(out[480 * 2..].iter().all(|v| *v == 0.5));
    }

    #[test]
    fn delay_decrease_drops_the_surplus_at_once() {
        let mut delay = AudioDelay::new();
        delay.set_delay_frames(960);
        run(&mut delay, 10, 0.5, 480);
        delay.set_delay_frames(480);
        assert_eq!(delay.fifos[&1].len(), 480 * 2);
    }
}
