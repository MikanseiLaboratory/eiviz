use eiviz_core::{
    AssetRef, AudioRoute, DeviceBinding, DomainError, Input, MixingUnit, Multiview, Output,
    OverlaySlot, Playback, Project, RouteMode, Scene, SceneItem, Transform2D, TransitionStyle,
};
use eiviz_core::{
    AudioBusId, ClientId, CommandId, InputId, MixingUnitId, OutputId, OverlayId, SceneId,
    SceneItemId,
};
use eiviz_time::MediaTime;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};

pub const COMMAND_ENVELOPE_VERSION: u32 = 1;

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
    #[error("unsupported command envelope version {actual}; expected {expected}")]
    UnsupportedVersion { expected: u32, actual: u32 },
    #[error("client {client} sequence {actual} is not greater than last sequence {last}")]
    ClientSequence {
        client: ClientId,
        last: u64,
        actual: u64,
    },
    #[error("transaction must contain at least one command")]
    EmptyTransaction,
    #[error("all commands in a transaction must have the same effective media time")]
    TransactionEffectiveTime,
    #[error("{0}")]
    Rejected(String),
}

pub type Result<T> = std::result::Result<T, CommandError>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandEnvelope {
    pub version: u32,
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
    SetInputPlayback {
        input: InputId,
        playback: Playback,
    },
    AddAsset {
        asset: AssetRef,
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
    /// Monotonic admission order. Acceptance is immediate, but media state is
    /// not changed until a frame boundary latches this command.
    pub revision: u64,
    /// Monotonic effective-order revision, populated after boundary latching.
    pub applied_revision: Option<u64>,
    pub duplicate: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PendingCommandDiagnostic {
    pub command_ids: Vec<CommandId>,
    pub accepted_revisions: Vec<u64>,
    pub effective_time: MediaTime,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SequencerDiagnostics {
    pub accepted_revision: u64,
    pub applied_revision: u64,
    pub pending_commands: usize,
    pub pending_batches: usize,
    pub pending_capacity: usize,
    pub retained_idempotency_records: usize,
    pub idempotency_capacity: usize,
    pub pending: Vec<PendingCommandDiagnostic>,
}

#[derive(Clone, Debug)]
pub struct LatchedState {
    pub project: Project,
    pub command_ids: Vec<CommandId>,
    pub accepted_revisions: Vec<u64>,
    pub applied_revision: u64,
}

#[derive(Clone, Debug)]
struct CommandRecord {
    accepted_revision: u64,
    applied_revision: Option<u64>,
}

#[derive(Clone, Debug)]
struct ScheduledBatch {
    effective_time: MediaTime,
    order: u64,
    envelopes: Vec<CommandEnvelope>,
    project: Project,
}

#[derive(Clone, Debug)]
pub struct Sequencer {
    accepted_revision: u64,
    applied_revision: u64,
    records: HashMap<CommandId, CommandRecord>,
    applied_order: VecDeque<CommandId>,
    client_sequences: HashMap<ClientId, u64>,
    pending: Vec<ScheduledBatch>,
    pending_commands: usize,
    pending_capacity: usize,
    history_capacity: usize,
    next_order: u64,
}

impl Default for Sequencer {
    fn default() -> Self {
        Self::new(4096)
    }
}

impl Sequencer {
    pub fn new(capacity: usize) -> Self {
        Self::with_capacities(capacity, capacity.saturating_mul(4).max(1))
    }

    pub fn with_capacities(pending_capacity: usize, history_capacity: usize) -> Self {
        Self {
            accepted_revision: 0,
            applied_revision: 0,
            records: HashMap::new(),
            applied_order: VecDeque::new(),
            client_sequences: HashMap::new(),
            pending: Vec::new(),
            pending_commands: 0,
            pending_capacity,
            history_capacity: history_capacity.max(1),
            next_order: 0,
        }
    }

    /// Latest accepted revision. It can be ahead of [`Self::applied_revision`].
    pub fn revision(&self) -> u64 {
        self.accepted_revision
    }

    pub fn applied_revision(&self) -> u64 {
        self.applied_revision
    }

    pub fn diagnostics(&self) -> SequencerDiagnostics {
        SequencerDiagnostics {
            accepted_revision: self.accepted_revision,
            applied_revision: self.applied_revision,
            pending_commands: self.pending_commands,
            pending_batches: self.pending.len(),
            pending_capacity: self.pending_capacity,
            retained_idempotency_records: self.records.len(),
            idempotency_capacity: self.history_capacity,
            pending: self
                .pending
                .iter()
                .map(|batch| PendingCommandDiagnostic {
                    command_ids: batch.envelopes.iter().map(|envelope| envelope.id).collect(),
                    accepted_revisions: batch
                        .envelopes
                        .iter()
                        .filter_map(|envelope| {
                            self.records
                                .get(&envelope.id)
                                .map(|record| record.accepted_revision)
                        })
                        .collect(),
                    effective_time: batch.effective_time,
                })
                .collect(),
        }
    }

    pub fn staged_project(&self) -> Option<&Project> {
        self.pending.last().map(|batch| &batch.project)
    }

    pub fn existing_ack(&self, envelope: &CommandEnvelope) -> Result<Option<CommandAck>> {
        if envelope.version != COMMAND_ENVELOPE_VERSION {
            return Err(CommandError::UnsupportedVersion {
                expected: COMMAND_ENVELOPE_VERSION,
                actual: envelope.version,
            });
        }
        Ok(self.records.get(&envelope.id).map(|record| CommandAck {
            id: envelope.id,
            revision: record.accepted_revision,
            applied_revision: record.applied_revision,
            duplicate: true,
        }))
    }

    /// Compatibility helper for non-realtime reducers. Engine code uses
    /// [`Self::stage`] and [`Self::latch_due`] explicitly.
    pub fn apply(&mut self, project: &mut Project, env: CommandEnvelope) -> Result<CommandAck> {
        let id = env.id;
        let acknowledgement = self.stage(project, env, MediaTime::ZERO)?;
        if acknowledgement.duplicate {
            return Ok(acknowledgement);
        }
        if let Some(latched) = self.latch_due(MediaTime::ZERO) {
            *project = latched.project;
        }
        self.records
            .get(&id)
            .map(|record| CommandAck {
                id,
                revision: record.accepted_revision,
                applied_revision: record.applied_revision,
                duplicate: false,
            })
            .ok_or_else(|| CommandError::Rejected("command acknowledgement was lost".into()))
    }

    /// Stages a validated command without changing the active project.
    pub fn stage(
        &mut self,
        active_project: &Project,
        envelope: CommandEnvelope,
        now: MediaTime,
    ) -> Result<CommandAck> {
        if let Some(acknowledgement) = self.existing_ack(&envelope)? {
            return Ok(acknowledgement);
        }
        let mut acknowledgements = self.stage_transaction(active_project, vec![envelope], now)?;
        Ok(acknowledgements.remove(0))
    }

    /// Stages all envelopes as one atomic boundary activation or stages none.
    pub fn stage_transaction(
        &mut self,
        active_project: &Project,
        envelopes: Vec<CommandEnvelope>,
        now: MediaTime,
    ) -> Result<Vec<CommandAck>> {
        if envelopes.is_empty() {
            return Err(CommandError::EmptyTransaction);
        }
        let mut seen = HashSet::with_capacity(envelopes.len());
        if envelopes.iter().any(|envelope| !seen.insert(envelope.id)) {
            return Err(CommandError::Rejected(
                "transaction contains a duplicate command id".into(),
            ));
        }
        let existing = envelopes
            .iter()
            .map(|envelope| self.existing_ack(envelope))
            .collect::<Result<Vec<_>>>()?;
        if existing.iter().all(Option::is_some) {
            return Ok(existing.into_iter().flatten().collect());
        }
        if existing.iter().any(Option::is_some) {
            return Err(CommandError::Rejected(
                "transaction must be either entirely new or an exact replay".into(),
            ));
        }
        if self.pending_commands.saturating_add(envelopes.len()) > self.pending_capacity {
            return Err(CommandError::Busy);
        }
        let effective_time = normalized_effective_time(&envelopes[0], now);
        if envelopes
            .iter()
            .skip(1)
            .any(|envelope| normalized_effective_time(envelope, now) != effective_time)
        {
            return Err(CommandError::TransactionEffectiveTime);
        }

        let mut candidate = self.clone();
        let mut acknowledgements = Vec::with_capacity(envelopes.len());
        for envelope in &envelopes {
            candidate.validate_envelope(envelope)?;
            candidate.accepted_revision = candidate.accepted_revision.saturating_add(1);
            candidate.records.insert(
                envelope.id,
                CommandRecord {
                    accepted_revision: candidate.accepted_revision,
                    applied_revision: None,
                },
            );
            if envelope.client_seq != 0 {
                candidate
                    .client_sequences
                    .insert(envelope.client, envelope.client_seq);
            }
            acknowledgements.push(CommandAck {
                id: envelope.id,
                revision: candidate.accepted_revision,
                applied_revision: None,
                duplicate: false,
            });
        }
        candidate.next_order = candidate.next_order.saturating_add(1);
        candidate.pending.push(ScheduledBatch {
            effective_time,
            order: candidate.next_order,
            envelopes,
            project: active_project.clone(),
        });
        candidate.pending_commands = candidate
            .pending_commands
            .saturating_add(acknowledgements.len());
        candidate.rebuild_pending(active_project)?;
        *self = candidate;
        Ok(acknowledgements)
    }

    /// Atomically removes and returns every batch due at this media boundary.
    /// Commands with the same effective time retain acceptance order.
    pub fn latch_due(&mut self, now: MediaTime) -> Option<LatchedState> {
        let due_count = self
            .pending
            .iter()
            .take_while(|batch| batch.effective_time <= now)
            .count();
        if due_count == 0 {
            return None;
        }
        let due = self.pending.drain(..due_count).collect::<Vec<_>>();
        let project = due.last().expect("due count is non-zero").project.clone();
        let mut command_ids = Vec::new();
        let mut accepted_revisions = Vec::new();
        for batch in due {
            for envelope in batch.envelopes {
                self.applied_revision = self.applied_revision.saturating_add(1);
                if let Some(record) = self.records.get_mut(&envelope.id) {
                    record.applied_revision = Some(self.applied_revision);
                    accepted_revisions.push(record.accepted_revision);
                }
                command_ids.push(envelope.id);
                self.applied_order.push_back(envelope.id);
            }
        }
        self.pending_commands = self.pending_commands.saturating_sub(command_ids.len());
        self.trim_history();
        Some(LatchedState {
            project,
            command_ids,
            accepted_revisions,
            applied_revision: self.applied_revision,
        })
    }

    fn validate_envelope(&self, envelope: &CommandEnvelope) -> Result<()> {
        if envelope.version != COMMAND_ENVELOPE_VERSION {
            return Err(CommandError::UnsupportedVersion {
                expected: COMMAND_ENVELOPE_VERSION,
                actual: envelope.version,
            });
        }
        if envelope.client_seq != 0
            && let Some(last) = self.client_sequences.get(&envelope.client)
            && envelope.client_seq <= *last
        {
            return Err(CommandError::ClientSequence {
                client: envelope.client,
                last: *last,
                actual: envelope.client_seq,
            });
        }
        if let Some(expected) = envelope.expected_revision
            && expected != self.accepted_revision
        {
            return Err(CommandError::RevisionMismatch {
                expected,
                actual: self.accepted_revision,
            });
        }
        if envelope.coalesce_key.is_some()
            && matches!(
                envelope.payload,
                Command::Take { .. }
                    | Command::SetOutputEnabled { .. }
                    | Command::AddInput { .. }
                    | Command::AddAsset { .. }
                    | Command::RemoveInput { .. }
            )
        {
            return Err(CommandError::Rejected(
                "this command must not be coalesced".into(),
            ));
        }
        Ok(())
    }

    fn rebuild_pending(&mut self, active_project: &Project) -> Result<()> {
        self.pending
            .sort_by_key(|batch| (batch.effective_time, batch.order));
        let mut candidate = active_project.clone();
        for batch in &mut self.pending {
            let mut transaction_candidate = candidate.clone();
            for envelope in &batch.envelopes {
                apply_payload(&mut transaction_candidate, &envelope.payload)?;
            }
            transaction_candidate.validate()?;
            candidate = transaction_candidate;
            batch.project = candidate.clone();
        }
        Ok(())
    }

    fn trim_history(&mut self) {
        while self.applied_order.len() > self.history_capacity {
            if let Some(id) = self.applied_order.pop_front()
                && self
                    .records
                    .get(&id)
                    .is_some_and(|record| record.applied_revision.is_some())
            {
                self.records.remove(&id);
            }
        }
    }

    /// Applies all envelopes immediately for reducer-only callers.
    pub fn apply_transaction(
        &mut self,
        project: &mut Project,
        envelopes: Vec<CommandEnvelope>,
    ) -> Result<Vec<CommandAck>> {
        let ids = envelopes
            .iter()
            .map(|envelope| envelope.id)
            .collect::<Vec<_>>();
        let acknowledgements = self.stage_transaction(project, envelopes, MediaTime::ZERO)?;
        if acknowledgements.iter().all(|ack| ack.duplicate) {
            return Ok(acknowledgements);
        }
        if let Some(latched) = self.latch_due(MediaTime::ZERO) {
            *project = latched.project;
        }
        Ok(ids
            .into_iter()
            .filter_map(|id| {
                self.records.get(&id).map(|record| CommandAck {
                    id,
                    revision: record.accepted_revision,
                    applied_revision: record.applied_revision,
                    duplicate: false,
                })
            })
            .collect())
    }
}

fn normalized_effective_time(envelope: &CommandEnvelope, now: MediaTime) -> MediaTime {
    envelope.effective_time.map_or(now, |time| time.max(now))
}

fn apply_payload(project: &mut Project, payload: &Command) -> Result<()> {
    match payload {
        Command::Noop | Command::SetName { .. } => {
            if let Command::SetName { name } = payload {
                project.name = name.clone();
            }
        }
        Command::AddInput { input } => project.insert_input(input.clone())?,
        Command::SetInputPlayback { input, playback } => {
            let input = project
                .inputs
                .get_mut(input)
                .ok_or_else(|| DomainError::UnknownId(input.to_string()))?;
            let eiviz_core::InputSource::Video {
                playback: current, ..
            } = &mut input.source
            else {
                return Err(CommandError::Rejected(
                    "input playback can only be set on a video input".into(),
                ));
            };
            *current = playback.clone();
        }
        Command::AddAsset { asset } => {
            if project.assets.contains_key(&asset.id) {
                return Err(CommandError::Domain(DomainError::DuplicateId(
                    asset.id.to_string(),
                )));
            }
            project.assets.insert(asset.id, asset.clone());
        }
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
            if let Ok(u) = project.mixing_unit_mut(owner)
                && !u.outputs.contains(&output.id)
            {
                u.outputs.push(output.id);
            }
        }
        Command::RemoveOutput { id } => {
            if let Some(o) = project.outputs.remove(id)
                && let Ok(u) = project.mixing_unit_mut(o.owner)
            {
                u.outputs.retain(|x| x != id);
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
            if let Ok(u) = project.mixing_unit_mut(owner)
                && !u.multiviews.contains(&view.id)
            {
                u.multiviews.push(view.id);
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
            version: COMMAND_ENVELOPE_VERSION,
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

    #[test]
    fn invalid_command_rolls_back_project_and_revision() {
        let mut project = Project::new("rollback");
        let before = project.clone();
        let mut sequencer = Sequencer::default();
        let input = Input {
            id: InputId::new(),
            name: "missing image".into(),
            tags: vec![],
            groups: vec![],
            source: InputSource::Image {
                asset: eiviz_core::AssetId::new(),
            },
        };
        let result = sequencer.apply(
            &mut project,
            CommandEnvelope::new(ClientId::new(), Command::AddInput { input }),
        );
        assert!(result.is_err());
        assert_eq!(project, before);
        assert_eq!(sequencer.revision(), 0);
    }

    #[test]
    fn transaction_rolls_back_every_command_and_sequence() {
        let mut project = Project::new("transaction");
        let before = project.clone();
        let client = ClientId::new();
        let mut sequencer = Sequencer::default();
        let mut first = CommandEnvelope::new(
            client,
            Command::SetName {
                name: "must roll back".into(),
            },
        );
        first.client_seq = 1;
        let mut invalid = CommandEnvelope::new(
            client,
            Command::RemoveMixingUnit {
                id: *project.mixing_units.keys().next().unwrap(),
            },
        );
        invalid.client_seq = 2;

        assert!(
            sequencer
                .apply_transaction(&mut project, vec![first, invalid])
                .is_err()
        );
        assert_eq!(project, before);
        assert_eq!(sequencer.revision(), 0);

        let mut retry = CommandEnvelope::new(
            client,
            Command::SetName {
                name: "committed".into(),
            },
        );
        retry.client_seq = 1;
        sequencer.apply(&mut project, retry).unwrap();
        assert_eq!(project.name, "committed");
    }

    #[test]
    fn client_sequence_and_envelope_version_are_validated() {
        let mut project = Project::new("sequence");
        let client = ClientId::new();
        let mut sequencer = Sequencer::default();
        let mut first = CommandEnvelope::new(client, Command::Noop);
        first.client_seq = 9;
        sequencer.apply(&mut project, first).unwrap();

        let mut stale = CommandEnvelope::new(client, Command::Noop);
        stale.client_seq = 9;
        assert!(matches!(
            sequencer.apply(&mut project, stale),
            Err(CommandError::ClientSequence { .. })
        ));

        let mut future = CommandEnvelope::new(client, Command::Noop);
        future.version += 1;
        assert!(matches!(
            sequencer.apply(&mut project, future),
            Err(CommandError::UnsupportedVersion { .. })
        ));
    }
}
