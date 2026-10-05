use crate::error::ProResult;
use crate::media::{AudioFrame, MediaOutput, VideoFrame};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckLinkDevice {
    /// Stable id. Decimal persistent id when the driver provides one.
    pub id: String,
    pub model: String,
    pub name: String,
    pub capture: bool,
    pub playback: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckLinkMode {
    /// FourCC display mode, decimal.
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
}

#[derive(Clone, Debug)]
pub struct DeckLinkCaptureConfig {
    pub device_id: String,
    pub mode_id: String,
}

#[derive(Clone, Debug)]
pub struct DeckLinkPlayoutConfig {
    pub device_id: String,
    pub mode_id: String,
    /// Enable the external keyer and deliver BGRA (alpha is the key).
    pub external_key: bool,
    pub fps_num: u32,
    pub fps_den: u32,
}

#[derive(Clone, Debug, Default)]
pub struct CaptureStats {
    pub frames: u64,
    pub no_signal: bool,
    pub last_error: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalStatus {
    Present,
    NoSignal,
}

/// Receives captured frames. Called from the capture thread, never re-entrantly.
pub trait CaptureSink: Send {
    fn video(&mut self, frame: VideoFrame);
    fn audio(&mut self, frame: AudioFrame);
    fn signal(&mut self, status: SignalStatus);
}

pub trait CaptureHandle: Send {
    fn stats(&self) -> CaptureStats;
}

pub trait DeckLinkBackend: Send + Sync {
    fn enumerate(&self) -> ProResult<Vec<DeckLinkDevice>>;
    fn modes(&self, device_id: &str) -> ProResult<Vec<DeckLinkMode>>;
    fn start_capture(
        &self,
        config: &DeckLinkCaptureConfig,
        sink: Box<dyn CaptureSink>,
    ) -> ProResult<Box<dyn CaptureHandle>>;
    fn start_playout(&self, config: &DeckLinkPlayoutConfig) -> ProResult<Box<dyn MediaOutput>>;
}
