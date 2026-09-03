use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::core::{GUID, HRESULT, IUnknown, Interface};

use super::graph::BusRing;
use super::pop_stereo_rate;
use crate::upload::AUDIO_RATE;

const IID_IASIO: GUID = GUID::from_u128(0x4533_a902_d579_11d0_89f4_00a0_c905_425c);
const ASIO_OK: i32 = 0;
const ASIOST_INT16_LSB: i32 = 16;
const ASIOST_INT24_LSB: i32 = 17;
const ASIOST_INT32_LSB: i32 = 18;
const ASIOST_FLOAT32_LSB: i32 = 19;
const ASIOST_FLOAT64_LSB: i32 = 20;

#[repr(C)]
struct Iasio {
    vtbl: *const IasioVtbl,
}

#[repr(C)]
struct IasioVtbl {
    query_interface:
        unsafe extern "system" fn(*mut Iasio, *const GUID, *mut *mut core::ffi::c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut Iasio) -> u32,
    release: unsafe extern "system" fn(*mut Iasio) -> u32,
    init: unsafe extern "system" fn(*mut Iasio, *mut core::ffi::c_void) -> i32,
    get_driver_name: unsafe extern "system" fn(*mut Iasio, *mut i8),
    get_driver_version: unsafe extern "system" fn(*mut Iasio) -> i32,
    get_error_message: unsafe extern "system" fn(*mut Iasio, *mut i8),
    start: unsafe extern "system" fn(*mut Iasio) -> i32,
    stop: unsafe extern "system" fn(*mut Iasio) -> i32,
    get_channels: unsafe extern "system" fn(*mut Iasio, *mut i32, *mut i32) -> i32,
    get_latencies: unsafe extern "system" fn(*mut Iasio, *mut i32, *mut i32) -> i32,
    get_buffer_size:
        unsafe extern "system" fn(*mut Iasio, *mut i32, *mut i32, *mut i32, *mut i32) -> i32,
    can_sample_rate: unsafe extern "system" fn(*mut Iasio, f64) -> i32,
    get_sample_rate: unsafe extern "system" fn(*mut Iasio, *mut f64) -> i32,
    set_sample_rate: unsafe extern "system" fn(*mut Iasio, f64) -> i32,
    get_clock_sources:
        unsafe extern "system" fn(*mut Iasio, *mut core::ffi::c_void, *mut i32) -> i32,
    set_clock_source: unsafe extern "system" fn(*mut Iasio, i32) -> i32,
    get_sample_position: unsafe extern "system" fn(
        *mut Iasio,
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
    ) -> i32,
    get_channel_info: unsafe extern "system" fn(*mut Iasio, *mut AsioChannelInfo) -> i32,
    create_buffers: unsafe extern "system" fn(
        *mut Iasio,
        *mut AsioBufferInfo,
        i32,
        i32,
        *mut AsioCallbacks,
    ) -> i32,
    dispose_buffers: unsafe extern "system" fn(*mut Iasio) -> i32,
    control_panel: unsafe extern "system" fn(*mut Iasio) -> i32,
    future: unsafe extern "system" fn(*mut Iasio, i32, *mut core::ffi::c_void) -> i32,
    output_ready: unsafe extern "system" fn(*mut Iasio) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AsioBufferInfo {
    is_input: i32,
    channel_num: i32,
    buffers: [*mut core::ffi::c_void; 2],
}

unsafe impl Send for AsioBufferInfo {}
unsafe impl Sync for AsioBufferInfo {}

#[repr(C)]
struct AsioChannelInfo {
    channel: i32,
    is_input: i32,
    is_active: i32,
    channel_group: i32,
    sample_type: i32,
    name: [i8; 32],
}

#[repr(C)]
struct AsioCallbacks {
    buffer_switch: Option<unsafe extern "C" fn(i32, i32)>,
    sample_rate_did_change: Option<unsafe extern "C" fn(f64)>,
    asio_message: Option<unsafe extern "C" fn(i32, i32, *mut core::ffi::c_void, *mut f64) -> i32>,
    buffer_switch_time_info:
        Option<unsafe extern "C" fn(*mut core::ffi::c_void, i32, i32) -> *mut core::ffi::c_void>,
}

struct AsioState {
    maps: Vec<(Arc<BusRing>, i32, i32)>,
    infos: Vec<AsioBufferInfo>,
    sample_types: Vec<i32>,
    buffer_size: i32,
    rate: f64,
}

static ASIO: Mutex<Option<Arc<Mutex<AsioState>>>> = Mutex::new(None);

pub fn run(
    device_id: &str,
    maps: &[(Arc<BusRing>, i32, i32)],
    stop: &AtomicBool,
) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let clsid = parse_guid(device_id).ok_or_else(|| "invalid ASIO CLSID".to_string())?;
        let unk: IUnknown = CoCreateInstance(&clsid, None, CLSCTX_INPROC_SERVER)
            .map_err(|error| format!("CoCreateInstance ASIO: {error}"))?;
        let mut raw: *mut core::ffi::c_void = std::ptr::null_mut();
        unk.query(&IID_IASIO, &mut raw)
            .ok()
            .map_err(|error| format!("IASIO QueryInterface: {error}"))?;
        if raw.is_null() {
            return Err("IASIO pointer null".into());
        }
        let asio = raw as *mut Iasio;
        let vtbl = &*(*asio).vtbl;
        if (vtbl.init)(asio, std::ptr::null_mut()) == 0 {
            let _ = (vtbl.release)(asio);
            return Err("ASIO init failed".into());
        }
        let mut ins = 0i32;
        let mut outs = 0i32;
        if (vtbl.get_channels)(asio, &mut ins, &mut outs) != ASIO_OK || outs < 2 {
            let _ = (vtbl.release)(asio);
            return Err("ASIO has no outputs".into());
        }
        let _ = (vtbl.set_sample_rate)(asio, f64::from(AUDIO_RATE));
        let mut rate = f64::from(AUDIO_RATE);
        let _ = (vtbl.get_sample_rate)(asio, &mut rate);
        if rate < 1.0 {
            rate = f64::from(AUDIO_RATE);
        }
        let mut min_size = 0i32;
        let mut max_size = 0i32;
        let mut pref = 0i32;
        let mut gran = 0i32;
        if (vtbl.get_buffer_size)(asio, &mut min_size, &mut max_size, &mut pref, &mut gran)
            != ASIO_OK
        {
            let _ = (vtbl.release)(asio);
            return Err("ASIO buffer size".into());
        }
        let buffer_size = if pref > 0 { pref } else { min_size.max(64) };
        let mut infos = Vec::with_capacity(outs as usize);
        let mut types = Vec::with_capacity(outs as usize);
        for ch in 0..outs {
            infos.push(AsioBufferInfo {
                is_input: 0,
                channel_num: ch,
                buffers: [std::ptr::null_mut(), std::ptr::null_mut()],
            });
            let mut info = AsioChannelInfo {
                channel: ch,
                is_input: 0,
                is_active: 0,
                channel_group: 0,
                sample_type: ASIOST_FLOAT32_LSB,
                name: [0; 32],
            };
            let _ = (vtbl.get_channel_info)(asio, &mut info);
            types.push(info.sample_type);
        }
        let state = Arc::new(Mutex::new(AsioState {
            maps: maps.to_vec(),
            infos: Vec::new(),
            sample_types: types,
            buffer_size,
            rate,
        }));
        *ASIO.lock().expect("asio slot") = Some(Arc::clone(&state));
        let mut callbacks = AsioCallbacks {
            buffer_switch: Some(buffer_switch),
            sample_rate_did_change: Some(sample_rate_did_change),
            asio_message: Some(asio_message),
            buffer_switch_time_info: Some(buffer_switch_time_info),
        };
        if (vtbl.create_buffers)(asio, infos.as_mut_ptr(), outs, buffer_size, &mut callbacks)
            != ASIO_OK
        {
            *ASIO.lock().expect("asio slot") = None;
            let _ = (vtbl.release)(asio);
            return Err("ASIO createBuffers failed".into());
        }
        state.lock().expect("asio").infos = infos;
        if (vtbl.start)(asio) != ASIO_OK {
            let _ = (vtbl.dispose_buffers)(asio);
            *ASIO.lock().expect("asio slot") = None;
            let _ = (vtbl.release)(asio);
            return Err("ASIO start failed".into());
        }
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = (vtbl.stop)(asio);
        let _ = (vtbl.dispose_buffers)(asio);
        let _ = (vtbl.release)(asio);
        *ASIO.lock().expect("asio slot") = None;
        let _ = unk;
        Ok(())
    }
}

