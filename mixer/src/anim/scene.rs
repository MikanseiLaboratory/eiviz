//! State-based Scene animation.
//!
//! A move always interpolates from the values on screen to one target state.
//! Playback does not rewrite the saved layout. The caller keeps that layout as
//! `base` and publishes the sampled slice only while something is moving.

use std::collections::{HashMap, VecDeque};

use crate::abi::{
    validate_camera, EivizActiveMove, EivizActiveSequence, EivizMotion, EivizReachedLayer,
    EivizSceneCamera, EivizSceneSequenceDesc, EivizSceneStateDesc, EivizSequenceStepDesc,
    OverlayDesc, Rect, SCENE_SEQ_PLAY, SCENE_SEQ_REVERSE, SCENE_SEQ_STOP,
};

pub(crate) type SceneCamera = EivizSceneCamera;

use super::{AnimClock, Curve};

const RETURN_FRAMES: u32 = 15;
const TAKEOVER_LOG: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
struct MotionDef {
    duration_frames: u32,
    curve: Curve,
}

#[derive(Clone, Debug)]
struct StateDef {
    layers: HashMap<u64, OverlayDesc>,
    camera: Option<SceneCamera>,
    enter: MotionDef,
}

#[derive(Clone, Debug)]
struct StepDef {
    state_id: u64,
    motion: Option<MotionDef>,
    hold_frames: u32,
}

#[derive(Clone, Debug)]
struct SequenceDef {
    steps: Vec<StepDef>,
}

struct Move {
    id: u64,
    state_id: u64,
    sequence_id: u64,
    from: HashMap<u64, OverlayDesc>,
    to: HashMap<u64, OverlayDesc>,
    camera: Option<(SceneCamera, SceneCamera)>,
    clock: AnimClock,
    curve: Curve,
}

/// Layers and camera the composer should draw for one frame.
pub(crate) struct Pose {
    pub layers: Vec<OverlayDesc>,
    pub camera: SceneCamera,
}

enum Phase {
    Moving { move_id: u64 },
    Holding { until_frame: u64 },
}

struct Playing {
    index: usize,
    dir: i8,
    phase: Phase,
}

/// Per-scene playback. Stored beside the saved layout, never inside it.
pub(crate) struct SceneRuntime {
    states: HashMap<u64, StateDef>,
    sequences: HashMap<u64, SequenceDef>,
    moves: Vec<Move>,
    owner: HashMap<u64, u64>,
    /// Last state a layer actually arrived at. `0` is the saved layout.
    reached: HashMap<u64, u64>,
    /// Frozen pose for layers that have moved and are not owned by a move.
    presented: HashMap<u64, OverlayDesc>,
    /// Move that currently owns the camera, mirroring `owner` for layers.
    camera_owner: Option<u64>,
    presented_camera: Option<SceneCamera>,
    /// Last state the camera arrived at. `0` is the saved camera.
    reached_camera: Option<u64>,
    playing: HashMap<u64, Playing>,
    next_move: u64,
    dirty: bool,
    takeovers: VecDeque<String>,
}

impl Default for SceneRuntime {
    fn default() -> Self {
        Self {
            states: HashMap::new(),
            sequences: HashMap::new(),
            moves: Vec::new(),
            owner: HashMap::new(),
            reached: HashMap::new(),
            presented: HashMap::new(),
            camera_owner: None,
            presented_camera: None,
            reached_camera: None,
            playing: HashMap::new(),
            next_move: 1,
            dirty: false,
            takeovers: VecDeque::new(),
        }
    }
}

impl SceneRuntime {
    fn define_states(&mut self, states: HashMap<u64, StateDef>) {
        let gone: Vec<u64> = self
            .moves
            .iter()
            .filter(|mv| mv.state_id != 0 && !states.contains_key(&mv.state_id))
            .map(|mv| mv.id)
            .collect();
        self.states = states;
        for id in gone {
            self.cancel_move(id);
        }
    }

    fn define_sequences(&mut self, sequences: HashMap<u64, SequenceDef>) {
        let stop: Vec<u64> = self
            .playing
            .keys()
            .copied()
            .filter(|id| !sequences.contains_key(id))
            .collect();
        self.sequences = sequences;
        for id in stop {
            self.stop_sequence(id);
        }
    }

