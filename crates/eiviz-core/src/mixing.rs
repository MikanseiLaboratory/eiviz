use crate::ids::{MixingUnitId, MultiviewId, OutputId, OverlayId, SceneId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MixingUnit {
    pub id: MixingUnitId,
    pub name: String,
    pub preview: MixBus,
    pub program: MixBus,
    pub transition: Transition,
    pub overlays: Vec<OverlaySlot>,
    pub outputs: Vec<OutputId>,
    pub multiviews: Vec<MultiviewId>,
    pub audio_follow: AudioFollowPolicy,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MixBus {
    pub scene: Option<SceneId>,
}

impl MixBus {
    pub fn empty() -> Self {
        Self { scene: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub style: TransitionStyle,
    pub duration_frames: u32,
    pub remaining_frames: u32,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            style: TransitionStyle::Cut,
            duration_frames: 0,
            remaining_frames: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionStyle {
    Cut,
    Mix,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OverlaySlot {
    pub id: OverlayId,
    pub name: String,
    pub scene: Option<SceneId>,
    pub enabled: bool,
    pub z_order: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioFollowPolicy {
    Off,
    Program,
    ProgramAndPreview,
}

impl MixingUnit {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: MixingUnitId::new(),
            name: name.into(),
            preview: MixBus::empty(),
            program: MixBus::empty(),
            transition: Transition::default(),
            overlays: Vec::new(),
            outputs: Vec::new(),
            multiviews: Vec::new(),
            audio_follow: AudioFollowPolicy::Program,
        }
    }

    /// Atomically swap preview into program. Preview becomes empty unless `swap`.
    pub fn take(&mut self, swap: bool) {
        let prev = self.preview.scene;
        if self.transition.style == TransitionStyle::Cut || self.transition.duration_frames == 0 {
            if swap {
                self.preview.scene = self.program.scene;
            }
            self.program.scene = prev;
            self.transition.remaining_frames = 0;
        } else {
            self.transition.remaining_frames = self.transition.duration_frames;
            if swap {
                self.preview.scene = self.program.scene;
            }
            // Program stays until mix completes; runtime blends using remaining_frames.
            // Store target on preview so the compiler can see both scenes.
            self.preview.scene = prev;
        }
    }

    pub fn tick_transition(&mut self) {
        if self.transition.remaining_frames == 0 {
            return;
        }
        self.transition.remaining_frames -= 1;
        if self.transition.remaining_frames == 0 {
            self.program.scene = self.preview.scene;
        }
    }

    pub fn mix_factor(&self) -> f32 {
        if self.transition.remaining_frames == 0 || self.transition.duration_frames == 0 {
            return 1.0;
        }
        1.0 - (self.transition.remaining_frames as f32 / self.transition.duration_frames as f32)
    }
}
