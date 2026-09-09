use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::upload::{AudioInputStore, AudioPacket};

use super::graph::{DEVICE_ASIO, DEVICE_COREAUDIO, DEVICE_WASAPI};
use super::info::{
    CAPTURE_MODE_ENDPOINT_LOOPBACK, CAPTURE_MODE_MIC, CAPTURE_MODE_PROCESS_LOOPBACK,
};

#[derive(Clone, Debug)]
pub struct AudioCaptureSpec {
    pub id: u64,
    pub kind: u32,
    pub device_id: String,
    pub mode: u32,
    pub map_left: i32,
    pub map_right: i32,
    pub process_exe: String,
    pub process_aumid: String,
}

struct CaptureHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            crate::diag::join_timeout(join, Duration::from_secs(2), "audio-cap");
        }
    }
}

#[derive(Default)]
pub struct AudioCaptureStore {
    captures: HashMap<u64, CaptureHandle>,
}

impl AudioCaptureStore {
    pub fn start(
        &mut self,
        spec: AudioCaptureSpec,
        uploads: Arc<Mutex<AudioInputStore>>,
    ) -> Result<(), String> {
        self.stop(spec.id);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_t = Arc::clone(&stop);
        let id = spec.id;
        let join = thread::Builder::new()
            .name(format!("eiviz-acap-{id}"))
            .spawn(move || run_capture(spec, uploads, stop_t))
            .map_err(|error| error.to_string())?;
        self.captures.insert(
            id,
            CaptureHandle {
                stop,
                join: Some(join),
            },
        );
        Ok(())
    }

    pub fn stop(&mut self, id: u64) {
        self.captures.remove(&id);
    }

    pub fn stop_all(&mut self) {
        self.captures.clear();
    }
}

fn run_capture(
    spec: AudioCaptureSpec,
    uploads: Arc<Mutex<AudioInputStore>>,
    stop: Arc<AtomicBool>,
) {
    if let Err(error) = run_capture_inner(&spec, &uploads, &stop) {
        crate::diag::error(&format!("audio capture {}: {error}", spec.id));
    }
}

fn run_capture_inner(
    spec: &AudioCaptureSpec,
    uploads: &Arc<Mutex<AudioInputStore>>,
    stop: &AtomicBool,
) -> Result<(), String> {
    match spec.kind {
        0 | DEVICE_WASAPI => {
            #[cfg(windows)]
            {
                return wasapi_capture(spec, uploads, stop);
            }
            #[cfg(not(windows))]
            {
                return Err("WASAPI capture is only available on Windows".into());
            }
        }
        DEVICE_ASIO => Err("ASIO capture is not implemented".into()),
        DEVICE_COREAUDIO => {
            #[cfg(target_os = "macos")]
            {
                let _ = (spec, uploads, stop);
                Err("Core Audio capture is not implemented".into())
            }
            #[cfg(not(target_os = "macos"))]
            {
                Err("Core Audio capture is only available on macOS".into())
            }
        }
        other => Err(format!("unknown audio capture backend {other}")),
    }
}

