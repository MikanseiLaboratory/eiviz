use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceId, HostId, SampleFormat, Stream, StreamConfig, SupportedStreamConfig};

use crate::upload::AudioInputStore;

use super::capture::{AudioCaptureSpec, send_ready};
use super::graph::BusRing;
use super::info::{CAPTURE_MODE_ENDPOINT_LOOPBACK, CAPTURE_MODE_MIC};
use super::pcm::{f32_to_i16, f32_to_i32, interleaved_f32_packet, mix_mapped_f32};
use super::pop_stereo_rate;

pub fn run_capture(
    spec: &AudioCaptureSpec,
    uploads: &Arc<Mutex<AudioInputStore>>,
    stop: &AtomicBool,
    ready: Option<&mpsc::Sender<Result<(), String>>>,
    signaled: &AtomicBool,
) -> Result<(), String> {
    if spec.mode != CAPTURE_MODE_MIC && spec.mode != CAPTURE_MODE_ENDPOINT_LOOPBACK {
        return Err(format!("unknown capture mode {}", spec.mode));
    }
    let loopback = spec.mode == CAPTURE_MODE_ENDPOINT_LOOPBACK;
    let follow_default = spec.device_id.is_empty();
    let host = platform_host()?;

    let mut started = false;
    let mut last_default = String::new();
    while !stop.load(Ordering::Relaxed) {
        if follow_default {
            last_default = default_id(&host, loopback);
        }
        let failed = Arc::new(Mutex::new(None::<String>));
        match open_input(spec, &host, loopback, uploads, &failed) {
            Ok(_stream) => {
                send_ready(ready, signaled, Ok(()))?;
                started = true;
                wait_stream(
                    stop,
                    follow_default,
                    loopback,
                    &host,
                    &last_default,
                    &failed,
                );
            }
            Err(error) => {
                if !started {
                    return Err(error);
                }
                crate::diag::error(&format!("audio capture {}: {error}", spec.id));
                thread::sleep(Duration::from_millis(250));
            }
        }
    }
    Ok(())
}

pub fn run_output(
    device_id: &str,
    maps: &[(Arc<BusRing>, i32, i32)],
    stop: &AtomicBool,
) -> Result<(), String> {
    let follow_default = device_id.is_empty();
    let host = platform_host()?;
    let maps = maps.to_vec();

    let mut started = false;
    let mut last_default = String::new();
    while !stop.load(Ordering::Relaxed) {
        if follow_default {
            last_default = default_id(&host, true);
        }
        let failed = Arc::new(Mutex::new(None::<String>));
        match open_output(device_id, &host, &maps, &failed) {
            Ok(_stream) => {
                started = true;
                wait_stream(stop, follow_default, true, &host, &last_default, &failed);
            }
            Err(error) => {
                if !started {
                    return Err(error);
                }
                crate::diag::error(&format!("audio output: {error}"));
                thread::sleep(Duration::from_millis(250));
            }
        }
    }
    Ok(())
}

// cpal WASAPI initialises COM as STA on the stream thread. Do not
// CoInitializeEx(MTA) here: RPC_E_CHANGED_MODE leaves capture events silent
// until another WASAPI client (for example output loopback) starts the engine.
fn platform_host() -> Result<cpal::Host, String> {
    cpal::host_from_id(platform_host_id()).map_err(|error| format!("cpal host: {error}"))
}

fn platform_host_id() -> HostId {
    #[cfg(windows)]
    {
        HostId::Wasapi
    }
    #[cfg(target_os = "macos")]
    {
        HostId::CoreAudio
    }
}