    /// `state_id == 0` returns every layer and the camera to the saved layout.
    pub(crate) fn go_to(
        &mut self,
        state_id: u64,
        base: &[OverlayDesc],
        base_camera: SceneCamera,
        frame: u64,
    ) -> Result<(), &'static str> {
        self.go_toward(state_id, base, base_camera, frame, None)
    }

    fn go_toward(
        &mut self,
        state_id: u64,
        base: &[OverlayDesc],
        base_camera: SceneCamera,
        frame: u64,
        motion: Option<MotionDef>,
    ) -> Result<(), &'static str> {
        let (targets, camera, motion) = if state_id == 0 {
            let targets = base
                .iter()
                .filter(|layer| layer.layer_id != 0)
                .map(|layer| (layer.layer_id, *layer))
                .collect::<HashMap<_, _>>();
            let motion = motion.unwrap_or(MotionDef {
                duration_frames: RETURN_FRAMES,
                curve: Curve::Linear,
            });
            (targets, Some(base_camera), motion)
        } else {
            let state = self
                .states
                .get(&state_id)
                .ok_or("scene state does not exist")?;
            let targets = state.layers.clone();
            let camera = state.camera;
            if targets.is_empty() && camera.is_none() {
                return Ok(());
            }
            (targets, camera, motion.unwrap_or(state.enter))
        };
        self.start_move(state_id, 0, targets, camera, motion, base, base_camera, frame);
        Ok(())
    }

    pub fn sequence(
        &mut self,
        sequence_id: u64,
        op: u32,
        base: &[OverlayDesc],
        base_camera: SceneCamera,
        frame: u64,
    ) -> Result<(), &'static str> {
        match op {
            SCENE_SEQ_STOP => {
                self.stop_sequence(sequence_id);
                Ok(())
            }
            SCENE_SEQ_PLAY | SCENE_SEQ_REVERSE => {
                let def = self
                    .sequences
                    .get(&sequence_id)
                    .ok_or("scene sequence does not exist")?;
                if def.steps.len() < 2 {
                    return Err("scene sequence needs at least two steps");
                }
                let index = if op == SCENE_SEQ_REVERSE {
                    def.steps.len() - 1
                } else {
                    0
                };
                let dir: i8 = if op == SCENE_SEQ_REVERSE { -1 } else { 1 };
                self.stop_sequence(sequence_id);
                self.playing.insert(
                    sequence_id,
                    Playing {
                        index,
                        dir,
                        phase: Phase::Holding { until_frame: frame },
                    },
                );
                self.begin_step(sequence_id, base, base_camera, frame)?;
                Ok(())
            }
            _ => Err("unknown scene sequence operation"),
        }
    }

    /// Drops playback for layers whose saved geometry changed and clears their arrival record.
    pub fn note_base_edit(&mut self, previous: &[OverlayDesc], next: &[OverlayDesc]) {
        let prev = index_layers(previous);
        let nxt = index_layers(next);
        let mut changed = Vec::new();
        for (id, layer) in &nxt {
            match prev.get(id) {
                Some(old) if same_geom(old, layer) => {}
                _ => changed.push(*id),
            }
        }
        for id in prev.keys() {
            if !nxt.contains_key(id) {
                changed.push(*id);
            }
        }
        if !changed.is_empty() {
            self.dirty = true;
        }
        for id in changed {
            self.release_layer(id);
            self.presented.remove(&id);
            self.reached.remove(&id);
        }
    }

    /// Samples this frame. `None` means the published pose can stay as it is.
    pub fn tick(
        &mut self,
        base: &[OverlayDesc],
        base_camera: SceneCamera,
        frame: u64,
    ) -> Option<Pose> {
        if self.moves.is_empty() && self.playing.is_empty() && !self.dirty {
            return None;
        }
        let sampled = self.sample(base, base_camera, frame);
        self.store_presented(&sampled);
        let done: Vec<u64> = self
            .moves
            .iter()
            .filter(|mv| mv.clock.finished(frame))
            .map(|mv| mv.id)
            .collect();
        for id in done {
            self.complete_move(id);
        }
        self.advance(base, base_camera, frame);
        self.dirty = !self.moves.is_empty() || !self.playing.is_empty();
        Some(sampled)
    }

    /// State the camera last arrived at. `0` means the saved camera.
    pub fn camera_reached(&self) -> u64 {
        self.reached_camera.unwrap_or(0)
    }

    pub fn fill_status(
        &self,
        frame: u64,
        reached: &mut [EivizReachedLayer],
        moves: &mut [EivizActiveMove],
        sequences: &mut [EivizActiveSequence],
    ) -> (u32, u32, u32) {
        let mut reached_n = 0u32;
        let mut ids: Vec<u64> = self.reached.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            if (reached_n as usize) >= reached.len() {
                break;
            }
            reached[reached_n as usize] = EivizReachedLayer {
                layer_id: id,
                state_id: self.reached[&id],
            };
            reached_n += 1;
        }
        let mut move_n = 0u32;
        for mv in &self.moves {
            if (move_n as usize) >= moves.len() {
                break;
            }
            moves[move_n as usize] = EivizActiveMove {
                move_id: mv.id,
                state_id: mv.state_id,
                sequence_id: mv.sequence_id,
                progress: mv.clock.progress(frame),
                layer_count: mv.from.len() as u32,
            };
            move_n += 1;
        }
        let mut seq_n = 0u32;
        let mut seq_ids: Vec<u64> = self.playing.keys().copied().collect();
        seq_ids.sort_unstable();
        for id in seq_ids {
            if (seq_n as usize) >= sequences.len() {
                break;
            }
            let play = &self.playing[&id];
            sequences[seq_n as usize] = EivizActiveSequence {
                sequence_id: id,
                step_index: play.index as u32,
                reverse: u32::from(play.dir < 0),
                holding: u32::from(matches!(play.phase, Phase::Holding { .. })),
            };
            seq_n += 1;
        }
        (reached_n, move_n, seq_n)
    }

    pub fn takeover_log(&self) -> impl Iterator<Item = &str> {
        self.takeovers.iter().map(String::as_str)
    }

    pub fn define_states_raw(
        &mut self,
        ptr: *const EivizSceneStateDesc,
        count: u32,
    ) -> Result<(), &'static str> {
        let states = unsafe { states_from_ffi(ptr, count)? };
        self.define_states(states);
        Ok(())
    }

    pub fn define_sequences_raw(
        &mut self,
        ptr: *const EivizSceneSequenceDesc,
        count: u32,
    ) -> Result<(), &'static str> {
        let sequences = unsafe { sequences_from_ffi(ptr, count)? };
        self.define_sequences(sequences);
        Ok(())
    }

    /// Pose to publish after a saved-layout edit. `None` when playback is untouched.
    pub fn republish(
        &self,
        base: &[OverlayDesc],
        base_camera: SceneCamera,
        frame: u64,
    ) -> Option<Pose> {
        if !self.dirty
            && self.moves.is_empty()
            && self.presented.is_empty()
            && self.presented_camera.is_none()
        {
            return None;
        }
        Some(self.sample(base, base_camera, frame))
    }

    /// Cuts the picture to this pose. The saved layout stays put, and the next
    /// move starts from here instead of from a transition that is still running.
    pub fn hold_pose(&mut self, pose: &[OverlayDesc], camera: Option<SceneCamera>, state_id: u64) {
        self.moves.clear();
        self.owner.clear();
        self.camera_owner = None;
        self.playing.clear();
        self.presented.clear();
        self.reached.clear();
        for layer in pose {
            if layer.layer_id == 0 {
                continue;
            }
            let mut stored = *layer;
            stored.label = std::ptr::null();
            self.presented.insert(stored.layer_id, stored);
            self.reached.insert(stored.layer_id, state_id);
        }
        if let Some(camera) = camera {
            self.presented_camera = Some(camera);
            self.reached_camera = Some(state_id);
        }
        self.dirty = false;
    }

    /// Drops camera playback when the saved camera changed.
    pub fn note_camera_edit(&mut self, previous: SceneCamera, next: SceneCamera) {
        if previous == next {
            return;
        }
        self.dirty = true;
        self.release_camera();
        self.presented_camera = None;
        self.reached_camera = None;
    }

    fn start_move(
        &mut self,
        state_id: u64,
        sequence_id: u64,
        targets: HashMap<u64, OverlayDesc>,
        camera: Option<SceneCamera>,
        motion: MotionDef,
        base: &[OverlayDesc],
        base_camera: SceneCamera,
        frame: u64,
    ) {
        let current = self.sample(base, base_camera, frame);
        let current_layers = index_layers(&current.layers);
        let mut from = HashMap::new();
        let mut to = HashMap::new();
        for (id, target) in targets {
            let Some(now) = current_layers
                .get(&id)
                .copied()
                .or_else(|| base.iter().find(|layer| layer.layer_id == id).copied())
            else {
                continue;
            };
            self.steal_layer(id, true);
            from.insert(id, now);
            to.insert(id, target);
        }
        let camera_move = camera.map(|target| (current.camera, target));
        if from.is_empty() && camera_move.is_none() {
            return;
        }
        if camera_move.is_some() {
            self.steal_camera(true);
        }
        let id = self.next_move;
        self.next_move = self.next_move.saturating_add(1);
        for layer_id in from.keys() {
            self.owner.insert(*layer_id, id);
        }
        if camera_move.is_some() {
            self.camera_owner = Some(id);
        }
        self.moves.push(Move {
            id,
            state_id,
            sequence_id,
            from,
            to,
            camera: camera_move,
            clock: AnimClock::new(frame, motion.duration_frames),
            curve: motion.curve,
        });
        self.dirty = true;
    }

    fn steal_layer(&mut self, layer_id: u64, log: bool) {
        let Some(old_id) = self.owner.remove(&layer_id) else {
            return;
        };
        let Some(mv) = self.moves.iter_mut().find(|mv| mv.id == old_id) else {
            return;
        };
        mv.from.remove(&layer_id);
        mv.to.remove(&layer_id);
        let emptied = mv.from.is_empty() && mv.camera.is_none();
        let sequence_id = mv.sequence_id;
        if log {
            self.note_takeover(layer_id, old_id);
        }
        if emptied {
            self.moves.retain(|mv| mv.id != old_id);
            if sequence_id != 0 {
                self.playing.remove(&sequence_id);
            }
        }
    }

    fn note_takeover(&mut self, layer_id: u64, move_id: u64) {
        if self.takeovers.len() >= TAKEOVER_LOG {
            self.takeovers.pop_front();
        }
        self.takeovers
            .push_back(format!("layer {layer_id} left move {move_id}"));
    }

    fn steal_camera(&mut self, log: bool) {
        let Some(old_id) = self.camera_owner.take() else {
            return;
        };
        let Some(mv) = self.moves.iter_mut().find(|mv| mv.id == old_id) else {
            return;
        };
        mv.camera = None;
        let emptied = mv.from.is_empty();
        let sequence_id = mv.sequence_id;
        if log {
            self.note_camera_takeover(old_id);
        }
        if emptied {
            self.moves.retain(|mv| mv.id != old_id);
            if sequence_id != 0 {
                self.playing.remove(&sequence_id);
            }
        }
    }

    fn note_camera_takeover(&mut self, move_id: u64) {
        if self.takeovers.len() >= TAKEOVER_LOG {
            self.takeovers.pop_front();
        }
        self.takeovers
            .push_back(format!("camera left move {move_id}"));
    }

    fn release_layer(&mut self, layer_id: u64) {
        self.steal_layer(layer_id, false);
        self.owner.remove(&layer_id);
    }

    fn release_camera(&mut self) {
        self.steal_camera(false);
    }

    fn complete_move(&mut self, move_id: u64) {
        let Some(index) = self.moves.iter().position(|mv| mv.id == move_id) else {
            return;
        };
        let mv = self.moves.remove(index);
        for id in mv.from.keys() {
            if self.owner.get(id) == Some(&move_id) {
                self.owner.remove(id);
                self.reached.insert(*id, mv.state_id);
            }
        }
        if self.camera_owner == Some(move_id) {
            self.camera_owner = None;
            self.reached_camera = Some(mv.state_id);
        }
    }

    fn cancel_move(&mut self, move_id: u64) {
        let Some(index) = self.moves.iter().position(|mv| mv.id == move_id) else {
            return;
        };
        let mv = self.moves.remove(index);
        for id in mv.from.keys() {
            if self.owner.get(id) == Some(&move_id) {
                self.owner.remove(id);
            }
        }
        if self.camera_owner == Some(move_id) {
            self.camera_owner = None;
        }
        if mv.sequence_id != 0 {
            self.playing.remove(&mv.sequence_id);
        }
    }

    fn stop_sequence(&mut self, sequence_id: u64) {
        let Some(play) = self.playing.remove(&sequence_id) else {
            return;
        };
        if let Phase::Moving { move_id } = play.phase {
            self.cancel_move(move_id);
        }
        self.dirty = true;
    }

    fn begin_step(
        &mut self,
        sequence_id: u64,
        base: &[OverlayDesc],
        base_camera: SceneCamera,
        frame: u64,
    ) -> Result<(), &'static str> {
        let Some(play) = self.playing.get(&sequence_id) else {
            return Ok(());
        };
        let index = play.index;
        let Some(def) = self.sequences.get(&sequence_id) else {
            self.playing.remove(&sequence_id);
            return Err("scene sequence does not exist");
        };
        let Some(step) = def.steps.get(index) else {
            self.playing.remove(&sequence_id);
            return Err("scene sequence step is missing");
        };
        let state_id = step.state_id;
        let motion = step.motion;
        if let Some(play) = self.playing.get_mut(&sequence_id) {
            play.phase = Phase::Moving { move_id: 0 };
        }
        if state_id == 0 {
            let targets = base
                .iter()
                .filter(|layer| layer.layer_id != 0)
                .map(|layer| (layer.layer_id, *layer))
                .collect();
            let motion = motion.unwrap_or(MotionDef {
                duration_frames: RETURN_FRAMES,
                curve: Curve::Linear,
            });
            self.start_move(
                0,
                sequence_id,
                targets,
                Some(base_camera),
                motion,
                base,
                base_camera,
                frame,
            );
        } else if !self.states.contains_key(&state_id) {
            self.playing.remove(&sequence_id);
            return Err("scene sequence references a missing state");
        } else {
            let state = &self.states[&state_id];
            let targets = state.layers.clone();
            let camera = state.camera;
            let motion = motion.unwrap_or(state.enter);
            self.start_move(
                state_id,
                sequence_id,
                targets,
                camera,
                motion,
                base,
                base_camera,
                frame,
            );
        }
        if let Some(mv) = self.moves.last() {
            let move_id = mv.id;
            if mv.sequence_id == sequence_id {
                if let Some(play) = self.playing.get_mut(&sequence_id) {
                    play.phase = Phase::Moving { move_id };
                }
            }
        }
        Ok(())
    }

    fn advance(&mut self, base: &[OverlayDesc], base_camera: SceneCamera, frame: u64) {
        let ids: Vec<u64> = self.playing.keys().copied().collect();
        for id in ids {
            let Some(play) = self.playing.get(&id) else {
                continue;
            };
            match play.phase {
                Phase::Moving { move_id } => {
                    if self.moves.iter().any(|mv| mv.id == move_id) {
                        continue;
                    }
                    let hold = self
                        .sequences
                        .get(&id)
                        .and_then(|def| def.steps.get(play.index))
                        .map(|step| step.hold_frames)
                        .unwrap_or(0);
                    if hold > 0 {
                        if let Some(play) = self.playing.get_mut(&id) {
                            play.phase = Phase::Holding {
                                until_frame: frame.saturating_add(u64::from(hold)),
                            };
                        }
                        continue;
                    }
                }
                Phase::Holding { until_frame } => {
                    if frame < until_frame {
                        continue;
                    }
                }
            }
            let Some(def) = self.sequences.get(&id).cloned() else {
                self.playing.remove(&id);
                continue;
            };
            let Some(play) = self.playing.get(&id) else {
                continue;
            };
            let Some((next, dir)) = next_index(def.steps.len(), play.index, play.dir) else {
                self.playing.remove(&id);
                continue;
            };
            if let Some(play) = self.playing.get_mut(&id) {
                play.index = next;
                play.dir = dir;
            }
            let _ = self.begin_step(id, base, base_camera, frame);
        }
    }

    fn sample(&self, base: &[OverlayDesc], base_camera: SceneCamera, frame: u64) -> Pose {
        Pose {
            layers: base
                .iter()
                .map(|layer| self.sample_layer(*layer, frame))
                .collect(),
            camera: self.sample_camera(base_camera, frame),
        }
    }

    fn sample_camera(&self, base: SceneCamera, frame: u64) -> SceneCamera {
        if let Some(move_id) = self.camera_owner {
            if let Some(mv) = self.moves.iter().find(|mv| mv.id == move_id) {
                if let Some((from, to)) = mv.camera {
                    let done = mv.clock.finished(frame);
                    let t = if done {
                        1.0
                    } else {
                        mv.curve.eval(mv.clock.progress(frame))
                    };
                    return lerp_camera(from, to, t);
                }
            }
        }
        self.presented_camera.unwrap_or(base)
    }

    fn sample_layer(&self, base: OverlayDesc, frame: u64) -> OverlayDesc {
        let id = base.layer_id;
        if id != 0 {
            if let Some(&move_id) = self.owner.get(&id) {
                if let Some(mv) = self.moves.iter().find(|mv| mv.id == move_id) {
                    if let (Some(from), Some(to)) = (mv.from.get(&id), mv.to.get(&id)) {
                        let done = mv.clock.finished(frame);
                        let t = if done {
                            1.0
                        } else {
                            mv.curve.eval(mv.clock.progress(frame))
                        };
                        return blend(*from, *to, t, done);
                    }
                }
            }
            if let Some(pose) = self.presented.get(&id) {
                return *pose;
            }
        }
        base
    }

    fn store_presented(&mut self, sampled: &Pose) {
        for layer in &sampled.layers {
            if layer.layer_id != 0 && self.owner.contains_key(&layer.layer_id) {
                self.presented.insert(layer.layer_id, *layer);
            }
        }
        if self.camera_owner.is_some() {
            self.presented_camera = Some(sampled.camera);
        }
    }
}

