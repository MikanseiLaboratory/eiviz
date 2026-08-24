mod asrc;

pub use asrc::{AsrcDiagnostics, AsrcError, StreamingAsrc};

use eiviz_core::{InputId, Playback};
use eiviz_time::{ClockDomain, FrameRate, MediaTime};
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("queue full ({0})")]
    QueueFull(&'static str),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("disconnected: {0}")]
    Disconnected(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, MediaError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba8,
    Bgra8,
    Nv12,
}

#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub id: u64,
    pub source: Option<InputId>,
    pub pts: MediaTime,
    pub capture_domain: ClockDomain,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub data: Arc<[u8]>,
    pub discontinuity: bool,
}

impl VideoFrame {
    pub fn rgba_solid(id: u64, pts: MediaTime, width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let len = (width as usize) * (height as usize) * 4;
        let mut data = vec![0u8; len];
        for px in data.chunks_exact_mut(4) {
            px.copy_from_slice(&rgba);
        }
        Self {
            id,
            source: None,
            pts,
            capture_domain: ClockDomain::Virtual,
            width,
            height,
            format: PixelFormat::Rgba8,
            data: data.into(),
            discontinuity: false,
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ]
    }
}

#[derive(Clone, Debug)]
pub struct AudioBuffer {
    pub sample_index: u64,
    pub sample_rate: u32,
    pub channels: u16,
    /// Planar f32, channel-major.
    pub planes: Vec<Vec<f32>>,
    /// Source-device clock correlation, when the adapter exposes one.
    pub capture_timestamp: Option<AudioCaptureTimestamp>,
    pub discontinuity: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioCaptureTimestamp {
    /// Absolute sample index in the source device's clock domain.
    pub device_sample_index: u64,
    /// Device callback instant, relative to the stream clock origin.
    pub callback_nanos: u64,
    /// Estimated ADC capture instant, relative to the same stream clock origin.
    pub capture_nanos: u64,
}

impl AudioBuffer {
    pub fn silence(sample_index: u64, sample_rate: u32, channels: u16, frames: usize) -> Self {
        Self {
            sample_index,
            sample_rate,
            channels,
            planes: vec![vec![0.0; frames]; channels as usize],
            capture_timestamp: None,
            discontinuity: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EncodedAccessUnit {
    pub pts: MediaTime,
    pub dts: Option<MediaTime>,
    pub keyframe: bool,
    /// Shared immutable payload. Fan-out clones the `Arc`, not the encoded
    /// bytes, so every sink receives the same encode result.
    pub bytes: Arc<[u8]>,
    pub kind: EncodedKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedStreamConfig {
    /// Annex-B SPS NAL without a start code.
    pub h264_sps: Arc<[u8]>,
    /// Annex-B PPS NAL without a start code.
    pub h264_pps: Arc<[u8]>,
    /// MPEG-4 AudioSpecificConfig for AAC-LC.
    pub aac_audio_specific_config: Arc<[u8]>,
    pub video_width: u16,
    pub video_height: u16,
    pub video_timescale: u32,
    pub video_sample_duration: u32,
    pub audio_sample_rate: u32,
    pub audio_channels: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodedKind {
    Avc,
    Aac,
    Pcm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueuePolicy {
    /// Program: keep cadence; overwrite only if explicitly repeating last-good.
    ProgramHold,
    /// Preview / multiview: latest wins.
    LatestWins,
    /// Recorder / network: drop local, never stall program.
    IndependentDrop,
}

#[derive(Debug)]
pub struct BoundedSlot<T> {
    inner: Mutex<Option<T>>,
    name: &'static str,
    policy: QueuePolicy,
}

impl<T> BoundedSlot<T> {
    pub fn new(name: &'static str, policy: QueuePolicy) -> Self {
        Self {
            inner: Mutex::new(None),
            name,
            policy,
        }
    }

    pub fn push(&self, value: T) -> Result<()> {
        let mut g = self.inner.lock();
        match self.policy {
            QueuePolicy::LatestWins | QueuePolicy::IndependentDrop | QueuePolicy::ProgramHold => {
                *g = Some(value);
                Ok(())
            }
        }
    }

    pub fn take(&self) -> Option<T> {
        self.inner.lock().take()
    }

    pub fn peek_clone(&self) -> Option<T>
    where
        T: Clone,
    {
        self.inner.lock().clone()
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capability {
    pub id: String,
    pub available: bool,
    pub detail: String,
}

pub trait MediaSource: Send + Sync {
    fn id(&self) -> InputId;
    fn pull_video(&self, pts: MediaTime, rate: FrameRate) -> Result<Option<VideoFrame>>;
    fn pull_audio(&self, sample_index: u64, frames: usize) -> Result<Option<AudioBuffer>>;

    /// Applies authoritative project playback state when this source supports it.
    fn update_playback(&self, _playback: &Playback) {}

    fn audio_diagnostics(&self) -> Option<AudioIoDiagnostics> {
        None
    }
}

pub trait MediaSink: Send + Sync {
    fn name(&self) -> &str;
    fn push_video(&self, frame: &VideoFrame) -> Result<()>;
    fn push_audio(&self, audio: &AudioBuffer) -> Result<()>;
}

/// Dedicated audio-only output attachment.
pub trait AudioSink: Send + Sync {
    fn name(&self) -> &str;
    fn push_audio(&self, audio: &AudioBuffer) -> Result<()>;
    fn diagnostics(&self) -> AudioIoDiagnostics;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterHealth {
    Running,
    Degraded,
    Unavailable,
    Failed,
}

#[derive(Clone, Debug)]
pub struct AudioIoDiagnostics {
    pub name: String,
    pub health: AdapterHealth,
    pub callbacks: u64,
    pub device_frames: u64,
    pub xruns: u64,
    pub queue_overflows: u64,
    pub queue_underflows: u64,
    pub last_device_sample_index: u64,
    pub last_callback_nanos: u64,
    pub last_device_nanos: u64,
    pub last_error: Option<String>,
    pub asrc: Option<AsrcDiagnostics>,
}
