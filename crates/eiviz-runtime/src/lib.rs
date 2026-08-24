//! Virtual-clock media runtime. Command snapshots are applied at frame boundaries.

use eiviz_core::{
    AssetId, AudioBusId, AudioFollowPolicy, InputId, InputSource, MixTap, MixingGraph,
    MixingUnitId, Project, RouteMode,
};
use eiviz_gpu::{color_bars, composite, mix_frames, plan_preview, plan_program};
use eiviz_media::{AudioBuffer, BoundedSlot, QueuePolicy, VideoFrame};
use eiviz_time::{ClockDomain, MediaTime, VirtualClock, audio_frame_sample_span};
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
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
    last_preview: HashMap<MixingUnitId, VideoFrame>,
    program_slots: HashMap<MixingUnitId, Arc<BoundedSlot<VideoFrame>>>,
    preview_slots: HashMap<MixingUnitId, Arc<BoundedSlot<VideoFrame>>>,
    pub metrics: TickMetrics,
    degraded_outputs: Mutex<HashMap<String, String>>,
    asset_root: Option<PathBuf>,
    image_cache: HashMap<AssetId, VideoFrame>,
    delay_lines: HashMap<(InputId, AudioBusId), Vec<VecDeque<f32>>>,
    simulated: HashMap<InputId, VideoFrame>,
    pub peak_meters: HashMap<AudioBusId, f32>,
}

