use crate::error::ProResult;
use crate::media::{MediaOutput, OutputStats};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoEncoderKind {
    OpenH264,
}

#[derive(Clone, Debug)]
pub struct AudioEncodeConfig {
    pub bitrate: u32,
    /// Explicit. There is no automatic fallback to a silent stream.
    pub video_only: bool,
}

/// One encode. Destinations are attached afterwards and share the packets.
#[derive(Clone, Debug)]
pub struct StreamEncodeConfig {
    pub video_bitrate: u32,
    pub keyint: u32,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub encoder: VideoEncoderKind,
    /// `0` means automatic: logical CPUs, clamped to the encoder's own limit.
    pub encoder_threads: u32,
    /// Milliseconds of encoded media to hold before the first send, then release
    /// at 1x. `0` sends immediately and keeps only a few packets queued.
    pub buffer_ms: u32,
    /// `0` low, `1` medium, `2` high.
    pub complexity: u32,
    /// `0` is treated as one reference frame.
    pub ref_frames: u32,
    pub cabac: bool,
    pub deblock: bool,
    pub background_detection: bool,
    pub adaptive_quant: bool,
    pub scene_change: bool,
    /// Extra delay added on top of the measured video encode time.
    pub audio_delay_ms: u32,
    pub audio: AudioEncodeConfig,
}

pub trait StreamBackend: Send + Sync {
    fn start_stream(&self, config: &StreamEncodeConfig) -> ProResult<Box<dyn StreamOutput>>;
}

/// An encoder plus a set of publish sockets. Destination calls may run on a
/// thread other than the one that submits frames; implementations lock internally.
pub trait StreamOutput: MediaOutput {
    fn add_destination(&mut self, id: u64, url: &str) -> ProResult<()>;
    fn remove_destination(&mut self, id: u64) -> ProResult<()>;
    fn destination_stats(&self, id: u64) -> ProResult<OutputStats>;
}