fn wait_stream(
    stop: &AtomicBool,
    follow_default: bool,
    output: bool,
    host: &cpal::Host,
    last_default: &str,
    failed: &Mutex<Option<String>>,
) {
    let mut follow_check = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        if failed.lock().expect("cpal err").is_some() {
            break;
        }
        if follow_default && follow_check.elapsed() >= Duration::from_millis(250) {
            follow_check = Instant::now();
            let now = default_id(host, output);
            if !now.is_empty() && now != last_default {
                break;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn default_id(host: &cpal::Host, output: bool) -> String {
    let device = if output {
        host.default_output_device()
    } else {
        host.default_input_device()
    };
    device
        .and_then(|device| device.id().ok())
        .map(|id| id.id().to_string())
        .unwrap_or_default()
}

fn resolve_device(
    host: &cpal::Host,
    output: bool,
    device_id: &str,
) -> Result<cpal::Device, String> {
    if device_id.is_empty() {
        return if output {
            host.default_output_device()
                .ok_or_else(|| "cpal: no default output device".into())
        } else {
            host.default_input_device()
                .ok_or_else(|| "cpal: no default input device".into())
        };
    }
    let want = DeviceId::new(platform_host_id(), device_id);
    host.device_by_id(&want).ok_or_else(|| {
        format!(
            "cpal: no {} device with id {device_id}",
            if output { "output" } else { "input" }
        )
    })
}

fn stream_config(device: &cpal::Device, output: bool) -> Result<SupportedStreamConfig, String> {
    if output {
        device
            .default_output_config()
            .map_err(|error| format!("cpal default output config: {error}"))
    } else {
        device
            .default_input_config()
            .map_err(|error| format!("cpal default input config: {error}"))
    }
}

fn open_input(
    spec: &AudioCaptureSpec,
    host: &cpal::Host,
    loopback: bool,
    uploads: &Arc<Mutex<AudioInputStore>>,
    failed: &Arc<Mutex<Option<String>>>,
) -> Result<Stream, String> {
    let device = resolve_device(host, loopback, &spec.device_id)?;
    let supported = stream_config(&device, loopback)?;
    let config: StreamConfig = supported.config();
    let channels = config.channels.max(1) as usize;
    let rate = config.sample_rate.max(1);
    let map_left = spec.map_left.max(0) as usize;
    let map_right = spec.map_right.max(0) as usize;
    let id = spec.id;
    let uploads = Arc::clone(uploads);
    let pts = Arc::new(AtomicI64::new(0));
    let err_flag = Arc::clone(failed);
    let err_cb = move |error| {
        *err_flag.lock().expect("cpal err") = Some(format!("cpal stream: {error}"));
    };

    let stream = match supported.sample_format() {
        SampleFormat::F32 => {
            let pts = Arc::clone(&pts);
            let uploads = Arc::clone(&uploads);
            device.build_input_stream(
                config,
                move |data: &[f32], _| {
                    ingest(
                        id, &uploads, &pts, data, channels, rate, map_left, map_right,
                    );
                },
                err_cb,
                None,
            )
        }
        SampleFormat::I16 => {
            let pts = Arc::clone(&pts);
            let uploads = Arc::clone(&uploads);
            device.build_input_stream(
                config,
                move |data: &[i16], _| {
                    let converted: Vec<f32> =
                        data.iter().map(|sample| *sample as f32 / 32768.0).collect();
                    ingest(
                        id, &uploads, &pts, &converted, channels, rate, map_left, map_right,
                    );
                },
                err_cb,
                None,
            )
        }
        SampleFormat::I32 => {
            let pts = Arc::clone(&pts);
            let uploads = Arc::clone(&uploads);
            device.build_input_stream(
                config,
                move |data: &[i32], _| {
                    let converted: Vec<f32> = data
                        .iter()
                        .map(|sample| *sample as f32 / 2_147_483_648.0)
                        .collect();
                    ingest(
                        id, &uploads, &pts, &converted, channels, rate, map_left, map_right,
                    );
                },
                err_cb,
                None,
            )
        }
        other => {
            return Err(format!("cpal sample format {other:?} is not supported"));
        }
    }
    .map_err(|error| format!("cpal input stream: {error}"))?;
    stream
        .play()
        .map_err(|error| format!("cpal play: {error}"))?;
    Ok(stream)
}

fn open_output(
    device_id: &str,
    host: &cpal::Host,
    maps: &[(Arc<BusRing>, i32, i32)],
    failed: &Arc<Mutex<Option<String>>>,
) -> Result<Stream, String> {
    let device = resolve_device(host, true, device_id)?;
    let supported = stream_config(&device, true)?;
    let config: StreamConfig = supported.config();
    let channels = config.channels.max(1) as usize;
    let rate = config.sample_rate.max(1);
    let maps = maps.to_vec();
    let err_flag = Arc::clone(failed);
    let err_cb = move |error| {
        *err_flag.lock().expect("cpal err") = Some(format!("cpal stream: {error}"));
    };

    let stream = match supported.sample_format() {
        SampleFormat::F32 => device.build_output_stream(
            config,
            {
                let maps = maps.clone();
                move |data: &mut [f32], _| {
                    let frames = data.len() / channels;
                    let mapped = pop_stereo_rate(&maps, frames, rate);
                    mix_mapped_f32(data, channels, &mapped);
                }
            },
            err_cb,
            None,
        ),
        SampleFormat::I16 => device.build_output_stream(
            config,
            {
                let maps = maps.clone();
                let mut scratch = Vec::new();
                move |data: &mut [i16], _| {
                    scratch.resize(data.len(), 0.0);
                    let frames = data.len() / channels;
                    let mapped = pop_stereo_rate(&maps, frames, rate);
                    mix_mapped_f32(&mut scratch, channels, &mapped);
                    f32_to_i16(&scratch, data);
                }
            },
            err_cb,
            None,
        ),
        SampleFormat::I32 => device.build_output_stream(
            config,
            {
                let maps = maps;
                let mut scratch = Vec::new();
                move |data: &mut [i32], _| {
                    scratch.resize(data.len(), 0.0);
                    let frames = data.len() / channels;
                    let mapped = pop_stereo_rate(&maps, frames, rate);
                    mix_mapped_f32(&mut scratch, channels, &mapped);
                    f32_to_i32(&scratch, data);
                }
            },
            err_cb,
            None,
        ),
        other => {
            return Err(format!("cpal sample format {other:?} is not supported"));
        }
    }
    .map_err(|error| format!("cpal output stream: {error}"))?;
    stream
        .play()
        .map_err(|error| format!("cpal play: {error}"))?;
    Ok(stream)
}

fn ingest(
    id: u64,
    uploads: &Mutex<AudioInputStore>,
    pts: &AtomicI64,
    interleaved: &[f32],
    channels: usize,
    rate: u32,
    map_left: usize,
    map_right: usize,
) {
    if interleaved.is_empty() {
        return;
    }
    let timestamp = pts.load(Ordering::Relaxed);
    let packet = interleaved_f32_packet(
        timestamp,
        rate as i32,
        interleaved,
        channels,
        map_left,
        map_right,
    );
    let frames = packet.samples_per_channel.max(0) as i64;
    uploads.lock().expect("audio").ingest_audio(id, packet);
    pts.store(
        timestamp.saturating_add(frames * 10_000_000 / i64::from(rate.max(1))),
        Ordering::Relaxed,
    );
}