fn next_index(len: usize, index: usize, dir: i8) -> Option<(usize, i8)> {
    if len == 0 {
        return None;
    }
    let next = index as isize + isize::from(dir);
    if (0..len as isize).contains(&next) {
        Some((next as usize, dir))
    } else {
        None
    }
}

fn blend(from: OverlayDesc, to: OverlayDesc, t: f32, done: bool) -> OverlayDesc {
    if done || t >= 1.0 {
        let mut done = to;
        done.label = std::ptr::null();
        return done;
    }
    if t <= 0.0 {
        let mut from = from;
        from.label = std::ptr::null();
        return from;
    }
    OverlayDesc {
        source_id: from.source_id,
        rect: lerp_rect(from.rect, to.rect, t),
        crop: lerp_rect(from.crop, to.crop, t),
        opacity: from.opacity + (to.opacity - from.opacity) * t,
        z: from.z,
        audio_follow: from.audio_follow,
        hidden: from.hidden,
        label: std::ptr::null(),
        layer_id: from.layer_id,
    }
}

fn lerp_camera(from: SceneCamera, to: SceneCamera, t: f32) -> SceneCamera {
    if t >= 1.0 {
        return to;
    }
    if t <= 0.0 {
        return from;
    }
    let zoom = (from.zoom.ln() + (to.zoom.ln() - from.zoom.ln()) * t).exp();
    SceneCamera {
        x: from.x + (to.x - from.x) * t,
        y: from.y + (to.y - from.y) * t,
        zoom,
    }
}

