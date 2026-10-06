use crate::error::ProResult;
use crate::media::MediaOutput;

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

#[derive(Clone, Debug)]
pub struct RtmpConfig {
    /// `rtmp://` or `rtmps://` URL including the application and stream name.
    /// The stream name may be a secret reference resolved by the mixer.
    pub url: String,
    pub video_bitrate: u32,
    pub keyint: u32,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub encoder: VideoEncoderKind,
    pub audio: AudioEncodeConfig,
}

pub trait StreamBackend: Send + Sync {
    fn start_rtmp(&self, config: &RtmpConfig) -> ProResult<Box<dyn MediaOutput>>;
}
