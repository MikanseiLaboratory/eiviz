use crate::session::{Document, InputDto, InputKind, OverlaySlot, SceneDto, SceneLayer, UnitDto};
use serde::{Deserialize, Serialize};

/// Client-originated mutation. Live ops change Mix Effect state. Session ops
/// edit the canonical document. Ops never carry GPU handles or native surfaces.
#[derive(Debug, Clone)]
pub enum Command {
    Preview {
        unit_id: u64,
        scene_id: u64,
    },
    Cut {
        unit_id: u64,
        swap: bool,
        incoming: Incoming,
    },
    Auto {
        unit_id: u64,
        kind: u32,
        duration_ms: u32,
        swap: bool,
        keep_preview: bool,
        easing: u32,
        direction: u32,
        dip_r: f32,
        dip_g: f32,
        dip_b: f32,
        dip_a: f32,
        incoming: Incoming,
        softness: f32,
        param: f32,
    },
    SetMix {
        unit_id: u64,
        value: f32,
    },
    OverlayAuto {
        unit_id: u64,
        index: u32,
        duration_ms: u32,
        to_on: bool,
    },
    VideoPlay {
        input_id: u64,
        playing: bool,
    },
    VideoLoop {
        input_id: u64,
        looping: bool,
    },
    VideoSeek {
        input_id: u64,
        position_hns: i64,
    },
    AudioSetInput {
        input_id: u64,
        bus_mask: u32,
        gain: f32,
        mute: bool,
    },
    AudioSetBus {
        bus_id: u64,
        gain: f32,
        mute: bool,
    },
    ReplaceSession {
        document: Box<Document>,
        expected_revision: Option<u64>,
    },
    MutateSession {
        mutation: Box<SessionMutation>,
        expected_revision: Option<u64>,
    },
    Snapshot {
        unit_id: u64,
        kind: SnapshotKind,
        path: String,
    },
    Discover {
        kind: DiscoverKind,
    },
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Incoming {
    Preview,
    Program,
    Source(u64),
}

impl Incoming {
    pub fn from_u64(value: u64) -> Self {
        match value {
            0 => Self::Preview,
            u64::MAX => Self::Program,
            other => Self::Source(other),
        }
    }

    pub fn to_u64(self) -> u64 {
        match self {
            Self::Preview => 0,
            Self::Program => u64::MAX,
            Self::Source(id) => id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotKind {
    Program,
    Preview,
    Source(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoverKind {
    Omt,
    Ndi,
    Audio,
}

impl Command {
    pub fn is_live(&self) -> bool {
        matches!(
            self,
            Self::Preview { .. }
                | Self::Cut { .. }
                | Self::Auto { .. }
                | Self::SetMix { .. }
                | Self::OverlayAuto { .. }
                | Self::VideoPlay { .. }
                | Self::VideoLoop { .. }
                | Self::VideoSeek { .. }
                | Self::AudioSetInput { .. }
                | Self::AudioSetBus { .. }
        )
    }

    pub fn is_session(&self) -> bool {
        matches!(
            self,
            Self::ReplaceSession { .. } | Self::MutateSession { .. }
        )
    }

    pub fn requires_idempotency(&self) -> bool {
        matches!(
            self,
            Self::Cut { .. } | Self::ReplaceSession { .. } | Self::Shutdown
        )
    }

    pub fn expected_input_kind_for_path(kind: InputKind) -> bool {
        matches!(kind, InputKind::Still | InputKind::Video)
    }
}

/// Collaborative document edit. Applied against a staged clone, then replaced
/// through the same reconcile path as `ReplaceSession`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SessionMutation {
    UpsertInput {
        input: Box<InputDto>,
    },
    DeleteInput {
        id: u64,
    },
    UpsertScene {
        scene: Box<SceneDto>,
    },
    DeleteScene {
        id: u64,
    },
    SetSceneLayers {
        scene_id: u64,
        layers: Vec<SceneLayer>,
    },
    UpsertUnit {
        unit: Box<UnitDto>,
    },
    DeleteUnit {
        id: u64,
    },
    SetOverlaySlot {
        unit_id: u64,
        index: u32,
        slot: Box<OverlaySlot>,
    },
    AddMediaInput {
        name: String,
        media_kind: InputKind,
        host_path: String,
        video_loop: bool,
        tags: Vec<String>,
    },
    UpsertMultiview {
        layout: Box<crate::session::MultiviewDto>,
    },
    DeleteMultiview {
        id: u64,
    },
    SetSettings {
        settings: Box<crate::session::SessionSettings>,
        outputs: Vec<crate::session::OutputDto>,
        buses: Vec<crate::session::BusDto>,
        headphone_copy_master: bool,
        next_output_id: u64,
        next_bus_id: u64,
    },
}

impl SessionMutation {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::UpsertInput { .. } => "UpsertInput",
            Self::DeleteInput { .. } => "DeleteInput",
            Self::UpsertScene { .. } => "UpsertScene",
            Self::DeleteScene { .. } => "DeleteScene",
            Self::SetSceneLayers { .. } => "SetSceneLayers",
            Self::UpsertUnit { .. } => "UpsertUnit",
            Self::DeleteUnit { .. } => "DeleteUnit",
            Self::SetOverlaySlot { .. } => "SetOverlaySlot",
            Self::AddMediaInput { .. } => "AddMediaInput",
            Self::UpsertMultiview { .. } => "UpsertMultiview",
            Self::DeleteMultiview { .. } => "DeleteMultiview",
            Self::SetSettings { .. } => "SetSettings",
        }
    }
}
