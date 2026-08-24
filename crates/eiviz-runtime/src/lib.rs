//! Virtual-clock media runtime. Command snapshots are applied at frame boundaries.

use eiviz_core::{
    AssetId, AudioBusId, AudioFollowPolicy, AudioMatrix, AudioResamplingPolicy, CompositorBackend,
    InputId, InputSource, MissingMediaPolicy, MixTap, MixingGraph, MixingUnitId, MultiviewId,
    MultiviewSource, Playback, Project, RouteMode, Transform2D, TransitionStyle,
};
use eiviz_gpu::{Layer, RenderPlan, color_bars, composite, mix_frames, plan_preview, plan_program};
use eiviz_media::{
    AsrcDiagnostics, AudioBuffer, AudioIoDiagnostics, BoundedSlot, MediaSource, QueuePolicy,
    StreamingAsrc, VideoFrame,
};
use eiviz_time::{ClockDomain, MediaTime, VirtualClock, audio_frame_sample_span};
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(feature = "wgpu-backend")]
type ConfiguredWgpu = Option<Arc<eiviz_gpu::WgpuCompositor>>;
#[cfg(not(feature = "wgpu-backend"))]
type ConfiguredWgpu = ();

/// Configured missing-media slate. Not a fake camera or decoded still.
pub const SLATE_RGBA: [u8; 4] = [32, 32, 48, 255];

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("unknown mixing unit {0}")]
    UnknownUnit(String),
    #[error("compositor backend mismatch: project={project:?} runtime={runtime:?}")]
    BackendMismatch {
        project: CompositorBackend,
        runtime: CompositorBackend,
    },
    #[error("wgpu compositor requested but feature wgpu-backend is not enabled")]
    WgpuFeatureDisabled,
    #[error("GPU compositor failed: {0}")]
    Gpu(String),
    #[error("missing media and policy is Fail: {0}")]
    MissingMedia(String),
    #[error(
        "audio source {input} is {source_rate} Hz but project is {project_rate} Hz and policy is ExactRate"
    )]
    AudioRateMismatch {
        input: InputId,
        source_rate: u32,
        project_rate: u32,
    },
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

#[derive(Clone, Debug)]
pub struct RenderPlanSnapshot {
    order: Arc<[MixingUnitId]>,
    programs: HashMap<MixingUnitId, RenderPlan>,
    previews: HashMap<MixingUnitId, RenderPlan>,
}

#[derive(Clone, Debug)]
pub struct AudioPlan {
    pub matrix: AudioMatrix,
    pub sample_rate: u32,
    pub channels: u16,
    pub resampling: AudioResamplingPolicy,
}

/// Fully validated, immutable state consumed by a media boundary.
#[derive(Clone, Debug)]
pub struct RuntimeSnapshot {
    accepted_revision: u64,
    applied_revision: u64,
    project: Arc<Project>,
    render: RenderPlanSnapshot,
    audio: AudioPlan,
}

impl RuntimeSnapshot {
    pub fn compile(
        project: Arc<Project>,
        accepted_revision: u64,
        applied_revision: u64,
    ) -> Result<Self> {
        project
            .validate()
            .map_err(|error| RuntimeError::Other(error.to_string()))?;
        let order = MixingGraph::topological_order(&project)
            .map_err(|error| RuntimeError::Other(error.to_string()))?;
        let mut programs = HashMap::with_capacity(project.mixing_units.len());
        let mut previews = HashMap::with_capacity(project.mixing_units.len());
        for (id, unit) in &project.mixing_units {
            programs.insert(*id, plan_program(&project, unit));
            previews.insert(*id, plan_preview(&project, unit));
        }
        let audio = AudioPlan {
            matrix: project.audio_matrix.clone(),
            sample_rate: project.audio.sample_rate,
            channels: project.audio.channels,
            resampling: project.audio.resampling,
        };
        Ok(Self {
            accepted_revision,
            applied_revision,
            project,
            render: RenderPlanSnapshot {
                order: order.into(),
                programs,
                previews,
            },
            audio,
        })
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn accepted_revision(&self) -> u64 {
        self.accepted_revision
    }

    pub fn applied_revision(&self) -> u64 {
        self.applied_revision
    }
}

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
    backend: CompositorBackend,
    last_program: HashMap<MixingUnitId, VideoFrame>,
    last_preview: HashMap<MixingUnitId, VideoFrame>,
    last_multiview: HashMap<MultiviewId, VideoFrame>,
    last_good_inputs: HashMap<InputId, VideoFrame>,
    program_slots: HashMap<MixingUnitId, Arc<BoundedSlot<VideoFrame>>>,
    preview_slots: HashMap<MixingUnitId, Arc<BoundedSlot<VideoFrame>>>,
    multiview_slots: HashMap<MultiviewId, Arc<BoundedSlot<VideoFrame>>>,
    pub metrics: TickMetrics,
    degraded_outputs: Mutex<HashMap<String, String>>,
    asset_root: Option<PathBuf>,
    image_cache: HashMap<AssetId, VideoFrame>,
    delay_lines: HashMap<(InputId, AudioBusId), Vec<VecDeque<f32>>>,
    input_asrc: HashMap<InputId, StreamingAsrc>,
    sources: HashMap<InputId, Arc<dyn MediaSource>>,
    simulated: HashMap<InputId, VideoFrame>,
    pub peak_meters: HashMap<AudioBusId, f32>,
    active_snapshot: Option<Arc<RuntimeSnapshot>>,
    transitions: HashMap<MixingUnitId, TransitionRuntime>,
    #[cfg(feature = "wgpu-backend")]
    wgpu: ConfiguredWgpu,
    #[cfg(feature = "wgpu-backend")]
    program_textures: HashMap<MixingUnitId, eiviz_gpu::WgpuTextureFrame>,
    #[cfg(feature = "wgpu-backend")]
    preview_textures: HashMap<MixingUnitId, eiviz_gpu::WgpuTextureFrame>,
    #[cfg(feature = "wgpu-backend")]
    multiview_textures: HashMap<MultiviewId, eiviz_gpu::WgpuTextureFrame>,
}

