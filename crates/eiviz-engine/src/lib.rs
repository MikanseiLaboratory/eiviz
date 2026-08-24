use eiviz_command::{Command, CommandAck, CommandEnvelope, Sequencer, state_hash};
use eiviz_core::{ClientId, MixingUnitId, Project};
use eiviz_media::{MediaSink, MediaSource, VideoFrame};
use eiviz_project::{append_journal, load, save_atomic, save_autosave};
use eiviz_runtime::{Runtime, TickResult};
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::VecDeque;
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
    #[error("admission denied: {0}")]
    Admission(String),
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
    sinks: Vec<Arc<dyn MediaSink>>,
    autosave_path: Option<PathBuf>,
    journal_path: Option<PathBuf>,
}

impl Engine {
    pub fn new(name: impl Into<String>) -> Self {
        Self::from_project(Project::new(name)).expect("default project uses CpuReference")
    }

    pub fn from_project(project: Project) -> Result<Self> {
        let runtime = Runtime::with_backend(project.audio.sample_rate, project.compositor)?;
        Ok(Self {
            inner: Mutex::new(Inner {
                project,
                sequencer: Sequencer::default(),
                runtime,
                client: ClientId::new(),
                budget: AdmissionBudget::default(),
                flight: VecDeque::with_capacity(FLIGHT_CAP),
                sinks: Vec::new(),
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

    pub fn set_autosave_path(&self, path: impl Into<PathBuf>) {
        let mut g = self.inner.lock();
        let p = path.into();
        g.journal_path = Some(p.with_extension("journal.jsonl"));
        g.autosave_path = Some(p);
    }

    pub fn attach_sink(&self, sink: Arc<dyn MediaSink>) {
        self.inner.lock().sinks.push(sink);
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

    pub fn metrics(&self) -> EngineMetrics {
        let g = self.inner.lock();
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
        }
    }

    pub fn flight_log(&self) -> Vec<FlightEvent> {
        self.inner.lock().flight.iter().cloned().collect()
    }

    pub fn submit(&self, env: CommandEnvelope) -> Result<CommandAck> {
        let mut g = self.inner.lock();
        Self::admit(&g, &env.payload)?;
        let Inner {
            sequencer, project, ..
        } = &mut *g;
        let ack = sequencer.apply(project, env)?;
        let hash = state_hash(project);
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
        let unit = *project.mixing_units.keys().next().expect("default mix");
        let program = result.programs.get(&unit).cloned();
        let audio = result.audio.clone();
        let sinks = g.sinks.clone();
        if let Some(frame) = program {
            for sink in &sinks {
                if let Err(e) = sink.push_video(&frame) {
                    g.runtime.mark_output_failed(sink.name(), e.to_string());
                }
                if let Err(e) = sink.push_audio(&audio) {
                    g.runtime.mark_output_failed(sink.name(), e.to_string());
                }
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
        if let Some(path) = g.autosave_path.clone() {
            if g.runtime.frame() % 60 == 0 {
                let _ = save_autosave(&g.project, &path);
            }
        }
        Ok(result)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let g = self.inner.lock();
        save_atomic(&g.project, path)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        Self::from_project(load(path)?)
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

    pub fn last_program(&self, unit: MixingUnitId) -> Option<VideoFrame> {
        self.inner.lock().runtime.last_program_frame(unit)
    }

    pub fn last_preview(&self, unit: MixingUnitId) -> Option<VideoFrame> {
        self.inner.lock().runtime.last_preview_frame(unit)
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
        Input, InputId, InputSource, Scene, SceneId, SceneItem, SceneItemId, Transform2D,
        TransitionStyle,
    };
    use eiviz_io_stream::FailingSink;

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
}