fn lerp_rect(from: Rect, to: Rect, t: f32) -> Rect {
    Rect {
        x: from.x + (to.x - from.x) * t,
        y: from.y + (to.y - from.y) * t,
        width: from.width + (to.width - from.width) * t,
        height: from.height + (to.height - from.height) * t,
    }
}

fn index_layers(layers: &[OverlayDesc]) -> HashMap<u64, OverlayDesc> {
    layers
        .iter()
        .filter(|layer| layer.layer_id != 0)
        .map(|layer| (layer.layer_id, *layer))
        .collect()
}

fn same_geom(a: &OverlayDesc, b: &OverlayDesc) -> bool {
    a.source_id == b.source_id
        && a.rect == b.rect
        && a.crop == b.crop
        && a.opacity == b.opacity
        && a.z == b.z
        && a.audio_follow == b.audio_follow
        && a.hidden == b.hidden
}

fn motion_from_ffi(motion: EivizMotion) -> Result<MotionDef, &'static str> {
    if motion.duration_frames == 0 {
        return Err("motion duration is zero");
    }
    let curve = if motion.easing == Curve::BEZIER {
        if motion.has_bezier == 0 {
            return Err("bezier motion is missing handles");
        }
        Curve::try_new(motion.easing, motion.x1, motion.y1, motion.x2, motion.y2)?
    } else {
        Curve::try_new(motion.easing, 0.0, 0.0, 1.0, 1.0)?
    };
    Ok(MotionDef {
        duration_frames: motion.duration_frames,
        curve,
    })
}

