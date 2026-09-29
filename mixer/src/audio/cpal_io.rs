use crate::guard::LockExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceId, HostId, SampleFormat, Stream, StreamConfig, SupportedStreamConfig};

use crate::upload::AudioInputStore;

use super::capture::{AudioCaptureSpec, send_ready};
use super::feed::CaptureFeed;
use super::info::{CAPTURE_MODE_ENDPOINT_LOOPBACK, CAPTURE_MODE_MIC};
use super::pcm::{f32_to_i16, f32_to_i32};
use super::pump::{OutputPump, Route};
use super::rt::{Published, guard_callback};

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
    let feed = Arc::new(CaptureFeed::new(48_000));
    let mut pts = 0i64;
    while !stop.load(Ordering::Relaxed) {
        if follow_default {
            last_default = default_id(&host, loopback);
        }
        let failed = Arc::new(Mutex::new(None::<String>));
        match open_input(spec, &host, loopback, &feed, &failed) {
            Ok(stream) => {
                send_ready(ready, signaled, Ok(()))?;
                started = true;
                wait_stream(
                    stop,
                    follow_default,
                    loopback,
                    &host,
                    &last_default,
                    &failed,
                    INPUT_POLL,
                    &mut || feed.drain(uploads, spec.id, &mut pts),
                );
                drop(stream);
                feed.drain(uploads, spec.id, &mut pts);
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
    routes: &Arc<Published<Vec<Route>>>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let follow_default = device_id.is_empty();
    let host = platform_host()?;

    let mut started = false;
    let mut last_default = String::new();
    while !stop.load(Ordering::Relaxed) {
        if follow_default {
            last_default = default_id(&host, true);
        }
        let failed = Arc::new(Mutex::new(None::<String>));
        match open_output(device_id, &host, routes, &failed) {
            Ok(_stream) => {
                started = true;
                wait_stream(
                    stop,
                    follow_default,
                    true,
                    &host,
                    &last_default,
                    &failed,
                    OUTPUT_POLL,
                    &mut || {},
                );
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

const INPUT_POLL: Duration = Duration::from_millis(2);
const OUTPUT_POLL: Duration = Duration::from_millis(10);

fn wait_stream(
    stop: &AtomicBool,
    follow_default: bool,
    output: bool,
    host: &cpal::Host,
    last_default: &str,
    failed: &Mutex<Option<String>>,
    poll: Duration,
    tick: &mut dyn FnMut(),
) {
    let mut follow_check = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        tick();
        if failed.lock_or_recover().is_some() {
            break;
        }
        if follow_default && follow_check.elapsed() >= Duration::from_millis(250) {
            follow_check = Instant::now();
            let now = default_id(host, output);
            if !now.is_empty() && now != last_default {
                break;
            }
        }
        thread::sleep(poll);
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
    feed: &Arc<CaptureFeed>,
    failed: &Arc<Mutex<Option<String>>>,
) -> Result<Stream, String> {
    let device = resolve_device(host, loopback, &spec.device_id)?;
    let supported = stream_config(&device, loopback)?;
    let config: StreamConfig = supported.config();
    if config.channels == 0 || config.sample_rate == 0 {
        return Err(format!(
            "cpal input reports {} channels at {} Hz",
            config.channels, config.sample_rate
        ));
    }
    let channels = usize::from(config.channels);
    let map_left = spec.map_left.max(0) as usize;
    let map_right = spec.map_right.max(0) as usize;
    feed.set_rate(config.sample_rate);
    let feed = Arc::clone(feed);
    let err_flag = Arc::clone(failed);
    let err_cb = move |error| {
        *err_flag.lock_or_recover() = Some(format!("cpal stream: {error}"));
    };

    let stream = match supported.sample_format() {
        SampleFormat::F32 => build_input::<f32>(
            &device,
            config,
            feed,
            (channels, map_left, map_right),
            |sample| sample,
            err_cb,
        ),
        SampleFormat::I16 => build_input::<i16>(
            &device,
            config,
            feed,
            (channels, map_left, map_right),
            |sample| sample as f32 / 32768.0,
            err_cb,
        ),
        SampleFormat::I32 => build_input::<i32>(
            &device,
            config,
            feed,
            (channels, map_left, map_right),
            |sample| sample as f32 / 2_147_483_648.0,
            err_cb,
        ),
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

fn build_input<T>(
    device: &cpal::Device,
    config: StreamConfig,
    feed: Arc<CaptureFeed>,
    (channels, map_left, map_right): (usize, usize, usize),
    convert: fn(T) -> f32,
    err_cb: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<Stream, cpal::Error>
where
    T: cpal::SizedSample + Send + 'static,
{
    let mut converted: Vec<f32> = Vec::with_capacity(16_384);
    let mut scratch: Vec<f32> = Vec::with_capacity(16_384);
    let panicked = AtomicBool::new(false);
    device.build_input_stream(
        config,
        move |data: &[T], _| {
            guard_callback("cpal input", &panicked, || {
                converted.clear();
                converted.extend(data.iter().map(|sample| convert(*sample)));
                feed.push_mapped(&converted, channels, map_left, map_right, &mut scratch);
            });
        },
        err_cb,
        None,
    )
}

fn open_output(
    device_id: &str,
    host: &cpal::Host,
    routes: &Arc<Published<Vec<Route>>>,
    failed: &Arc<Mutex<Option<String>>>,
) -> Result<Stream, String> {
    let device = resolve_device(host, true, device_id)?;
    let supported = stream_config(&device, true)?;
    let config: StreamConfig = supported.config();
    if config.channels == 0 || config.sample_rate == 0 {
        return Err(format!(
            "cpal output reports {} channels at {} Hz",
            config.channels, config.sample_rate
        ));
    }
    let channels = usize::from(config.channels);
    let rate = config.sample_rate;
    let routes = Arc::clone(routes);
    let err_flag = Arc::clone(failed);
    let err_cb = move |error| {
        *err_flag.lock_or_recover() = Some(format!("cpal stream: {error}"));
    };

    let stream = match supported.sample_format() {
        SampleFormat::F32 => build_output::<f32>(
            &device,
            config,
            routes,
            channels,
            rate,
            |src, dst| dst.copy_from_slice(src),
            err_cb,
        ),
        SampleFormat::I16 => {
            build_output::<i16>(&device, config, routes, channels, rate, f32_to_i16, err_cb)
        }
        SampleFormat::I32 => {
            build_output::<i32>(&device, config, routes, channels, rate, f32_to_i32, err_cb)
        }
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

fn build_output<T>(
    device: &cpal::Device,
    config: StreamConfig,
    routes: Arc<Published<Vec<Route>>>,
    channels: usize,
    rate: u32,
    convert: fn(&[f32], &mut [T]),
    err_cb: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<Stream, cpal::Error>
where
    T: cpal::SizedSample + Send + 'static,
{
    let mut pump = OutputPump::new();
    let mut mix: Vec<f32> = Vec::with_capacity(16_384);
    let panicked = AtomicBool::new(false);
    device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let rendered = guard_callback("cpal output", &panicked, || {
                mix.resize(data.len(), 0.0);
                pump.render(&routes, &mut mix, channels, rate);
                convert(&mix, data);
            });
            if !rendered {
                data.fill(T::EQUILIBRIUM);
            }
        },
        err_cb,
        None,
    )
}
