use crate::ids::{InputId, MixingUnitId, MultiviewId, OutputId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Output {
    pub id: OutputId,
    pub name: String,
    pub owner: MixingUnitId,
    pub kind: OutputKind,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OutputKind {
    PreviewWindow,
    ProgramWindow,
    Ndi { name: String },
    Omt { url: String },
    DeckLink { binding: crate::DeviceBindingId },
    AudioDevice { binding: crate::DeviceBindingId },
    Rtmp { url: String },
    Srt { url: String },
    Mp4 { path: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Multiview {
    pub id: MultiviewId,
    pub name: String,
    pub owner: MixingUnitId,
    pub columns: u32,
    pub rows: u32,
    pub tiles: Vec<MultiviewTile>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MultiviewTile {
    pub column: u32,
    pub row: u32,
    pub source: MultiviewSource,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MultiviewSource {
    Black,
    Input(InputId),
    Preview(MixingUnitId),
    Program(MixingUnitId),
}