unsafe fn states_from_ffi(
    ptr: *const EivizSceneStateDesc,
    count: u32,
) -> Result<HashMap<u64, StateDef>, &'static str> {
    if count > 256 {
        return Err("too many scene states");
    }
    if count > 0 && ptr.is_null() {
        return Err("scene states pointer is null");
    }
    let slice = if count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, count as usize) }
    };
    let mut out = HashMap::new();
    for desc in slice {
        if desc.id == 0 {
            return Err("scene state id 0 is reserved");
        }
        if out.contains_key(&desc.id) {
            return Err("duplicate scene state id");
        }
        let layers = unsafe { layers_from_ffi(desc.layers, desc.layer_count)? };
        let camera = if desc.has_camera == 0 {
            None
        } else {
            validate_camera(desc.camera)?;
            Some(desc.camera)
        };
        out.insert(
            desc.id,
            StateDef {
                layers,
                camera,
                enter: motion_from_ffi(desc.enter)?,
            },
        );
    }
    Ok(out)
}

unsafe fn layers_from_ffi(
    ptr: *const OverlayDesc,
    count: u32,
) -> Result<HashMap<u64, OverlayDesc>, &'static str> {
    if count > 64 {
        return Err("too many state layers");
    }
    if count > 0 && ptr.is_null() {
        return Err("state layers pointer is null");
    }
    let slice = if count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, count as usize) }
    };
    let mut out = HashMap::new();
    for layer in slice {
        if layer.layer_id == 0 {
            return Err("state layer is missing an id");
        }
        if !layer.is_finite() {
            return Err("state layer geometry is not finite");
        }
        let mut layer = *layer;
        layer.label = std::ptr::null();
        out.insert(layer.layer_id, layer);
    }
    Ok(out)
}

