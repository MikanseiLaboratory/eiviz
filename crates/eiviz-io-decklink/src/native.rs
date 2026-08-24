use crate::{
    AUDIO_TIME_SCALE, DeviceDirection, DeviceInfo, SDK_ABI_VERSION, VIDEO_FRAME_DURATION,
    VIDEO_HEIGHT, VIDEO_TIME_SCALE, VIDEO_WIDTH, decklink_audio_time_to_sample_index,
    decklink_ticks_to_media_time, ffi, media_time_to_decklink_ticks, push_latest, resolve_binding,
    sample_index_to_decklink_ticks,
};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use eiviz_core::{DeviceBinding, InputId};
use eiviz_media::{
    AdapterHealth, AudioBuffer, Capability, MediaError, MediaSink, MediaSource, PixelFormat,
    VideoFrame,
};
use eiviz_time::{ClockDomain, FrameRate, MediaTime, NTSC_5994};
use parking_lot::Mutex;
use std::ffi::{CStr, CString, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const HEALTH_RUNNING: u8 = 0;
const HEALTH_DEGRADED: u8 = 1;
const HEALTH_UNAVAILABLE: u8 = 2;
const HEALTH_FAILED: u8 = 3;
const ERROR_CAPACITY: usize = 512;

#[derive(Debug, thiserror::Error)]
pub enum DeckLinkError {
    #[error("DeckLink shim ABI mismatch: expected {expected}, got {actual}")]
    AbiMismatch { expected: u32, actual: u32 },
    #[error(transparent)]
    Binding(#[from] crate::BindingResolutionError),
    #[error("invalid DeckLink configuration: {0}")]
    InvalidConfiguration(String),
    #[error("invalid DeckLink frame: {0}")]
    InvalidFrame(String),
    #[error("DeckLink SDK: {0}")]
    Native(String),
    #[error("failed to spawn DeckLink worker: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("DeckLink worker stopped during startup")]
    WorkerStartup,
}

#[derive(Clone, Debug)]
pub struct DeckLinkConfig {
    pub video_queue_capacity: usize,
    pub audio_queue_capacity: usize,
    pub output_queue_capacity: usize,
    pub output_preroll_video_frames: u32,
    pub audio_channels: u16,
}

impl Default for DeckLinkConfig {
    fn default() -> Self {
        Self {
            video_queue_capacity: 2,
            audio_queue_capacity: 8,
            output_queue_capacity: 8,
            output_preroll_video_frames: 3,
            audio_channels: 2,
        }
    }
}

impl DeckLinkConfig {
    fn validate(&self) -> Result<(), DeckLinkError> {
        if self.video_queue_capacity == 0
            || self.audio_queue_capacity == 0
            || self.output_queue_capacity == 0
            || self.output_preroll_video_frames == 0
        {
            return Err(DeckLinkError::InvalidConfiguration(
                "queue capacities must be greater than zero".into(),
            ));
        }
        if !matches!(self.audio_channels, 2 | 8 | 16) {
            return Err(DeckLinkError::InvalidConfiguration(
                "audio channel count must be 2, 8, or 16 for this vertical slice".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeckLinkPlaybackDiagnostics {
    pub scheduled_video: u64,
    pub completed_video: u64,
    pub late_video: u64,
    pub dropped_video: u64,
    pub flushed_video: u64,
    pub buffered_video: u32,
    pub buffered_audio_frames: u32,
    pub reference_locked: Option<bool>,
    pub queue_rejections: u64,
}

pub fn probe() -> Capability {
    match enumerate_devices() {
        Ok(devices) if devices.is_empty() => Capability {
            id: "decklink".into(),
            available: false,
            detail: "Desktop Video SDK loaded; no DeckLink devices found".into(),
        },
        Ok(devices) => Capability {
            id: "decklink".into(),
            available: true,
            detail: format!(
                "Desktop Video SDK ready; {} DeckLink device(s), fixed 1080p59.94 BGRA/48 kHz profile",
                devices.len()
            ),
        },
        Err(error) => Capability {
            id: "decklink".into(),
            available: false,
            detail: error.to_string(),
        },
    }
}

pub fn enumerate_devices() -> Result<Vec<DeviceInfo>, DeckLinkError> {
    ensure_abi()?;
    unsafe extern "C" fn visit(context: *mut c_void, device: *const ffi::Device) {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            if context.is_null() || device.is_null() {
                return;
            }
            // SAFETY: The shim guarantees these pointers for the duration of this callback.
            let info = unsafe { &*device };
            // SAFETY: The shim supplies non-null, NUL-terminated strings.
            let persistent_id = unsafe { CStr::from_ptr(info.persistent_id) }
                .to_string_lossy()
                .into_owned();
            // SAFETY: The shim supplies non-null, NUL-terminated strings.
            let display_name = unsafe { CStr::from_ptr(info.display_name) }
                .to_string_lossy()
                .into_owned();
            // SAFETY: `context` points to the Vec owned by enumerate_devices.
            unsafe { &mut *context.cast::<Vec<DeviceInfo>>() }.push(DeviceInfo {
                persistent_id,
                display_name,
                supports_capture: info.capabilities & ffi::DEVICE_CAPTURE != 0,
                supports_playback: info.capabilities & ffi::DEVICE_PLAYBACK != 0,
            });
        }));
    }

    let mut devices = Vec::new();
    let mut error = [0 as c_char; ERROR_CAPACITY];
    // SAFETY: The callback is synchronous and `devices` outlives the call.
    let result = unsafe {
        ffi::eiviz_decklink_enumerate(
            visit,
            ptr::from_mut(&mut devices).cast(),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    check_native(result, &error)?;
    Ok(devices)
}

struct CaptureContext {
    id: InputId,
    video_tx: Sender<VideoFrame>,
    video_drop_rx: Receiver<VideoFrame>,
    audio_tx: Sender<AudioBuffer>,
    audio_drop_rx: Receiver<AudioBuffer>,
    dropped_video: Arc<AtomicU64>,
    dropped_audio: Arc<AtomicU64>,
    next_video_id: AtomicU64,
    previous_video_time: Mutex<Option<MediaTime>>,
    video_active: Arc<AtomicBool>,
    health: Arc<AtomicU8>,
    last_error: Arc<Mutex<Option<String>>>,
}

pub struct DeckLinkSource {
    id: InputId,
    device: DeviceInfo,
    capture: *mut c_void,
    context: *mut CaptureContext,
    video_rx: Receiver<VideoFrame>,
    audio_rx: Receiver<AudioBuffer>,
    last_video: Mutex<Option<VideoFrame>>,
    video_active: Arc<AtomicBool>,
    health: Arc<AtomicU8>,
    last_error: Arc<Mutex<Option<String>>>,
    dropped_video: Arc<AtomicU64>,
    dropped_audio: Arc<AtomicU64>,
}

// The SDK serializes close against callbacks; queues and state are thread-safe.
unsafe impl Send for DeckLinkSource {}
// MediaSource may be pulled from the runtime while SDK callbacks enqueue frames.
unsafe impl Sync for DeckLinkSource {}

impl DeckLinkSource {
    pub fn open(
        id: InputId,
        binding: &DeviceBinding,
        config: DeckLinkConfig,
    ) -> Result<Self, DeckLinkError> {
        config.validate()?;
        ensure_abi()?;
        let devices = enumerate_devices()?;
        let device = resolve_binding(binding, DeviceDirection::Capture, &devices)?.clone();
        let hardware_id = CString::new(device.persistent_id.as_str()).map_err(|_| {
            DeckLinkError::InvalidConfiguration("hardware ID contains a NUL byte".into())
        })?;
        let (video_tx, video_rx) = bounded(config.video_queue_capacity);
        let (audio_tx, audio_rx) = bounded(config.audio_queue_capacity);
        let video_active = Arc::new(AtomicBool::new(false));
        let health = Arc::new(AtomicU8::new(HEALTH_DEGRADED));
        let last_error = Arc::new(Mutex::new(None));
        let dropped_video = Arc::new(AtomicU64::new(0));
        let dropped_audio = Arc::new(AtomicU64::new(0));
        let context = Box::new(CaptureContext {
            id,
            video_drop_rx: video_rx.clone(),
            video_tx,
            audio_drop_rx: audio_rx.clone(),
            audio_tx,
            dropped_video: dropped_video.clone(),
            dropped_audio: dropped_audio.clone(),
            next_video_id: AtomicU64::new(0),
            previous_video_time: Mutex::new(None),
            video_active: video_active.clone(),
            health: health.clone(),
            last_error: last_error.clone(),
        });
        let context = Box::into_raw(context);
        let mut capture = ptr::null_mut();
        let mut error = [0 as c_char; ERROR_CAPACITY];
        // SAFETY: All pointers are valid; the context is retained until capture_close returns.
        let result = unsafe {
            ffi::eiviz_decklink_capture_open(
                hardware_id.as_ptr(),
                u32::from(config.audio_channels),
                capture_video,
                capture_audio,
                context.cast(),
                &mut capture,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if let Err(error) = check_native(result, &error) {
            // SAFETY: open failed synchronously and no SDK callback can retain the context.
            unsafe { drop(Box::from_raw(context)) };
            return Err(error);
        }
        Ok(Self {
            id,
            device,
            capture,
            context,
            video_rx,
            audio_rx,
            last_video: Mutex::new(None),
            video_active,
            health,
            last_error,
            dropped_video,
            dropped_audio,
        })
    }

    pub fn device(&self) -> &DeviceInfo {
        &self.device
    }

    pub fn health(&self) -> AdapterHealth {
        health_from_atomic(&self.health)
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().clone()
    }

    pub fn dropped_frames(&self) -> (u64, u64) {
        (
            self.dropped_video.load(Ordering::Relaxed),
            self.dropped_audio.load(Ordering::Relaxed),
        )
    }
}

impl MediaSource for DeckLinkSource {
    fn id(&self) -> InputId {
        self.id
    }

    fn pull_video(
        &self,
        _pts: MediaTime,
        rate: FrameRate,
    ) -> eiviz_media::Result<Option<VideoFrame>> {
        if rate != NTSC_5994 {
            return Err(MediaError::Unsupported(format!(
                "DeckLink vertical slice requires {NTSC_5994}, got {rate}"
            )));
        }
        let mut latest = None;
        for frame in self.video_rx.try_iter() {
            latest = Some(frame);
        }
        if let Some(frame) = latest {
            *self.last_video.lock() = Some(frame.clone());
            return Ok(Some(frame));
        }
        if self.video_active.load(Ordering::Acquire) {
            return Ok(self.last_video.lock().clone());
        }
        Ok(None)
    }

    fn pull_audio(
        &self,
        _sample_index: u64,
        _frames: usize,
    ) -> eiviz_media::Result<Option<AudioBuffer>> {
        Ok(self.audio_rx.try_recv().ok())
    }
}

impl Drop for DeckLinkSource {
    fn drop(&mut self) {
        // SAFETY: This source exclusively owns the SDK capture handle.
        unsafe { ffi::eiviz_decklink_capture_close(self.capture) };
        // SAFETY: capture_close has stopped streams and synchronized callbacks.
        unsafe { drop(Box::from_raw(self.context)) };
    }
}

unsafe extern "C" fn capture_video(context: *mut c_void, frame: *const ffi::VideoFrame) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if context.is_null() || frame.is_null() {
            return;
        }
        // SAFETY: The shim keeps both objects valid through this callback.
        let (context, frame) = unsafe {
            (
                &*context.cast::<CaptureContext>(),
                &*frame.cast::<ffi::VideoFrame>(),
            )
        };
        if frame.flags & ffi::FRAME_NO_INPUT != 0 {
            context.video_active.store(false, Ordering::Release);
            context.health.store(HEALTH_DEGRADED, Ordering::Release);
            *context.last_error.lock() = Some("DeckLink reports no input signal".into());
            return;
        }
        match convert_capture_video(context, frame) {
            Ok(frame) => {
                push_latest(
                    &context.video_tx,
                    &context.video_drop_rx,
                    frame,
                    &context.dropped_video,
                );
                context.video_active.store(true, Ordering::Release);
                context.health.store(HEALTH_RUNNING, Ordering::Release);
                *context.last_error.lock() = None;
            }
            Err(error) => {
                context.video_active.store(false, Ordering::Release);
                context.health.store(HEALTH_DEGRADED, Ordering::Release);
                *context.last_error.lock() = Some(error.to_string());
            }
        }
    }));
}

unsafe extern "C" fn capture_audio(context: *mut c_void, packet: *const ffi::AudioPacket) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if context.is_null() || packet.is_null() {
            return;
        }
        // SAFETY: The shim keeps both objects valid through this callback.
        let (context, packet) = unsafe {
            (
                &*context.cast::<CaptureContext>(),
                &*packet.cast::<ffi::AudioPacket>(),
            )
        };
        match convert_capture_audio(packet) {
            Ok(audio) => {
                push_latest(
                    &context.audio_tx,
                    &context.audio_drop_rx,
                    audio,
                    &context.dropped_audio,
                );
                context.health.store(HEALTH_RUNNING, Ordering::Release);
            }
            Err(error) => {
                context.health.store(HEALTH_DEGRADED, Ordering::Release);
                *context.last_error.lock() = Some(error.to_string());
            }
        }
    }));
}

