use crate::{
    NdiConfig, NdiOutputPixelFormat, frame_to_nv12, media_time_to_ndi_ticks,
    ndi_ticks_to_media_time, ndi_ticks_to_sample_index, push_latest, sample_index_to_ndi_ticks,
};
use crossbeam_channel::{Receiver as QueueReceiver, Sender as QueueSender, TrySendError, bounded};
use eiviz_core::InputId;
use eiviz_media::{
    AdapterHealth, AudioBuffer, BoundedMetadataQueue, Capability, InputTally, MediaError,
    MediaSink, MediaSource, PixelFormat as EivizPixelFormat, SourceControlDiagnostics,
    SourceMetadata, VideoFrame as EivizVideoFrame,
};
use eiviz_time::{
    ClockDomain, ClockObservation, ClockTimestamp, FrameRate, MediaTime, monotonic_nanos,
};
use grafton_ndi::{
    AudioFrame, Finder, FinderOptions, LineStrideOrSize, MetadataFrame, NDI, PixelFormat, Receiver,
    ReceiverColorFormat, ReceiverOptions, ScanType, Sender, SenderOptions, Source, Tally,
    VideoFrame,
};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const HEALTH_RUNNING: u8 = 0;
const HEALTH_DEGRADED: u8 = 1;
const HEALTH_UNAVAILABLE: u8 = 2;
const HEALTH_FAILED: u8 = 3;

