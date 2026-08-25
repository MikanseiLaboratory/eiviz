use crate::ids::{InputId, MixingUnitId, MultiviewId, OutputId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Output {
    pub id: OutputId,
    pub name: String,
    pub owner: MixingUnitId,
    /// Video routed to this sink. Older projects default to the owning unit's
    /// Program feed; no source is inferred from the sink kind.
    #[serde(default)]
    pub video_source: OutputVideoSource,
    pub kind: OutputKind,
    pub enabled: bool,
    /// Required for distribution outputs. Kept separate from `kind` so old
    /// project files deserialize and can be rejected with an actionable error
    /// instead of silently receiving a codec default.
    #[serde(default)]
    pub distribution: Option<DistributionProfile>,
    /// CPU egress pixel format. Window outputs stay `None` and display GPU
    /// textures. Older projects omit the field; [`Self::effective_color_format`]
    /// fills the historical default for that kind.
    #[serde(default)]
    pub color_format: Option<OutputColorFormat>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OutputColorFormat {
    Rgba8,
    Bgra8,
    Nv12,
    Uyvy,
}

impl OutputColorFormat {
    pub fn allowed_for(kind: &OutputKind) -> &'static [Self] {
        match kind {
            OutputKind::Ndi { .. }
            | OutputKind::Rtmp { .. }
            | OutputKind::Srt { .. }
            | OutputKind::Mp4 { .. } => &[Self::Rgba8, Self::Nv12],
            OutputKind::Omt { .. } => &[Self::Bgra8, Self::Uyvy],
            OutputKind::DeckLink { .. } => &[Self::Bgra8],
            OutputKind::PreviewWindow
            | OutputKind::ProgramWindow
            | OutputKind::AudioDevice { .. } => &[],
        }
    }

    pub fn recommended_for(kind: &OutputKind) -> Option<Self> {
        match kind {
            OutputKind::Ndi { .. }
            | OutputKind::Rtmp { .. }
            | OutputKind::Srt { .. }
            | OutputKind::Mp4 { .. } => Some(Self::Nv12),
            OutputKind::Omt { .. } => Some(Self::Uyvy),
            OutputKind::DeckLink { .. } => Some(Self::Bgra8),
            OutputKind::PreviewWindow
            | OutputKind::ProgramWindow
            | OutputKind::AudioDevice { .. } => None,
        }
    }

    pub fn legacy_for(kind: &OutputKind) -> Option<Self> {
        match kind {
            OutputKind::Ndi { .. }
            | OutputKind::Rtmp { .. }
            | OutputKind::Srt { .. }
            | OutputKind::Mp4 { .. } => Some(Self::Rgba8),
            OutputKind::Omt { .. } | OutputKind::DeckLink { .. } => Some(Self::Bgra8),
            OutputKind::PreviewWindow
            | OutputKind::ProgramWindow
            | OutputKind::AudioDevice { .. } => None,
        }
    }

    pub fn allows(kind: &OutputKind, format: Self) -> bool {
        Self::allowed_for(kind).contains(&format)
    }
}

impl Output {
    pub fn effective_color_format(&self) -> Option<OutputColorFormat> {
        self.color_format
            .or_else(|| OutputColorFormat::legacy_for(&self.kind))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OutputVideoSource {
    #[default]
    Program,
    Preview,
    Multiview(MultiviewId),
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionProfile {
    pub video: H264EncoderProfile,
    pub audio: AacEncoderProfile,
    pub transport: TransportProfile,
    /// Per-sink access-unit capacity. Full queues drop locally and recover at
    /// the next H.264 keyframe.
    pub queue_capacity: usize,
    pub reconnect: ReconnectProfile,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum H264EncoderProfile {
    /// Cisco's separately installed OpenH264 2.6.0 binary. The in-tree I_PCM
    /// encoder is intentionally not a selectable product profile.
    CiscoOpenH26426 {
        bitrate_bps: u32,
        keyframe_interval_frames: u32,
        level_idc: u8,
    },
    /// Access units supplied by a named, external encoder adapter.
    ExternalAnnexB {
        adapter: String,
        bitrate_bps: u32,
        keyframe_interval_frames: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AacEncoderProfile {
    /// FDK AAC is a separately reviewed build because its upstream license
    /// grants no patent rights.
    FdkAacLc {
        bitrate_bps: u32,
        sample_rate: u32,
        channels: u16,
    },
    /// Raw AAC access units supplied by a named, external encoder adapter.
    ExternalRawAacLc {
        adapter: String,
        bitrate_bps: u32,
        sample_rate: u32,
        channels: u16,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportProfile {
    RtmpPublish {
        chunk_size: u32,
        connect_timeout_ms: u64,
    },
    SrtCallerMpegTs {
        latency_ms: u32,
        stream_id: Option<String>,
        connect_timeout_ms: u64,
    },
    FragmentedMp4 {
        recover_incomplete_tail: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconnectProfile {
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    /// Zero means retry until the output is explicitly stopped.
    pub max_attempts: u32,
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