impl Runtime {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            clock: VirtualClock::new(),
            frame: 0,
            sample_rate,
            last_program: HashMap::new(),
            last_preview: HashMap::new(),
            program_slots: HashMap::new(),
            preview_slots: HashMap::new(),
            metrics: TickMetrics::default(),
            degraded_outputs: Mutex::new(HashMap::new()),
            asset_root: None,
            image_cache: HashMap::new(),
            delay_lines: HashMap::new(),
            simulated: HashMap::new(),
            peak_meters: HashMap::new(),
        }
    }

    pub fn set_asset_root(&mut self, root: impl Into<PathBuf>) {
        self.asset_root = Some(root.into());
        self.image_cache.clear();
    }

    pub fn inject_simulated(&mut self, id: InputId, frame: VideoFrame) {
        self.simulated.insert(id, frame);
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

    pub fn last_preview_frame(&self, unit: MixingUnitId) -> Option<VideoFrame> {
        self.last_preview.get(&unit).cloned()
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
        let order = MixingGraph::topological_order(project)
            .map_err(|e| RuntimeError::Other(e.to_string()))?;
        let pts = MediaTime::from_frame_index(self.frame, project.video.frame_rate)
            .map_err(|e| RuntimeError::Other(e.to_string()))?;
        self.clock.seek_frame(self.frame, project.video.frame_rate);
        let mut sources = self.generate_sources(project, pts);
        let mut programs = HashMap::new();
        let mut previews = HashMap::new();
        for unit_id in order {
            let Some(mut unit) = project.mixing_units.get(&unit_id).cloned() else {
                continue;
            };
            if unit.transition.remaining_frames > 0 {
                unit.tick_transition();
                if let Some(live) = project.mixing_units.get_mut(&unit.id) {
                    live.transition.remaining_frames = unit.transition.remaining_frames;
                    live.program.scene = unit.program.scene;
                }
            }
            fill_mixfeeds(project, &mut sources, &programs, &previews);
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
            self.last_preview.insert(unit.id, pv.clone());
            programs.insert(unit.id, pg);
            previews.insert(unit.id, pv);
        }
        let (sample_index, sample_count) =
            audio_frame_sample_span(self.frame, self.sample_rate, project.video.frame_rate)
                .map_err(|e| RuntimeError::Other(e.to_string()))?;
        let (audio, meters) = self.mix_audio(
            project,
            sample_index,
            sample_count as usize,
            self.sample_rate,
        );
        self.peak_meters = meters.clone();
        self.metrics.frame = self.frame;
        self.frame += 1;
        Ok(TickResult {
            pts,
            programs,
            previews,
            audio,
            peak_meters: meters,
        })
    }

    fn generate_sources(
        &mut self,
        project: &Project,
        pts: MediaTime,
    ) -> HashMap<InputId, VideoFrame> {
        let mut out = HashMap::new();
        let w = project.video.width.clamp(16, 1920);
        let h = project.video.height.clamp(16, 1080);
        for input in project.inputs.values() {
            if let Some(sim) = self.simulated.get(&input.id) {
                let mut f = sim.clone();
                f.pts = pts;
                f.source = Some(input.id);
                out.insert(input.id, f);
                continue;
            }
            let mut generated = match &input.source {
                InputSource::ColorBars => color_bars(self.frame, pts, w, h),
                InputSource::SolidColor { r, g, b, a } => {
                    VideoFrame::rgba_solid(self.frame, pts, w, h, [*r, *g, *b, *a])
                }
                InputSource::Image { asset } => self.load_image(project, *asset, pts, w, h),
                InputSource::Video { asset, playback } => {
                    let mut frame = self.load_image(project, *asset, pts, w, h);
                    if !playback.playing {
                        frame.discontinuity = false;
                    }
                    frame
                }
                InputSource::Ndi { .. }
                | InputSource::Omt { .. }
                | InputSource::DeckLink { .. }
                | InputSource::AudioDevice { .. } => {
                    VideoFrame::rgba_solid(self.frame, pts, w, h, [16, 16, 16, 255])
                }
                InputSource::MixFeed { .. } => {
                    VideoFrame::rgba_solid(self.frame, pts, w, h, [0, 0, 0, 255])
                }
            };
            generated.source = Some(input.id);
            out.insert(input.id, generated);
        }
        out
    }

    fn load_image(
        &mut self,
        project: &Project,
        asset: AssetId,
        pts: MediaTime,
        w: u32,
        h: u32,
    ) -> VideoFrame {
        if let Some(cached) = self.image_cache.get(&asset) {
            let mut f = cached.clone();
            f.pts = pts;
            f.id = self.frame;
            return f;
        }
        let Some(meta) = project.assets.get(&asset) else {
            return VideoFrame::rgba_solid(self.frame, pts, w, h, [48, 0, 0, 255]);
        };
        if meta.missing {
            return VideoFrame::rgba_solid(self.frame, pts, w, h, [48, 0, 0, 255]);
        }
        let Some(root) = &self.asset_root else {
            return VideoFrame::rgba_solid(self.frame, pts, w, h, [48, 0, 0, 255]);
        };
        let path = root.join(&meta.relative_path);
        match decode_rgba(&path) {
            Some((width, height, data)) => {
                let frame = VideoFrame {
                    id: self.frame,
                    source: None,
                    pts,
                    capture_domain: ClockDomain::SourceMedia,
                    width,
                    height,
                    format: eiviz_media::PixelFormat::Rgba8,
                    data: data.into(),
                    discontinuity: false,
                };
                self.image_cache.insert(asset, frame.clone());
                frame
            }
            None => VideoFrame::rgba_solid(self.frame, pts, w, h, [48, 0, 0, 255]),
        }
    }

    fn mix_audio(
        &mut self,
        project: &Project,
        sample_index: u64,
        frames: usize,
        sample_rate: u32,
    ) -> (AudioBuffer, HashMap<AudioBusId, f32>) {
        let ch = project.audio.channels.max(1) as usize;
        let mut mixed =
            AudioBuffer::silence(sample_index, sample_rate, project.audio.channels, frames);
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
            let delay = ((route.delay_ms.max(0.0) / 1000.0) * sample_rate as f32).round() as usize;
            let lines = self
                .delay_lines
                .entry((route.input, route.bus))
                .or_insert_with(|| vec![VecDeque::new(); ch]);
            while lines.len() < ch {
                lines.push(VecDeque::new());
            }
            let pan = route.pan.clamp(-1.0, 1.0);
            let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
            let g_l = angle.cos() * std::f32::consts::SQRT_2;
            let g_r = angle.sin() * std::f32::consts::SQRT_2;
            for n in 0..frames {
                let t = (sample_index + n as u64) as f32 / sample_rate as f32;
                let s = (2.0 * std::f32::consts::PI * 1000.0 * t).sin() * gain * 0.1;
                #[allow(clippy::needless_range_loop)]
                for c in 0..ch {
                    let g = if ch >= 2 {
                        if c == 0 {
                            g_l
                        } else if c == 1 {
                            g_r
                        } else {
                            1.0
                        }
                    } else {
                        1.0
                    };
                    lines[c].push_back(s * g);
                    let out = if lines[c].len() > delay {
                        lines[c].pop_front().unwrap_or(0.0)
                    } else {
                        0.0
                    };
                    mixed.planes[c][n] += out;
                }
            }
        }
        let mut meters = HashMap::new();
        for bus in &project.audio_matrix.buses {
            let mut peak = 0.0f32;
            for plane in &mixed.planes {
                for s in plane {
                    peak = peak.max(s.abs());
                }
            }
            meters.insert(bus.id, peak);
        }
        (mixed, meters)
    }
}