unsafe fn sequences_from_ffi(
    ptr: *const EivizSceneSequenceDesc,
    count: u32,
) -> Result<HashMap<u64, SequenceDef>, &'static str> {
    if count > 256 {
        return Err("too many scene sequences");
    }
    if count > 0 && ptr.is_null() {
        return Err("scene sequences pointer is null");
    }
    let slice = if count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, count as usize) }
    };
    let mut out = HashMap::new();
    for desc in slice {
        if desc.id == 0 {
            return Err("scene sequence id is zero");
        }
        if out.contains_key(&desc.id) {
            return Err("duplicate scene sequence id");
        }
        if desc.step_count > 256 {
            return Err("too many sequence steps");
        }
        if desc.step_count > 0 && desc.steps.is_null() {
            return Err("sequence steps pointer is null");
        }
        let steps = if desc.step_count == 0 {
            Vec::new()
        } else {
            let raw = unsafe { std::slice::from_raw_parts(desc.steps, desc.step_count as usize) };
            let mut steps = Vec::with_capacity(raw.len());
            for step in raw {
                steps.push(step_from_ffi(*step)?);
            }
            steps
        };
        if steps.len() < 2 {
            return Err("scene sequence needs at least two steps");
        }
        out.insert(desc.id, SequenceDef { steps });
    }
    Ok(out)
}