#[derive(Debug, thiserror::Error)]
pub enum NdiError {
    #[error(transparent)]
    Sdk(#[from] grafton_ndi::Error),
    #[error("invalid NDI configuration: {0}")]
    InvalidConfiguration(String),
    #[error("invalid NDI frame: {0}")]
    InvalidFrame(String),
    #[error("failed to spawn NDI worker: {0}")]
    Spawn(#[from] std::io::Error),
}

#[derive(Clone, Debug)]
pub struct NdiSourceInfo {
    name: String,
    label: String,
    sdk_source: Source,
}

impl NdiSourceInfo {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

pub fn probe() -> Capability {
    match NDI::new() {
        Ok(_) => Capability {
            id: "ndi".into(),
            available: true,
            detail: "grafton-ndi 1.0.0 initialized the installed NDI runtime".into(),
        },
        Err(error) => Capability {
            id: "ndi".into(),
            available: false,
            detail: format!("NDI runtime initialization failed: {error}"),
        },
    }
}

pub fn discover_sources(timeout: Duration) -> Result<Vec<NdiSourceInfo>, NdiError> {
    let ndi = NDI::new()?;
    let options = FinderOptions::builder().show_local_sources(true).build();
    let finder = Finder::new(&ndi, &options)?;
    Ok(finder
        .find_sources(timeout)?
        .into_iter()
        .map(|source| NdiSourceInfo {
            name: source.name.clone(),
            label: source.to_string(),
            sdk_source: source,
        })
        .collect())
}

pub struct NdiSource {
    id: InputId,
    name: String,
    video_rx: QueueReceiver<EivizVideoFrame>,
    audio_rx: QueueReceiver<AudioBuffer>,
    metadata: Arc<BoundedMetadataQueue>,
    last_video: Mutex<Option<EivizVideoFrame>>,
    video_active: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    health: Arc<AtomicU8>,
    last_error: Arc<Mutex<Option<String>>>,
    dropped_video: Arc<AtomicU64>,
    dropped_audio: Arc<AtomicU64>,
    reconnects: Arc<AtomicU64>,
    discontinuities: Arc<AtomicU64>,
    metadata_received: Arc<AtomicU64>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl NdiSource {
    pub fn connect(
        id: InputId,
        source: &NdiSourceInfo,
        config: NdiConfig,
    ) -> Result<Self, NdiError> {
        validate_config(&config)?;
        let ndi = NDI::new()?;
        let options = ReceiverOptions::builder(source.sdk_source.clone())
            .color(ReceiverColorFormat::RGBX_RGBA)
            .allow_video_fields(false)
            .name("eiviz NDI input")
            .build();
        let receiver = Arc::new(Receiver::new(&ndi, &options)?);
        let (video_tx, video_rx) = bounded(config.video_queue_capacity);
        let video_drop_rx = video_rx.clone();
        let (audio_tx, audio_rx) = bounded(config.audio_queue_capacity);
        let audio_drop_rx = audio_rx.clone();
        let metadata = Arc::new(BoundedMetadataQueue::new(config.metadata_queue_capacity));
        let video_active = Arc::new(AtomicBool::new(false));
        let reconnect_pending = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let health = Arc::new(AtomicU8::new(HEALTH_DEGRADED));
        let last_error = Arc::new(Mutex::new(None));
        let dropped_video = Arc::new(AtomicU64::new(0));
        let dropped_audio = Arc::new(AtomicU64::new(0));
        let reconnects = Arc::new(AtomicU64::new(0));
        let discontinuities = Arc::new(AtomicU64::new(0));
        let metadata_received = Arc::new(AtomicU64::new(0));

        let video_worker = {
            let receiver = receiver.clone();
            let stop = stop.clone();
            let health = health.clone();
            let last_error = last_error.clone();
            let dropped = dropped_video.clone();
            let video_active = video_active.clone();
            let reconnect_pending = reconnect_pending.clone();
            let reconnects = reconnects.clone();
            let discontinuities = discontinuities.clone();
            let poll = config.capture_poll;
            thread::Builder::new()
                .name(format!("ndi-video-{}", id))
                .spawn(move || {
                    let mut sequence = 0_u64;
                    let mut previous_timestamp = None;
                    let mut received_frame = false;
                    while !stop.load(Ordering::Acquire) {
                        match receiver.video().try_capture(poll) {
                            Ok(Some(frame)) => {
                                let timestamp =
                                    frame_timestamp(frame.timestamp(), frame.timecode());
                                match receive_video(id, sequence, frame) {
                                    Ok(mut converted) => {
                                        let reconnected = received_frame
                                            && reconnect_pending.swap(false, Ordering::AcqRel);
                                        converted.discontinuity = reconnected
                                            || previous_timestamp
                                                .is_some_and(|old| timestamp <= old);
                                        if let Some(observation) =
                                            converted.clock_observation.as_mut()
                                        {
                                            observation.discontinuity = converted.discontinuity;
                                        }
                                        if reconnected {
                                            reconnects.fetch_add(1, Ordering::Relaxed);
                                        }
                                        if converted.discontinuity {
                                            discontinuities.fetch_add(1, Ordering::Relaxed);
                                        }
                                        previous_timestamp = Some(timestamp);
                                        received_frame = true;
                                        sequence = sequence.saturating_add(1);
                                        push_latest(&video_tx, &video_drop_rx, converted, &dropped);
                                        video_active.store(true, Ordering::Release);
                                        health.store(HEALTH_RUNNING, Ordering::Release);
                                        *last_error.lock() = None;
                                    }
                                    Err(error) => {
                                        video_active.store(false, Ordering::Release);
                                        health.store(HEALTH_DEGRADED, Ordering::Release);
                                        *last_error.lock() = Some(error.to_string());
                                    }
                                }
                            }
                            Ok(None) => {
                                if !receiver.is_connected() {
                                    video_active.store(false, Ordering::Release);
                                    if received_frame {
                                        reconnect_pending.store(true, Ordering::Release);
                                    }
                                    health.store(HEALTH_DEGRADED, Ordering::Release);
                                }
                            }
                            Err(error) => {
                                video_active.store(false, Ordering::Release);
                                if received_frame {
                                    reconnect_pending.store(true, Ordering::Release);
                                }
                                health.store(HEALTH_DEGRADED, Ordering::Release);
                                *last_error.lock() = Some(error.to_string());
                            }
                        }
                    }
                })?
        };

        let audio_worker_result = {
            let receiver = receiver.clone();
            let stop = stop.clone();
            let health = health.clone();
            let last_error = last_error.clone();
            let dropped = dropped_audio.clone();
            let discontinuities = discontinuities.clone();
            let poll = config.capture_poll;
            thread::Builder::new()
                .name(format!("ndi-audio-{}", id))
                .spawn(move || {
                    let mut previous_sample_index = None;
                    let mut disconnected_since_frame = false;
                    while !stop.load(Ordering::Acquire) {
                        match receiver.audio().try_capture(poll) {
                            Ok(Some(frame)) => match receive_audio(frame) {
                                Ok(mut converted) => {
                                    converted.discontinuity = disconnected_since_frame
                                        || previous_sample_index.is_some_and(|previous| {
                                            converted.sample_index <= previous
                                        });
                                    if converted.discontinuity {
                                        discontinuities.fetch_add(1, Ordering::Relaxed);
                                    }
                                    previous_sample_index = Some(converted.sample_index);
                                    disconnected_since_frame = false;
                                    push_latest(&audio_tx, &audio_drop_rx, converted, &dropped);
                                    health.store(HEALTH_RUNNING, Ordering::Release);
                                    *last_error.lock() = None;
                                }
                                Err(error) => {
                                    health.store(HEALTH_DEGRADED, Ordering::Release);
                                    *last_error.lock() = Some(error.to_string());
                                }
                            },
                            Ok(None) => {
                                if !receiver.is_connected() {
                                    if previous_sample_index.is_some() {
                                        disconnected_since_frame = true;
                                    }
                                    health.store(HEALTH_DEGRADED, Ordering::Release);
                                }
                            }
                            Err(error) => {
                                if previous_sample_index.is_some() {
                                    disconnected_since_frame = true;
                                }
                                health.store(HEALTH_DEGRADED, Ordering::Release);
                                *last_error.lock() = Some(error.to_string());
                            }
                        }
                    }
                })
        };
        let audio_worker = match audio_worker_result {
            Ok(worker) => worker,
            Err(error) => {
                stop.store(true, Ordering::Release);
                let _ = video_worker.join();
                return Err(NdiError::Spawn(error));
            }
        };

        let metadata_worker_result = {
            let receiver = receiver.clone();
            let stop = stop.clone();
            let health = health.clone();
            let last_error = last_error.clone();
            let metadata = metadata.clone();
            let metadata_received = metadata_received.clone();
            let poll = config.capture_poll;
            thread::Builder::new()
                .name(format!("ndi-metadata-{}", id))
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        match receiver.metadata().try_capture(poll) {
                            Ok(Some(frame)) => {
                                metadata_received.fetch_add(1, Ordering::Relaxed);
                                metadata.push(SourceMetadata {
                                    input: id,
                                    protocol: "ndi",
                                    timestamp: ndi_ticks_to_media_time(frame.timecode()),
                                    payload: std::sync::Arc::<str>::from(frame.data()),
                                    categories: vec!["opaque".into()],
                                });
                            }
                            Ok(None) => {}
                            Err(error) => {
                                health.store(HEALTH_DEGRADED, Ordering::Release);
                                *last_error.lock() = Some(error.to_string());
                            }
                        }
                    }
                })
        };
        let metadata_worker = match metadata_worker_result {
            Ok(worker) => worker,
            Err(error) => {
                stop.store(true, Ordering::Release);
                let _ = video_worker.join();
                let _ = audio_worker.join();
                return Err(NdiError::Spawn(error));
            }
        };

        Ok(Self {
            id,
            name: source.name.clone(),
            video_rx,
            audio_rx,
            metadata,
            last_video: Mutex::new(None),
            video_active,
            stop,
            health,
            last_error,
            dropped_video,
            dropped_audio,
            reconnects,
            discontinuities,
            metadata_received,
            workers: Mutex::new(vec![video_worker, audio_worker, metadata_worker]),
        })
    }

    pub fn source_name(&self) -> &str {
        &self.name
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

    pub fn tally_support(&self) -> &'static str {
        "unsupported: grafton-ndi 1.0.0 does not expose NDIlib_recv_set_tally"
    }
}

impl MediaSource for NdiSource {
    fn id(&self) -> InputId {
        self.id
    }

