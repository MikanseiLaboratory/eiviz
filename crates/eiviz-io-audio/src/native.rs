use crate::{
    AudioBackend, AudioDeviceInfo, AudioError, AudioStreamConfig, DeviceDirection, Result,
    resolve_binding,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device, ErrorKind, FromSample, Host, Sample, SampleFormat, SizedSample,
    StreamConfig,
};
use eiviz_core::{DeviceBinding, InputId};
use eiviz_media::{
    AdapterHealth, AudioBuffer, AudioCaptureTimestamp, AudioIoDiagnostics, AudioSink, MediaError,
    MediaSource, VideoFrame,
};
use eiviz_time::{FrameRate, MediaTime};
use parking_lot::Mutex;
use rtrb::{Consumer, Producer, RingBuffer};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

const STAMP_CAPACITY: usize = 2_048;
const HEALTH_RUNNING: u8 = 0;
const HEALTH_DEGRADED: u8 = 1;
const HEALTH_FAILED: u8 = 2;

#[derive(Default)]
struct SharedDiagnostics {
    health: AtomicU8,
    callbacks: AtomicU64,
    device_frames: AtomicU64,
    xruns: AtomicU64,
    queue_overflows: AtomicU64,
    queue_underflows: AtomicU64,
    last_device_sample_index: AtomicU64,
    last_callback_nanos: AtomicU64,
    last_device_nanos: AtomicU64,
    last_error: AtomicU8,
}

impl SharedDiagnostics {
    fn stream_error(&self, error: cpal::Error) {
        let (code, health) = match error.kind() {
            ErrorKind::Xrun => {
                self.xruns.fetch_add(1, Ordering::Relaxed);
                (1, HEALTH_DEGRADED)
            }
            ErrorKind::RealtimeDenied => (2, HEALTH_DEGRADED),
            ErrorKind::DeviceChanged => (3, HEALTH_DEGRADED),
            ErrorKind::DeviceNotAvailable => (4, HEALTH_FAILED),
            ErrorKind::StreamInvalidated => (5, HEALTH_FAILED),
            ErrorKind::PermissionDenied => (6, HEALTH_FAILED),
            _ => (7, HEALTH_FAILED),
        };
        self.last_error.store(code, Ordering::Relaxed);
        self.health.store(health, Ordering::Release);
    }

