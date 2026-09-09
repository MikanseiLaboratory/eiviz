use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::abi::{MixInputSpec, OverlayDesc, UnitSnap, SRC_KIND_MU_MULTIVIEW};
use crate::generator_audio;
use crate::upload::{AudioInputStore, AudioPacket, AUDIO_RATE};

use super::{AudioEngine, MixedAudio};

pub const AUDIO_BLOCK_FRAMES: usize = 480;

#[derive(Clone)]
pub struct AudioMixSnapshot {
    pub units: Vec<UnitSnap>,
    pub scenes: Vec<(u64, u32, u32, Arc<[OverlayDesc]>, crate::MvLabelStyle)>,
    pub mix_inputs: HashMap<u64, MixInputSpec>,
    pub fps_num: u32,
    pub fps_den: u32,
    pub generators: HashMap<u64, (f32, f32)>,
    pub outputs: Vec<AudioOutputRoute>,
    pub buffer_frames: u32,
}

impl Default for AudioMixSnapshot {
    fn default() -> Self {
        Self {
            units: Vec::new(),
            scenes: Vec::new(),
            mix_inputs: HashMap::new(),
            fps_num: 60_000,
            fps_den: 1_001,
            generators: HashMap::new(),
            outputs: Vec::new(),
            buffer_frames: 3,
        }
    }
}

#[derive(Clone)]
pub struct AudioOutputRoute {
    pub audio_bus_id: u64,
    pub source_kind: u32,
    pub send: Arc<dyn Fn(AudioPacket) + Send + Sync>,
}

pub struct AudioScheduler {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl AudioScheduler {
    pub fn start(
        audio: AudioEngine,
        uploads: Arc<Mutex<AudioInputStore>>,
        snapshot: Arc<Mutex<AudioMixSnapshot>>,
        monitor_pcm: Arc<Mutex<std::collections::VecDeque<f32>>>,
        follow_primed: Arc<AtomicBool>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_t = Arc::clone(&stop);
        let join = thread::Builder::new()
            .name("eiviz-audio".into())
            .spawn(move || {
                run_scheduler(audio, uploads, snapshot, monitor_pcm, follow_primed, stop_t)
            })
            .ok();
        Self { stop, join }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            crate::diag::join_timeout(join, Duration::from_secs(2), "audio");
        }
    }
}