#[cfg(windows)]
fn wasapi_capture(
    spec: &AudioCaptureSpec,
    uploads: &Arc<Mutex<AudioInputStore>>,
    stop: &AtomicBool,
) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::Media::Audio::{
        eCapture, eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDevice,
        IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT,
        AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
        WAVE_FORMAT_PCM,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

    const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
    const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

    if spec.mode == CAPTURE_MODE_PROCESS_LOOPBACK {
        return Err(format!(
            "WASAPI process loopback is not implemented (exe='{}' aumid='{}')",
            spec.process_exe, spec.process_aumid
        ));
    }
    let loopback = spec.mode == CAPTURE_MODE_ENDPOINT_LOOPBACK;
    let follow_default = spec.device_id.is_empty();
    if spec.mode != CAPTURE_MODE_MIC && spec.mode != CAPTURE_MODE_ENDPOINT_LOOPBACK {
        return Err(format!("unknown capture mode {}", spec.mode));
    }

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|error| format!("enumerator: {error}"))?;
        let mut last_default = String::new();
        while !stop.load(Ordering::Relaxed) {
            let flow = if loopback { eRender } else { eCapture };
            let device: IMMDevice = if follow_default {
                enumerator
                    .GetDefaultAudioEndpoint(flow, eConsole)
                    .map_err(|error| format!("default endpoint: {error}"))?
            } else {
                let wide: Vec<u16> = spec
                    .device_id
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                enumerator
                    .GetDevice(PCWSTR(wide.as_ptr()))
                    .map_err(|error| format!("get device: {error}"))?
            };
            if follow_default {
                last_default = default_id(&enumerator, flow);
            }
            let client: IAudioClient = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|error| format!("activate: {error}"))?;
            let mix = client
                .GetMixFormat()
                .map_err(|error| format!("mix format: {error}"))?;
            if mix.is_null() {
                return Err("mix format null".into());
            }
            let format = *mix;
            let channels = format.nChannels.max(1) as usize;
            let rate = format.nSamplesPerSec.max(1);
            let bits = format.wBitsPerSample;
            let float = format.wFormatTag == WAVE_FORMAT_IEEE_FLOAT
                || (format.wFormatTag == WAVE_FORMAT_EXTENSIBLE && format.wBitsPerSample == 32);
            let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                | if loopback {
                    AUDCLNT_STREAMFLAGS_LOOPBACK
                } else {
                    0
                };
            client
                .Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 200_000, 0, mix, None)
                .map_err(|error| format!("initialize: {error}"))?;
            let event = CreateEventW(None, false, false, None)
                .map_err(|error| format!("event: {error}"))?;
            client
                .SetEventHandle(event)
                .map_err(|error| format!("set event: {error}"))?;
            let capture: IAudioCaptureClient = client
                .GetService()
                .map_err(|error| format!("capture client: {error}"))?;
            client.Start().map_err(|error| format!("start: {error}"))?;
            let map_left = spec.map_left.max(0) as usize;
            let map_right = spec.map_right.max(0) as usize;
            let mut pts = 0i64;
            let mut follow_check = Instant::now();
            let mut reopen = false;
            while !stop.load(Ordering::Relaxed) && !reopen {
                if WaitForSingleObject(event, 50) != WAIT_OBJECT_0 && !stop.load(Ordering::Relaxed)
                {
                    if follow_default && follow_check.elapsed() >= Duration::from_millis(250) {
                        follow_check = Instant::now();
                        let now = default_id(&enumerator, flow);
                        if !now.is_empty() && now != last_default {
                            reopen = true;
                        }
                    }
                    continue;
                }
                loop {
                    let mut data: *mut u8 = std::ptr::null_mut();
                    let mut frames = 0u32;
                    let mut flags = 0u32;
                    if capture
                        .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                        .is_err()
                    {
                        break;
                    }
                    if frames == 0 || data.is_null() {
                        let _ = capture.ReleaseBuffer(0);
                        break;
                    }
                    let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
                    let packet = if silent {
                        silent_packet(pts, rate as i32, frames)
                    } else {
                        let bytes = frames as usize * format.nBlockAlign as usize;
                        let src = std::slice::from_raw_parts(data, bytes);
                        mapped_packet(
                            pts,
                            rate as i32,
                            frames,
                            src,
                            channels,
                            bits,
                            float,
                            map_left,
                            map_right,
                        )
                    };
                    uploads.lock().expect("audio").ingest_audio(spec.id, packet);
                    pts =
                        pts.saturating_add(i64::from(frames) * 10_000_000 / i64::from(rate.max(1)));
                    let _ = capture.ReleaseBuffer(frames);
                }
                if follow_default && follow_check.elapsed() >= Duration::from_millis(250) {
                    follow_check = Instant::now();
                    let now = default_id(&enumerator, flow);
                    if !now.is_empty() && now != last_default {
                        reopen = true;
                    }
                }
            }
            let _ = client.Stop();
            CoTaskMemFree(Some(mix.cast()));
            let _ = CloseHandle(HANDLE(event.0));
            let _ = WAVE_FORMAT_PCM;
            if !reopen {
                break;
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
fn default_id(
    enumerator: &windows::Win32::Media::Audio::IMMDeviceEnumerator,
    flow: windows::Win32::Media::Audio::EDataFlow,
) -> String {
    use windows::Win32::Media::Audio::eConsole;
    use windows::Win32::System::Com::CoTaskMemFree;
    unsafe {
        let Ok(device) = enumerator.GetDefaultAudioEndpoint(flow, eConsole) else {
            return String::new();
        };
        match device.GetId() {
            Ok(id) => {
                let text = id.to_string().unwrap_or_default();
                CoTaskMemFree(Some(id.0.cast()));
                text
            }
            Err(_) => String::new(),
        }
    }
}

fn silent_packet(timestamp: i64, sample_rate: i32, frames: u32) -> AudioPacket {
    AudioPacket {
        timestamp,
        sample_rate,
        channels: 2,
        samples_per_channel: frames as i32,
        pcm_planar_f32: vec![0.0; frames as usize * 2],
    }
}

fn mapped_packet(
    timestamp: i64,
    sample_rate: i32,
    frames: u32,
    src: &[u8],
    channels: usize,
    bits: u16,
    float: bool,
    map_left: usize,
    map_right: usize,
) -> AudioPacket {
    let frames = frames as usize;
    let sample_bytes = (bits as usize / 8).max(1);
    let frame_bytes = channels * sample_bytes;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    for i in 0..frames {
        let base = i * frame_bytes;
        if base + frame_bytes > src.len() {
            break;
        }
        left[i] = read_sample(src, base, channels, sample_bytes, float, bits, map_left);
        right[i] = read_sample(src, base, channels, sample_bytes, float, bits, map_right);
    }
    let mut pcm = left;
    pcm.extend(right);
    AudioPacket {
        timestamp,
        sample_rate,
        channels: 2,
        samples_per_channel: frames as i32,
        pcm_planar_f32: pcm,
    }
}

fn read_sample(
    src: &[u8],
    base: usize,
    channels: usize,
    sample_bytes: usize,
    float: bool,
    bits: u16,
    channel: usize,
) -> f32 {
    if channel >= channels {
        return 0.0;
    }
    let offset = base + channel * sample_bytes;
    if offset + sample_bytes > src.len() {
        return 0.0;
    }
    if float && sample_bytes == 4 {
        return f32::from_le_bytes([
            src[offset],
            src[offset + 1],
            src[offset + 2],
            src[offset + 3],
        ]);
    }
    match bits {
        16 => {
            let value = i16::from_le_bytes([src[offset], src[offset + 1]]);
            value as f32 / 32768.0
        }
        24 if sample_bytes >= 3 => {
            let value =
                i32::from_le_bytes([src[offset], src[offset + 1], src[offset + 2], 0]) << 8 >> 8;
            value as f32 / 8_388_608.0
        }
        32 => {
            let value = i32::from_le_bytes([
                src[offset],
                src[offset + 1],
                src[offset + 2],
                src[offset + 3],
            ]);
            value as f32 / 2_147_483_648.0
        }
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::{mapped_packet, silent_packet};
    use crate::upload::AUDIO_RATE;

    #[test]
    fn silent_packet_is_stereo_planar() {
        let packet = silent_packet(10, AUDIO_RATE, 4);
        assert_eq!(packet.channels, 2);
        assert_eq!(packet.samples_per_channel, 4);
        assert_eq!(packet.pcm_planar_f32.len(), 8);
        assert!(packet.pcm_planar_f32.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn mapped_packet_reads_float_interleaved() {
        let mut src = Vec::new();
        for frame in 0..2u32 {
            src.extend((frame as f32).to_le_bytes());
            src.extend((frame as f32 + 0.5).to_le_bytes());
        }
        let packet = mapped_packet(0, 48_000, 2, &src, 2, 32, true, 0, 1);
        assert_eq!(packet.pcm_planar_f32, vec![0.0, 1.0, 0.5, 1.5]);
    }
}