#[derive(Clone, Debug)]
struct TransitionRuntime {
    from: RenderPlan,
    remaining_frames: u32,
    duration_frames: u32,
}

impl Runtime {
    /// Explicit [`CompositorBackend::CpuReference`]. Used by CI and the default engine.
    pub fn new(sample_rate: u32) -> Self {
        Self::with_backend(sample_rate, CompositorBackend::CpuReference)
            .expect("CpuReference backend is always available")
    }

    pub fn with_backend(sample_rate: u32, backend: CompositorBackend) -> Result<Self> {
        #[cfg(feature = "wgpu-backend")]
        let wgpu = match backend {
            CompositorBackend::CpuReference => None,
            CompositorBackend::Wgpu => Some(Arc::new(
                eiviz_gpu::WgpuCompositor::new_headless_hardware()
                    .map_err(|e| RuntimeError::Gpu(e.to_string()))?,
            )),
        };
        #[cfg(not(feature = "wgpu-backend"))]
        {
            if matches!(backend, CompositorBackend::Wgpu) {
                return Err(RuntimeError::WgpuFeatureDisabled);
            }
            Self::with_configured_wgpu(sample_rate, backend, ())
        }
        #[cfg(feature = "wgpu-backend")]
        Self::with_configured_wgpu(sample_rate, backend, wgpu)
    }

    /// Construct the desktop runtime around eframe's already-created device.
    #[cfg(feature = "wgpu-backend")]
    pub fn with_wgpu_compositor(
        sample_rate: u32,
        compositor: Arc<eiviz_gpu::WgpuCompositor>,
    ) -> Result<Self> {
        Self::with_configured_wgpu(sample_rate, CompositorBackend::Wgpu, Some(compositor))
    }

    fn with_configured_wgpu(
        sample_rate: u32,
        backend: CompositorBackend,
        _wgpu: ConfiguredWgpu,
    ) -> Result<Self> {
        Ok(Self {
            clock: VirtualClock::new(),
            frame: 0,
            sample_rate,
            backend,
            last_program: HashMap::new(),
            last_preview: HashMap::new(),
            last_multiview: HashMap::new(),
            last_good_inputs: HashMap::new(),
            program_slots: HashMap::new(),
            preview_slots: HashMap::new(),
            multiview_slots: HashMap::new(),
            metrics: TickMetrics::default(),
            degraded_outputs: Mutex::new(HashMap::new()),
            asset_root: None,
            image_cache: HashMap::new(),
            delay_lines: HashMap::new(),
            input_asrc: HashMap::new(),
            sources: HashMap::new(),
            simulated: HashMap::new(),
            peak_meters: HashMap::new(),
            active_snapshot: None,
            transitions: HashMap::new(),
            #[cfg(feature = "wgpu-backend")]
            wgpu: _wgpu,
            #[cfg(feature = "wgpu-backend")]
            program_textures: HashMap::new(),
            #[cfg(feature = "wgpu-backend")]
            preview_textures: HashMap::new(),
            #[cfg(feature = "wgpu-backend")]
            multiview_textures: HashMap::new(),
        })
    }

    pub fn backend(&self) -> CompositorBackend {
        self.backend
    }

    pub fn compositor_detail(&self) -> String {
        match self.backend {
            CompositorBackend::CpuReference => "CpuReference (explicit profile)".into(),
            CompositorBackend::Wgpu => {
                #[cfg(feature = "wgpu-backend")]
                {
                    if let Some(gpu) = &self.wgpu {
                        let info = gpu.adapter_info();
                        return format!(
                            "Wgpu {} ({:?}, {:?})",
                            info.name, info.backend, info.device_type
                        );
                    }
                }
                "Wgpu unavailable".into()
            }
        }
    }

    pub fn set_asset_root(&mut self, root: impl Into<PathBuf>) {
        self.asset_root = Some(root.into());
        self.image_cache.clear();
    }

    pub fn inject_simulated(&mut self, id: InputId, frame: VideoFrame) {
        self.simulated.insert(id, frame);
    }

    pub fn attach_source(&mut self, source: Arc<dyn MediaSource>) {
        self.sources.insert(source.id(), source);
    }

    pub fn detach_source(&mut self, id: InputId) {
        self.sources.remove(&id);
        self.last_good_inputs.remove(&id);
        self.input_asrc.remove(&id);
    }

    pub fn audio_source_diagnostics(&self) -> Vec<AudioIoDiagnostics> {
        self.sources
            .values()
            .filter_map(|source| source.audio_diagnostics())
            .collect()
    }

    pub fn audio_asrc_diagnostics(&self) -> Vec<(InputId, AsrcDiagnostics)> {
        self.input_asrc
            .iter()
            .map(|(input, converter)| (*input, converter.diagnostics()))
            .collect()
    }

    pub fn update_source_playback(&self, id: InputId, playback: &Playback) {
        if let Some(source) = self.sources.get(&id) {
            source.update_playback(playback);
        }
    }