impl Drop for AudioScheduler {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_scheduler(
    audio: AudioEngine,
    uploads: Arc<Mutex<AudioInputStore>>,
    snapshot: Arc<Mutex<AudioMixSnapshot>>,
    monitor_pcm: Arc<Mutex<std::collections::VecDeque<f32>>>,
    follow_primed: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    let mut produced = 0u64;
    let origin = Instant::now();
    let mut tone_phase: HashMap<u64, f64> = HashMap::new();
    let mut last_buffer = 0u32;
    while !stop.load(Ordering::Relaxed) && !crate::diag::is_fatal() {
        produced = produced.saturating_add(AUDIO_BLOCK_FRAMES as u64);
        let deadline =
            origin + Duration::from_secs_f64(produced as f64 / f64::from(AUDIO_RATE.max(1)));
        let now = Instant::now();
        if deadline > now {
            thread::sleep(deadline.saturating_duration_since(now));
        } else {
            let late = now.saturating_duration_since(deadline);
            let late_frames = (late.as_secs_f64() * f64::from(AUDIO_RATE.max(1))).floor() as usize;
            if late_frames > AUDIO_BLOCK_FRAMES * 2 {
                crate::diag::warn(&format!(
                    "audio scheduler late {late:?}; resync without dropping the current block"
                ));
                produced = (now.saturating_duration_since(origin).as_secs_f64()
                    * f64::from(AUDIO_RATE.max(1)))
                .floor() as u64;
            }
        }
        let snap = snapshot.lock().expect("audio snap").clone();
        if snap.buffer_frames != last_buffer {
            audio.set_video_delay(snap.buffer_frames, snap.fps_num, snap.fps_den);
            last_buffer = snap.buffer_frames;
        }
        let pts = produced as i64 * 10_000_000 / i64::from(AUDIO_RATE.max(1));
        let mut tones = Vec::new();
        for (id, (hz, level)) in &snap.generators {
            if *hz <= 0.0 {
                tone_phase.remove(id);
                continue;
            }
            let phase = tone_phase.entry(*id).or_insert(0.0);
            tones.push((
                *id,
                generator_audio::sine_packet(phase, *hz, *level, AUDIO_BLOCK_FRAMES, pts),
            ));
        }
        let mixed = {
            let mut uploads = uploads.lock().expect("uploads");
            for (id, packet) in tones {
                uploads.ingest_audio(id, packet);
            }
            audio.mix(
                &mut uploads,
                &snap.units,
                &snap.scenes,
                AUDIO_BLOCK_FRAMES,
                true,
                &snap.mix_inputs,
                snap.fps_num,
                snap.fps_den,
            )
        };
        publish_monitor(&monitor_pcm, &follow_primed, &mixed);
        dispatch_outputs(&snap.outputs, &mixed, pts);
    }
}

fn publish_monitor(
    monitor_pcm: &Mutex<std::collections::VecDeque<f32>>,
    follow_primed: &AtomicBool,
    mixed: &MixedAudio,
) {
    let mut guard = monitor_pcm.lock().expect("monitor pcm");
    guard.extend(mixed.master.iter().copied());
    let cap = AUDIO_RATE as usize;
    while guard.len() > cap {
        guard.pop_front();
    }
    if guard.len() >= (AUDIO_RATE as usize) / 5 {
        follow_primed.store(true, Ordering::Relaxed);
    }
}

fn dispatch_outputs(routes: &[AudioOutputRoute], mixed: &MixedAudio, pts: i64) {
    for route in routes {
        if route.audio_bus_id == 0 || route.source_kind == SRC_KIND_MU_MULTIVIEW {
            continue;
        }
        let packet = interleaved_to_packet(mixed.for_bus(route.audio_bus_id), pts);
        if packet.samples_per_channel <= 0 {
            continue;
        }
        (route.send)(packet);
    }
}

fn interleaved_to_packet(interleaved: &[f32], pts: i64) -> AudioPacket {
    if interleaved.len() < 2 {
        return AudioPacket {
            timestamp: pts,
            sample_rate: AUDIO_RATE,
            channels: 2,
            samples_per_channel: 0,
            pcm_planar_f32: Vec::new(),
        };
    }
    let frames = interleaved.len() / 2;
    let mut planar = vec![0.0; frames * 2];
    for i in 0..frames {
        planar[i] = interleaved[i * 2];
        planar[frames + i] = interleaved[i * 2 + 1];
    }
    AudioPacket {
        timestamp: pts,
        sample_rate: AUDIO_RATE,
        channels: 2,
        samples_per_channel: frames as i32,
        pcm_planar_f32: planar,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_minute_virtual_clock_keeps_sample_count() {
        let rate = AUDIO_RATE as u64;
        let mut produced = 0u64;
        let blocks = (rate * 600) / AUDIO_BLOCK_FRAMES as u64;
        for _ in 0..blocks {
            produced += AUDIO_BLOCK_FRAMES as u64;
        }
        assert_eq!(produced, rate * 600);
    }

    #[test]
    fn scheduler_block_is_ten_milliseconds() {
        assert_eq!(AUDIO_RATE as usize / AUDIO_BLOCK_FRAMES, 100);
    }

    #[test]
    fn scheduler_keeps_producing_without_a_render_tick() {
        use crate::audio::AudioEngine;
        use crate::upload::AudioInputStore;
        use std::collections::VecDeque;
        use std::thread;

        let audio = AudioEngine::new();
        let uploads = Arc::new(Mutex::new(AudioInputStore::default()));
        let snapshot = Arc::new(Mutex::new(AudioMixSnapshot::default()));
        let pcm = Arc::new(Mutex::new(VecDeque::new()));
        let primed = Arc::new(AtomicBool::new(false));
        let mut sched = AudioScheduler::start(
            audio,
            uploads,
            snapshot,
            Arc::clone(&pcm),
            Arc::clone(&primed),
        );
        thread::sleep(Duration::from_millis(250));
        sched.stop();
        let samples = pcm.lock().expect("pcm").len();
        assert!(
            samples >= AUDIO_BLOCK_FRAMES * 8,
            "audio must keep filling the monitor ring while GPU/render is idle (got {samples})"
        );
        assert!(primed.load(Ordering::Relaxed));
    }
}