    fn snapshot(&self, name: &str) -> AudioIoDiagnostics {
        let health = match self.health.load(Ordering::Acquire) {
            HEALTH_RUNNING => AdapterHealth::Running,
            HEALTH_DEGRADED => AdapterHealth::Degraded,
            _ => AdapterHealth::Failed,
        };
        let last_error = match self.last_error.load(Ordering::Relaxed) {
            0 => None,
            1 => Some("backend xrun".into()),
            2 => Some("realtime scheduling denied".into()),
            3 => Some("device route changed".into()),
            4 => Some("device unavailable".into()),
            5 => Some("stream invalidated".into()),
            6 => Some("permission denied".into()),
            _ => Some("backend stream error".into()),
        };
        AudioIoDiagnostics {
            name: name.into(),
            health,
            callbacks: self.callbacks.load(Ordering::Relaxed),
            device_frames: self.device_frames.load(Ordering::Relaxed),
            xruns: self.xruns.load(Ordering::Relaxed),
            queue_overflows: self.queue_overflows.load(Ordering::Relaxed),
            queue_underflows: self.queue_underflows.load(Ordering::Relaxed),
            last_device_sample_index: self.last_device_sample_index.load(Ordering::Relaxed),
            last_callback_nanos: self.last_callback_nanos.load(Ordering::Relaxed),
            last_device_nanos: self.last_device_nanos.load(Ordering::Relaxed),
            last_error,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct CaptureStamp {
    first_sample_index: u64,
    frames: usize,
    callback_nanos: u64,
    capture_nanos: u64,
}

struct CaptureConsumer {
    samples: Consumer<f32>,
    stamps: Consumer<CaptureStamp>,
    current_stamp: Option<CaptureStamp>,
    stamp_offset: usize,
    expected_device_sample: Option<u64>,
}

impl CaptureConsumer {
    fn pull(
        &mut self,
        project_sample_index: u64,
        frames: usize,
        sample_rate: u32,
        channels: u16,
    ) -> Option<AudioBuffer> {
        let sample_count = frames.checked_mul(channels as usize)?;
        if self.samples.slots() < sample_count {
            return None;
        }
        if self.current_stamp.is_none() {
            self.current_stamp = self.stamps.pop().ok();
            self.stamp_offset = 0;
        }
        let first_stamp = self.current_stamp?;
        let first_device_sample = first_stamp
            .first_sample_index
            .saturating_add(self.stamp_offset as u64);
        let mut discontinuity = self
            .expected_device_sample
            .is_some_and(|expected| expected != first_device_sample);
        let mut buffer = AudioBuffer::silence(project_sample_index, sample_rate, channels, frames);
        buffer.capture_timestamp = Some(AudioCaptureTimestamp {
            device_sample_index: first_device_sample,
            callback_nanos: first_stamp.callback_nanos,
            capture_nanos: first_stamp.capture_nanos,
        });
        for frame in 0..frames {
            if self
                .current_stamp
                .is_some_and(|stamp| self.stamp_offset == stamp.frames)
            {
                let prior_end = self
                    .current_stamp
                    .map(|stamp| stamp.first_sample_index.saturating_add(stamp.frames as u64));
                self.current_stamp = self.stamps.pop().ok();
                self.stamp_offset = 0;
                if let (Some(expected), Some(next)) = (prior_end, self.current_stamp) {
                    discontinuity |= expected != next.first_sample_index;
                }
            }
            for channel in 0..channels as usize {
                buffer.planes[channel][frame] = self.samples.pop().ok()?;
            }
            self.stamp_offset += 1;
        }
        self.expected_device_sample = Some(first_device_sample.saturating_add(frames as u64));
        buffer.discontinuity = discontinuity;
        Some(buffer)
    }
}

pub struct CpalInput {
    id: InputId,
    name: String,
    config: AudioStreamConfig,
    consumer: Mutex<CaptureConsumer>,
    diagnostics: Arc<SharedDiagnostics>,
    _stream: Mutex<cpal::Stream>,
}

impl CpalInput {
    pub fn open(
        id: InputId,
        binding: &DeviceBinding,
        backend: AudioBackend,
        config: AudioStreamConfig,
    ) -> Result<Self> {
        validate_config(config)?;
        let (host, device, info) = resolve_cpal_device(binding, backend, DeviceDirection::Input)?;
        let _ = host;
        let selected = select_config(&device, DeviceDirection::Input, config)?;
        let sample_capacity = config
            .ring_frames
            .checked_mul(config.channels as usize)
            .ok_or_else(|| AudioError::InvalidBuffer("ring capacity overflow".into()))?;
        let (sample_producer, sample_consumer) = RingBuffer::new(sample_capacity);
        let (stamp_producer, stamp_consumer) = RingBuffer::new(STAMP_CAPACITY);
        let diagnostics = Arc::new(SharedDiagnostics::default());
        let stream = build_input_stream(
            &device,
            selected,
            sample_producer,
            stamp_producer,
            diagnostics.clone(),
        )?;
        stream.play().map_err(backend_error)?;
        Ok(Self {
            id,
            name: info.display_name,
            config,
            consumer: Mutex::new(CaptureConsumer {
                samples: sample_consumer,
                stamps: stamp_consumer,
                current_stamp: None,
                stamp_offset: 0,
                expected_device_sample: None,
            }),
            diagnostics,
            _stream: Mutex::new(stream),
        })
    }

    pub fn diagnostics(&self) -> AudioIoDiagnostics {
        self.diagnostics.snapshot(&self.name)
    }
}

impl MediaSource for CpalInput {
    fn id(&self) -> InputId {
        self.id
    }

    fn pull_video(
        &self,
        _pts: MediaTime,
        _rate: FrameRate,
    ) -> eiviz_media::Result<Option<VideoFrame>> {
        Ok(None)
    }

    fn pull_audio(
        &self,
        sample_index: u64,
        frames: usize,
    ) -> eiviz_media::Result<Option<AudioBuffer>> {
        let buffer = self.consumer.lock().pull(
            sample_index,
            frames,
            self.config.sample_rate,
            self.config.channels,
        );
        if buffer.is_none() {
            self.diagnostics
                .queue_underflows
                .fetch_add(1, Ordering::Relaxed);
        }
        Ok(buffer)
    }

    fn audio_diagnostics(&self) -> Option<AudioIoDiagnostics> {
        Some(self.diagnostics())
    }
}

pub struct CpalOutput {
    name: String,
    config: AudioStreamConfig,
    producer: Mutex<Producer<f32>>,
    diagnostics: Arc<SharedDiagnostics>,
    _stream: Mutex<cpal::Stream>,
}

impl CpalOutput {
    pub fn open(
        name: impl Into<String>,
        binding: &DeviceBinding,
        backend: AudioBackend,
        config: AudioStreamConfig,
    ) -> Result<Self> {
        validate_config(config)?;
        let (host, device, _) = resolve_cpal_device(binding, backend, DeviceDirection::Output)?;
        let _ = host;
        let selected = select_config(&device, DeviceDirection::Output, config)?;
        let sample_capacity = config
            .ring_frames
            .checked_mul(config.channels as usize)
            .ok_or_else(|| AudioError::InvalidBuffer("ring capacity overflow".into()))?;
        let (producer, consumer) = RingBuffer::new(sample_capacity);
        let diagnostics = Arc::new(SharedDiagnostics::default());
        let stream = build_output_stream(&device, selected, consumer, diagnostics.clone())?;
        stream.play().map_err(backend_error)?;
        Ok(Self {
            name: name.into(),
            config,
            producer: Mutex::new(producer),
            diagnostics,
            _stream: Mutex::new(stream),
        })
    }

    pub fn diagnostics(&self) -> AudioIoDiagnostics {
        self.diagnostics.snapshot(&self.name)
    }
}

impl AudioSink for CpalOutput {
    fn name(&self) -> &str {
        &self.name
    }

    fn push_audio(&self, audio: &AudioBuffer) -> eiviz_media::Result<()> {
        if audio.sample_rate != self.config.sample_rate || audio.channels != self.config.channels {
            return Err(MediaError::Unsupported(format!(
                "audio output requires {} Hz/{} channels, got {} Hz/{} channels; no implicit ASRC",
                self.config.sample_rate, self.config.channels, audio.sample_rate, audio.channels
            )));
        }
        let frames = audio.planes.first().map_or(0, Vec::len);
        if audio
            .planes
            .iter()
            .take(audio.channels as usize)
            .any(|plane| plane.len() != frames)
        {
            return Err(MediaError::Other(
                "inconsistent planar audio lengths".into(),
            ));
        }
        let needed = frames.saturating_mul(audio.channels as usize);
        let mut producer = self.producer.lock();
        if producer.slots() < needed {
            self.diagnostics
                .queue_overflows
                .fetch_add(1, Ordering::Relaxed);
            self.diagnostics.xruns.fetch_add(1, Ordering::Relaxed);
            self.diagnostics
                .health
                .store(HEALTH_DEGRADED, Ordering::Release);
            return Err(MediaError::QueueFull("cpal-output"));
        }
        for frame in 0..frames {
            for channel in 0..audio.channels as usize {
                producer
                    .push(audio.planes[channel][frame])
                    .expect("capacity checked before bounded write");
            }
        }
        Ok(())
    }

    fn diagnostics(&self) -> AudioIoDiagnostics {
        self.diagnostics()
    }
}

pub fn enumerate_devices(backend: AudioBackend) -> Result<Vec<AudioDeviceInfo>> {
    let host = explicit_host(backend)?;
    let default_input = host
        .default_input_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
    let default_output = host
        .default_output_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
    let devices = host.devices().map_err(backend_error)?;
    let mut result = Vec::new();
    for device in devices {
        let persistent_id = device.id().map_err(backend_error)?.to_string();
        let supports_input = device
            .supported_input_configs()
            .is_ok_and(|mut configs| configs.next().is_some());
        let supports_output = device
            .supported_output_configs()
            .is_ok_and(|mut configs| configs.next().is_some());
        if !supports_input && !supports_output {
            continue;
        }
        result.push(AudioDeviceInfo {
            backend,
            persistent_id: persistent_id.clone(),
            display_name: device.to_string(),
            supports_input,
            supports_output,
            default_input: default_input.as_deref() == Some(&persistent_id),
            default_output: default_output.as_deref() == Some(&persistent_id),
        });
    }
    Ok(result)
}

fn validate_config(config: AudioStreamConfig) -> Result<()> {
    if config.sample_rate == 0 || config.channels == 0 || config.ring_frames == 0 {
        return Err(AudioError::InvalidBuffer(
            "sample rate, channels, and ring frames must be non-zero".into(),
        ));
    }
    Ok(())
}

fn explicit_host(backend: AudioBackend) -> Result<Host> {
    if !AudioBackend::compiled().contains(&backend) {
        return Err(AudioError::BackendNotCompiled(backend.id().into()));
    }
    let host_id =
        cpal::HostId::from_str(backend.id()).map_err(|error| AudioError::BackendUnavailable {
            backend,
            detail: error.to_string(),
        })?;
    if !cpal::available_hosts().contains(&host_id) {
        return Err(AudioError::BackendUnavailable {
            backend,
            detail: "host is not available at runtime".into(),
        });
    }
    cpal::host_from_id(host_id).map_err(|error| AudioError::BackendUnavailable {
        backend,
        detail: error.to_string(),
    })
}

fn resolve_cpal_device(
    binding: &DeviceBinding,
    backend: AudioBackend,
    direction: DeviceDirection,
) -> Result<(Host, Device, AudioDeviceInfo)> {
    let devices = enumerate_devices(backend)?;
    let resolved = resolve_binding(binding, backend, direction, &devices)?;
    let host = explicit_host(backend)?;
    let id = cpal::DeviceId::from_str(&resolved.device.persistent_id).map_err(backend_error)?;
    let device = host
        .device_by_id(&id)
        .ok_or_else(|| AudioError::DeviceNotFound(resolved.device.persistent_id.clone()))?;
    Ok((host, device, resolved.device))
}

fn select_config(
    device: &Device,
    direction: DeviceDirection,
    requested: AudioStreamConfig,
) -> Result<(StreamConfig, SampleFormat)> {
    let ranges = match direction {
        DeviceDirection::Input => device
            .supported_input_configs()
            .map_err(backend_error)?
            .collect::<Vec<_>>(),
        DeviceDirection::Output => device
            .supported_output_configs()
            .map_err(backend_error)?
            .collect::<Vec<_>>(),
    };
    let mut supported = ranges
        .into_iter()
        .filter(|range| {
            range.channels() == requested.channels
                && range.contains_rate(requested.sample_rate)
                && sample_priority(range.sample_format()).is_some()
        })
        .collect::<Vec<_>>();
    supported.sort_by_key(|range| sample_priority(range.sample_format()));
    let selected = supported
        .first()
        .copied()
        .ok_or_else(|| AudioError::UnsupportedFormat {
            device: device.to_string(),
            sample_rate: requested.sample_rate,
            channels: requested.channels,
        })?;
    let supported_config = selected.with_sample_rate(requested.sample_rate);
    let mut stream_config = supported_config.config();
    stream_config.buffer_size = requested
        .buffer_frames
        .map_or(BufferSize::Default, BufferSize::Fixed);
    Ok((stream_config, supported_config.sample_format()))
}

fn sample_priority(format: SampleFormat) -> Option<u8> {
    match format {
        SampleFormat::F32 => Some(0),
        SampleFormat::I32 => Some(1),
        SampleFormat::I24 => Some(2),
        SampleFormat::I16 => Some(3),
        SampleFormat::F64 => Some(4),
        SampleFormat::I8 => Some(5),
        SampleFormat::I64 => Some(6),
        SampleFormat::U8 => Some(7),
        SampleFormat::U16 => Some(8),
        SampleFormat::U24 => Some(9),
        SampleFormat::U32 => Some(10),
        SampleFormat::U64 => Some(11),
        _ => None,
    }
}

fn build_input_stream(
    device: &Device,
    selected: (StreamConfig, SampleFormat),
    samples: Producer<f32>,
    stamps: Producer<CaptureStamp>,
    diagnostics: Arc<SharedDiagnostics>,
) -> Result<cpal::Stream> {
    let (config, format) = selected;
    match format {
        SampleFormat::I8 => build_input::<i8>(device, config, samples, stamps, diagnostics),
        SampleFormat::I16 => build_input::<i16>(device, config, samples, stamps, diagnostics),
        SampleFormat::I24 => build_input::<cpal::I24>(device, config, samples, stamps, diagnostics),
        SampleFormat::I32 => build_input::<i32>(device, config, samples, stamps, diagnostics),
        SampleFormat::I64 => build_input::<i64>(device, config, samples, stamps, diagnostics),
        SampleFormat::U8 => build_input::<u8>(device, config, samples, stamps, diagnostics),
        SampleFormat::U16 => build_input::<u16>(device, config, samples, stamps, diagnostics),
        SampleFormat::U24 => build_input::<cpal::U24>(device, config, samples, stamps, diagnostics),
        SampleFormat::U32 => build_input::<u32>(device, config, samples, stamps, diagnostics),
        SampleFormat::U64 => build_input::<u64>(device, config, samples, stamps, diagnostics),
        SampleFormat::F32 => build_input::<f32>(device, config, samples, stamps, diagnostics),
        SampleFormat::F64 => build_input::<f64>(device, config, samples, stamps, diagnostics),
        _ => Err(AudioError::UnsupportedFormat {
            device: device.to_string(),
            sample_rate: config.sample_rate,
            channels: config.channels,
        }),
    }
}

fn build_input<T>(
    device: &Device,
    config: StreamConfig,
    mut samples: Producer<f32>,
    mut stamps: Producer<CaptureStamp>,
    diagnostics: Arc<SharedDiagnostics>,
) -> Result<cpal::Stream>
where
    T: SizedSample + Copy + Send + 'static,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let data_diagnostics = diagnostics.clone();
    let error_diagnostics = diagnostics.clone();
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data, info| {
                let frames = data.len() / channels;
                let first_sample_index = data_diagnostics
                    .device_frames
                    .fetch_add(frames as u64, Ordering::Relaxed);
                data_diagnostics.callbacks.fetch_add(1, Ordering::Relaxed);
                data_diagnostics
                    .last_device_sample_index
                    .store(first_sample_index, Ordering::Relaxed);
                let timestamp = info.timestamp();
                let callback_nanos = instant_nanos(timestamp.callback);
                let capture_nanos = instant_nanos(timestamp.capture);
                data_diagnostics
                    .last_callback_nanos
                    .store(callback_nanos, Ordering::Relaxed);
                data_diagnostics
                    .last_device_nanos
                    .store(capture_nanos, Ordering::Relaxed);
                if samples.slots() < data.len() || stamps.slots() == 0 {
                    data_diagnostics
                        .queue_overflows
                        .fetch_add(1, Ordering::Relaxed);
                    data_diagnostics.xruns.fetch_add(1, Ordering::Relaxed);
                    data_diagnostics
                        .health
                        .store(HEALTH_DEGRADED, Ordering::Release);
                    return;
                }
                stamps
                    .push(CaptureStamp {
                        first_sample_index,
                        frames,
                        callback_nanos,
                        capture_nanos,
                    })
                    .expect("capacity checked before bounded write");
                enqueue_input_samples(data, &mut samples);
            },
            move |error| error_diagnostics.stream_error(error),
            None,
        )
        .map_err(backend_error)
}

fn build_output_stream(
    device: &Device,
    selected: (StreamConfig, SampleFormat),
    samples: Consumer<f32>,
    diagnostics: Arc<SharedDiagnostics>,
) -> Result<cpal::Stream> {
    let (config, format) = selected;
    match format {
        SampleFormat::I8 => build_output::<i8>(device, config, samples, diagnostics),
        SampleFormat::I16 => build_output::<i16>(device, config, samples, diagnostics),
        SampleFormat::I24 => build_output::<cpal::I24>(device, config, samples, diagnostics),
        SampleFormat::I32 => build_output::<i32>(device, config, samples, diagnostics),
        SampleFormat::I64 => build_output::<i64>(device, config, samples, diagnostics),
        SampleFormat::U8 => build_output::<u8>(device, config, samples, diagnostics),
        SampleFormat::U16 => build_output::<u16>(device, config, samples, diagnostics),
        SampleFormat::U24 => build_output::<cpal::U24>(device, config, samples, diagnostics),
        SampleFormat::U32 => build_output::<u32>(device, config, samples, diagnostics),
        SampleFormat::U64 => build_output::<u64>(device, config, samples, diagnostics),
        SampleFormat::F32 => build_output::<f32>(device, config, samples, diagnostics),
        SampleFormat::F64 => build_output::<f64>(device, config, samples, diagnostics),
        _ => Err(AudioError::UnsupportedFormat {
            device: device.to_string(),
            sample_rate: config.sample_rate,
            channels: config.channels,
        }),
    }
}

fn build_output<T>(
    device: &Device,
    config: StreamConfig,
    mut samples: Consumer<f32>,
    diagnostics: Arc<SharedDiagnostics>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let channels = config.channels as usize;
    let data_diagnostics = diagnostics.clone();
    let error_diagnostics = diagnostics.clone();
    device
        .build_output_stream::<T, _, _>(
            config,
            move |data, info| {
                let frames = data.len() / channels;
                let first_sample_index = data_diagnostics
                    .device_frames
                    .fetch_add(frames as u64, Ordering::Relaxed);
                data_diagnostics.callbacks.fetch_add(1, Ordering::Relaxed);
                data_diagnostics
                    .last_device_sample_index
                    .store(first_sample_index, Ordering::Relaxed);
                let timestamp = info.timestamp();
                let callback_nanos = instant_nanos(timestamp.callback);
                let playback_nanos = instant_nanos(timestamp.playback);
                data_diagnostics
                    .last_callback_nanos
                    .store(callback_nanos, Ordering::Relaxed);
                data_diagnostics
                    .last_device_nanos
                    .store(playback_nanos, Ordering::Relaxed);
                let mut underflow = false;
                dequeue_output_samples(data, &mut samples, &mut underflow);
                if underflow {
                    data_diagnostics
                        .queue_underflows
                        .fetch_add(1, Ordering::Relaxed);
                    data_diagnostics.xruns.fetch_add(1, Ordering::Relaxed);
                    data_diagnostics
                        .health
                        .store(HEALTH_DEGRADED, Ordering::Release);
                }
            },
            move |error| error_diagnostics.stream_error(error),
            None,
        )
        .map_err(backend_error)
}

fn instant_nanos(instant: cpal::StreamInstant) -> u64 {
    u64::try_from(instant.as_nanos()).unwrap_or(u64::MAX)
}

fn enqueue_input_samples<T>(data: &[T], samples: &mut Producer<f32>)
where
    T: Copy,
    f32: FromSample<T>,
{
    for sample in data {
        samples
            .push(f32::from_sample(*sample))
            .expect("capacity checked before bounded write");
    }
}

fn dequeue_output_samples<T>(data: &mut [T], samples: &mut Consumer<f32>, underflow: &mut bool)
where
    T: Sample + FromSample<f32>,
{
    for sample in data {
        match samples.pop() {
            Ok(value) => *sample = T::from_sample(value),
            Err(_) => {
                *sample = T::EQUILIBRIUM;
                *underflow = true;
            }
        }
    }
}

fn backend_error(error: cpal::Error) -> AudioError {
    AudioError::Backend(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_sample_conversion_uses_bounded_rings() {
        let (mut input_producer, mut input_consumer) = RingBuffer::new(3);
        enqueue_input_samples(&[i16::MIN, 0, i16::MAX], &mut input_producer);
        let captured = [
            input_consumer.pop().unwrap(),
            input_consumer.pop().unwrap(),
            input_consumer.pop().unwrap(),
        ];
        assert_eq!(captured[0], -1.0);
        assert_eq!(captured[1], 0.0);
        assert!(captured[2] > 0.999);

        let (mut output_producer, mut output_consumer) = RingBuffer::new(3);
        for sample in [-1.0, 0.0, 1.0] {
            output_producer.push(sample).unwrap();
        }
        let mut rendered = [0i16; 3];
        let mut underflow = false;
        dequeue_output_samples(&mut rendered, &mut output_consumer, &mut underflow);
        assert_eq!(rendered, [i16::MIN, 0, i16::MAX]);
        assert!(!underflow);

        let mut silence = [1i16; 1];
        dequeue_output_samples(&mut silence, &mut output_consumer, &mut underflow);
        assert_eq!(silence, [0]);
        assert!(underflow);
    }
}