    fn pull_video(
        &self,
        _pts: MediaTime,
        _rate: FrameRate,
    ) -> eiviz_media::Result<Option<EivizVideoFrame>> {
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

    fn set_tally(&self, _tally: InputTally) -> eiviz_media::Result<()> {
        Err(MediaError::Unsupported(
            "grafton-ndi 1.0.0 does not expose receiver set_tally".into(),
        ))
    }

    fn poll_metadata(&self) -> eiviz_media::Result<Vec<SourceMetadata>> {
        Ok(self.metadata.drain())
    }

    fn control_diagnostics(&self) -> Option<SourceControlDiagnostics> {
        Some(SourceControlDiagnostics {
            reconnects: self.reconnects.load(Ordering::Relaxed),
            discontinuities: self.discontinuities.load(Ordering::Relaxed),
            metadata_received: self.metadata_received.load(Ordering::Relaxed),
            metadata_dropped: self.metadata.dropped(),
            tally_updates: 0,
        })
    }
}

impl Drop for NdiSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for worker in self.workers.get_mut().drain(..) {
            let _ = worker.join();
        }
    }
}

enum OutputFrame {
    Video(EivizVideoFrame),
    Audio(AudioBuffer),
    Metadata {
        timestamp: MediaTime,
        payload: String,
    },
}

