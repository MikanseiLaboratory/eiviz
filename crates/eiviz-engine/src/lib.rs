use eiviz_command::{Command, CommandAck, CommandEnvelope, Sequencer, state_hash};
use eiviz_core::{ClientId, MixingUnitId, Project};
use eiviz_media::VideoFrame;
use eiviz_project::{load, save_atomic};
use eiviz_runtime::{Runtime, TickResult};
use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Command(#[from] eiviz_command::CommandError),
    #[error(transparent)]
    Runtime(#[from] eiviz_runtime::RuntimeError),
    #[error(transparent)]
    Persist(#[from] eiviz_project::ProjectError),
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// Process-wide composition root. GUI and control adapters only talk to this.
pub struct Engine {
    inner: Mutex<Inner>,
}

struct Inner {
    project: Project,
    sequencer: Sequencer,
    runtime: Runtime,
    client: ClientId,
}

impl Engine {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            inner: Mutex::new(Inner {
                project: Project::new(name),
                sequencer: Sequencer::default(),
                runtime: Runtime::new(48_000),
                client: ClientId::new(),
            }),
        }
    }

    pub fn shared(self) -> Arc<Self> {
        Arc::new(self)
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

    pub fn submit(&self, env: CommandEnvelope) -> Result<CommandAck> {
        let mut g = self.inner.lock();
        let Inner {
            sequencer, project, ..
        } = &mut *g;
        Ok(sequencer.apply(project, env)?)
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
        Ok(runtime.tick(project)?)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let g = self.inner.lock();
        save_atomic(&g.project, path)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let project = load(path)?;
        Ok(Self {
            inner: Mutex::new(Inner {
                project,
                sequencer: Sequencer::default(),
                runtime: Runtime::new(48_000),
                client: ClientId::new(),
            }),
        })
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

    pub fn mark_output_failed(&self, name: &str, reason: impl Into<String>) {
        self.inner.lock().runtime.mark_output_failed(name, reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_core::{
        Input, InputId, InputSource, Scene, SceneId, SceneItem, SceneItemId, Transform2D,
        TransitionStyle,
    };

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
    }
}
