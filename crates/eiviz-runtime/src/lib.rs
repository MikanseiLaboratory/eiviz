//! Virtual-clock media runtime. Command snapshots are applied at frame boundaries.

use eiviz_core::{
    AudioFollowPolicy, InputId, InputSource, MixingGraph, MixingUnitId, Project, RouteMode,
};
use eiviz_gpu::{color_bars, composite, mix_frames, plan_preview, plan_program};
use eiviz_media::{AudioBuffer, BoundedSlot, QueuePolicy, VideoFrame};
use eiviz_time::{MediaTime, VirtualClock, audio_frame_sample_span};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("unknown mixing unit {0}")]
    UnknownUnit(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

#[derive(Clone, Debug, Default)]
pub struct TickMetrics {
    pub frame: u64,
    pub dropped_preview: u64,
    pub program_repeats: u64,
}

pub struct Runtime {
    clock: VirtualClock,
    frame: u64,
    sample_rate: u32,
    last_program: HashMap<MixingUnitId, VideoFrame>,
    program_slots: HashMap<MixingUnitId, Arc<BoundedSlot<VideoFrame>>>,
    preview_slots: HashMap<MixingUnitId, Arc<BoundedSlot<VideoFrame>>>,
    pub metrics: TickMetrics,
    degraded_outputs: Mutex<HashMap<String, String>>,
}

impl Runtime {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            clock: VirtualClock::new(),
            frame: 0,
            sample_rate,
            last_program: HashMap::new(),
            program_slots: HashMap::new(),
            preview_slots: HashMap::new(),
            metrics: TickMetrics::default(),
            degraded_outputs: Mutex::new(HashMap::new()),
        }
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn clock(&self) -> &VirtualClock {
        &self.clock
    }

    pub fn program_slot(&mut self, unit: MixingUnitId) -> Arc<BoundedSlot<VideoFrame>> {
        self.program_slots
            .entry(unit)
            .or_insert_with(|| Arc::new(BoundedSlot::new("program", QueuePolicy::ProgramHold)))
            .clone()
    }

    pub fn preview_slot(&mut self, unit: MixingUnitId) -> Arc<BoundedSlot<VideoFrame>> {
        self.preview_slots
            .entry(unit)
            .or_insert_with(|| Arc::new(BoundedSlot::new("preview", QueuePolicy::LatestWins)))
            .clone()
    }

    pub fn last_program_frame(&self, unit: MixingUnitId) -> Option<VideoFrame> {
        self.last_program.get(&unit).cloned()
    }

    pub fn mark_output_failed(&self, name: &str, reason: impl Into<String>) {
        self.degraded_outputs
            .lock()
            .insert(name.to_string(), reason.into());
    }

    pub fn failed_outputs(&self) -> Vec<(String, String)> {
        self.degraded_outputs
            .lock()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn tick(&mut self, project: &mut Project) -> Result<TickResult> {
        MixingGraph::assert_acyclic(project).map_err(|e| RuntimeError::Other(e.to_string()))?;
        let pts = MediaTime::from_frame_index(self.frame, project.video.frame_rate)
            .map_err(|e| RuntimeError::Other(e.to_string()))?;
        self.clock.seek_frame(self.frame, project.video.frame_rate);
        let sources = generate_sources(project, pts, self.frame);
        let mut programs = HashMap::new();
        let mut previews = HashMap::new();
        let units: Vec<_> = project.mixing_units.values().cloned().collect();
        for mut unit in units {
            if unit.transition.remaining_frames > 0 {
                unit.tick_transition();
                if let Some(live) = project.mixing_units.get_mut(&unit.id) {
                    live.transition.remaining_frames = unit.transition.remaining_frames;
                    live.program.scene = unit.program.scene;
                }
            }
            let pg_plan = plan_program(project, &unit);
            let mut pg = composite(&pg_plan, &sources, pts, self.frame);
            if unit.transition.remaining_frames > 0 {
                let pv_plan = plan_preview(project, &unit);
                let pv = composite(&pv_plan, &sources, pts, self.frame);
                pg = mix_frames(&pv, &pg, unit.mix_factor(), pts, self.frame);
            }
            let pv_plan = plan_preview(project, &unit);
            let pv = composite(&pv_plan, &sources, pts, self.frame);
            self.program_slot(unit.id)
                .push(pg.clone())
                .map_err(|e| RuntimeError::Other(e.to_string()))?;
            let _ = self.preview_slot(unit.id).push(pv.clone());
            self.last_program.insert(unit.id, pg.clone());
            programs.insert(unit.id, pg);
            previews.insert(unit.id, pv);
        }
        let (sample_index, sample_count) =
            audio_frame_sample_span(self.frame, self.sample_rate, project.video.frame_rate)
                .map_err(|e| RuntimeError::Other(e.to_string()))?;
        let audio = mix_audio(
            project,
            sample_index,
            sample_count as usize,
            self.sample_rate,
        );
        self.metrics.frame = self.frame;
        self.frame += 1;
        Ok(TickResult {
            pts,
            programs,
            previews,
            audio,
        })
    }
}

#[derive(Clone, Debug)]
pub struct TickResult {
    pub pts: MediaTime,
    pub programs: HashMap<MixingUnitId, VideoFrame>,
    pub previews: HashMap<MixingUnitId, VideoFrame>,
    pub audio: AudioBuffer,
}

fn generate_sources(project: &Project, pts: MediaTime, frame: u64) -> HashMap<InputId, VideoFrame> {
    let mut out = HashMap::new();
    let w = project.video.width.clamp(16, 1920);
    let h = project.video.height.clamp(16, 1080);
    for input in project.inputs.values() {
        let mut generated = match &input.source {
            InputSource::ColorBars => color_bars(frame, pts, w, h),
            InputSource::SolidColor { r, g, b, a } => {
                VideoFrame::rgba_solid(frame, pts, w, h, [*r, *g, *b, *a])
            }
            InputSource::Image { .. } | InputSource::Video { .. } => {
                VideoFrame::rgba_solid(frame, pts, w, h, [32, 32, 32, 255])
            }
            InputSource::Ndi { .. }
            | InputSource::Omt { .. }
            | InputSource::DeckLink { .. }
            | InputSource::AudioDevice { .. } => {
                VideoFrame::rgba_solid(frame, pts, w, h, [16, 16, 16, 255])
            }
            InputSource::MixFeed { .. } => VideoFrame::rgba_solid(frame, pts, w, h, [0, 0, 0, 255]),
        };
        generated.source = Some(input.id);
        out.insert(input.id, generated);
    }
    out
}

fn mix_audio(project: &Project, sample_index: u64, frames: usize, sample_rate: u32) -> AudioBuffer {
    let mut mixed = AudioBuffer::silence(sample_index, sample_rate, project.audio.channels, frames);
    for route in &project.audio_matrix.routes {
        let follow_active = match route.mode {
            RouteMode::Manual => true,
            RouteMode::Follow { unit } => {
                let Some(u) = project.mixing_units.get(&unit) else {
                    continue;
                };
                match u.audio_follow {
                    AudioFollowPolicy::Off => false,
                    AudioFollowPolicy::Program => {
                        MixingGraph::input_visible_on_program(project, unit, route.input)
                    }
                    AudioFollowPolicy::ProgramAndPreview => {
                        MixingGraph::input_visible_on_program(project, unit, route.input)
                            || MixingGraph::input_visible_on_preview(project, unit, route.input)
                    }
                }
            }
        };
        let gain = project
            .audio_matrix
            .effective_linear_gain(route, follow_active);
        if gain == 0.0 {
            continue;
        }
        for n in 0..frames {
            let t = (sample_index + n as u64) as f32 / sample_rate as f32;
            let s = (2.0 * std::f32::consts::PI * 1000.0 * t).sin() * gain * 0.1;
            for ch in 0..mixed.channels as usize {
                mixed.planes[ch][n] += s;
            }
        }
    }
    mixed
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_core::{
        AudioRoute, Input, InputSource, MixingUnitId, RouteMode, Scene, SceneItem, Transform2D,
        TransitionStyle,
    };
    use eiviz_core::{InputId, SceneId, SceneItemId};

    fn setup() -> (Project, InputId, SceneId, MixingUnitId) {
        let mut p = Project::new("rt");
        let unit = *p.mixing_units.keys().next().unwrap();
        let input = Input {
            id: InputId::new(),
            name: "red".into(),
            tags: vec![],
            groups: vec![],
            source: InputSource::SolidColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
        };
        let iid = input.id;
        let scene = Scene {
            id: SceneId::new(),
            name: "red".into(),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: iid,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        let sid = scene.id;
        p.inputs.insert(iid, input);
        p.scenes.insert(sid, scene);
        p.mixing_units.get_mut(&unit).unwrap().preview.scene = Some(sid);
        let bus = p.audio_matrix.buses[0].id;
        p.audio_matrix.routes.push(AudioRoute {
            input: iid,
            bus,
            mode: RouteMode::Follow { unit },
            gain_db: 0.0,
            muted: false,
            solo: false,
            delay_ms: 0.0,
        });
        (p, iid, sid, unit)
    }

    #[test]
    fn take_changes_program_pixels_on_same_boundary() {
        let (mut p, _iid, sid, unit) = setup();
        let mut rt = Runtime::new(48000);
        let before = rt.tick(&mut p).unwrap();
        assert_eq!(before.programs[&unit].pixel(10, 10)[0], 0);
        p.mixing_units.get_mut(&unit).unwrap().transition.style = TransitionStyle::Cut;
        p.mixing_units.get_mut(&unit).unwrap().take(false);
        let after = rt.tick(&mut p).unwrap();
        assert_eq!(after.programs[&unit].pixel(10, 10), [255, 0, 0, 255]);
        assert_eq!(p.mixing_units[&unit].program.scene, Some(sid));
        let peak = after.audio.planes[0]
            .iter()
            .fold(0.0f32, |a, x| a.max(x.abs()));
        assert!(peak > 0.01, "audio follow should unmute on take");
    }

    #[test]
    fn slow_sink_does_not_stop_program() {
        let (mut p, _, _, unit) = setup();
        let mut rt = Runtime::new(48000);
        rt.mark_output_failed("rtmp-primary", "connection reset");
        for _ in 0..10 {
            rt.tick(&mut p).unwrap();
        }
        assert_eq!(rt.frame(), 10);
        assert!(!rt.failed_outputs().is_empty());
        let _ = unit;
    }
}