pub struct NdiSink {
    name: String,
    tx: QueueSender<OutputFrame>,
    stop: Arc<AtomicBool>,
    health: Arc<AtomicU8>,
    last_error: Arc<Mutex<Option<String>>>,
    remote_tally: Arc<Mutex<Option<InputTally>>>,
    dropped: Arc<AtomicU64>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl NdiSink {
    pub fn create(
        name: impl Into<String>,
        frame_rate: FrameRate,
        config: NdiConfig,
    ) -> Result<Self, NdiError> {
        validate_config(&config)?;
        let name = name.into();
        if name.trim().is_empty() {
            return Err(NdiError::InvalidConfiguration(
                "output name must not be empty".into(),
            ));
        }
        let ndi = NDI::new()?;
        let options = SenderOptions::builder(name.clone())
            .clock_video(true)
            .clock_audio(true)
            .build();
        let sender = Sender::new(&ndi, &options)?;
        let (tx, rx) = bounded(config.output_queue_capacity);
        let stop = Arc::new(AtomicBool::new(false));
        let health = Arc::new(AtomicU8::new(HEALTH_RUNNING));
        let last_error = Arc::new(Mutex::new(None));
        let remote_tally = Arc::new(Mutex::new(None));
        let dropped = Arc::new(AtomicU64::new(0));
        let worker = {
            let stop = stop.clone();
            let health = health.clone();
            let last_error = last_error.clone();
            let remote_tally = remote_tally.clone();
            let output_pixel_format = config.output_pixel_format;
            let output_color_profile = config.output_color_profile;
            thread::Builder::new()
                .name(format!("ndi-output-{name}"))
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) || !rx.is_empty() {
                        let Ok(frame) = rx.recv_timeout(Duration::from_millis(10)) else {
                            update_sender_tally(&sender, &remote_tally, &last_error);
                            continue;
                        };
                        let result = match frame {
                            OutputFrame::Video(frame) => send_video(
                                &sender,
                                &frame,
                                frame_rate,
                                output_pixel_format,
                                output_color_profile,
                            ),
                            OutputFrame::Audio(audio) => send_audio(&sender, &audio),
                            OutputFrame::Metadata { timestamp, payload } => {
                                send_metadata(&sender, timestamp, &payload)
                            }
                        };
                        update_sender_tally(&sender, &remote_tally, &last_error);
                        match result {
                            Ok(()) => {
                                health.store(HEALTH_RUNNING, Ordering::Release);
                                *last_error.lock() = None;
                            }
                            Err(error) => {
                                health.store(HEALTH_DEGRADED, Ordering::Release);
                                *last_error.lock() = Some(error.to_string());
                            }
                        }
                    }
                })?
        };
        Ok(Self {
            name,
            tx,
            stop,
            health,
            last_error,
            remote_tally,
            dropped,
            worker: Mutex::new(Some(worker)),
        })
    }

    pub fn health(&self) -> AdapterHealth {
        health_from_atomic(&self.health)
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().clone()
    }

    pub fn dropped_frames(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn receiver_tally(&self) -> Option<InputTally> {
        *self.remote_tally.lock()
    }

    pub fn send_metadata(
        &self,
        timestamp: MediaTime,
        payload: impl Into<String>,
    ) -> eiviz_media::Result<()> {
        self.enqueue(OutputFrame::Metadata {
            timestamp,
            payload: payload.into(),
        })
    }

    fn enqueue(&self, frame: OutputFrame) -> eiviz_media::Result<()> {
        match self.tx.try_send(frame) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                self.health.store(HEALTH_DEGRADED, Ordering::Release);
                *self.last_error.lock() = Some("bounded output queue is full".into());
                Err(MediaError::QueueFull("ndi-output"))
            }
            Err(TrySendError::Disconnected(_)) => {
                self.health.store(HEALTH_FAILED, Ordering::Release);
                Err(MediaError::Disconnected("NDI output worker stopped".into()))
            }
        }
    }
}