unsafe extern "C" fn buffer_switch(index: i32, _direct: i32) {
    let Some(state) = ASIO.lock().ok().and_then(|guard| guard.clone()) else {
        return;
    };
    let guard = state.lock().expect("asio state");
    let frames = guard.buffer_size.max(1) as usize;
    let rate = guard.rate.max(1.0) as u32;
    let mapped = pop_stereo_rate(&guard.maps, frames, rate);
    let infos = guard.infos.clone();
    let types = guard.sample_types.clone();
    drop(guard);
    let idx = if index == 0 { 0usize } else { 1usize };
    for (channel, info) in infos.iter().enumerate() {
        let ptr = info.buffers[idx];
        if ptr.is_null() {
            continue;
        }
        let ty = types.get(channel).copied().unwrap_or(ASIOST_FLOAT32_LSB);
        fill_asio_channel(ptr, ty, frames, &mapped, channel);
    }
}

fn fill_asio_channel(
    ptr: *mut core::ffi::c_void,
    ty: i32,
    frames: usize,
    mapped: &std::collections::HashMap<(i32, i32), Vec<(f32, f32)>>,
    channel: usize,
) {
    let mut samples = vec![0.0f32; frames];
    for ((left, right), stereo) in mapped {
        let use_left = *left as usize == channel;
        let use_right = *right as usize == channel;
        if !use_left && !use_right {
            continue;
        }
        for (i, (l, r)) in stereo.iter().enumerate().take(frames) {
            samples[i] += if use_left { *l } else { *r };
        }
    }
    unsafe {
        match ty {
            ASIOST_INT16_LSB => {
                let dest = std::slice::from_raw_parts_mut(ptr as *mut i16, frames);
                for (slot, sample) in dest.iter_mut().zip(samples) {
                    *slot = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
                }
            }
            ASIOST_INT24_LSB => {
                let dest = std::slice::from_raw_parts_mut(ptr as *mut u8, frames * 3);
                for i in 0..frames {
                    let code = (samples[i].clamp(-1.0, 1.0) * 8_388_607.0) as i32;
                    dest[i * 3] = code as u8;
                    dest[i * 3 + 1] = (code >> 8) as u8;
                    dest[i * 3 + 2] = (code >> 16) as u8;
                }
            }
            ASIOST_INT32_LSB => {
                let dest = std::slice::from_raw_parts_mut(ptr as *mut i32, frames);
                for (slot, sample) in dest.iter_mut().zip(samples) {
                    *slot = (sample.clamp(-1.0, 1.0) * 2_147_483_647.0) as i32;
                }
            }
            ASIOST_FLOAT64_LSB => {
                let dest = std::slice::from_raw_parts_mut(ptr as *mut f64, frames);
                for (slot, sample) in dest.iter_mut().zip(samples) {
                    *slot = f64::from(sample);
                }
            }
            _ => {
                let dest = std::slice::from_raw_parts_mut(ptr as *mut f32, frames);
                dest.copy_from_slice(&samples);
            }
        }
    }
}

unsafe extern "C" fn sample_rate_did_change(rate: f64) {
    if let Some(state) = ASIO.lock().ok().and_then(|guard| guard.clone()) {
        state.lock().expect("asio").rate = rate;
    }
}

unsafe extern "C" fn asio_message(
    selector: i32,
    _value: i32,
    _message: *mut core::ffi::c_void,
    _opt: *mut f64,
) -> i32 {
    match selector {
        7 | 6 => 1,
        _ => 0,
    }
}

unsafe extern "C" fn buffer_switch_time_info(
    params: *mut core::ffi::c_void,
    index: i32,
    direct: i32,
) -> *mut core::ffi::c_void {
    unsafe { buffer_switch(index, direct) };
    params
}

pub fn parse_guid(text: &str) -> Option<GUID> {
    let trimmed = text.trim().trim_matches('{').trim_end_matches('}').trim();
    let hex: String = trimmed.chars().filter(|ch| *ch != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    let value = u128::from_str_radix(&hex, 16).ok()?;
    Some(GUID::from_u128(value))
}