fn step_from_ffi(step: EivizSequenceStepDesc) -> Result<StepDef, &'static str> {
    let motion = if step.has_motion == 0 {
        None
    } else {
        Some(motion_from_ffi(step.motion)?)
    };
    Ok(StepDef {
        state_id: step.state_id,
        motion,
        hold_frames: step.hold_frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::Rect;

    const CAM: SceneCamera = SceneCamera::IDENTITY;

    fn layer(id: u64, x: f32) -> OverlayDesc {
        OverlayDesc {
            source_id: 10 + id,
            rect: Rect {
                x,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            crop: Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            opacity: 1.0,
            z: 1,
            audio_follow: 1,
            hidden: 0,
            label: std::ptr::null(),
            layer_id: id,
        }
    }

    fn linear(frames: u32) -> MotionDef {
        MotionDef {
            duration_frames: frames,
            curve: Curve::Linear,
        }
    }

    fn state(id: u64, layers: &[(u64, f32)], frames: u32) -> (u64, StateDef) {
        (
            id,
            StateDef {
                layers: layers
                    .iter()
                    .map(|(layer_id, x)| (*layer_id, layer(*layer_id, *x)))
                    .collect(),
                camera: None,
                enter: linear(frames),
            },
        )
    }

    fn x_of(layers: &[OverlayDesc], id: u64) -> f32 {
        layers
            .iter()
            .find(|layer| layer.layer_id == id)
            .unwrap()
            .rect
            .x
    }

    #[test]
    fn go_to_from_mid_move_does_not_jump() {
        let mut rt = SceneRuntime::default();
        let (id_a, a) = state(1, &[(1, 1.0)], 30);
        let (id_b, b) = state(2, &[(1, 0.0)], 10);
        rt.define_states(HashMap::from([(id_a, a), (id_b, b)]));
        let base = vec![layer(1, 0.0)];
        rt.go_to(1, &base, CAM, 100).unwrap();
        let mid = rt.tick(&base, CAM, 115).unwrap().layers;
        assert!((x_of(&mid, 1) - 0.5).abs() < 1e-4);
        rt.go_to(2, &base, CAM, 115).unwrap();
        let next = rt.tick(&base, CAM, 116).unwrap().layers;
        let jumped = (x_of(&next, 1) - 0.0).abs() < 1e-3;
        assert!(!jumped, "x snapped to the base instead of leaving 0.5");
        assert!(x_of(&next, 1) < 0.5);
        assert!(x_of(&next, 1) > 0.4);
    }

    #[test]
    fn arrival_motion_ignores_where_the_layer_came_from() {
        let mut rt = SceneRuntime::default();
        rt.define_states(HashMap::from([
            state(1, &[(1, 0.2)], 2),
            state(2, &[(1, 1.0)], 30),
        ]));
        let base = vec![layer(1, 0.0)];
        rt.go_to(1, &base, CAM, 0).unwrap();
        let _ = rt.tick(&base, CAM, 2);
        rt.go_to(2, &base, CAM, 2).unwrap();
        assert_eq!(rt.moves[0].clock.duration_frames, 30);
    }

    #[test]
    fn partial_states_run_together_and_takeover_keeps_the_other_layer() {
        let mut rt = SceneRuntime::default();
        rt.define_states(HashMap::from([
            state(1, &[(1, 1.0)], 10),
            state(2, &[(2, 1.0)], 10),
        ]));
        let base = vec![layer(1, 0.0), layer(2, 0.0)];
        rt.go_to(1, &base, CAM, 0).unwrap();
        rt.go_to(2, &base, CAM, 0).unwrap();
        assert_eq!(rt.moves.len(), 2);
        let frame = rt.tick(&base, CAM, 5).unwrap().layers;
        assert!((x_of(&frame, 1) - 0.5).abs() < 1e-4);
        assert!((x_of(&frame, 2) - 0.5).abs() < 1e-4);

        rt.go_toward(1, &base, CAM, 5, Some(linear(10))).unwrap();
        assert_eq!(rt.moves.len(), 2);
        assert!(rt.takeover_log().any(|line| line.contains("layer 1")));
        let frame = rt.tick(&base, CAM, 6).unwrap().layers;
        assert!(x_of(&frame, 2) > 0.5);
    }

    #[test]
    fn sequence_holds_then_stops_at_either_end() {
        let mut rt = SceneRuntime::default();
        rt.define_states(HashMap::from([
            state(1, &[(1, 0.0)], 2),
            state(2, &[(1, 1.0)], 2),
            state(3, &[(1, 2.0)], 2),
        ]));
        rt.define_sequences(HashMap::from([(
            7,
            SequenceDef {
                steps: vec![
                    StepDef {
                        state_id: 1,
                        motion: None,
                        hold_frames: 3,
                    },
                    StepDef {
                        state_id: 2,
                        motion: None,
                        hold_frames: 0,
                    },
                    StepDef {
                        state_id: 3,
                        motion: None,
                        hold_frames: 0,
                    },
                ],
            },
        )]));
        let base = vec![layer(1, 0.5)];
        rt.sequence(7, SCENE_SEQ_PLAY, &base, CAM, 0).unwrap();
        let at_hold = rt.tick(&base, CAM, 2).unwrap().layers;
        assert!((x_of(&at_hold, 1) - 0.0).abs() < 1e-4);
        let still = rt.tick(&base, CAM, 4).unwrap().layers;
        assert!((x_of(&still, 1) - 0.0).abs() < 1e-4);
        let held = rt.tick(&base, CAM, 7).unwrap().layers;
        assert!((x_of(&held, 1) - 0.0).abs() < 1e-4);
        let leaving = rt.tick(&base, CAM, 8).unwrap().layers;
        assert!(x_of(&leaving, 1) > 0.0);

        rt.define_sequences(HashMap::from([(
            8,
            SequenceDef {
                steps: vec![
                    StepDef {
                        state_id: 1,
                        motion: Some(linear(2)),
                        hold_frames: 0,
                    },
                    StepDef {
                        state_id: 3,
                        motion: Some(linear(2)),
                        hold_frames: 0,
                    },
                ],
            },
        )]));
        rt.sequence(8, SCENE_SEQ_REVERSE, &base, CAM, 10).unwrap();
        assert_eq!(rt.playing[&8].index, 1);
        assert_eq!(rt.playing[&8].dir, -1);
        let _ = rt.tick(&base, CAM, 12);
        assert_eq!(rt.playing[&8].index, 0);
        assert_eq!(rt.playing[&8].dir, -1);
        let _ = rt.tick(&base, CAM, 14);
        assert!(rt.playing.get(&8).is_none());

        let mut rt = SceneRuntime::default();
        rt.define_states(HashMap::from([
            state(1, &[(1, 0.0)], 2),
            state(2, &[(1, 1.0)], 2),
        ]));
        rt.define_sequences(HashMap::from([(
            9,
            SequenceDef {
                steps: vec![
                    StepDef {
                        state_id: 1,
                        motion: Some(linear(2)),
                        hold_frames: 0,
                    },
                    StepDef {
                        state_id: 2,
                        motion: Some(linear(2)),
                        hold_frames: 0,
                    },
                ],
            },
        )]));
        let base = vec![layer(1, 0.25)];
        rt.sequence(9, SCENE_SEQ_PLAY, &base, CAM, 0).unwrap();
        let _ = rt.tick(&base, CAM, 2);
        assert_eq!(rt.playing[&9].index, 1);
        let _ = rt.tick(&base, CAM, 4);
        assert!(rt.playing.get(&9).is_none());
    }

    #[test]
    fn editing_the_saved_layout_stops_that_layer_and_clears_reached() {
        let mut rt = SceneRuntime::default();
        rt.define_states(HashMap::from([state(1, &[(1, 1.0), (2, 1.0)], 10)]));
        let base = vec![layer(1, 0.0), layer(2, 0.0)];
        rt.go_to(1, &base, CAM, 0).unwrap();
        let _ = rt.tick(&base, CAM, 10);
        assert_eq!(rt.reached.get(&1), Some(&1));
        let mut edited = base.clone();
        edited[0].rect.x = 0.25;
        rt.note_base_edit(&base, &edited);
        assert!(rt.reached.get(&1).is_none());
        assert_eq!(rt.reached.get(&2), Some(&1));
        assert!(rt.owner.get(&1).is_none());
        assert!(rt.owner.get(&2).is_none() || rt.moves.iter().any(|mv| mv.from.contains_key(&2)));
        let shown = rt.sample(&edited, CAM, 10).layers;
        assert!((x_of(&shown, 1) - 0.25).abs() < 1e-4);
    }

    #[test]
    fn thirty_frame_move_completes_on_the_thirtieth_tick() {
        let mut rt = SceneRuntime::default();
        rt.define_states(HashMap::from([state(1, &[(1, 1.0)], 30)]));
        let base = vec![layer(1, 0.0)];
        rt.go_to(1, &base, CAM, 100).unwrap();
        let mut frame = 101u64;
        let mut ticks = 0u32;
        loop {
            ticks += 1;
            let _ = rt.tick(&base, CAM, frame);
            if rt.moves.is_empty() {
                break;
            }
            frame += 1;
            assert!(ticks <= 30);
        }
        assert_eq!(ticks, 30);
        assert_eq!(frame, 130);
        assert_eq!(rt.reached.get(&1), Some(&1));
    }

    fn camera(zoom: f32, x: f32) -> SceneCamera {
        SceneCamera { x, y: 0.5, zoom }
    }

    #[test]
    fn holding_a_pose_stops_playback_and_the_next_move_starts_there() {
        let mut rt = SceneRuntime::default();
        rt.define_states(HashMap::from([(
            1,
            StateDef {
                layers: HashMap::from([(1, layer(1, 0.0))]),
                camera: Some(camera(4.0, 0.5)),
                enter: linear(30),
            },
        )]));
        let base = vec![layer(1, 0.0)];
        rt.go_to(1, &base, CAM, 0).unwrap();
        let _ = rt.tick(&base, CAM, 10);
        rt.hold_pose(&[layer(1, 0.4)], Some(camera(2.0, 0.6)), 3);
        assert!(rt.moves.is_empty());
        let shown = rt.sample(&base, CAM, 10);
        assert!((x_of(&shown.layers, 1) - 0.4).abs() < 1.0e-4);
        assert!((shown.camera.zoom - 2.0).abs() < 1.0e-4);
        assert_eq!(rt.camera_reached(), 3);
        rt.go_to(1, &base, CAM, 10).unwrap();
        let start = rt.tick(&base, CAM, 11).unwrap();
        assert!(x_of(&start.layers, 1) > 0.3);
        assert!(start.camera.zoom > 1.8);
    }

    #[test]
    fn camera_zoom_interpolates_on_a_log_scale_and_layers_stay() {
        let mut rt = SceneRuntime::default();
        rt.define_states(HashMap::from([(
            1,
            StateDef {
                layers: HashMap::new(),
                camera: Some(camera(4.0, 0.75)),
                enter: linear(30),
            },
        )]));
        let base = vec![layer(1, 0.0)];
        rt.go_to(1, &base, CAM, 0).unwrap();
        let mid = rt.tick(&base, CAM, 15).unwrap();
        assert!((mid.camera.zoom - 2.0).abs() < 1.0e-3, "zoom {}", mid.camera.zoom);
        assert!((mid.camera.x - 0.625).abs() < 1.0e-3);
        assert!((x_of(&mid.layers, 1) - 0.0).abs() < 1.0e-4);
    }

    #[test]
    fn camera_takeover_continues_from_the_value_on_screen() {
        let mut rt = SceneRuntime::default();
        rt.define_states(HashMap::from([
            (
                1,
                StateDef {
                    layers: HashMap::new(),
                    camera: Some(camera(4.0, 0.5)),
                    enter: linear(30),
                },
            ),
            (
                2,
                StateDef {
                    layers: HashMap::new(),
                    camera: Some(camera(1.0, 0.5)),
                    enter: linear(30),
                },
            ),
        ]));
        let base = vec![layer(1, 0.0)];
        rt.go_to(1, &base, CAM, 0).unwrap();
        let mid = rt.tick(&base, CAM, 15).unwrap();
        assert!((mid.camera.zoom - 2.0).abs() < 1.0e-3);
        rt.go_to(2, &base, CAM, 15).unwrap();
        let next = rt.tick(&base, CAM, 16).unwrap();
        assert!(next.camera.zoom < mid.camera.zoom);
        assert!(next.camera.zoom > 1.9, "zoom {}", next.camera.zoom);
        assert!(rt.takeover_log().any(|line| line.contains("camera")));
    }

    #[test]
    fn returning_to_the_saved_layout_restores_the_camera() {
        let mut rt = SceneRuntime::default();
        rt.define_states(HashMap::from([(
            1,
            StateDef {
                layers: HashMap::new(),
                camera: Some(camera(4.0, 0.5)),
                enter: linear(10),
            },
        )]));
        let base = vec![layer(1, 0.0)];
        rt.go_to(1, &base, CAM, 0).unwrap();
        let _ = rt.tick(&base, CAM, 10);
        assert_eq!(rt.camera_reached(), 1);
        rt.go_to(0, &base, CAM, 10).unwrap();
        let back = rt.tick(&base, CAM, 25).unwrap();
        assert!((back.camera.zoom - 1.0).abs() < 1.0e-3);
        assert_eq!(rt.camera_reached(), 0);
    }

    #[test]
    fn editing_the_saved_camera_drops_camera_playback() {
        let mut rt = SceneRuntime::default();
        rt.define_states(HashMap::from([(
            1,
            StateDef {
                layers: HashMap::new(),
                camera: Some(camera(4.0, 0.5)),
                enter: linear(10),
            },
        )]));
        let base = vec![layer(1, 0.0)];
        rt.go_to(1, &base, CAM, 0).unwrap();
        let _ = rt.tick(&base, CAM, 10);
        assert_eq!(rt.camera_reached(), 1);
        let edited = camera(2.0, 0.5);
        rt.note_camera_edit(CAM, edited);
        assert_eq!(rt.camera_reached(), 0);
        let shown = rt.sample(&base, edited, 10);
        assert!((shown.camera.zoom - 2.0).abs() < 1.0e-4);
    }

    #[test]
    fn a_layer_only_move_leaves_the_camera_running() {
        let mut rt = SceneRuntime::default();
        rt.define_states(HashMap::from([
            (
                1,
                StateDef {
                    layers: HashMap::from([(1, layer(1, 1.0))]),
                    camera: Some(camera(4.0, 0.5)),
                    enter: linear(30),
                },
            ),
            (
                2,
                StateDef {
                    layers: HashMap::from([(1, layer(1, 0.0))]),
                    camera: None,
                    enter: linear(30),
                },
            ),
        ]));
        let base = vec![layer(1, 0.0)];
        rt.go_to(1, &base, CAM, 0).unwrap();
        let _ = rt.tick(&base, CAM, 10);
        rt.go_to(2, &base, CAM, 10).unwrap();
        let next = rt.tick(&base, CAM, 15).unwrap();
        assert!((next.camera.zoom - 2.0).abs() < 1.0e-3, "zoom {}", next.camera.zoom);
        assert!(x_of(&next.layers, 1) < 0.5);
    }
}