impl MediaSink for NdiSink {
    fn name(&self) -> &str {
        &self.name
    }

    fn push_video(&self, frame: &EivizVideoFrame) -> eiviz_media::Result<()> {
        self.enqueue(OutputFrame::Video(frame.clone()))
    }

    fn push_audio(&self, audio: &AudioBuffer) -> eiviz_media::Result<()> {
        self.enqueue(OutputFrame::Audio(audio.clone()))
    }
}

impl Drop for NdiSink {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.get_mut().take() {
            let _ = worker.join();
        }
    }
}

fn frame_timestamp(timestamp: i64, timecode: i64) -> i64 {
    if timestamp > 0 {
        timestamp
    } else if timecode > 0 && timecode != i64::MAX {
        timecode
    } else {
        0
    }
}

fn receive_video(
    source: InputId,
    sequence: u64,
    frame: VideoFrame,
) -> Result<EivizVideoFrame, NdiError> {
    let width = u32::try_from(frame.width())
        .map_err(|_| NdiError::InvalidFrame("video width is negative".into()))?;
    let height = u32::try_from(frame.height())
        .map_err(|_| NdiError::InvalidFrame("video height is negative".into()))?;
    if width == 0 || height == 0 {
        return Err(NdiError::InvalidFrame(
            "video dimensions must be non-zero".into(),
        ));
    }
    let stride = match frame.line_stride_or_size() {
        LineStrideOrSize::LineStrideBytes(value) => usize::try_from(value)
            .map_err(|_| NdiError::InvalidFrame("video stride is negative".into()))?,
        LineStrideOrSize::DataSizeBytes(_) => {
            return Err(NdiError::InvalidFrame(
                "compressed NDI video is not accepted by the RGBA receiver".into(),
            ));
        }
    };
    let active = width as usize * 4;
    if stride < active || frame.data().len() < stride.saturating_mul(height as usize) {
        return Err(NdiError::InvalidFrame(
            "truncated or invalid packed video frame".into(),
        ));
    }
    let mut rgba = Vec::with_capacity(active * height as usize);
    for row in frame.data().chunks(stride).take(height as usize) {
        for pixel in row[..active].chunks_exact(4) {
            match frame.pixel_format() {
                PixelFormat::RGBA => rgba.extend_from_slice(pixel),
                PixelFormat::RGBX => rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]),
                PixelFormat::BGRA => {
                    rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                }
                PixelFormat::BGRX => {
                    rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
                }
                other => {
                    return Err(NdiError::InvalidFrame(format!(
                        "unexpected receiver pixel format {other:?}"
                    )));
                }
            }
        }
    }
    let timestamp = frame_timestamp(frame.timestamp(), frame.timecode());
    let pts = ndi_ticks_to_media_time(timestamp);
    let clock_observation = ClockObservation {
        source: ClockTimestamp::from_media(ClockDomain::SourceMedia, pts)
            .map_err(|error| NdiError::InvalidFrame(error.to_string()))?,
        target: ClockTimestamp::nanoseconds(ClockDomain::Monotonic, monotonic_nanos())
            .map_err(|error| NdiError::InvalidFrame(error.to_string()))?,
        discontinuity: false,
    };
    Ok(EivizVideoFrame {
        id: sequence,
        source: Some(source),
        pts,
        capture_domain: ClockDomain::SourceMedia,
        clock_observation: Some(clock_observation),
        width,
        height,
        format: EivizPixelFormat::Rgba8,
        data: rgba.into(),
        discontinuity: false,
    })
}