#[derive(Clone, Debug)]
pub struct TickResult {
    pub pts: MediaTime,
    pub programs: HashMap<MixingUnitId, VideoFrame>,
    pub previews: HashMap<MixingUnitId, VideoFrame>,
    pub audio: AudioBuffer,
    pub peak_meters: HashMap<AudioBusId, f32>,
}

fn fill_mixfeeds(
    project: &Project,
    sources: &mut HashMap<InputId, VideoFrame>,
    programs: &HashMap<MixingUnitId, VideoFrame>,
    previews: &HashMap<MixingUnitId, VideoFrame>,
) {
    for input in project.inputs.values() {
        if let InputSource::MixFeed { unit, tap } = input.source {
            let frame = match tap {
                MixTap::Program => programs.get(&unit),
                MixTap::Preview => previews.get(&unit),
            };
            if let Some(src) = frame {
                let mut f = src.clone();
                f.source = Some(input.id);
                sources.insert(input.id, f);
            }
        }
    }
}

fn decode_rgba(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let img = image::open(path).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some((w, h, img.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_core::{
        AudioRoute, Input, InputSource, MixingUnit, MixingUnitId, RouteMode, Scene, SceneItem,
        Transform2D, TransitionStyle,
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
            pan: 0.0,
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
        assert!(after.peak_meters.values().any(|m| *m > 0.01));
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

    #[test]
    fn mixfeed_nested_program_is_not_black() {
        let mut p = Project::new("dag");
        let a = *p.mixing_units.keys().next().unwrap();
        let b_unit = MixingUnit::new("Mix 2");
        let b = b_unit.id;
        p.mixing_units.insert(b, b_unit);
        let red = Input {
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
        let feed = Input {
            id: InputId::new(),
            name: "feed-a".into(),
            tags: vec![],
            groups: vec![],
            source: InputSource::MixFeed {
                unit: a,
                tap: MixTap::Program,
            },
        };
        let scene_a = Scene {
            id: SceneId::new(),
            name: "a".into(),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: red.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        let scene_b = Scene {
            id: SceneId::new(),
            name: "b".into(),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: feed.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        p.inputs.insert(red.id, red);
        p.inputs.insert(feed.id, feed);
        p.scenes.insert(scene_a.id, scene_a.clone());
        p.scenes.insert(scene_b.id, scene_b.clone());
        p.mixing_units.get_mut(&a).unwrap().program.scene = Some(scene_a.id);
        p.mixing_units.get_mut(&b).unwrap().program.scene = Some(scene_b.id);
        p.validate().unwrap();
        let mut rt = Runtime::new(48000);
        let tick = rt.tick(&mut p).unwrap();
        assert_eq!(tick.programs[&a].pixel(8, 8), [255, 0, 0, 255]);
        assert_eq!(tick.programs[&b].pixel(8, 8), [255, 0, 0, 255]);
    }

    #[test]
    fn delay_line_silences_first_buffer() {
        let (mut p, _, _, unit) = setup();
        p.audio_matrix.routes[0].mode = RouteMode::Manual;
        p.audio_matrix.routes[0].delay_ms = 1000.0;
        let mut rt = Runtime::new(48000);
        let first = rt.tick(&mut p).unwrap();
        let peak = first.audio.planes[0]
            .iter()
            .fold(0.0f32, |a, x| a.max(x.abs()));
        assert!(peak < 1e-6, "1s delay must zero the first 59.94 frame");
        let _ = unit;
    }
}