fn convert_capture_video(
    context: &CaptureContext,
    frame: &ffi::VideoFrame,
) -> Result<VideoFrame, DeckLinkError> {
    if frame.width != VIDEO_WIDTH || frame.height != VIDEO_HEIGHT {
        return Err(DeckLinkError::InvalidFrame(format!(
            "capture is {}x{}, expected {}x{}",
            frame.width, frame.height, VIDEO_WIDTH, VIDEO_HEIGHT
        )));
    }
    let active_row = VIDEO_WIDTH as usize * 4;
    let row_bytes = frame.row_bytes as usize;
    let required = row_bytes
        .checked_mul(VIDEO_HEIGHT as usize)
        .ok_or_else(|| DeckLinkError::InvalidFrame("video byte count overflow".into()))?;
    if frame.data.is_null() || row_bytes < active_row || frame.data_len < required {
        return Err(DeckLinkError::InvalidFrame(
            "truncated or invalid BGRA capture frame".into(),
        ));
    }
    // SAFETY: The callback frame exposes at least `required` bytes.
    let source = unsafe { std::slice::from_raw_parts(frame.data, required) };
    let mut rgba = Vec::with_capacity(active_row * VIDEO_HEIGHT as usize);
    for row in source.chunks(row_bytes).take(VIDEO_HEIGHT as usize) {
        for pixel in row[..active_row].chunks_exact(4) {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    let pts = decklink_ticks_to_media_time(frame.stream_time, frame.time_scale)
        .ok_or_else(|| DeckLinkError::InvalidFrame("invalid video time scale".into()))?;
    let mut previous = context.previous_video_time.lock();
    let discontinuity = previous.is_some_and(|old| pts <= old);
    *previous = Some(pts);
    Ok(VideoFrame {
        id: context.next_video_id.fetch_add(1, Ordering::Relaxed),
        source: Some(context.id),
        pts,
        capture_domain: ClockDomain::SourceMedia,
        width: frame.width,
        height: frame.height,
        format: PixelFormat::Rgba8,
        data: rgba.into(),
        discontinuity,
    })
}

fn convert_capture_audio(packet: &ffi::AudioPacket) -> Result<AudioBuffer, DeckLinkError> {
    let channels = u16::try_from(packet.channels)
        .map_err(|_| DeckLinkError::InvalidFrame("audio channel count exceeds u16".into()))?;
    let frames = packet.frame_count as usize;
    let required = frames
        .checked_mul(channels as usize)
        .ok_or_else(|| DeckLinkError::InvalidFrame("audio sample count overflow".into()))?;
    if channels == 0
        || frames == 0
        || packet.sample_rate == 0
        || packet.samples.is_null()
        || packet.sample_count < required
    {
        return Err(DeckLinkError::InvalidFrame(
            "truncated or invalid PCM capture packet".into(),
        ));
    }
    // SAFETY: The callback packet exposes at least `required` i16 samples.
    let interleaved = unsafe { std::slice::from_raw_parts(packet.samples, required) };
    let mut planes = vec![Vec::with_capacity(frames); channels as usize];
    for frame in interleaved.chunks_exact(channels as usize) {
        for (channel, sample) in frame.iter().enumerate() {
            planes[channel].push(f32::from(*sample) / 32768.0);
        }
    }
    Ok(AudioBuffer {
        sample_index: decklink_audio_time_to_sample_index(
            packet.packet_time,
            packet.time_scale,
            packet.sample_rate,
        ),
        sample_rate: packet.sample_rate,
        channels,
        planes,
    })
}

enum OutputPacket {
    Video(VideoFrame),
    Audio(AudioBuffer),
}

pub struct DeckLinkSink {
    name: String,
    device: DeviceInfo,
    tx: Sender<OutputPacket>,
    stop: Arc<AtomicBool>,
    health: Arc<AtomicU8>,
    last_error: Arc<Mutex<Option<String>>>,
    diagnostics: Arc<Mutex<DeckLinkPlaybackDiagnostics>>,
    queue_rejections: Arc<AtomicU64>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl DeckLinkSink {
    pub fn create(
        name: impl Into<String>,
        binding: &DeviceBinding,
        config: DeckLinkConfig,
    ) -> Result<Self, DeckLinkError> {
        config.validate()?;
        ensure_abi()?;
        let devices = enumerate_devices()?;
        let device = resolve_binding(binding, DeviceDirection::Playback, &devices)?.clone();
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DeckLinkError::InvalidConfiguration(
                "output name must not be empty".into(),
            ));
        }
        let (tx, rx) = bounded(config.output_queue_capacity);
        let stop = Arc::new(AtomicBool::new(false));
        let health = Arc::new(AtomicU8::new(HEALTH_DEGRADED));
        let last_error = Arc::new(Mutex::new(None));
        let diagnostics = Arc::new(Mutex::new(DeckLinkPlaybackDiagnostics::default()));
        let queue_rejections = Arc::new(AtomicU64::new(0));
        let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
        let worker = {
            let hardware_id = device.persistent_id.clone();
            let stop = stop.clone();
            let health = health.clone();
            let last_error = last_error.clone();
            let diagnostics = diagnostics.clone();
            let queue_rejections = queue_rejections.clone();
            thread::Builder::new()
                .name(format!("decklink-output-{name}"))
                .spawn(move || {
                    let open = PlaybackHandle::open(&hardware_id, config.audio_channels);
                    let mut handle = match open {
                        Ok(handle) => {
                            let _ = startup_tx.send(Ok(()));
                            handle
                        }
                        Err(error) => {
                            let message = error.to_string();
                            health.store(HEALTH_FAILED, Ordering::Release);
                            *last_error.lock() = Some(message.clone());
                            let _ = startup_tx.send(Err(message));
                            return;
                        }
                    };
                    let mut first_video_time = None;
                    let mut scheduled_video = 0_u32;
                    let mut has_audio = false;
                    while !stop.load(Ordering::Acquire) || !rx.is_empty() {
                        let Ok(packet) = rx.recv_timeout(Duration::from_millis(10)) else {
                            continue;
                        };
                        let result = match packet {
                            OutputPacket::Video(frame) => {
                                let time =
                                    media_time_to_decklink_ticks(frame.pts, VIDEO_TIME_SCALE);
                                if first_video_time.is_none() {
                                    first_video_time = time;
                                }
                                let result = handle.schedule_video(&frame);
                                if result.is_ok() {
                                    scheduled_video = scheduled_video.saturating_add(1);
                                }
                                result
                            }
                            OutputPacket::Audio(audio) => {
                                let result = handle.schedule_audio(&audio);
                                has_audio |= result.is_ok();
                                result
                            }
                        };
                        let result = result.and_then(|()| {
                            if !handle.started
                                && scheduled_video >= config.output_preroll_video_frames
                                && has_audio
                            {
                                handle.start(first_video_time.unwrap_or(0))
                            } else {
                                Ok(())
                            }
                        });
                        match result {
                            Ok(()) => {
                                health.store(HEALTH_RUNNING, Ordering::Release);
                                *last_error.lock() = None;
                                if let Ok(mut value) = handle.diagnostics() {
                                    value.queue_rejections =
                                        queue_rejections.load(Ordering::Relaxed);
                                    *diagnostics.lock() = value;
                                }
                            }
                            Err(error) => {
                                health.store(HEALTH_DEGRADED, Ordering::Release);
                                *last_error.lock() = Some(error.to_string());
                            }
                        }
                    }
                })?
        };
        match startup_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let _ = worker.join();
                return Err(DeckLinkError::Native(error));
            }
            Err(_) => {
                let _ = worker.join();
                return Err(DeckLinkError::WorkerStartup);
            }
        }
        Ok(Self {
            name,
            device,
            tx,
            stop,
            health,
            last_error,
            diagnostics,
            queue_rejections,
            worker: Mutex::new(Some(worker)),
        })
    }

    pub fn device(&self) -> &DeviceInfo {
        &self.device
    }

    pub fn health(&self) -> AdapterHealth {
        health_from_atomic(&self.health)
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().clone()
    }

    pub fn diagnostics(&self) -> DeckLinkPlaybackDiagnostics {
        let mut diagnostics = *self.diagnostics.lock();
        diagnostics.queue_rejections = self.queue_rejections.load(Ordering::Relaxed);
        diagnostics
    }

    fn enqueue(&self, packet: OutputPacket) -> eiviz_media::Result<()> {
        match self.tx.try_send(packet) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.queue_rejections.fetch_add(1, Ordering::Relaxed);
                self.health.store(HEALTH_DEGRADED, Ordering::Release);
                *self.last_error.lock() = Some("bounded DeckLink output queue is full".into());
                Err(MediaError::QueueFull("decklink-output"))
            }
            Err(TrySendError::Disconnected(_)) => {
                self.health.store(HEALTH_FAILED, Ordering::Release);
                Err(MediaError::Disconnected(
                    "DeckLink output worker stopped".into(),
                ))
            }
        }
    }
}