fn receive_audio(frame: AudioFrame) -> Result<AudioBuffer, NdiError> {
    let sample_rate = u32::try_from(frame.sample_rate())
        .map_err(|_| NdiError::InvalidFrame("audio sample rate is negative".into()))?;
    let channels = u16::try_from(frame.num_channels())
        .map_err(|_| NdiError::InvalidFrame("audio channel count is invalid".into()))?;
    let samples = usize::try_from(frame.num_samples())
        .map_err(|_| NdiError::InvalidFrame("audio sample count is negative".into()))?;
    if sample_rate == 0 || channels == 0 || samples == 0 {
        return Err(NdiError::InvalidFrame(
            "audio sample rate, channels, and sample count must be non-zero".into(),
        ));
    }
    let required = channels as usize * samples;
    if frame.data().len() != required {
        return Err(NdiError::InvalidFrame(format!(
            "audio has {} samples, expected {required}",
            frame.data().len()
        )));
    }
    let planes = frame
        .data()
        .chunks_exact(samples)
        .map(<[f32]>::to_vec)
        .collect();
    let timestamp = frame_timestamp(frame.timestamp(), frame.timecode());
    let sample_index = ndi_ticks_to_sample_index(timestamp, sample_rate);
    let capture_nanos = monotonic_nanos();
    Ok(AudioBuffer {
        sample_index,
        sample_rate,
        channels,
        planes,
        capture_timestamp: Some(eiviz_media::AudioCaptureTimestamp {
            device_sample_index: sample_index,
            callback_nanos: capture_nanos,
            capture_nanos,
        }),
        discontinuity: false,
    })
}

fn send_video(
    sender: &Sender,
    frame: &EivizVideoFrame,
    frame_rate: FrameRate,
    output_pixel_format: NdiOutputPixelFormat,
    output_color_profile: Option<crate::NdiColorProfile>,
) -> Result<(), NdiError> {
    let width = i32::try_from(frame.width)
        .map_err(|_| NdiError::InvalidFrame("video width exceeds NDI range".into()))?;
    let height = i32::try_from(frame.height)
        .map_err(|_| NdiError::InvalidFrame("video height exceeds NDI range".into()))?;
    let timestamp = media_time_to_ndi_ticks(frame.pts);
    let (pixel_format, data) = match output_pixel_format {
        NdiOutputPixelFormat::Rgba => (
            PixelFormat::RGBA,
            match frame.format {
                EivizPixelFormat::Rgba8 => frame.data.to_vec(),
                EivizPixelFormat::Bgra8 => frame
                    .data
                    .chunks_exact(4)
                    .flat_map(|pixel| [pixel[2], pixel[1], pixel[0], pixel[3]])
                    .collect(),
                EivizPixelFormat::Nv12 => {
                    return Err(NdiError::InvalidFrame(
                        "NV12 input cannot be implicitly interpreted as RGBA output".into(),
                    ));
                }
            },
        ),
        NdiOutputPixelFormat::Nv12 => {
            let profile = output_color_profile.ok_or_else(|| {
                NdiError::InvalidConfiguration(
                    "NV12 output requires an explicit color profile".into(),
                )
            })?;
            (
                PixelFormat::NV12,
                frame_to_nv12(frame, profile).map_err(NdiError::InvalidFrame)?,
            )
        }
    };
    let mut ndi_frame = VideoFrame::builder()
        .resolution(width, height)
        .pixel_format(pixel_format)
        .frame_rate(
            i32::try_from(frame_rate.numerator())
                .map_err(|_| NdiError::InvalidFrame("frame rate numerator exceeds i32".into()))?,
            i32::try_from(frame_rate.denominator())
                .map_err(|_| NdiError::InvalidFrame("frame rate denominator exceeds i32".into()))?,
        )
        .aspect_ratio(frame.width as f32 / frame.height.max(1) as f32)
        .scan_type(ScanType::Progressive)
        .timecode(timestamp)
        .timestamp(timestamp)
        .build()?;
    ndi_frame.replace_data(data)?;
    sender.send_video(&ndi_frame);
    Ok(())
}

