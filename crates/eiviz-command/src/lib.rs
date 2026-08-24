use eiviz_core::{
    AudioBusId, ClientId, CommandId, InputId, MixingUnitId, OutputId, OverlayId, SceneId,
    SceneItemId,
};
use eiviz_core::{
    AudioRoute, DeviceBinding, DomainError, Input, MixingUnit, Multiview, Output, OverlaySlot,
    Playback, Project, RouteMode, Scene, SceneItem, Transform2D, TransitionStyle,
};
use eiviz_time::MediaTime;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum CommandError {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("revision mismatch: expected {expected}, actual {actual}")]
    RevisionMismatch { expected: u64, actual: u64 },
    #[error("duplicate command {0}")]
    Duplicate(CommandId),
    #[error("busy: command queue is full")]
    Busy,
    #[error("{0}")]
    Rejected(String),
}

pub type Result<T> = std::result::Result<T, CommandError>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandEnvelope {
    pub id: CommandId,
    pub client: ClientId,
    pub client_seq: u64,
    pub expected_revision: Option<u64>,
    pub effective_time: Option<MediaTime>,
    pub coalesce_key: Option<String>,
    pub payload: Command,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    Noop,
    SetName {
        name: String,
    },
    AddInput {
        input: Input,
    },
    RemoveInput {
        id: InputId,
    },
    AddScene {
        scene: Scene,
    },
    RemoveScene {
        id: SceneId,
    },
    AddSceneItem {
        scene: SceneId,
        item: SceneItem,
    },
    RemoveSceneItem {
        scene: SceneId,
        item: SceneItemId,
    },
    UpdateTransform {
        scene: SceneId,
        item: SceneItemId,
        transform: Transform2D,
    },
    SetPlayback {
        scene: SceneId,
        item: SceneItemId,
        playback: Playback,
    },
    AddMixingUnit {
        unit: MixingUnit,
    },
    RemoveMixingUnit {
        id: MixingUnitId,
    },
    SetPreview {
        unit: MixingUnitId,
        scene: Option<SceneId>,
    },
    SetProgram {
        unit: MixingUnitId,
        scene: Option<SceneId>,
    },
    Take {
        unit: MixingUnitId,
        swap: bool,
        style: TransitionStyle,
        duration_frames: u32,
    },
    AddOverlay {
        unit: MixingUnitId,
        overlay: OverlaySlot,
    },
    SetOverlayEnabled {
        unit: MixingUnitId,
        overlay: OverlayId,
        enabled: bool,
    },
    SetOverlayScene {
        unit: MixingUnitId,
        overlay: OverlayId,
        scene: Option<SceneId>,
    },
    AddOutput {
        output: Output,
    },
    RemoveOutput {
        id: OutputId,
    },
    SetOutputEnabled {
        id: OutputId,
        enabled: bool,
    },
    AddMultiview {
        view: Multiview,
    },
    SetAudioRoute {
        route: AudioRoute,
    },
    ClearAudioRoute {
        input: eiviz_core::InputId,
        bus: AudioBusId,
    },
    SetMute {
        input: InputId,
        bus: AudioBusId,
        muted: bool,
    },
    AddDeviceBinding {
        binding: DeviceBinding,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandAck {
    pub id: CommandId,
    pub revision: u64,
    pub duplicate: bool,
}

#[derive(Clone, Debug)]
pub struct Sequencer {
    revision: u64,
    applied: std::collections::HashMap<CommandId, u64>,
    last_coalesce: std::collections::HashMap<String, CommandId>,
    capacity: usize,
}

impl Default for Sequencer {
    fn default() -> Self {
        Self::new(4096)
    }
}

impl Sequencer {
    pub fn new(capacity: usize) -> Self {
        Self {
            revision: 0,
            applied: std::collections::HashMap::new(),
            last_coalesce: std::collections::HashMap::new(),
            capacity,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn apply(&mut self, project: &mut Project, env: CommandEnvelope) -> Result<CommandAck> {
        if self.applied.len() >= self.capacity && !self.applied.contains_key(&env.id) {
            return Err(CommandError::Busy);
        }
        if let Some(rev) = self.applied.get(&env.id) {
            return Ok(CommandAck {
                id: env.id,
                revision: *rev,
                duplicate: true,
            });
        }
        if let Some(expected) = env.expected_revision {
            if expected != self.revision {
                return Err(CommandError::RevisionMismatch {
                    expected,
                    actual: self.revision,
                });
            }
        }
        if let Some(key) = &env.coalesce_key {
            if matches!(
                env.payload,
                Command::Take { .. }
                    | Command::SetOutputEnabled { .. }
                    | Command::AddInput { .. }
                    | Command::RemoveInput { .. }
            ) {
                return Err(CommandError::Rejected(
                    "this command must not be coalesced".into(),
                ));
            }
            self.last_coalesce.insert(key.clone(), env.id);
            let _ = self.last_coalesce.get(key);
        }
        apply_payload(project, &env.payload)?;
        project.validate()?;
        self.revision += 1;
        self.applied.insert(env.id, self.revision);
        Ok(CommandAck {
            id: env.id,
            revision: self.revision,
            duplicate: false,
        })
    }
}

fn apply_payload(project: &mut Project, payload: &Command) -> Result<()> {
    match payload {
        Command::Noop | Command::SetName { .. } => {
            if let Command::SetName { name } = payload {
                project.name = name.clone();
            }
        }
        Command::AddInput { input } => project.insert_input(input.clone())?,
        Command::RemoveInput { id } => {
            project.inputs.remove(id);
        }
        Command::AddScene { scene } => project.insert_scene(scene.clone())?,
        Command::RemoveScene { id } => {
            project.scenes.remove(id);
        }
        Command::AddSceneItem { scene, item } => {
            let s = project
                .scenes
                .get_mut(scene)
                .ok_or_else(|| DomainError::UnknownId(scene.to_string()))?;
            s.items.push(item.clone());
        }
        Command::RemoveSceneItem { scene, item } => {
            if let Some(s) = project.scenes.get_mut(scene) {
                s.items.retain(|i| i.id != *item);
            }
        }
        Command::UpdateTransform {
            scene,
            item,
            transform,
        } => {
            let s = project
                .scenes
                .get_mut(scene)
                .ok_or_else(|| DomainError::UnknownId(scene.to_string()))?;
            let it = s
                .items
                .iter_mut()
                .find(|i| i.id == *item)
                .ok_or_else(|| DomainError::UnknownId(item.to_string()))?;
            it.transform = *transform;
        }
        Command::SetPlayback {
            scene,
            item,
            playback,
        } => {
            let s = project
                .scenes
                .get_mut(scene)
                .ok_or_else(|| DomainError::UnknownId(scene.to_string()))?;
            let it = s
                .items
                .iter_mut()
                .find(|i| i.id == *item)
                .ok_or_else(|| DomainError::UnknownId(item.to_string()))?;
            it.playback = playback.clone();
        }
        Command::AddMixingUnit { unit } => {
            if project.mixing_units.contains_key(&unit.id) {
                return Err(DomainError::DuplicateId(unit.id.to_string()).into());
            }
            project.mixing_units.insert(unit.id, unit.clone());
        }
        Command::RemoveMixingUnit { id } => {
            project.mixing_units.remove(id);
        }
        Command::SetPreview { unit, scene } => {
            project.mixing_unit_mut(*unit)?.preview.scene = *scene;
        }
        Command::SetProgram { unit, scene } => {
            project.mixing_unit_mut(*unit)?.program.scene = *scene;
        }
        Command::Take {
            unit,
            swap,
            style,
            duration_frames,
        } => {
            let u = project.mixing_unit_mut(*unit)?;
            u.transition.style = *style;
            u.transition.duration_frames = *duration_frames;
            u.take(*swap);
        }
        Command::AddOverlay { unit, overlay } => {
            project
                .mixing_unit_mut(*unit)?
                .overlays
                .push(overlay.clone());
        }
        Command::SetOverlayEnabled {
            unit,
            overlay,
            enabled,
        } => {
            let u = project.mixing_unit_mut(*unit)?;
            if let Some(slot) = u.overlays.iter_mut().find(|o| o.id == *overlay) {
                slot.enabled = *enabled;
            }
        }
        Command::SetOverlayScene {
            unit,
            overlay,
            scene,
        } => {
            let u = project.mixing_unit_mut(*unit)?;
            if let Some(slot) = u.overlays.iter_mut().find(|o| o.id == *overlay) {
                slot.scene = *scene;
            }
        }
        Command::AddOutput { output } => {
            let owner = output.owner;
            project.outputs.insert(output.id, output.clone());
            if let Ok(u) = project.mixing_unit_mut(owner) {
                if !u.outputs.contains(&output.id) {
                    u.outputs.push(output.id);
                }
            }
        }
        Command::RemoveOutput { id } => {
            if let Some(o) = project.outputs.remove(id) {
                if let Ok(u) = project.mixing_unit_mut(o.owner) {
                    u.outputs.retain(|x| x != id);
                }
            }
        }
        Command::SetOutputEnabled { id, enabled } => {
            if let Some(o) = project.outputs.get_mut(id) {
                o.enabled = *enabled;
            }
        }
        Command::AddMultiview { view } => {
            let owner = view.owner;
            project.multiviews.insert(view.id, view.clone());
            if let Ok(u) = project.mixing_unit_mut(owner) {
                if !u.multiviews.contains(&view.id) {
                    u.multiviews.push(view.id);
                }
            }
        }
        Command::SetAudioRoute { route } => {
            project
                .audio_matrix
                .routes
                .retain(|r| !(r.input == route.input && r.bus == route.bus));
            project.audio_matrix.routes.push(route.clone());
        }
        Command::ClearAudioRoute { input, bus } => {
            project
                .audio_matrix
                .routes
                .retain(|r| !(r.input == *input && r.bus == *bus));
        }
        Command::SetMute { input, bus, muted } => {
            if let Some(r) = project
                .audio_matrix
                .routes
                .iter_mut()
                .find(|r| r.input == *input && r.bus == *bus)
            {
                r.muted = *muted;
            }
        }
        Command::AddDeviceBinding { binding } => {
            project.device_bindings.insert(binding.id, binding.clone());
        }
    }
    let _ = RouteMode::Manual;
    Ok(())
}

pub fn state_hash(project: &Project) -> String {
    let json = serde_json::to_vec(project).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(&json);
    hex::encode(h.finalize())
}

impl CommandEnvelope {
    pub fn new(client: ClientId, payload: Command) -> Self {
        Self {
            id: CommandId::new(),
            client,
            client_seq: 0,
            expected_revision: None,
            effective_time: None,
            coalesce_key: None,
            payload,
        }
    }

    pub fn with_coalesce_key(mut self, key: impl Into<String>) -> Self {
        self.coalesce_key = Some(key.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_core::{Input, InputSource, Scene, SceneItem, Transform2D};

    #[test]
    fn take_and_replay_are_idempotent() {
        let mut p = Project::new("demo");
        let unit = *p.mixing_units.keys().next().unwrap();
        let input = Input {
            id: InputId::new(),
            name: "bars".into(),
            tags: vec!["cam".into()],
            groups: vec![],
            source: InputSource::ColorBars,
        };
        let scene = Scene {
            id: SceneId::new(),
            name: "cam1".into(),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: input.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        let client = ClientId::new();
        let mut seq = Sequencer::default();
        seq.apply(
            &mut p,
            CommandEnvelope::new(client, Command::AddInput { input }),
        )
        .unwrap();
        seq.apply(
            &mut p,
            CommandEnvelope::new(
                client,
                Command::AddScene {
                    scene: scene.clone(),
                },
            ),
        )
        .unwrap();
        seq.apply(
            &mut p,
            CommandEnvelope::new(
                client,
                Command::SetPreview {
                    unit,
                    scene: Some(scene.id),
                },
            ),
        )
        .unwrap();
        let take = CommandEnvelope::new(
            client,
            Command::Take {
                unit,
                swap: false,
                style: TransitionStyle::Cut,
                duration_frames: 0,
            },
        );
        let ack = seq.apply(&mut p, take.clone()).unwrap();
        let ack2 = seq.apply(&mut p, take).unwrap();
        assert_eq!(ack.revision, ack2.revision);
        assert!(ack2.duplicate);
        assert_eq!(p.mixing_units[&unit].program.scene, Some(scene.id));
        assert!(!state_hash(&p).is_empty());
    }
}