impl MediaSink for DeckLinkSink {
    fn name(&self) -> &str {
        &self.name
    }

    fn push_video(&self, frame: &VideoFrame) -> eiviz_media::Result<()> {
        self.enqueue(OutputPacket::Video(frame.clone()))
    }

    fn push_audio(&self, audio: &AudioBuffer) -> eiviz_media::Result<()> {
        self.enqueue(OutputPacket::Audio(audio.clone()))
    }
}

impl Drop for DeckLinkSink {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.get_mut().take() {
            let _ = worker.join();
        }
    }
}

struct PlaybackHandle {
    raw: *mut c_void,
    audio_channels: u16,
    started: bool,
}

impl PlaybackHandle {
    fn open(hardware_id: &str, audio_channels: u16) -> Result<Self, DeckLinkError> {
        let hardware_id = CString::new(hardware_id).map_err(|_| {
            DeckLinkError::InvalidConfiguration("hardware ID contains a NUL byte".into())
        })?;
        let mut raw = ptr::null_mut();
        let mut error = [0 as c_char; ERROR_CAPACITY];
        // SAFETY: All pointers are valid for the duration of this synchronous call.
        let result = unsafe {
            ffi::eiviz_decklink_playback_open(
                hardware_id.as_ptr(),
                u32::from(audio_channels),
                &mut raw,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        check_native(result, &error)?;
        Ok(Self {
            raw,
            audio_channels,
            started: false,
        })
    }

    fn schedule_video(&mut self, frame: &VideoFrame) -> Result<(), DeckLinkError> {
        if frame.width != VIDEO_WIDTH || frame.height != VIDEO_HEIGHT {
            return Err(DeckLinkError::InvalidFrame(format!(
                "output is {}x{}, expected {}x{}",
                frame.width, frame.height, VIDEO_WIDTH, VIDEO_HEIGHT
            )));
        }
        let expected = VIDEO_WIDTH as usize * VIDEO_HEIGHT as usize * 4;
        if frame.data.len() < expected {
            return Err(DeckLinkError::InvalidFrame(
                "output video frame is truncated".into(),
            ));
        }
        let bgra;
        let bytes = match frame.format {
            PixelFormat::Bgra8 => &frame.data,
            PixelFormat::Rgba8 => {
                bgra = frame
                    .data
                    .chunks_exact(4)
                    .flat_map(|pixel| [pixel[2], pixel[1], pixel[0], pixel[3]])
                    .collect::<Vec<_>>();
                bgra.as_slice()
            }
            PixelFormat::Nv12 => {
                return Err(DeckLinkError::InvalidFrame(
                    "NV12 to BGRA conversion is not implemented".into(),
                ));
            }
        };
        let display_time = media_time_to_decklink_ticks(frame.pts, VIDEO_TIME_SCALE)
            .ok_or_else(|| DeckLinkError::InvalidFrame("video timestamp overflow".into()))?;
        let mut error = [0 as c_char; ERROR_CAPACITY];
        // SAFETY: The shim copies `bytes` before returning.
        let result = unsafe {
            ffi::eiviz_decklink_playback_schedule_video(
                self.raw,
                bytes.as_ptr(),
                bytes.len(),
                VIDEO_WIDTH * 4,
                display_time,
                VIDEO_FRAME_DURATION,
                VIDEO_TIME_SCALE,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        check_native(result, &error)
    }

    fn schedule_audio(&mut self, audio: &AudioBuffer) -> Result<(), DeckLinkError> {
        if audio.sample_rate != AUDIO_TIME_SCALE as u32
            || audio.channels != self.audio_channels
            || audio.planes.len() != self.audio_channels as usize
        {
            return Err(DeckLinkError::InvalidFrame(format!(
                "output audio must be 48 kHz/{} channel planar f32",
                self.audio_channels
            )));
        }
        let frames = audio.planes.first().map_or(0, Vec::len);
        if frames == 0 || audio.planes.iter().any(|plane| plane.len() != frames) {
            return Err(DeckLinkError::InvalidFrame(
                "output audio planes are empty or unequal".into(),
            ));
        }
        let frame_count = u32::try_from(frames)
            .map_err(|_| DeckLinkError::InvalidFrame("audio packet is too large".into()))?;
        let mut interleaved = Vec::with_capacity(frames * self.audio_channels as usize);
        for frame in 0..frames {
            for channel in &audio.planes {
                let value = (channel[frame].clamp(-1.0, 1.0) * 32767.0).round();
                interleaved.push(value as i16);
            }
        }
        let stream_time =
            sample_index_to_decklink_ticks(audio.sample_index, audio.sample_rate, AUDIO_TIME_SCALE)
                .ok_or_else(|| DeckLinkError::InvalidFrame("audio timestamp overflow".into()))?;
        let mut error = [0 as c_char; ERROR_CAPACITY];
        // SAFETY: The SDK copies or consumes the packet synchronously per ScheduleAudioSamples.
        let result = unsafe {
            ffi::eiviz_decklink_playback_schedule_audio(
                self.raw,
                interleaved.as_ptr(),
                frame_count,
                stream_time,
                AUDIO_TIME_SCALE,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        check_native(result, &error)
    }

    fn start(&mut self, start_time: i64) -> Result<(), DeckLinkError> {
        let mut error = [0 as c_char; ERROR_CAPACITY];
        // SAFETY: `self.raw` is a live, exclusively worker-owned playback handle.
        let result = unsafe {
            ffi::eiviz_decklink_playback_start(
                self.raw,
                start_time,
                VIDEO_TIME_SCALE,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        check_native(result, &error)?;
        self.started = true;
        Ok(())
    }

    fn diagnostics(&mut self) -> Result<DeckLinkPlaybackDiagnostics, DeckLinkError> {
        let mut native = ffi::PlaybackDiagnostics::default();
        let mut error = [0 as c_char; ERROR_CAPACITY];
        // SAFETY: `self.raw` and the diagnostics pointer are valid.
        let result = unsafe {
            ffi::eiviz_decklink_playback_get_diagnostics(
                self.raw,
                &mut native,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        check_native(result, &error)?;
        Ok(DeckLinkPlaybackDiagnostics {
            scheduled_video: native.scheduled_video,
            completed_video: native.completed_video,
            late_video: native.late_video,
            dropped_video: native.dropped_video,
            flushed_video: native.flushed_video,
            buffered_video: native.buffered_video,
            buffered_audio_frames: native.buffered_audio_frames,
            reference_locked: match native.reference_locked {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            },
            queue_rejections: 0,
        })
    }
}

impl Drop for PlaybackHandle {
    fn drop(&mut self) {
        // SAFETY: The worker exclusively owns this live playback handle.
        unsafe { ffi::eiviz_decklink_playback_close(self.raw) };
    }
}

fn ensure_abi() -> Result<(), DeckLinkError> {
    // SAFETY: This function has no arguments and returns a constant from the linked shim.
    let actual = unsafe { ffi::eiviz_decklink_abi_version() };
    if actual != SDK_ABI_VERSION {
        return Err(DeckLinkError::AbiMismatch {
            expected: SDK_ABI_VERSION,
            actual,
        });
    }
    Ok(())
}

fn check_native(result: i32, error: &[c_char]) -> Result<(), DeckLinkError> {
    if result == 0 {
        return Ok(());
    }
    // SAFETY: Every error buffer is initialized with zeros and remains NUL-terminated.
    let detail = unsafe { CStr::from_ptr(error.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    Err(DeckLinkError::Native(if detail.is_empty() {
        format!("native operation failed with status {result}")
    } else {
        detail
    }))
}

fn health_from_atomic(health: &AtomicU8) -> AdapterHealth {
    match health.load(Ordering::Acquire) {
        HEALTH_RUNNING => AdapterHealth::Running,
        HEALTH_DEGRADED => AdapterHealth::Degraded,
        HEALTH_UNAVAILABLE => AdapterHealth::Unavailable,
        _ => AdapterHealth::Failed,
    }
}

impl From<DeckLinkError> for MediaError {
    fn from(value: DeckLinkError) -> Self {
        MediaError::Other(value.to_string())
    }
}