fn send_audio(sender: &Sender, audio: &AudioBuffer) -> Result<(), NdiError> {
    if audio.sample_rate == 0 || audio.channels == 0 {
        return Err(NdiError::InvalidFrame(
            "audio sample rate and channels must be non-zero".into(),
        ));
    }
    let samples = audio.planes.first().map_or(0, Vec::len);
    if samples == 0
        || audio.planes.len() != audio.channels as usize
        || audio.planes.iter().any(|plane| plane.len() != samples)
    {
        return Err(NdiError::InvalidFrame(
            "audio planes do not match the declared layout".into(),
        ));
    }
    let data = audio
        .planes
        .iter()
        .flat_map(|plane| plane.iter().copied())
        .collect();
    let timestamp = sample_index_to_ndi_ticks(audio.sample_index, audio.sample_rate);
    let frame = AudioFrame::builder()
        .sample_rate(
            i32::try_from(audio.sample_rate)
                .map_err(|_| NdiError::InvalidFrame("audio sample rate exceeds i32".into()))?,
        )
        .channels(i32::from(audio.channels))
        .samples(
            i32::try_from(samples)
                .map_err(|_| NdiError::InvalidFrame("audio sample count exceeds i32".into()))?,
        )
        .timecode(timestamp)
        .timestamp(timestamp)
        .data(data)
        .build()?;
    sender.send_audio(&frame);
    Ok(())
}

fn send_metadata(sender: &Sender, timestamp: MediaTime, payload: &str) -> Result<(), NdiError> {
    let frame = MetadataFrame::with_data(payload, media_time_to_ndi_ticks(timestamp))?;
    sender.send_metadata(&frame)?;
    Ok(())
}

fn update_sender_tally(
    sender: &Sender,
    remote_tally: &Mutex<Option<InputTally>>,
    last_error: &Mutex<Option<String>>,
) {
    match sender.tally(Duration::ZERO) {
        Ok(Some(Tally {
            on_program,
            on_preview,
        })) => {
            *remote_tally.lock() = Some(InputTally {
                preview: on_preview,
                program: on_program,
            });
        }
        Ok(None) => {}
        Err(error) => {
            *last_error.lock() = Some(format!("NDI sender tally query failed: {error}"));
        }
    }
}

fn validate_config(config: &NdiConfig) -> Result<(), NdiError> {
    if let Some(error) = config.validation_error() {
        return Err(NdiError::InvalidConfiguration(error.into()));
    }
    Ok(())
}

fn health_from_atomic(health: &AtomicU8) -> AdapterHealth {
    match health.load(Ordering::Acquire) {
        HEALTH_RUNNING => AdapterHealth::Running,
        HEALTH_DEGRADED => AdapterHealth::Degraded,
        HEALTH_UNAVAILABLE => AdapterHealth::Unavailable,
        _ => AdapterHealth::Failed,
    }
}

impl From<NdiError> for MediaError {
    fn from(value: NdiError) -> Self {
        MediaError::Other(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_queue_capacity_is_rejected_before_runtime_initialization() {
        let config = NdiConfig {
            video_queue_capacity: 0,
            ..NdiConfig::default()
        };
        assert!(matches!(
            validate_config(&config),
            Err(NdiError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn timestamp_prefers_sdk_timestamp_and_ignores_synthesize_sentinel() {
        assert_eq!(frame_timestamp(20, 10), 20);
        assert_eq!(frame_timestamp(0, 10), 10);
        assert_eq!(frame_timestamp(0, i64::MAX), 0);
    }

    #[test]
    fn ndi_clock_is_one_hundred_nanoseconds() {
        assert_eq!(crate::NDI_TICKS_PER_SECOND, 10_000_000);
    }
}