    pub fn clear_simulated(&mut self, id: InputId) {
        self.simulated.remove(&id);
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

    pub fn last_multiview_frame(&self, view: MultiviewId) -> Option<VideoFrame> {
        self.last_multiview.get(&view).cloned()
    }

    #[cfg(feature = "wgpu-backend")]
    pub fn last_program_texture(&self, unit: MixingUnitId) -> Option<eiviz_gpu::WgpuTextureFrame> {
        self.program_textures.get(&unit).cloned()
    }

    #[cfg(feature = "wgpu-backend")]
    pub fn last_preview_texture(&self, unit: MixingUnitId) -> Option<eiviz_gpu::WgpuTextureFrame> {
        self.preview_textures.get(&unit).cloned()
    }

    #[cfg(feature = "wgpu-backend")]
    pub fn last_multiview_texture(&self, view: MultiviewId) -> Option<eiviz_gpu::WgpuTextureFrame> {
        self.multiview_textures.get(&view).cloned()
    }

    #[cfg(feature = "wgpu-backend")]
    pub fn wgpu_diagnostics(&self) -> Option<eiviz_gpu::WgpuDiagnostics> {
        self.wgpu.as_ref().map(|gpu| gpu.diagnostics())
    }

    #[cfg(feature = "wgpu-backend")]
    pub fn wgpu_compositor(&self) -> Option<Arc<eiviz_gpu::WgpuCompositor>> {
        self.wgpu.clone()
    }

    #[cfg(feature = "wgpu-backend")]
    fn capture_program_texture(&mut self, unit: MixingUnitId) {
        if let Some(texture) = self.wgpu.as_ref().and_then(|gpu| gpu.latest_output()) {
            self.program_textures.insert(unit, texture);
        }
    }

    #[cfg(feature = "wgpu-backend")]
    fn capture_preview_texture(&mut self, unit: MixingUnitId) {
        if let Some(texture) = self.wgpu.as_ref().and_then(|gpu| gpu.latest_output()) {
            self.preview_textures.insert(unit, texture);
        }
    }

    #[cfg(feature = "wgpu-backend")]
    fn capture_multiview_texture(&mut self, view: MultiviewId) {
        if let Some(texture) = self.wgpu.as_ref().and_then(|gpu| gpu.latest_output()) {
            self.multiview_textures.insert(view, texture);
        }
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

    /// Latches a precompiled snapshot. Transition progress remains runtime
    /// state and never writes back into the Project.
    pub fn activate_snapshot(&mut self, snapshot: Arc<RuntimeSnapshot>) -> Result<()> {
        let project = snapshot.project();
        if project.compositor != self.backend {
            return Err(RuntimeError::BackendMismatch {
                project: project.compositor,
                runtime: self.backend,
            });
        }
        if snapshot.audio.sample_rate != self.sample_rate {
            return Err(RuntimeError::Other(format!(
                "snapshot sample rate {} does not match runtime {}",
                snapshot.audio.sample_rate, self.sample_rate
            )));
        }
        if let Some(previous) = &self.active_snapshot {
            if previous.audio.resampling != snapshot.audio.resampling {
                self.input_asrc.clear();
            }
            for (unit_id, unit) in &project.mixing_units {
                let old_program = previous
                    .project
                    .mixing_units
                    .get(unit_id)
                    .and_then(|old| old.program.scene);
                if old_program == unit.program.scene {
                    continue;
                }
                if unit.transition.style == TransitionStyle::Mix
                    && unit.transition.duration_frames > 0
                    && let Some(from) = previous.render.programs.get(unit_id)
                {
                    self.transitions.insert(
                        *unit_id,
                        TransitionRuntime {
                            from: from.clone(),
                            remaining_frames: unit.transition.duration_frames,
                            duration_frames: unit.transition.duration_frames,
                        },
                    );
                } else {
                    self.transitions.remove(unit_id);
                }
            }
            self.transitions
                .retain(|unit, _| project.mixing_units.contains_key(unit));
        }
        self.active_snapshot = Some(snapshot);
        Ok(())
    }

    /// Convenience entry point for model tests. Engine uses precompiled
    /// snapshots and [`Self::tick_active`] directly.
    pub fn tick(&mut self, project: &Project) -> Result<TickResult> {
        let changed = self
            .active_snapshot
            .as_ref()
            .is_none_or(|active| active.project() != project);
        if changed {
            let snapshot = Arc::new(RuntimeSnapshot::compile(Arc::new(project.clone()), 0, 0)?);
            self.activate_snapshot(snapshot)?;
        }
        self.tick_active()
    }

    pub fn tick_active(&mut self) -> Result<TickResult> {
        let snapshot = self
            .active_snapshot
            .clone()
            .ok_or_else(|| RuntimeError::Other("no active runtime snapshot".into()))?;
        let project = snapshot.project();
        let pts = MediaTime::from_frame_index(self.frame, project.video.frame_rate)
            .map_err(|e| RuntimeError::Other(e.to_string()))?;
        self.clock.seek_frame(self.frame, project.video.frame_rate);
        let mut sources = self.generate_sources(project, pts)?;
        let mut programs = HashMap::new();
        let mut previews = HashMap::new();
        for &unit_id in snapshot.render.order.iter() {
            let Some(unit) = project.mixing_units.get(&unit_id) else {
                continue;
            };
            fill_mixfeeds(project, &mut sources, &programs, &previews);
            let pg_plan = snapshot
                .render
                .programs
                .get(&unit_id)
                .expect("compiled plan exists for every unit");
            let mut pg = self.composite_plan(pg_plan, &sources, pts)?;
            if let Some(transition) = self.transitions.get(&unit_id).cloned() {
                let from = self.composite_plan(&transition.from, &sources, pts)?;
                let elapsed = transition
                    .duration_frames
                    .saturating_sub(transition.remaining_frames)
                    .saturating_add(1);
                let factor = elapsed as f32 / transition.duration_frames as f32;
                pg = self.mix_transition(&from, &pg, factor.min(1.0), pts)?;
                if transition.remaining_frames <= 1 {
                    self.transitions.remove(&unit_id);
                } else if let Some(live) = self.transitions.get_mut(&unit_id) {
                    live.remaining_frames -= 1;
                }
            }
            #[cfg(feature = "wgpu-backend")]
            self.capture_program_texture(unit.id);
            let pv_plan = snapshot
                .render
                .previews
                .get(&unit_id)
                .expect("compiled plan exists for every unit");
            let pv = self.composite_plan(pv_plan, &sources, pts)?;
            #[cfg(feature = "wgpu-backend")]
            self.capture_preview_texture(unit.id);
            self.program_slot(unit.id)
                .push(pg.clone())
                .map_err(|e| RuntimeError::Other(e.to_string()))?;
            let _ = self.preview_slot(unit.id).push(pv.clone());
            self.last_program.insert(unit.id, pg.clone());
            self.last_preview.insert(unit.id, pv.clone());
            programs.insert(unit.id, pg);
            previews.insert(unit.id, pv);
        }
        let multiviews = self.render_multiviews(project, &sources, &programs, &previews, pts)?;
        let (sample_index, sample_count) =
            audio_frame_sample_span(self.frame, self.sample_rate, project.video.frame_rate)
                .map_err(|e| RuntimeError::Other(e.to_string()))?;
        let (audio, meters) = self.mix_audio(
            project,
            &snapshot.audio,
            sample_index,
            sample_count as usize,
            self.sample_rate,
        )?;
        self.peak_meters = meters.clone();
        self.metrics.frame = self.frame;
        self.frame += 1;
        Ok(TickResult {
            pts,
            programs,
            previews,
            multiviews,
            audio,
            peak_meters: meters,
        })
    }

    fn composite_plan(
        &self,
        plan: &RenderPlan,
        sources: &HashMap<InputId, VideoFrame>,
        pts: MediaTime,
    ) -> Result<VideoFrame> {
        match self.backend {
            CompositorBackend::CpuReference => Ok(composite(plan, sources, pts, self.frame)),
            CompositorBackend::Wgpu => {
                #[cfg(not(feature = "wgpu-backend"))]
                {
                    Err(RuntimeError::WgpuFeatureDisabled)
                }
                #[cfg(feature = "wgpu-backend")]
                {
                    let gpu = self.wgpu.as_ref().ok_or_else(|| {
                        RuntimeError::Gpu("wgpu compositor was not constructed".into())
                    })?;
                    gpu.composite(plan, sources, pts, self.frame)
                        .map_err(|e| RuntimeError::Gpu(e.to_string()))
                }
            }
        }
    }

    fn mix_transition(
        &self,
        a: &VideoFrame,
        b: &VideoFrame,
        factor: f32,
        pts: MediaTime,
    ) -> Result<VideoFrame> {
        match self.backend {
            CompositorBackend::CpuReference => Ok(mix_frames(a, b, factor, pts, self.frame)),
            CompositorBackend::Wgpu => {
                #[cfg(not(feature = "wgpu-backend"))]
                {
                    Err(RuntimeError::WgpuFeatureDisabled)
                }
                #[cfg(feature = "wgpu-backend")]
                {
                    let gpu = self.wgpu.as_ref().ok_or_else(|| {
                        RuntimeError::Gpu("wgpu compositor was not constructed".into())
                    })?;
                    gpu.mix(a, b, factor, pts, self.frame)
                        .map_err(|error| RuntimeError::Gpu(error.to_string()))
                }
            }
        }
    }

    fn render_multiviews(
        &mut self,
        project: &Project,
        input_sources: &HashMap<InputId, VideoFrame>,
        programs: &HashMap<MixingUnitId, VideoFrame>,
        previews: &HashMap<MixingUnitId, VideoFrame>,
        pts: MediaTime,
    ) -> Result<HashMap<MultiviewId, VideoFrame>> {
        let mut rendered = HashMap::new();
        for view in project.multiviews.values() {
            let mut sources = input_sources.clone();
            let mut layers = Vec::with_capacity(view.tiles.len());
            let mut synthetic = u128::MAX;
            for tile in &view.tiles {
                let input = match tile.source {
                    MultiviewSource::Black => continue,
                    MultiviewSource::Input(input) => input,
                    MultiviewSource::Program(unit) => {
                        let frame = programs.get(&unit).ok_or_else(|| {
                            RuntimeError::Other(format!(
                                "multiview {} missing Program for {}",
                                view.id, unit
                            ))
                        })?;
                        let id = next_synthetic_input(&sources, &mut synthetic)?;
                        sources.insert(id, frame.clone());
                        id
                    }
                    MultiviewSource::Preview(unit) => {
                        let frame = previews.get(&unit).ok_or_else(|| {
                            RuntimeError::Other(format!(
                                "multiview {} missing Preview for {}",
                                view.id, unit
                            ))
                        })?;
                        let id = next_synthetic_input(&sources, &mut synthetic)?;
                        sources.insert(id, frame.clone());
                        id
                    }
                };
                layers.push(Layer {
                    input,
                    transform: Transform2D {
                        x: tile.column as f32 / view.columns as f32,
                        y: tile.row as f32 / view.rows as f32,
                        width: 1.0 / view.columns as f32,
                        height: 1.0 / view.rows as f32,
                        ..Transform2D::default()
                    },
                    opacity: 1.0,
                });
            }
            let plan = RenderPlan {
                width: project.video.width,
                height: project.video.height,
                layers,
            };
            let frame = self.composite_plan(&plan, &sources, pts)?;
            #[cfg(feature = "wgpu-backend")]
            self.capture_multiview_texture(view.id);
            self.multiview_slots
                .entry(view.id)
                .or_insert_with(|| Arc::new(BoundedSlot::new("multiview", QueuePolicy::LatestWins)))
                .push(frame.clone())
                .map_err(|error| RuntimeError::Other(error.to_string()))?;
            self.last_multiview.insert(view.id, frame.clone());
            rendered.insert(view.id, frame);
        }
        Ok(rendered)
    }

    fn generate_sources(
        &mut self,
        project: &Project,
        pts: MediaTime,
    ) -> Result<HashMap<InputId, VideoFrame>> {
        let mut out = HashMap::new();
        let w = project.video.width.clamp(16, 1920);
        let h = project.video.height.clamp(16, 1080);
        for input in project.inputs.values() {
            if matches!(input.source, InputSource::AudioDevice { .. }) {
                let mut frame = VideoFrame::rgba_solid(self.frame, pts, w, h, [0, 0, 0, 255]);
                frame.source = Some(input.id);
                out.insert(input.id, frame);
                continue;
            }
            if let Some(source) = self.sources.get(&input.id) {
                match source.pull_video(pts, project.video.frame_rate) {
                    Ok(Some(mut frame)) => {
                        frame.source = Some(input.id);
                        self.last_good_inputs.insert(input.id, frame.clone());
                        out.insert(input.id, frame);
                        continue;
                    }
                    Ok(None) => {
                        let frame =
                            self.missing_frame(project, input.id, w, h, pts, "source pending")?;
                        out.insert(input.id, frame);
                        continue;
                    }
                    Err(error) => {
                        return Err(RuntimeError::Other(format!(
                            "source {} video pull failed: {error}",
                            input.id
                        )));
                    }
                }
            }
            if let Some(sim) = self.simulated.get(&input.id) {
                let mut f = sim.clone();
                f.pts = pts;
                f.source = Some(input.id);
                self.last_good_inputs.insert(input.id, f.clone());
                out.insert(input.id, f);
                continue;
            }
            let (mut generated, authentic) = match &input.source {
                InputSource::ColorBars => (color_bars(self.frame, pts, w, h), true),
                InputSource::SolidColor { r, g, b, a } => (
                    VideoFrame::rgba_solid(self.frame, pts, w, h, [*r, *g, *b, *a]),
                    true,
                ),
                InputSource::Image { asset } => match self.try_load_image(project, *asset, pts) {
                    Some(frame) => (frame, true),
                    None => (
                        self.missing_frame(project, input.id, w, h, pts, "image")?,
                        false,
                    ),
                },
                InputSource::Video { .. } => (
                    self.missing_frame(
                        project,
                        input.id,
                        w,
                        h,
                        pts,
                        "video decoder source is not attached",
                    )?,
                    false,
                ),
                InputSource::Ndi { .. }
                | InputSource::Omt { .. }
                | InputSource::DeckLink { .. } => (
                    self.missing_frame(project, input.id, w, h, pts, "live input")?,
                    false,
                ),
                InputSource::AudioDevice { .. } => (
                    VideoFrame::rgba_solid(self.frame, pts, w, h, [0, 0, 0, 255]),
                    true,
                ),
                InputSource::MixFeed { .. } => (
                    VideoFrame::rgba_solid(self.frame, pts, w, h, [0, 0, 0, 255]),
                    false,
                ),
            };
            generated.source = Some(input.id);
            if authentic {
                self.last_good_inputs.insert(input.id, generated.clone());
            }
            out.insert(input.id, generated);
        }
        Ok(out)
    }

    fn missing_frame(
        &self,
        project: &Project,
        id: InputId,
        w: u32,
        h: u32,
        pts: MediaTime,
        why: &str,
    ) -> Result<VideoFrame> {
        match project.missing_media {
            MissingMediaPolicy::Fail => Err(RuntimeError::MissingMedia(why.into())),
            MissingMediaPolicy::Slate => {
                Ok(VideoFrame::rgba_solid(self.frame, pts, w, h, SLATE_RGBA))
            }
            MissingMediaPolicy::LastGood => {
                self.last_good_inputs.get(&id).cloned().ok_or_else(|| {
                    RuntimeError::MissingMedia(format!(
                        "LastGood requested but no prior frame ({why})"
                    ))
                })
            }
        }
    }

    fn try_load_image(
        &mut self,
        project: &Project,
        asset: AssetId,
        pts: MediaTime,
    ) -> Option<VideoFrame> {
        if let Some(cached) = self.image_cache.get(&asset) {
            let mut f = cached.clone();
            f.pts = pts;
            f.id = self.frame;
            return Some(f);
        }
        let meta = project.assets.get(&asset)?;
        if meta.missing {
            return None;
        }
        let root = self.asset_root.as_ref()?;
        let path = root.join(&meta.relative_path);
        let (width, height, data) = decode_rgba(&path)?;
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
        Some(frame)
    }

    fn mix_audio(
        &mut self,
        project: &Project,
        plan: &AudioPlan,
        sample_index: u64,
        frames: usize,
        sample_rate: u32,
    ) -> Result<(AudioBuffer, HashMap<AudioBusId, f32>)> {
        let ch = plan.channels.max(1) as usize;
        let mut mixed = AudioBuffer::silence(sample_index, sample_rate, plan.channels, frames);
        let mut source_audio = HashMap::new();
        for route in &plan.matrix.routes {
            if source_audio.contains_key(&route.input) {
                continue;
            }
            let Some(source) = self.sources.get(&route.input) else {
                if project
                    .inputs
                    .get(&route.input)
                    .is_some_and(|input| matches!(input.source, InputSource::AudioDevice { .. }))
                {
                    return Err(RuntimeError::MissingMedia(format!(
                        "audio device source {} is selected but not attached",
                        route.input
                    )));
                }
                continue;
            };
            let requested_frames = self
                .input_asrc
                .get(&route.input)
                .map_or(frames, |converter| {
                    ((frames as u128 * converter.input_rate() as u128)
                        .div_ceil(sample_rate as u128)) as usize
                });
            let buffer = source
                .pull_audio(sample_index, requested_frames)
                .map_err(|error| {
                    RuntimeError::Other(format!(
                        "source {} audio pull failed: {error}",
                        route.input
                    ))
                })?;
            if let Some(buffer) = buffer {
                if buffer.sample_rate != sample_rate {
                    let AudioResamplingPolicy::Asrc { profile } = plan.resampling else {
                        return Err(RuntimeError::AudioRateMismatch {
                            input: route.input,
                            source_rate: buffer.sample_rate,
                            project_rate: sample_rate,
                        });
                    };
                    let replace = self.input_asrc.get(&route.input).is_none_or(|converter| {
                        converter.input_rate() != buffer.sample_rate
                            || converter.output_rate() != sample_rate
                            || converter.channels() != buffer.channels
                    });
                    if replace {
                        if self.input_asrc.contains_key(&route.input) && !buffer.discontinuity {
                            return Err(RuntimeError::Other(format!(
                                "source {} changed audio format without a discontinuity marker",
                                route.input
                            )));
                        }
                        self.input_asrc.insert(
                            route.input,
                            StreamingAsrc::new(
                                buffer.sample_rate,
                                sample_rate,
                                buffer.channels,
                                profile,
                            )
                            .map_err(|error| RuntimeError::Other(error.to_string()))?,
                        );
                    }
                    let converted = self
                        .input_asrc
                        .get_mut(&route.input)
                        .expect("ASRC inserted above")
                        .process(&buffer, sample_index, frames)
                        .map_err(|error| RuntimeError::Other(error.to_string()))?;
                    source_audio.insert(route.input, converted);
                    continue;
                }
                self.input_asrc.remove(&route.input);
                source_audio.insert(route.input, buffer);
            }
        }
        for route in &plan.matrix.routes {
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
            let gain = plan.matrix.effective_linear_gain(route, follow_active);
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
                #[allow(clippy::needless_range_loop)]
                for c in 0..ch {
                    let source_sample = source_audio
                        .get(&route.input)
                        .and_then(|buffer| {
                            buffer
                                .planes
                                .get(c.min(buffer.planes.len().saturating_sub(1)))
                        })
                        .and_then(|plane| plane.get(n))
                        .copied()
                        .unwrap_or(0.0);
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
                    lines[c].push_back(source_sample * gain * g);
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
        for bus in &plan.matrix.buses {
            let mut peak = 0.0f32;
            for plane in &mixed.planes {
                for s in plane {
                    peak = peak.max(s.abs());
                }
            }
            meters.insert(bus.id, peak);
        }
        Ok((mixed, meters))
    }
}

#[derive(Clone, Debug)]
pub struct TickResult {
    pub pts: MediaTime,
    pub programs: HashMap<MixingUnitId, VideoFrame>,
    pub previews: HashMap<MixingUnitId, VideoFrame>,
    pub multiviews: HashMap<MultiviewId, VideoFrame>,
    pub audio: AudioBuffer,
    pub peak_meters: HashMap<AudioBusId, f32>,
}

fn next_synthetic_input(
    sources: &HashMap<InputId, VideoFrame>,
    cursor: &mut u128,
) -> Result<InputId> {
    loop {
        let id = InputId::from_u128(*cursor);
        *cursor = (*cursor)
            .checked_sub(1)
            .ok_or_else(|| RuntimeError::Other("synthetic multiview id exhausted".into()))?;
        if !sources.contains_key(&id) {
            return Ok(id);
        }
    }
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
    let bytes = std::fs::read(path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some((w, h, img.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_core::{
        AudioRoute, DeviceBinding, DeviceBindingId, Input, InputSource, MixingUnit, MixingUnitId,
        Multiview, MultiviewId, MultiviewSource, MultiviewTile, RouteMode, Scene, SceneItem,
        Transform2D, TransitionStyle,
    };
    use eiviz_core::{InputId, SceneId, SceneItemId};

    struct RegisteredSource {
        id: InputId,
        video: VideoFrame,
        audio: AudioBuffer,
    }

    impl MediaSource for RegisteredSource {
        fn id(&self) -> InputId {
            self.id
        }

        fn pull_video(
            &self,
            _pts: MediaTime,
            _rate: eiviz_time::FrameRate,
        ) -> eiviz_media::Result<Option<VideoFrame>> {
            Ok(Some(self.video.clone()))
        }

        fn pull_audio(
            &self,
            _sample_index: u64,
            _frames: usize,
        ) -> eiviz_media::Result<Option<AudioBuffer>> {
            Ok(Some(self.audio.clone()))
        }
    }

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
        let before = rt.tick(&p).unwrap();
        assert_eq!(before.programs[&unit].pixel(10, 10)[0], 0);
        p.mixing_units.get_mut(&unit).unwrap().transition.style = TransitionStyle::Cut;
        p.mixing_units.get_mut(&unit).unwrap().take(false);
        let after = rt.tick(&p).unwrap();
        assert_eq!(after.programs[&unit].pixel(10, 10), [255, 0, 0, 255]);
        assert_eq!(p.mixing_units[&unit].program.scene, Some(sid));
        let peak = after.audio.planes[0]
            .iter()
            .fold(0.0f32, |a, x| a.max(x.abs()));
        assert!(
            peak < 1e-6,
            "a route without an attached source must not invent a tone"
        );
        assert!(after.peak_meters.values().all(|m| *m < 1e-6));
    }

    #[test]
    fn transition_progress_never_mutates_project_snapshot() {
        let (mut project, _, scene, unit) = setup();
        let mut runtime = Runtime::new(48_000);
        runtime.tick(&project).unwrap();
        {
            let unit = project.mixing_units.get_mut(&unit).unwrap();
            unit.transition.style = TransitionStyle::Mix;
            unit.transition.duration_frames = 3;
            unit.take(false);
        }
        let latched = project.clone();
        for _ in 0..4 {
            runtime.tick(&project).unwrap();
            assert_eq!(project, latched);
        }
        assert_eq!(project.mixing_units[&unit].program.scene, Some(scene));
    }

    #[test]
    fn slow_sink_does_not_stop_program() {
        let (p, _, _, unit) = setup();
        let mut rt = Runtime::new(48000);
        rt.mark_output_failed("rtmp-primary", "connection reset");
        for _ in 0..10 {
            rt.tick(&p).unwrap();
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
        let tick = rt.tick(&p).unwrap();
        assert_eq!(tick.programs[&a].pixel(8, 8), [255, 0, 0, 255]);
        assert_eq!(tick.programs[&b].pixel(8, 8), [255, 0, 0, 255]);
    }

    #[test]
    fn delay_line_silences_first_buffer() {
        let (mut p, _, _, unit) = setup();
        p.audio_matrix.routes[0].mode = RouteMode::Manual;
        p.audio_matrix.routes[0].delay_ms = 1000.0;
        let mut rt = Runtime::new(48000);
        let first = rt.tick(&p).unwrap();
        let peak = first.audio.planes[0]
            .iter()
            .fold(0.0f32, |a, x| a.max(x.abs()));
        assert!(peak < 1e-6, "1s delay must zero the first 59.94 frame");
        let _ = unit;
    }

    #[cfg(not(feature = "wgpu-backend"))]
    #[test]
    fn wgpu_without_feature_is_hard_error() {
        match Runtime::with_backend(48_000, CompositorBackend::Wgpu) {
            Err(RuntimeError::WgpuFeatureDisabled) => {}
            Err(e) => panic!("unexpected error: {e}"),
            Ok(_) => panic!("Wgpu must not construct a CPU runtime"),
        }
    }

    #[test]
    fn wgpu_project_does_not_run_on_cpu_runtime() {
        let (mut p, _, _, _) = setup();
        p.compositor = CompositorBackend::Wgpu;
        let mut rt = Runtime::new(48_000);
        let err = rt.tick(&p).unwrap_err();
        assert!(matches!(err, RuntimeError::BackendMismatch { .. }));
    }

    #[test]
    fn missing_live_fail_does_not_invent_a_camera() {
        let (mut p, iid, _, _) = setup();
        p.missing_media = MissingMediaPolicy::Fail;
        p.inputs.get_mut(&iid).unwrap().source = InputSource::Ndi {
            source_name: "cam".into(),
        };
        let mut rt = Runtime::new(48_000);
        let err = rt.tick(&p).unwrap_err();
        assert!(matches!(err, RuntimeError::MissingMedia(_)));
    }

    #[test]
    fn selected_audio_device_without_attachment_is_a_hard_error() {
        let (mut project, input, _, _) = setup();
        let binding = DeviceBinding {
            id: DeviceBindingId::new(),
            kind: "audio:alsa".into(),
            logical_name: "missing interface".into(),
            last_seen_hardware_id: Some("alsa:missing".into()),
        };
        project.device_bindings.insert(binding.id, binding.clone());
        project.inputs.get_mut(&input).unwrap().source = InputSource::AudioDevice {
            binding: binding.id,
        };
        project.audio_matrix.routes[0].mode = RouteMode::Manual;
        let mut runtime = Runtime::new(48_000);
        let error = runtime.tick(&project).unwrap_err();
        assert!(matches!(error, RuntimeError::MissingMedia(_)));
    }

    #[test]
    fn exact_rate_policy_rejects_mismatched_live_audio() {
        let (mut project, input, _, _) = setup();
        project.audio_matrix.routes[0].mode = RouteMode::Manual;
        let source = RegisteredSource {
            id: input,
            video: VideoFrame::rgba_solid(1, MediaTime::ZERO, 2, 2, [0, 0, 0, 255]),
            audio: AudioBuffer::silence(0, 44_100, 2, 736),
        };
        let mut runtime = Runtime::new(48_000);
        runtime.attach_source(Arc::new(source));
        let error = runtime.tick(&project).unwrap_err();
        assert!(matches!(
            error,
            RuntimeError::AudioRateMismatch {
                input: source,
                source_rate: 44_100,
                project_rate: 48_000,
            } if source == input
        ));
        assert!(runtime.audio_asrc_diagnostics().is_empty());
    }

    #[test]
    fn selected_asrc_is_compiled_into_audio_plan_and_preserves_channels() {
        let (mut project, input, _, _) = setup();
        project.audio.resampling = AudioResamplingPolicy::Asrc {
            profile: eiviz_core::AsrcProfile::broadcast(),
        };
        project.audio_matrix.routes[0].mode = RouteMode::Manual;
        let mut audio = AudioBuffer::silence(0, 44_100, 2, 2_000);
        audio.planes[0].fill(0.25);
        audio.planes[1].fill(-0.5);
        let source = RegisteredSource {
            id: input,
            video: VideoFrame::rgba_solid(1, MediaTime::ZERO, 2, 2, [0, 0, 0, 255]),
            audio,
        };
        let mut runtime = Runtime::new(48_000);
        runtime.attach_source(Arc::new(source));
        let tick = runtime.tick(&project).unwrap();
        assert_eq!(tick.audio.sample_rate, 48_000);
        assert_eq!(tick.audio.channels, 2);
        assert_eq!(tick.audio.planes.len(), 2);
        let diagnostics = runtime.audio_asrc_diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].1.input_rate, 44_100);
        assert_eq!(diagnostics[0].1.output_rate, 48_000);
    }

    #[test]
    fn missing_live_slate_is_configured_not_a_fake_feed() {
        let (mut p, iid, sid, unit) = setup();
        p.missing_media = MissingMediaPolicy::Slate;
        p.inputs.get_mut(&iid).unwrap().source = InputSource::Ndi {
            source_name: "cam".into(),
        };
        p.mixing_units.get_mut(&unit).unwrap().program.scene = Some(sid);
        let mut rt = Runtime::new(48_000);
        let tick = rt.tick(&p).unwrap();
        assert_eq!(tick.programs[&unit].pixel(8, 8), SLATE_RGBA);
    }

    #[test]
    fn last_good_replays_prior_authentic_frame() {
        let (mut p, iid, sid, unit) = setup();
        p.missing_media = MissingMediaPolicy::LastGood;
        p.mixing_units.get_mut(&unit).unwrap().program.scene = Some(sid);
        let mut rt = Runtime::new(48_000);
        let first = rt.tick(&p).unwrap();
        assert_eq!(first.programs[&unit].pixel(8, 8), [255, 0, 0, 255]);
        p.inputs.get_mut(&iid).unwrap().source = InputSource::Ndi {
            source_name: "cam".into(),
        };
        let second = rt.tick(&p).unwrap();
        assert_eq!(second.programs[&unit].pixel(8, 8), [255, 0, 0, 255]);
    }

    #[test]
    fn last_good_without_prior_frame_errors() {
        let (mut p, iid, _, _) = setup();
        p.missing_media = MissingMediaPolicy::LastGood;
        p.inputs.get_mut(&iid).unwrap().source = InputSource::Ndi {
            source_name: "cam".into(),
        };
        let mut rt = Runtime::new(48_000);
        let err = rt.tick(&p).unwrap_err();
        assert!(matches!(err, RuntimeError::MissingMedia(_)));
    }

    #[test]
    fn registered_live_source_reaches_program_and_audio_matrix() {
        let (mut p, iid, sid, unit) = setup();
        p.inputs.get_mut(&iid).unwrap().source = InputSource::Omt {
            url: "omt://test".into(),
        };
        p.mixing_units.get_mut(&unit).unwrap().program.scene = Some(sid);
        p.audio_matrix.routes[0].mode = RouteMode::Manual;
        let video = VideoFrame::rgba_solid(1, MediaTime::ZERO, 16, 16, [7, 8, 9, 255]);
        let mut audio = AudioBuffer::silence(0, 48_000, 2, 801);
        audio.planes[0].fill(0.5);
        audio.planes[1].fill(-0.25);
        let mut rt = Runtime::new(48_000);
        rt.attach_source(Arc::new(RegisteredSource {
            id: iid,
            video,
            audio,
        }));
        let tick = rt.tick(&p).unwrap();
        assert_eq!(tick.programs[&unit].pixel(8, 8), [7, 8, 9, 255]);
        assert!(tick.audio.planes[0].iter().any(|sample| *sample > 0.4));
        assert!(tick.audio.planes[1].iter().any(|sample| *sample < -0.2));
    }

    #[test]
    fn multiview_renders_input_preview_and_program_tiles() {
        let (mut p, iid, sid, unit) = setup();
        let blue = Input {
            id: InputId::new(),
            name: "blue".into(),
            tags: vec![],
            groups: vec![],
            source: InputSource::SolidColor {
                r: 0,
                g: 0,
                b: 255,
                a: 255,
            },
        };
        let blue_scene = Scene {
            id: SceneId::new(),
            name: "blue".into(),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: blue.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        p.inputs.insert(blue.id, blue.clone());
        p.scenes.insert(blue_scene.id, blue_scene.clone());
        p.mixing_units.get_mut(&unit).unwrap().program.scene = Some(sid);
        p.mixing_units.get_mut(&unit).unwrap().preview.scene = Some(blue_scene.id);
        let view = Multiview {
            id: MultiviewId::new(),
            name: "three-up".into(),
            owner: unit,
            columns: 3,
            rows: 1,
            tiles: vec![
                MultiviewTile {
                    column: 0,
                    row: 0,
                    source: MultiviewSource::Input(iid),
                },
                MultiviewTile {
                    column: 1,
                    row: 0,
                    source: MultiviewSource::Program(unit),
                },
                MultiviewTile {
                    column: 2,
                    row: 0,
                    source: MultiviewSource::Preview(unit),
                },
            ],
        };
        p.multiviews.insert(view.id, view.clone());
        p.mixing_units
            .get_mut(&unit)
            .unwrap()
            .multiviews
            .push(view.id);
        p.validate().unwrap();
        let mut rt = Runtime::new(48_000);
        let tick = rt.tick(&p).unwrap();
        let frame = &tick.multiviews[&view.id];
        assert_eq!(frame.pixel(100, 100), [255, 0, 0, 255]);
        assert_eq!(frame.pixel(800, 100), [255, 0, 0, 255]);
        assert_eq!(frame.pixel(1500, 100), [0, 0, 255, 255]);
        assert_eq!(
            rt.last_multiview_frame(view.id).unwrap().pixel(1500, 100),
            [0, 0, 255, 255]
        );
    }
}
