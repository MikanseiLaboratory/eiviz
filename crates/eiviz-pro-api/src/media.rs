use std::sync::Arc;

use crate::error::ProResult;

/// Pixel layout of [`VideoFrame::data`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelLayout {
    /// 8-bit packed UYVY, `stride` bytes per row.
    Uyvy,
    /// 8-bit B, G, R, A. Used for DeckLink external key (alpha is the key).
    Bgra,
}

#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub layout: PixelLayout,
    /// 100 ns ticks, the same clock as OMT and NDI.
    pub pts_100ns: i64,
    pub fps_num: u32,
    pub fps_den: u32,
    pub data: Arc<[u8]>,
}

/// 32-bit float planar PCM, one plane per channel.
#[derive(Clone, Debug)]
pub struct AudioFrame {
    pub sample_rate: u32,
    pub channels: u32,
    pub frames: u32,
    pub pts_100ns: i64,
    pub planar_f32: Arc<[f32]>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutputStats {
    pub connected: bool,
    pub video_frames: u64,
    pub repeated: u64,
    pub dropped: u64,
    pub bitrate: u32,
    pub message: String,
}

/// Something that accepts paced frames from the mixer send thread.
pub trait MediaOutput: Send {
    fn submit_video(&mut self, frame: VideoFrame) -> ProResult<()>;
    fn submit_audio(&mut self, frame: AudioFrame) -> ProResult<()>;
    fn stats(&self) -> OutputStats;
}
