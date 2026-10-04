use std::collections::HashMap;

use crate::ids::ResourceKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourcePhase {
    Ready,
    Retrying,
    Failed,
    Stopped,
}

impl ResourcePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Retrying => "retrying",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceStatus {
    pub kind: ResourceKind,
    pub id: u64,
    pub phase: ResourcePhase,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LivePeak {
    pub id: u64,
    pub left: f32,
    pub right: f32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveState {
    pub units: HashMap<u64, UnitLiveState>,
    #[serde(default)]
    pub peaks: Vec<LivePeak>,
    /// Keyed by the scene GPU id. Playback is live-only and is not part of the saved layout.
    #[serde(default)]
    pub scenes: HashMap<u64, SceneAnimLive>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReachedLayer {
    pub layer_id: u64,
    pub state_id: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveMoveLive {
    pub move_id: u64,
    pub state_id: u64,
    pub sequence_id: u64,
    pub progress: f32,
    pub layer_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSequenceLive {
    pub sequence_id: u64,
    pub step_index: u32,
    pub reverse: bool,
    pub holding: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneAnimLive {
    pub reached: Vec<ReachedLayer>,
    pub moves: Vec<ActiveMoveLive>,
    pub sequences: Vec<ActiveSequenceLive>,
    pub takeovers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitLiveState {
    pub program_source: u64,
    pub preview_source: u64,
    pub mix: f32,
    pub transitioning: bool,
    pub incoming_source: u64,
    pub overlay_sources: Vec<u64>,
}

impl Default for UnitLiveState {
    fn default() -> Self {
        Self {
            program_source: 0,
            preview_source: 0,
            mix: 0.0,
            transitioning: false,
            incoming_source: 0,
            overlay_sources: Vec::new(),
        }
    }
}
