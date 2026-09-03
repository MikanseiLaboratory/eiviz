use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    WAVE_FORMAT_PCM, eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::PCWSTR;

use super::graph::BusRing;
use super::pop_stereo_rate;

const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

pub fn run(
    device_id: &str,
    exclusive: bool,
    maps: &[(Arc<BusRing>, i32, i32)],
    stop: &AtomicBool,
) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|error| format!("enumerator: {error}"))?;
        let device: IMMDevice = if device_id.is_empty() {
            enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|error| format!("default endpoint: {error}"))?
        } else {
            let wide: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
            enumerator
                .GetDevice(PCWSTR(wide.as_ptr()))
                .map_err(|error| format!("get device: {error}"))?
        };
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
        let share = if exclusive {
            AUDCLNT_SHAREMODE_EXCLUSIVE
        } else {
            AUDCLNT_SHAREMODE_SHARED
        };
        let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
        let hns = 200_000i64;
        let init = client.Initialize(share, flags, hns, 0, mix, None);
        if init.is_err() && exclusive {
            client
                .Initialize(AUDCLNT_SHAREMODE_SHARED, flags, hns, 0, mix, None)
                .map_err(|error| format!("initialize: {error}"))?;
        } else {
            init.map_err(|error| format!("initialize: {error}"))?;
        }
        let event =
            CreateEventW(None, false, false, None).map_err(|error| format!("event: {error}"))?;
        client
            .SetEventHandle(event)
            .map_err(|error| format!("set event: {error}"))?;
        let render: IAudioRenderClient = client
            .GetService()
            .map_err(|error| format!("render client: {error}"))?;
        let buffer_frames = client
            .GetBufferSize()
            .map_err(|error| format!("buffer: {error}"))?;
        client.Start().map_err(|error| format!("start: {error}"))?;
        while !stop.load(Ordering::Relaxed) {
            let wait = WaitForSingleObject(event, 50);
            if wait != WAIT_OBJECT_0 && !stop.load(Ordering::Relaxed) {
                continue;
            }
            let padding = client.GetCurrentPadding().unwrap_or(0);
            let available = buffer_frames.saturating_sub(padding);
            if available == 0 {
                continue;
            }
            let ptr = match render.GetBuffer(available) {
                Ok(ptr) => ptr,
                Err(_) => continue,
            };
            let bytes = available as usize * format.nBlockAlign as usize;
            let dest = std::slice::from_raw_parts_mut(ptr, bytes);
            dest.fill(0);
            let mapped = pop_stereo_rate(maps, available as usize, rate);
            for ((left, right), stereo) in mapped {
                write_mapped(
                    dest,
                    channels,
                    bits,
                    float,
                    &stereo,
                    left.max(0) as usize,
                    right.max(0) as usize,
                );
            }
            let _ = render.ReleaseBuffer(available, 0);
        }
        let _ = client.Stop();
        CoTaskMemFree(Some(mix.cast()));
        let _ = CloseHandle(HANDLE(event.0));
        let _ = WAVE_FORMAT_PCM;
        Ok(())
    }
}

fn write_mapped(
    dest: &mut [u8],
    channels: usize,
    bits: u16,
    float: bool,
    stereo: &[(f32, f32)],
    map_left: usize,
    map_right: usize,
) {
    let frames = stereo.len();
    let sample_bytes = (bits as usize / 8).max(1);
    let frame_bytes = channels * sample_bytes;
    if dest.len() < frames * frame_bytes {
        return;
    }
    for (i, (left, right)) in stereo.iter().enumerate() {
        write_sample(
            dest,
            i,
            channels,
            sample_bytes,
            float,
            bits,
            map_left,
            *left,
        );
        if map_right != map_left {
            write_sample(
                dest,
                i,
                channels,
                sample_bytes,
                float,
                bits,
                map_right,
                *right,
            );
        }
    }
}

fn write_sample(
    dest: &mut [u8],
    frame: usize,
    channels: usize,
    sample_bytes: usize,
    float: bool,
    bits: u16,
    channel: usize,
    value: f32,
) {
    if channel >= channels {
        return;
    }
    let offset = frame * channels * sample_bytes + channel * sample_bytes;
    if offset + sample_bytes > dest.len() {
        return;
    }
    let clipped = value.clamp(-1.0, 1.0);
    if float && sample_bytes == 4 {
        dest[offset..offset + 4].copy_from_slice(&clipped.to_le_bytes());
        return;
    }
    match bits {
        16 => {
            let code = (clipped * 32767.0) as i16;
            dest[offset..offset + 2].copy_from_slice(&code.to_le_bytes());
        }
        24 => {
            let code = (clipped * 8_388_607.0) as i32;
            dest[offset] = code as u8;
            dest[offset + 1] = (code >> 8) as u8;
            dest[offset + 2] = (code >> 16) as u8;
        }
        32 => {
            let code = (clipped * 2_147_483_647.0) as i32;
            dest[offset..offset + 4].copy_from_slice(&code.to_le_bytes());
        }
        _ => {}
    }
}
