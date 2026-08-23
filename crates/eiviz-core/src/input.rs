use crate::ids::{AssetId, DeviceBindingId, InputId, MixingUnitId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Input {
    pub id: InputId,
    pub name: String,
    pub tags: Vec<String>,
    pub groups: Vec<String>,
    pub source: InputSource,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InputSource {
    ColorBars,
    SolidColor {
        r: u8,
        g: u8,
        b: u8,
        a: u8,
    },
    Image {
        asset: AssetId,
    },
    Video {
        asset: AssetId,
        playback: Playback,
    },
    Ndi {
        source_name: String,
    },
    Omt {
        url: String,
    },
    DeckLink {
        binding: DeviceBindingId,
    },
    AudioDevice {
        binding: DeviceBindingId,
    },
    /// Feed from another mixing unit. Creates a DAG edge.
    MixFeed {
        unit: MixingUnitId,
        tap: MixTap,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MixTap {
    Program,
    Preview,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Playback {
    pub playing: bool,
    pub loop_playback: bool,
    pub position_us: u64,
    pub in_us: u64,
    pub out_us: Option<u64>,
    pub speed: f32,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            playing: true,
            loop_playback: true,
            position_us: 0,
            in_us: 0,
            out_us: None,
            speed: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceBinding {
    pub id: DeviceBindingId,
    pub kind: String,
    pub logical_name: String,
    pub last_seen_hardware_id: Option<String>,
}
