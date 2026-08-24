use eiviz_codec_software::{
    DynamicEncoderFactory, EncoderDiagnostics, EncoderSessionRequest, ProgramEncoder,
    ProgramEncoderFactory,
};
use eiviz_command::{Command, CommandAck, CommandEnvelope, Sequencer, state_hash};
use eiviz_core::{
    AacEncoderProfile, AssetRef, ClientId, H264EncoderProfile, Input, InputId, InputSource,
    MixingUnitId, MultiviewId, Output, OutputId, OutputKind, Playback, Project,
};
use eiviz_io_stream::{EncodedFanout, EncoderCapabilities, SinkDiagnostics};
use eiviz_media::{AudioIoDiagnostics, AudioSink, MediaSink, MediaSource, VideoFrame};
use eiviz_project::{
    append_journal, export_portable as export_project_portable,
    import_portable as import_project_portable, load, save_atomic, save_autosave, stage_asset,
};
use eiviz_runtime::{Runtime, TickResult};
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const FLIGHT_CAP: usize = 1800;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Command(#[from] eiviz_command::CommandError),
    #[error(transparent)]
    Runtime(#[from] eiviz_runtime::RuntimeError),
    #[error(transparent)]
    Persist(#[from] eiviz_project::ProjectError),
    #[error(transparent)]
    Media(#[from] eiviz_media::MediaError),
    #[error("admission denied: {0}")]
    Admission(String),
    #[error("unknown output {0}")]
    UnknownOutput(OutputId),
    #[error("output {0} is not an audio-device output")]
    NotAudioOutput(OutputId),
    #[error("output {0} is not a distribution output")]
    NotDistributionOutput(OutputId),
}

pub type Result<T> = std::result::Result<T, EngineError>;

#[derive(Clone, Debug)]
pub struct AdmissionBudget {
    pub max_inputs: usize,
    pub max_units: usize,
    pub max_pixel_bytes: usize,
}

impl Default for AdmissionBudget {
    fn default() -> Self {
        Self {
            max_inputs: 4096,
            max_units: 1024,
            max_pixel_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FlightEvent {
    pub frame: u64,
    pub revision: u64,
    pub hash: String,
    pub dropped_preview: u64,
    pub failed_outputs: Vec<String>,
    pub note: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct EngineMetrics {
    pub revision: u64,
    pub frame: u64,
    pub state_hash: String,
    pub failed_outputs: Vec<(String, String)>,
    pub peak_meters: Vec<(String, f32)>,
    pub audio_devices: Vec<EngineAudioDiagnostics>,
    pub distribution_outputs: Vec<DistributionOutputDiagnostics>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DistributionOutputDiagnostics {
    pub output_id: String,
    pub name: String,
    pub enabled: bool,
    pub state: String,
    pub detail: String,
    pub queue_depth: usize,
    pub queue_high_water: usize,
    pub enqueued: u64,
    pub sent: u64,
    pub dropped: u64,
    pub reconnects: u64,
    pub video_frames: u64,
    pub keyframes: u64,
    pub audio_access_units: u64,
    pub idr_requests: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct EngineAudioDiagnostics {
    pub name: String,
    pub health: String,
    pub callbacks: u64,
    pub device_frames: u64,
    pub xruns: u64,
    pub queue_overflows: u64,
    pub queue_underflows: u64,
    pub last_device_sample_index: u64,
    pub last_callback_nanos: u64,
    pub last_device_nanos: u64,
    pub last_error: Option<String>,
}

/// Process-wide composition root. GUI and control adapters only talk to this.
pub struct Engine {
    inner: Mutex<Inner>,
}

struct Inner {
    project: Project,
    sequencer: Sequencer,
    runtime: Runtime,
    client: ClientId,
    budget: AdmissionBudget,
    flight: VecDeque<FlightEvent>,
    sinks: HashMap<OutputId, Arc<dyn MediaSink>>,
    audio_sinks: HashMap<OutputId, Arc<dyn AudioSink>>,
    encoder_factory: Option<Arc<dyn ProgramEncoderFactory>>,
    encoder_capabilities: EncoderCapabilities,
    encoder_sessions: HashMap<EncodingProfileKey, EncodingSession>,
    distribution_bindings: HashMap<OutputId, DistributionBinding>,
    autosave_path: Option<PathBuf>,
    journal_path: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct EncodingProfileKey {
    owner: MixingUnitId,
    video: H264EncoderProfile,
    audio: AacEncoderProfile,
}

struct EncodingSession {
    encoder: Box<dyn ProgramEncoder>,
    fanout: EncodedFanout,
    last_error: Option<String>,
}

struct DistributionBinding {
    profile: EncodingProfileKey,
    sink_name: String,
}

impl Engine {
    pub fn new(name: impl Into<String>) -> Self {
        Self::from_project(Project::new(name)).expect("default project uses CpuReference")
    }

    pub fn from_project(project: Project) -> Result<Self> {
        validate_enabled_distribution_outputs(&project)?;
        let runtime = Runtime::with_backend(project.audio.sample_rate, project.compositor)?;
        Ok(Self {
            inner: Mutex::new(Inner {
                project,
                sequencer: Sequencer::default(),
                runtime,
                client: ClientId::new(),
                budget: AdmissionBudget::default(),
                flight: VecDeque::with_capacity(FLIGHT_CAP),
                sinks: HashMap::new(),
                audio_sinks: HashMap::new(),
                encoder_factory: None,
                encoder_capabilities: EncoderCapabilities::default(),
                encoder_sessions: HashMap::new(),
                distribution_bindings: HashMap::new(),
                autosave_path: None,
                journal_path: None,
            }),
        })
    }

    pub fn shared(self) -> Arc<Self> {
        Arc::new(self)
    }

    pub fn set_asset_root(&self, root: impl Into<PathBuf>) {
        self.inner.lock().runtime.set_asset_root(root);
    }

    pub fn ingest_asset(&self, file: &Path, asset_root: &Path) -> Result<AssetRef> {
        let asset = stage_asset(file, asset_root)?;
        self.submit_payload(Command::AddAsset {
            asset: asset.clone(),
        })?;
        self.set_asset_root(asset_root.to_path_buf());
        Ok(asset)
    }

    /// Stages an MP4, validates the explicit Cisco binary, and attaches its real decoder source.
    pub fn ingest_video(
        &self,
        file: &Path,
        asset_root: &Path,
        openh264_binary: &Path,
        playback: Playback,
    ) -> Result<Input> {
        if self.snapshot().video.color != eiviz_core::ColorSpace::Bt709Sdr {
            return Err(EngineError::Admission(
                "H.264 software decode currently requires explicit Bt709Sdr project profile".into(),
            ));
        }
        let asset = stage_asset(file, asset_root)?;
        let input = Input {
            id: InputId::new(),
            name: asset.original_name.clone(),
            tags: vec!["video".into(), "h264".into()],
            groups: vec![],
            source: InputSource::Video {
                asset: asset.id,
                playback: playback.clone(),
            },
        };
        let source = Arc::new(eiviz_io_file::VideoFileSource::open(
            input.id,
            &asset_root.join(&asset.relative_path),
            openh264_binary,
            playback,
        )?);
        self.submit_payload(Command::AddAsset { asset })?;
        self.submit_payload(Command::AddInput {
            input: input.clone(),
        })?;
        self.set_asset_root(asset_root.to_path_buf());
        self.attach_source(source);
        Ok(input)
    }

    pub fn set_video_playback(&self, input: InputId, playback: Playback) -> Result<CommandAck> {
        self.submit_payload(Command::SetInputPlayback { input, playback })
    }

    pub fn set_autosave_path(&self, path: impl Into<PathBuf>) {
        let mut g = self.inner.lock();
        let p = path.into();
        g.journal_path = Some(p.with_extension("journal.jsonl"));
        g.autosave_path = Some(p);
    }

    pub fn attach_sink(&self, sink: Arc<dyn MediaSink>) {
        let mut inner = self.inner.lock();
        if let Some(output) = inner.project.outputs.keys().next().copied() {
            inner.sinks.insert(output, sink);
        }
    }

    pub fn attach_output_sink(&self, output: OutputId, sink: Arc<dyn MediaSink>) -> Result<()> {
        let mut inner = self.inner.lock();
        if !inner.project.outputs.contains_key(&output) {
            return Err(EngineError::UnknownOutput(output));
        }
        inner.sinks.insert(output, sink);
        Ok(())
    }

    pub fn detach_output_sink(&self, output: OutputId) {
        self.inner.lock().sinks.remove(&output);
    }

    pub fn attach_audio_output(&self, output: OutputId, sink: Arc<dyn AudioSink>) -> Result<()> {
        let mut inner = self.inner.lock();
        let Some(configured) = inner.project.outputs.get(&output) else {
            return Err(EngineError::UnknownOutput(output));
        };
        if !matches!(configured.kind, OutputKind::AudioDevice { .. }) {
            return Err(EngineError::NotAudioOutput(output));
        }
        inner.audio_sinks.insert(output, sink);
        Ok(())
    }

    pub fn detach_audio_output(&self, output: OutputId) {
        self.inner.lock().audio_sinks.remove(&output);
    }

    pub fn attach_source(&self, source: Arc<dyn MediaSource>) {
        self.inner.lock().runtime.attach_source(source);
    }

    pub fn detach_source(&self, id: eiviz_core::InputId) {
        self.inner.lock().runtime.detach_source(id);
    }

    pub fn client(&self) -> ClientId {
        self.inner.lock().client
    }

    pub fn revision(&self) -> u64 {
        self.inner.lock().sequencer.revision()
    }

    pub fn snapshot(&self) -> Project {
        self.inner.lock().project.clone()
    }

    pub fn state_hash(&self) -> String {
        let g = self.inner.lock();
        state_hash(&g.project)
    }

    pub fn compositor_detail(&self) -> String {
        self.inner.lock().runtime.compositor_detail()
    }

    pub fn metrics(&self) -> EngineMetrics {
        let g = self.inner.lock();
        let mut audio_devices = g.runtime.audio_source_diagnostics();
        audio_devices.extend(g.audio_sinks.values().map(|sink| sink.diagnostics()));
        EngineMetrics {
            revision: g.sequencer.revision(),
            frame: g.runtime.frame(),
            state_hash: state_hash(&g.project),
            failed_outputs: g.runtime.failed_outputs(),
            peak_meters: g
                .runtime
                .peak_meters
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect(),
            audio_devices: audio_devices
                .into_iter()
                .map(EngineAudioDiagnostics::from)
                .collect(),
            distribution_outputs: distribution_diagnostics(&g),
        }
    }

    pub fn distribution_capabilities(&self) -> Vec<eiviz_media::Capability> {
        let g = self.inner.lock();
        let mut capabilities = eiviz_io_stream::capabilities();
        let description = g.encoder_factory.as_ref().map_or_else(
            || "no explicit dynamic encoder binaries configured".into(),
            |factory| factory.description(),
        );
        capabilities.push(eiviz_media::Capability {
            id: "distribution-cisco-openh264-2.6".into(),
            available: g.encoder_capabilities.cisco_openh264_26,
            detail: description.clone(),
        });
        capabilities.push(eiviz_media::Capability {
            id: "distribution-fdk-aac-lc".into(),
            available: g.encoder_capabilities.fdk_aac_lc,
            detail: description,
        });
        capabilities
    }

    /// Select explicit operator-provided dynamic binaries. OpenH264 is
    /// hash/version verified when a session starts. FDK is loaded only through
    /// its AAC-LC raw access-unit ABI; absence is a hard failure.
    pub fn configure_distribution_binaries(
        &self,
        openh264_binary: impl Into<PathBuf>,
        fdk_aac_binary: Option<PathBuf>,
    ) -> Result<()> {
        let mut g = self.inner.lock();
        if !g.encoder_sessions.is_empty() {
            return Err(EngineError::Admission(
                "stop all distribution outputs before changing encoder binaries".into(),
            ));
        }
        let fdk_configured = fdk_aac_binary.is_some();
        g.encoder_factory = Some(Arc::new(DynamicEncoderFactory::new(
            openh264_binary,
            fdk_aac_binary,
        )));
        g.encoder_capabilities = EncoderCapabilities {
            cisco_openh264_26: true,
            fdk_aac_lc: fdk_configured,
            ..Default::default()
        };
        Ok(())
    }

    /// Explicit adapter injection point for licensed external encoders and
    /// deterministic mocks. Replacing a live factory is rejected.
    pub fn install_distribution_encoder_factory(
        &self,
        factory: Arc<dyn ProgramEncoderFactory>,
        capabilities: EncoderCapabilities,
    ) -> Result<()> {
        let mut g = self.inner.lock();
        if !g.encoder_sessions.is_empty() {
            return Err(EngineError::Admission(
                "stop all distribution outputs before changing encoder adapters".into(),
            ));
        }
        g.encoder_factory = Some(factory);
        g.encoder_capabilities = capabilities;
        Ok(())
    }

    /// Persist an explicit output mapping in the stopped state. Starting it is
    /// a separate operation so an unavailable codec never mutates the project
    /// into a falsely-running state.
    pub fn configure_distribution_output(
        &self,
        mut output: eiviz_core::Output,
    ) -> Result<CommandAck> {
        if output.distribution.is_none()
            || !matches!(
                output.kind,
                OutputKind::Rtmp { .. } | OutputKind::Srt { .. } | OutputKind::Mp4 { .. }
            )
        {
            return Err(EngineError::NotDistributionOutput(output.id));
        }
        output.enabled = false;
        self.submit_payload(Command::AddOutput { output })
    }

    pub fn set_distribution_enabled(&self, output: OutputId, enabled: bool) -> Result<CommandAck> {
        let snapshot = self.snapshot();
        let configured = snapshot
            .outputs
            .get(&output)
            .ok_or(EngineError::UnknownOutput(output))?;
        configured
            .distribution
            .as_ref()
            .ok_or(EngineError::NotDistributionOutput(output))?;
        self.submit_payload(Command::SetOutputEnabled {
            id: output,
            enabled,
        })
    }

    pub fn flight_log(&self) -> Vec<FlightEvent> {
        self.inner.lock().flight.iter().cloned().collect()
    }

    pub fn submit(&self, env: CommandEnvelope) -> Result<CommandAck> {
        let mut g = self.inner.lock();
        if let Some(acknowledgement) = g.sequencer.existing_ack(&env)? {
            return Ok(acknowledgement);
        }
        let activation = match &env.payload {
            Command::SetOutputEnabled { id, enabled: true } => g
                .project
                .outputs
                .get(id)
                .filter(|output| output.distribution.is_some())
                .cloned(),
            Command::AddOutput { output } if output.enabled && output.distribution.is_some() => {
                Some(output.clone())
            }
            _ => None,
        };
        let deactivate_after = match &env.payload {
            Command::SetOutputEnabled { id, enabled: false }
                if g.distribution_bindings.contains_key(id) =>
            {
                Some(*id)
            }
            Command::RemoveOutput { id } if g.distribution_bindings.contains_key(id) => Some(*id),
            _ => None,
        };
        let activated_now = if let Some(output) = activation.as_ref() {
            activate_distribution_output(&mut g, output)?
        } else {
            false
        };
        let playback_update = match &env.payload {
            Command::SetInputPlayback { input, playback } => Some((*input, playback.clone())),
            _ => None,
        };
        if let Err(error) = Self::admit(&g, &env.payload) {
            if activated_now && let Some(output) = activation.as_ref() {
                deactivate_distribution_output(&mut g, output.id);
            }
            return Err(error);
        }
        let applied = {
            let Inner {
                sequencer, project, ..
            } = &mut *g;
            sequencer.apply(project, env)
        };
        let ack = match applied {
            Ok(ack) => ack,
            Err(error) => {
                if activated_now && let Some(output) = activation.as_ref() {
                    deactivate_distribution_output(&mut g, output.id);
                }
                return Err(error.into());
            }
        };
        if let Some(output) = deactivate_after {
            deactivate_distribution_output(&mut g, output);
        }
        let hash = state_hash(&g.project);
        if let Some((input, playback)) = playback_update {
            g.runtime.update_source_playback(input, &playback);
        }
        let ev = FlightEvent {
            frame: g.runtime.frame(),
            revision: ack.revision,
            hash: hash.clone(),
            dropped_preview: g.runtime.metrics.dropped_preview,
            failed_outputs: g
                .runtime
                .failed_outputs()
                .into_iter()
                .map(|(n, _)| n)
                .collect(),
            note: "command".into(),
        };
        push_flight(&mut g.flight, ev);
        if let Some(path) = g.journal_path.clone() {
            let _ = append_journal(&path, ack.revision, &hash);
        }
        Ok(ack)
    }

    pub fn submit_transaction(&self, envelopes: Vec<CommandEnvelope>) -> Result<Vec<CommandAck>> {
        let mut g = self.inner.lock();
        if envelopes.iter().any(|envelope| {
            matches!(
                &envelope.payload,
                Command::SetOutputEnabled { id, .. } | Command::RemoveOutput { id }
                    if g.project.outputs.get(id).is_some_and(|output| output.distribution.is_some())
            ) || matches!(
                &envelope.payload,
                Command::AddOutput { output }
                    if output.enabled && output.distribution.is_some()
            )
        }) {
            return Err(EngineError::Admission(
                "distribution start/stop/remove commands must be submitted individually so adapter lifecycle remains atomic".into(),
            ));
        }
        for envelope in &envelopes {
            if g.sequencer.existing_ack(envelope)?.is_none() {
                Self::admit(&g, &envelope.payload)?;
            }
        }
        let playback_updates = envelopes
            .iter()
            .filter_map(|envelope| match &envelope.payload {
                Command::SetInputPlayback { input, playback } => Some((*input, playback.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        let (acknowledgements, hash) = {
            let Inner {
                sequencer, project, ..
            } = &mut *g;
            let acknowledgements = sequencer.apply_transaction(project, envelopes)?;
            let hash = state_hash(project);
            (acknowledgements, hash)
        };
        for (input, playback) in playback_updates {
            g.runtime.update_source_playback(input, &playback);
        }
        let revision = acknowledgements
            .last()
            .map_or_else(|| g.sequencer.revision(), |ack| ack.revision);
        let event = FlightEvent {
            frame: g.runtime.frame(),
            revision,
            hash: hash.clone(),
            dropped_preview: g.runtime.metrics.dropped_preview,
            failed_outputs: g
                .runtime
                .failed_outputs()
                .into_iter()
                .map(|(name, _)| name)
                .collect(),
            note: format!("transaction:{}", acknowledgements.len()),
        };
        push_flight(&mut g.flight, event);
        if let Some(path) = g.journal_path.clone() {
            let _ = append_journal(&path, revision, &hash);
        }
        Ok(acknowledgements)
    }

    pub fn submit_payload(&self, payload: Command) -> Result<CommandAck> {
        let env = CommandEnvelope::new(self.client(), payload);
        self.submit(env)
    }

    pub fn tick(&self) -> Result<TickResult> {
        let mut g = self.inner.lock();
        let Inner {
            runtime, project, ..
        } = &mut *g;
        let result = runtime.tick(project)?;
        let audio = result.audio.clone();
        let distribution_profiles = g.encoder_sessions.keys().cloned().collect::<Vec<_>>();
        let mut distribution_errors = Vec::new();
        for profile in distribution_profiles {
            let Some(video) = result.programs.get(&profile.owner) else {
                continue;
            };
            let session = g
                .encoder_sessions
                .get_mut(&profile)
                .expect("profile key came from encoder session map");
            if session.fanout.keyframe_required()
                && let Err(error) = session.encoder.request_idr()
            {
                let reason = error.to_string();
                session.last_error = Some(reason.clone());
                distribution_errors
                    .push((format!("distribution encoder {}", profile.owner), reason));
                continue;
            }
            match session.encoder.encode(video, &audio) {
                Ok(access_units) => {
                    session.last_error = None;
                    for access_unit in access_units {
                        session.fanout.publish(access_unit);
                    }
                }
                Err(error) => {
                    let reason = error.to_string();
                    session.last_error = Some(reason.clone());
                    distribution_errors
                        .push((format!("distribution encoder {}", profile.owner), reason));
                }
            }
        }
        for (name, reason) in distribution_errors {
            g.runtime.mark_output_failed(&name, reason);
        }
        let sinks = g.sinks.clone();
        for (output_id, sink) in sinks {
            let Some((enabled, owner)) = g
                .project
                .outputs
                .get(&output_id)
                .map(|output| (output.enabled, output.owner))
            else {
                continue;
            };
            if !enabled {
                continue;
            }
            if let Some(frame) = result.programs.get(&owner) {
                if let Err(e) = sink.push_video(frame) {
                    g.runtime.mark_output_failed(sink.name(), e.to_string());
                }
                if let Err(e) = sink.push_audio(&audio) {
                    g.runtime.mark_output_failed(sink.name(), e.to_string());
                }
            }
        }
        let audio_sinks = g.audio_sinks.clone();
        for (output_id, sink) in audio_sinks {
            let enabled = g
                .project
                .outputs
                .get(&output_id)
                .is_some_and(|output| output.enabled);
            if enabled && let Err(error) = sink.push_audio(&audio) {
                g.runtime.mark_output_failed(sink.name(), error.to_string());
            }
        }
        let ev = FlightEvent {
            frame: g.runtime.frame().saturating_sub(1),
            revision: g.sequencer.revision(),
            hash: state_hash(&g.project),
            dropped_preview: g.runtime.metrics.dropped_preview,
            failed_outputs: g
                .runtime
                .failed_outputs()
                .into_iter()
                .map(|(n, _)| n)
                .collect(),
            note: "tick".into(),
        };
        push_flight(&mut g.flight, ev);
        if let Some(path) = g.autosave_path.clone()
            && g.runtime.frame().is_multiple_of(60)
        {
            let _ = save_autosave(&g.project, &path);
        }
        Ok(result)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let g = self.inner.lock();
        save_atomic(&g.project, path)?;
        Ok(())
    }

    pub fn export_portable(&self, path: &Path, asset_root: &Path) -> Result<()> {
        let project = self.snapshot();
        export_project_portable(&project, path, asset_root)?;
        Ok(())
    }

    pub fn import_portable(path: &Path, destination: &Path) -> Result<Self> {
        let project = import_project_portable(path, destination)?;
        let engine = Self::from_project(project)?;
        engine.set_asset_root(destination.to_path_buf());
        Ok(engine)
    }

    pub fn load(path: &Path) -> Result<Self> {
        Self::from_project(load(path)?)
    }

    pub fn load_project(&self, path: &Path, asset_root: Option<&Path>) -> Result<()> {
        self.replace_project(load(path)?, asset_root.map(std::path::Path::to_path_buf))
    }

    pub fn import_portable_into(&self, path: &Path, destination: &Path) -> Result<()> {
        let project = import_project_portable(path, destination)?;
        self.replace_project(project, Some(destination.to_path_buf()))
    }

    pub fn primary_unit(&self) -> MixingUnitId {
        *self
            .inner
            .lock()
            .project
            .mixing_units
            .keys()
            .next()
            .expect("default mix")
    }

    fn replace_project(&self, project: Project, asset_root: Option<PathBuf>) -> Result<()> {
        validate_enabled_distribution_outputs(&project)?;
        let mut runtime = Runtime::with_backend(project.audio.sample_rate, project.compositor)?;
        if let Some(root) = asset_root {
            runtime.set_asset_root(root);
        }
        let mut inner = self.inner.lock();
        inner.project = project;
        inner.runtime = runtime;
        inner.sequencer = Sequencer::default();
        inner.flight.clear();
        inner.sinks.clear();
        inner.audio_sinks.clear();
        inner.distribution_bindings.clear();
        inner.encoder_sessions.clear();
        Ok(())
    }

    pub fn last_program(&self, unit: MixingUnitId) -> Option<VideoFrame> {
        self.inner.lock().runtime.last_program_frame(unit)
    }

    pub fn last_preview(&self, unit: MixingUnitId) -> Option<VideoFrame> {
        self.inner.lock().runtime.last_preview_frame(unit)
    }

    pub fn last_multiview(&self, view: MultiviewId) -> Option<VideoFrame> {
        self.inner.lock().runtime.last_multiview_frame(view)
    }

    pub fn mark_output_failed(&self, name: &str, reason: impl Into<String>) {
        self.inner.lock().runtime.mark_output_failed(name, reason);
    }

    fn admit(inner: &Inner, payload: &Command) -> Result<()> {
        let p = &inner.project;
        let b = &inner.budget;
        match payload {
            Command::AddInput { .. } => {
                if p.inputs.len() >= b.max_inputs {
                    return Err(EngineError::Admission("input count".into()));
                }
                let bytes =
                    (p.inputs.len() + 1) * p.video.width as usize * p.video.height as usize * 4;
                if bytes > b.max_pixel_bytes {
                    return Err(EngineError::Admission("pixel budget".into()));
                }
            }
            Command::AddMixingUnit { .. } if p.mixing_units.len() >= b.max_units => {
                return Err(EngineError::Admission("mixing unit count".into()));
            }
            _ => {}
        }
        Ok(())
    }
}

fn encoding_profile_key(output: &Output) -> Result<EncodingProfileKey> {
    let profile = output
        .distribution
        .as_ref()
        .ok_or(EngineError::NotDistributionOutput(output.id))?;
    Ok(EncodingProfileKey {
        owner: output.owner,
        video: profile.video.clone(),
        audio: profile.audio.clone(),
    })
}

fn activate_distribution_output(inner: &mut Inner, output: &Output) -> Result<bool> {
    if inner.distribution_bindings.contains_key(&output.id) {
        return Ok(false);
    }
    let key = encoding_profile_key(output)?;
    let mut created_session = false;
    if !inner.encoder_sessions.contains_key(&key) {
        let factory = inner.encoder_factory.as_ref().ok_or_else(|| {
            EngineError::Admission(
                "no explicit distribution encoder factory/binary paths configured; I_PCM/PCM fallback is forbidden".into(),
            )
        })?;
        inner
            .encoder_capabilities
            .validate(output.distribution.as_ref().expect("profile checked"))?;
        let request = EncoderSessionRequest {
            width: inner.project.video.width,
            height: inner.project.video.height,
            frame_rate: inner.project.video.frame_rate,
            video: key.video.clone(),
            audio: key.audio.clone(),
        };
        let encoder = factory.create(&request)?;
        let fanout = EncodedFanout::new(encoder.stream_config().clone());
        inner.encoder_sessions.insert(
            key.clone(),
            EncodingSession {
                encoder,
                fanout,
                last_error: None,
            },
        );
        created_session = true;
    }
    let session = inner
        .encoder_sessions
        .get(&key)
        .expect("encoder session inserted above");
    let sink_name = match eiviz_io_stream::attach_profiled_sink(
        &session.fanout,
        output,
        &inner.encoder_capabilities,
    ) {
        Ok(name) => name,
        Err(error) => {
            if created_session {
                inner.encoder_sessions.remove(&key);
            }
            return Err(error.into());
        }
    };
    inner.distribution_bindings.insert(
        output.id,
        DistributionBinding {
            profile: key,
            sink_name,
        },
    );
    Ok(true)
}

fn deactivate_distribution_output(inner: &mut Inner, output: OutputId) {
    let Some(binding) = inner.distribution_bindings.remove(&output) else {
        return;
    };
    if let Some(session) = inner.encoder_sessions.get(&binding.profile) {
        session.fanout.remove_sink(&binding.sink_name);
    }
    let remove_session = inner
        .encoder_sessions
        .get(&binding.profile)
        .is_some_and(|session| session.fanout.sink_count() == 0);
    if remove_session {
        inner.encoder_sessions.remove(&binding.profile);
    }
}

fn validate_enabled_distribution_outputs(project: &Project) -> Result<()> {
    let capabilities = eiviz_io_stream::EncoderCapabilities::default();
    for output in project.outputs.values().filter(|output| output.enabled) {
        if let Some(profile) = &output.distribution {
            capabilities.validate(profile)?;
        }
    }
    Ok(())
}

fn distribution_diagnostics(inner: &Inner) -> Vec<DistributionOutputDiagnostics> {
    inner
        .project
        .outputs
        .values()
        .filter_map(|output| {
            let profile = output.distribution.as_ref()?;
            let capability = inner.encoder_capabilities.validate(profile);
            let binding = inner.distribution_bindings.get(&output.id);
            let session = binding.and_then(|binding| inner.encoder_sessions.get(&binding.profile));
            let sink = binding.and_then(|binding| {
                session.and_then(|session| {
                    session
                        .fanout
                        .diagnostics()
                        .into_iter()
                        .find(|diagnostic| diagnostic.name == binding.sink_name)
                })
            });
            let encoder = session
                .map(|session| session.encoder.diagnostics())
                .unwrap_or_default();
            let state = if !output.enabled {
                "stopped".into()
            } else if let Some(sink) = &sink {
                format!("{:?}", sink.state).to_lowercase()
            } else {
                "failed".into()
            };
            let detail = distribution_detail(capability.err(), session, sink.as_ref(), &encoder);
            Some(DistributionOutputDiagnostics {
                output_id: output.id.to_string(),
                name: output.name.clone(),
                enabled: output.enabled,
                state,
                detail,
                queue_depth: sink.as_ref().map_or(0, |value| value.queue_depth),
                queue_high_water: sink.as_ref().map_or(0, |value| value.queue_high_water),
                enqueued: sink.as_ref().map_or(0, |value| value.enqueued),
                sent: sink.as_ref().map_or(0, |value| value.sent),
                dropped: sink.as_ref().map_or(0, |value| value.dropped),
                reconnects: sink.as_ref().map_or(0, |value| value.reconnects),
                video_frames: encoder.video_frames,
                keyframes: encoder.keyframes,
                audio_access_units: encoder.audio_access_units,
                idr_requests: encoder.idr_requests,
            })
        })
        .collect()
}

fn distribution_detail(
    capability_error: Option<eiviz_media::MediaError>,
    session: Option<&EncodingSession>,
    sink: Option<&SinkDiagnostics>,
    encoder: &EncoderDiagnostics,
) -> String {
    if let Some(error) = capability_error {
        return error.to_string();
    }
    if let Some(error) = session.and_then(|session| session.last_error.as_deref()) {
        return format!("encoder failed: {error}");
    }
    if let Some(error) = sink.and_then(|sink| sink.last_error.as_deref()) {
        return format!(
            "{} + {}; sink: {error}",
            encoder.video_backend, encoder.audio_backend
        );
    }
    if session.is_some() {
        format!("{} + {}", encoder.video_backend, encoder.audio_backend)
    } else {
        "explicit encoder adapter configured; output stopped".into()
    }
}

impl From<AudioIoDiagnostics> for EngineAudioDiagnostics {
    fn from(value: AudioIoDiagnostics) -> Self {
        Self {
            name: value.name,
            health: format!("{:?}", value.health),
            callbacks: value.callbacks,
            device_frames: value.device_frames,
            xruns: value.xruns,
            queue_overflows: value.queue_overflows,
            queue_underflows: value.queue_underflows,
            last_device_sample_index: value.last_device_sample_index,
            last_callback_nanos: value.last_callback_nanos,
            last_device_nanos: value.last_device_nanos,
            last_error: value.last_error,
        }
    }
}

fn push_flight(buf: &mut VecDeque<FlightEvent>, ev: FlightEvent) {
    if buf.len() >= FLIGHT_CAP {
        buf.pop_front();
    }
    buf.push_back(ev);
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_core::{
        Input, InputId, InputSource, MixingUnit, Output, OutputId, OutputKind, Scene, SceneId,
        SceneItem, SceneItemId, Transform2D, TransitionStyle,
    };
    use eiviz_io_stream::FailingSink;
    use eiviz_media::{EncodedAccessUnit, EncodedKind, EncodedStreamConfig};
    use std::sync::atomic::{AtomicU64, Ordering};

    struct MockEncoderFactory {
        creates: Arc<AtomicU64>,
        encodes: Arc<AtomicU64>,
        idr_requests: Arc<AtomicU64>,
    }

    struct MockEncoder {
        config: EncodedStreamConfig,
        encodes: Arc<AtomicU64>,
        idr_requests: Arc<AtomicU64>,
    }

    impl ProgramEncoderFactory for MockEncoderFactory {
        fn create(
            &self,
            _request: &EncoderSessionRequest,
        ) -> eiviz_media::Result<Box<dyn ProgramEncoder>> {
            self.creates.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(MockEncoder {
                config: EncodedStreamConfig {
                    h264_sps: vec![0x67, 66, 0, 31].into(),
                    h264_pps: vec![0x68, 0].into(),
                    aac_audio_specific_config: vec![0x11, 0x90].into(),
                    video_width: 1920,
                    video_height: 1080,
                    video_timescale: 60_000,
                    video_sample_duration: 1001,
                    audio_sample_rate: 48_000,
                    audio_channels: 2,
                },
                encodes: self.encodes.clone(),
                idr_requests: self.idr_requests.clone(),
            }))
        }

        fn description(&self) -> String {
            "deterministic mock encoder".into()
        }
    }

    impl ProgramEncoder for MockEncoder {
        fn stream_config(&self) -> &EncodedStreamConfig {
            &self.config
        }

        fn encode(
            &mut self,
            video: &VideoFrame,
            _audio: &eiviz_media::AudioBuffer,
        ) -> eiviz_media::Result<Vec<Arc<EncodedAccessUnit>>> {
            self.encodes.fetch_add(1, Ordering::Relaxed);
            Ok(vec![
                Arc::new(EncodedAccessUnit {
                    pts: video.pts,
                    dts: Some(video.pts),
                    keyframe: true,
                    bytes: vec![0, 0, 0, 1, 0x65, 1].into(),
                    kind: EncodedKind::Avc,
                }),
                Arc::new(EncodedAccessUnit {
                    pts: video.pts,
                    dts: Some(video.pts),
                    keyframe: false,
                    bytes: vec![0x21, 0x10].into(),
                    kind: EncodedKind::Aac,
                }),
            ])
        }

        fn request_idr(&mut self) -> eiviz_media::Result<()> {
            self.idr_requests.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn diagnostics(&self) -> EncoderDiagnostics {
            EncoderDiagnostics {
                video_backend: "mock-avc".into(),
                audio_backend: "mock-aac".into(),
                video_frames: self.encodes.load(Ordering::Relaxed),
                keyframes: self.encodes.load(Ordering::Relaxed),
                audio_access_units: self.encodes.load(Ordering::Relaxed),
                idr_requests: self.idr_requests.load(Ordering::Relaxed),
                last_error: None,
            }
        }
    }

    struct PixelSink {
        name: String,
        pixels: Mutex<Vec<[u8; 4]>>,
    }

    impl PixelSink {
        fn new(name: &str) -> Self {
            Self {
                name: name.into(),
                pixels: Mutex::new(Vec::new()),
            }
        }
    }

    impl MediaSink for PixelSink {
        fn name(&self) -> &str {
            &self.name
        }

        fn push_video(&self, frame: &VideoFrame) -> eiviz_media::Result<()> {
            self.pixels.lock().push(frame.pixel(0, 0));
            Ok(())
        }

        fn push_audio(&self, _audio: &eiviz_media::AudioBuffer) -> eiviz_media::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn save_reload_preserves_ids_and_take_is_atomic() {
        let engine = Engine::new("walk");
        let unit = engine.primary_unit();
        let input = Input {
            id: InputId::new(),
            name: "green".into(),
            tags: vec!["cam".into()],
            groups: vec![],
            source: InputSource::SolidColor {
                r: 0,
                g: 255,
                b: 0,
                a: 255,
            },
        };
        let scene = Scene {
            id: SceneId::new(),
            name: "green".into(),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: input.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        engine
            .submit_payload(Command::AddInput {
                input: input.clone(),
            })
            .unwrap();
        engine
            .submit_payload(Command::AddScene {
                scene: scene.clone(),
            })
            .unwrap();
        engine
            .submit_payload(Command::SetPreview {
                unit,
                scene: Some(scene.id),
            })
            .unwrap();
        engine.tick().unwrap();
        engine
            .submit_payload(Command::Take {
                unit,
                swap: false,
                style: TransitionStyle::Cut,
                duration_frames: 0,
            })
            .unwrap();
        let result = engine.tick().unwrap();
        assert_eq!(result.programs[&unit].pixel(4, 4), [0, 255, 0, 255]);

        let dir = std::env::temp_dir().join(format!("eiviz-engine-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("project.json");
        engine.save(&path).unwrap();
        let loaded = Engine::load(&path).unwrap();
        assert_eq!(loaded.snapshot().id, engine.snapshot().id);
        assert_eq!(
            loaded.snapshot().mixing_units[&unit].program.scene,
            Some(scene.id)
        );
        assert_eq!(loaded.state_hash(), engine.state_hash());
        assert!(!engine.flight_log().is_empty());
    }

    #[test]
    fn failing_sink_does_not_stop_program() {
        let engine = Engine::new("iso");
        engine.attach_sink(Arc::new(FailingSink::new("aux-record")));
        for _ in 0..5 {
            engine.tick().unwrap();
        }
        assert!(engine.metrics().frame >= 5);
        assert!(!engine.metrics().failed_outputs.is_empty());
    }

    #[test]
    fn output_registry_routes_each_mixing_unit_program() {
        let engine = Engine::new("routes");
        let unit_a = engine.primary_unit();
        let unit_b = MixingUnit::new("Mix B");
        let output_a = *engine.snapshot().outputs.keys().next().unwrap();
        engine
            .submit_payload(Command::AddMixingUnit {
                unit: unit_b.clone(),
            })
            .unwrap();

        let make_input_scene = |name: &str, rgba: [u8; 4]| {
            let input = Input {
                id: InputId::new(),
                name: name.into(),
                tags: vec![],
                groups: vec![],
                source: InputSource::SolidColor {
                    r: rgba[0],
                    g: rgba[1],
                    b: rgba[2],
                    a: rgba[3],
                },
            };
            let scene = Scene {
                id: SceneId::new(),
                name: name.into(),
                items: vec![SceneItem {
                    id: SceneItemId::new(),
                    input: input.id,
                    transform: Transform2D::fullscreen(),
                    z_order: 0,
                    playback: Default::default(),
                }],
            };
            (input, scene)
        };
        let (red, red_scene) = make_input_scene("red", [255, 0, 0, 255]);
        let (blue, blue_scene) = make_input_scene("blue", [0, 0, 255, 255]);
        for input in [red, blue] {
            engine.submit_payload(Command::AddInput { input }).unwrap();
        }
        for scene in [red_scene.clone(), blue_scene.clone()] {
            engine.submit_payload(Command::AddScene { scene }).unwrap();
        }
        engine
            .submit_payload(Command::SetProgram {
                unit: unit_a,
                scene: Some(red_scene.id),
            })
            .unwrap();
        engine
            .submit_payload(Command::SetProgram {
                unit: unit_b.id,
                scene: Some(blue_scene.id),
            })
            .unwrap();
        let output_b = Output {
            id: OutputId::new(),
            name: "Mix B output".into(),
            owner: unit_b.id,
            kind: OutputKind::Omt { url: "test".into() },
            enabled: true,
            distribution: None,
        };
        engine
            .submit_payload(Command::AddOutput {
                output: output_b.clone(),
            })
            .unwrap();
        let sink_a = Arc::new(PixelSink::new("a"));
        let sink_b = Arc::new(PixelSink::new("b"));
        engine.attach_output_sink(output_a, sink_a.clone()).unwrap();
        engine
            .attach_output_sink(output_b.id, sink_b.clone())
            .unwrap();
        engine.tick().unwrap();
        assert_eq!(sink_a.pixels.lock()[0], [255, 0, 0, 255]);
        assert_eq!(sink_b.pixels.lock()[0], [0, 0, 255, 255]);
    }

    #[test]
    fn asset_ingest_and_portable_import_replace_project_in_place() {
        let root = std::env::temp_dir().join(format!("eiviz-asset-engine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("still.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([12, 34, 56, 255]))
            .save(&source)
            .unwrap();
        let asset_root = root.join("store");
        let package = root.join("portable.eiviz");
        let imported_root = root.join("imported");

        let engine = Engine::new("asset project");
        let asset = engine.ingest_asset(&source, &asset_root).unwrap();
        assert_eq!(
            engine.snapshot().assets[&asset.id].sha256_hex,
            asset.sha256_hex
        );
        let input = Input {
            id: InputId::new(),
            name: "still".into(),
            tags: vec![],
            groups: vec![],
            source: InputSource::Image { asset: asset.id },
        };
        let scene = Scene {
            id: SceneId::new(),
            name: "still".into(),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: input.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        let unit = engine.primary_unit();
        engine.submit_payload(Command::AddInput { input }).unwrap();
        engine
            .submit_payload(Command::AddScene {
                scene: scene.clone(),
            })
            .unwrap();
        engine
            .submit_payload(Command::SetProgram {
                unit,
                scene: Some(scene.id),
            })
            .unwrap();
        let tick = engine.tick().unwrap();
        assert_eq!(tick.programs[&unit].pixel(0, 0), [12, 34, 56, 255]);
        engine.export_portable(&package, &asset_root).unwrap();

        let same_engine_id = engine.client();
        engine
            .import_portable_into(&package, &imported_root)
            .unwrap();
        assert_eq!(engine.client(), same_engine_id);
        assert_eq!(
            engine.snapshot().assets[&asset.id].original_name,
            "still.png"
        );
        assert!(
            imported_root
                .join(&engine.snapshot().assets[&asset.id].relative_path)
                .exists()
        );
        let imported_tick = engine.tick().unwrap();
        assert_eq!(imported_tick.programs[&unit].pixel(0, 0), [12, 34, 56, 255]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(not(feature = "wgpu-backend"))]
    #[test]
    fn wgpu_project_does_not_construct_cpu_engine() {
        let mut project = Project::new("gpu-required");
        project.compositor = eiviz_core::CompositorBackend::Wgpu;
        match Engine::from_project(project) {
            Err(EngineError::Runtime(eiviz_runtime::RuntimeError::WgpuFeatureDisabled))
            | Err(EngineError::Runtime(eiviz_runtime::RuntimeError::Gpu(_))) => {}
            Err(e) => panic!("must not substitute CPU runtime: {e}"),
            Ok(_) => panic!("must not construct an engine when Wgpu is unavailable"),
        }
    }

    #[cfg(feature = "wgpu-backend")]
    #[test]
    fn wgpu_project_constructs_only_with_hardware_backend() {
        let mut project = Project::new("gpu-required");
        project.compositor = eiviz_core::CompositorBackend::Wgpu;
        match Engine::from_project(project) {
            Ok(engine) => {
                assert_eq!(
                    engine.snapshot().compositor,
                    eiviz_core::CompositorBackend::Wgpu
                );
            }
            Err(EngineError::Runtime(eiviz_runtime::RuntimeError::Gpu(_))) => {}
            Err(error) => panic!("unexpected Wgpu construction result: {error}"),
        }
    }

    #[test]
    fn engine_encodes_once_for_two_sinks_with_one_shared_profile() {
        let engine = Engine::new("shared distribution");
        let creates = Arc::new(AtomicU64::new(0));
        let encodes = Arc::new(AtomicU64::new(0));
        let idr_requests = Arc::new(AtomicU64::new(0));
        engine
            .install_distribution_encoder_factory(
                Arc::new(MockEncoderFactory {
                    creates: creates.clone(),
                    encodes: encodes.clone(),
                    idr_requests: idr_requests.clone(),
                }),
                EncoderCapabilities::dynamic_openh264_fdk(),
            )
            .unwrap();
        let root =
            std::env::temp_dir().join(format!("eiviz-engine-shared-encode-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&root);
        let profile = |path: PathBuf| Output {
            id: OutputId::new(),
            name: format!("record {}", path.display()),
            owner: engine.primary_unit(),
            kind: OutputKind::Mp4 {
                path: path.display().to_string(),
            },
            enabled: false,
            distribution: Some(eiviz_core::DistributionProfile {
                video: H264EncoderProfile::CiscoOpenH26426 {
                    bitrate_bps: 8_000_000,
                    keyframe_interval_frames: 120,
                    level_idc: 42,
                },
                audio: AacEncoderProfile::FdkAacLc {
                    bitrate_bps: 192_000,
                    sample_rate: 48_000,
                    channels: 2,
                },
                transport: eiviz_core::TransportProfile::FragmentedMp4 {
                    recover_incomplete_tail: true,
                },
                queue_capacity: 8,
                reconnect: eiviz_core::ReconnectProfile {
                    initial_delay_ms: 1,
                    max_delay_ms: 2,
                    max_attempts: 1,
                },
            }),
        };
        let output_a = profile(root.join("a.mp4"));
        let output_b = profile(root.join("b.mp4"));
        engine
            .configure_distribution_output(output_a.clone())
            .unwrap();
        engine
            .configure_distribution_output(output_b.clone())
            .unwrap();
        engine.set_distribution_enabled(output_a.id, true).unwrap();
        engine.set_distribution_enabled(output_b.id, true).unwrap();

        engine.tick().unwrap();

        assert_eq!(creates.load(Ordering::Relaxed), 1);
        assert_eq!(encodes.load(Ordering::Relaxed), 1);
        assert_eq!(idr_requests.load(Ordering::Relaxed), 1);
        let diagnostics = engine.metrics().distribution_outputs;
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.enabled && diagnostic.video_frames == 1)
                .count(),
            2
        );

        engine.set_distribution_enabled(output_a.id, false).unwrap();
        engine.set_distribution_enabled(output_b.id, false).unwrap();
        assert!(
            engine
                .metrics()
                .distribution_outputs
                .iter()
                .all(|diagnostic| diagnostic.state == "stopped")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn distribution_mapping_is_stopped_and_encoder_activation_hard_fails() {
        let engine = Engine::new("distribution");
        let output = Output {
            id: OutputId::new(),
            name: "RTMP".into(),
            owner: engine.primary_unit(),
            kind: OutputKind::Rtmp {
                url: "rtmp://127.0.0.1/live/key".into(),
            },
            enabled: true,
            distribution: Some(eiviz_core::DistributionProfile {
                video: eiviz_core::H264EncoderProfile::CiscoOpenH26426 {
                    bitrate_bps: 8_000_000,
                    keyframe_interval_frames: 120,
                    level_idc: 42,
                },
                audio: eiviz_core::AacEncoderProfile::FdkAacLc {
                    bitrate_bps: 192_000,
                    sample_rate: 48_000,
                    channels: 2,
                },
                transport: eiviz_core::TransportProfile::RtmpPublish {
                    chunk_size: 4096,
                    connect_timeout_ms: 5_000,
                },
                queue_capacity: 128,
                reconnect: eiviz_core::ReconnectProfile {
                    initial_delay_ms: 100,
                    max_delay_ms: 5_000,
                    max_attempts: 0,
                },
            }),
        };
        engine
            .configure_distribution_output(output.clone())
            .unwrap();
        assert!(!engine.snapshot().outputs[&output.id].enabled);
        let error = engine
            .set_distribution_enabled(output.id, true)
            .unwrap_err();
        assert!(error.to_string().contains("I_PCM test encoder"));
        assert!(
            engine
                .metrics()
                .distribution_outputs
                .iter()
                .any(|diagnostic| diagnostic.state == "stopped")
        );
    }
}
