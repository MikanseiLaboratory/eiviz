//! Core Audio HAL output for macOS buses.

use std::ffi::{c_void, CStr};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::graph::{BusRing, DEVICE_COREAUDIO};
use super::info::AudioDeviceInfo;
use super::pop_stereo_rate;

type OSStatus = i32;
type AudioObjectID = u32;
type AudioDeviceID = u32;
type AudioComponent = *mut c_void;
type AudioUnit = *mut c_void;
type CFStringRef = *const c_void;
type CFTypeRef = *const c_void;

const K_AUDIO_OBJECT_SYSTEM_OBJECT: AudioObjectID = 1;
const K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL: u32 = 0x676c6f62; // 'glob'
const K_AUDIO_OBJECT_PROPERTY_SCOPE_OUTPUT: u32 = 0x6f757470; // 'outp'
const K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN: u32 = 0;
const K_AUDIO_HARDWARE_PROPERTY_DEVICES: u32 = 0x64657623; // 'dev#'
const K_AUDIO_HARDWARE_PROPERTY_DEFAULT_OUTPUT_DEVICE: u32 = 0x644f7574; // 'dOut'
const K_AUDIO_DEVICE_PROPERTY_DEVICE_UID: u32 = 0x75696420; // 'uid '
const K_AUDIO_OBJECT_PROPERTY_NAME: u32 = 0x6c6e616d; // 'lnam'
const K_AUDIO_DEVICE_PROPERTY_STREAM_CONFIGURATION: u32 = 0x736c6179; // 'slay'
const K_AUDIO_DEVICE_PROPERTY_NOMINAL_SAMPLE_RATE: u32 = 0x6e737274; // 'nsrt'
const K_AUDIO_UNIT_TYPE_OUTPUT: u32 = 0x61756f75; // 'auou'
const K_AUDIO_UNIT_SUB_TYPE_HAL_OUTPUT: u32 = 0x6168616c; // 'ahal'
const K_AUDIO_UNIT_MANUFACTURER_APPLE: u32 = 0x6170706c; // 'appl'
const K_AUDIO_OUTPUT_UNIT_PROPERTY_CURRENT_DEVICE: u32 = 2000;
const K_AUDIO_OUTPUT_UNIT_PROPERTY_ENABLE_IO: u32 = 2003;
const K_AUDIO_UNIT_PROPERTY_STREAM_FORMAT: u32 = 8;
const K_AUDIO_UNIT_PROPERTY_SET_RENDER_CALLBACK: u32 = 23;
const K_AUDIO_UNIT_SCOPE_GLOBAL: u32 = 0;
const K_AUDIO_UNIT_SCOPE_INPUT: u32 = 1;
const K_AUDIO_UNIT_SCOPE_OUTPUT: u32 = 2;
const K_AUDIO_FORMAT_LINEAR_PCM: u32 = 0x6c70636d; // 'lpcm'
const K_AUDIO_FORMAT_FLAG_IS_FLOAT: u32 = 1;
const K_AUDIO_FORMAT_FLAG_IS_PACKED: u32 = 8;
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const NO_ERR: OSStatus = 0;

#[repr(C)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

#[repr(C)]
struct AudioComponentDescription {
    component_type: u32,
    component_sub_type: u32,
    component_manufacturer: u32,
    component_flags: u32,
    component_flags_mask: u32,
}

#[repr(C)]
struct AudioStreamBasicDescription {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_sample: u32,
    reserved: u32,
}

#[repr(C)]
struct AudioBuffer {
    number_channels: u32,
    data_byte_size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    number_buffers: u32,
    buffers: [AudioBuffer; 8],
}

#[repr(C)]
struct AURenderCallbackStruct {
    input_proc: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *mut u32,
            *const c_void,
            u32,
            u32,
            *mut AudioBufferList,
        ) -> OSStatus,
    >,
    input_proc_ref_con: *mut c_void,
}

#[link(name = "CoreAudio", kind = "framework")]
#[link(name = "AudioToolbox", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyDataSize(
        id: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        out_size: *mut u32,
    ) -> OSStatus;
    fn AudioObjectGetPropertyData(
        id: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        io_size: *mut u32,
        out: *mut c_void,
    ) -> OSStatus;
    fn AudioComponentFindNext(
        current: AudioComponent,
        desc: *const AudioComponentDescription,
    ) -> AudioComponent;
    fn AudioComponentInstanceNew(comp: AudioComponent, out: *mut AudioUnit) -> OSStatus;
    fn AudioComponentInstanceDispose(unit: AudioUnit) -> OSStatus;
    fn AudioUnitInitialize(unit: AudioUnit) -> OSStatus;
    fn AudioUnitSetProperty(
        unit: AudioUnit,
        id: u32,
        scope: u32,
        element: u32,
        data: *const c_void,
        size: u32,
    ) -> OSStatus;
    fn AudioOutputUnitStart(unit: AudioUnit) -> OSStatus;
    fn AudioOutputUnitStop(unit: AudioUnit) -> OSStatus;
    fn CFRelease(cf: CFTypeRef);
    fn CFStringGetCString(
        string: CFStringRef,
        buffer: *mut i8,
        size: isize,
        encoding: u32,
    ) -> u8;
}

struct RenderCtx {
    maps: Vec<(Arc<BusRing>, i32, i32)>,
    rate: u32,
}

pub fn run(
    device_id: &str,
    maps: &[(Arc<BusRing>, i32, i32)],
    stop: &AtomicBool,
) -> Result<(), String> {
    let device = resolve_device(device_id)?;
    let rate = device_rate(device).max(8_000.0);
    let desc = AudioComponentDescription {
        component_type: K_AUDIO_UNIT_TYPE_OUTPUT,
        component_sub_type: K_AUDIO_UNIT_SUB_TYPE_HAL_OUTPUT,
        component_manufacturer: K_AUDIO_UNIT_MANUFACTURER_APPLE,
        component_flags: 0,
        component_flags_mask: 0,
    };
    let comp = unsafe { AudioComponentFindNext(ptr::null_mut(), &desc) };
    if comp.is_null() {
        return Err("HAL output component missing".into());
    }
    let mut unit: AudioUnit = ptr::null_mut();
    status(unsafe { AudioComponentInstanceNew(comp, &mut unit) }, "instance")?;
    let enable: u32 = 1;
    status(
        unsafe {
            AudioUnitSetProperty(
                unit,
                K_AUDIO_OUTPUT_UNIT_PROPERTY_ENABLE_IO,
                K_AUDIO_UNIT_SCOPE_OUTPUT,
                0,
                &enable as *const u32 as *const c_void,
                4,
            )
        },
        "enable io",
    )?;
    status(
        unsafe {
            AudioUnitSetProperty(
                unit,
                K_AUDIO_OUTPUT_UNIT_PROPERTY_CURRENT_DEVICE,
                K_AUDIO_UNIT_SCOPE_GLOBAL,
                0,
                &device as *const AudioDeviceID as *const c_void,
                4,
            )
        },
        "current device",
    )?;
    let asbd = AudioStreamBasicDescription {
        sample_rate: rate,
        format_id: K_AUDIO_FORMAT_LINEAR_PCM,
        format_flags: K_AUDIO_FORMAT_FLAG_IS_FLOAT | K_AUDIO_FORMAT_FLAG_IS_PACKED,
        bytes_per_packet: 8,
        frames_per_packet: 1,
        bytes_per_frame: 8,
        channels_per_frame: 2,
        bits_per_sample: 32,
        reserved: 0,
    };
    status(
        unsafe {
            AudioUnitSetProperty(
                unit,
                K_AUDIO_UNIT_PROPERTY_STREAM_FORMAT,
                K_AUDIO_UNIT_SCOPE_INPUT,
                0,
                &asbd as *const AudioStreamBasicDescription as *const c_void,
                std::mem::size_of::<AudioStreamBasicDescription>() as u32,
            )
        },
        "stream format",
    )?;
    let ctx = Box::new(RenderCtx {
        maps: maps.to_vec(),
        rate: rate as u32,
    });
    let ctx_ptr = Box::into_raw(ctx);
    let callback = AURenderCallbackStruct {
        input_proc: Some(render),
        input_proc_ref_con: ctx_ptr as *mut c_void,
    };
    let set_cb = unsafe {
        AudioUnitSetProperty(
            unit,
            K_AUDIO_UNIT_PROPERTY_SET_RENDER_CALLBACK,
            K_AUDIO_UNIT_SCOPE_INPUT,
            0,
            &callback as *const AURenderCallbackStruct as *const c_void,
            std::mem::size_of::<AURenderCallbackStruct>() as u32,
        )
    };
    if set_cb != NO_ERR {
        unsafe {
            drop(Box::from_raw(ctx_ptr));
            AudioComponentInstanceDispose(unit);
        }
        return Err(format!("render callback: {set_cb}"));
    }
    if let Err(error) = status(unsafe { AudioUnitInitialize(unit) }, "initialize") {
        unsafe {
            drop(Box::from_raw(ctx_ptr));
            AudioComponentInstanceDispose(unit);
        }
        return Err(error);
    }
    if let Err(error) = status(unsafe { AudioOutputUnitStart(unit) }, "start") {
        unsafe {
            drop(Box::from_raw(ctx_ptr));
            AudioComponentInstanceDispose(unit);
        }
        return Err(error);
    }
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(50));
    }
    unsafe {
        let _ = AudioOutputUnitStop(unit);
        let _ = AudioComponentInstanceDispose(unit);
        drop(Box::from_raw(ctx_ptr));
    }
    Ok(())
}

unsafe extern "C" fn render(
    ref_con: *mut c_void,
    _flags: *mut u32,
    _stamp: *const c_void,
    _bus: u32,
    frames: u32,
    io_data: *mut AudioBufferList,
) -> OSStatus {
    if ref_con.is_null() || io_data.is_null() {
        return NO_ERR;
    }
    let ctx = unsafe { &*(ref_con as *const RenderCtx) };
    let list = unsafe { &mut *io_data };
    if list.number_buffers == 0 {
        return NO_ERR;
    }
    let mapped = pop_stereo_rate(&ctx.maps, frames as usize, ctx.rate.max(1));
    if list.number_buffers >= 2 {
        let left_buf = &list.buffers[0];
        let right_buf = &list.buffers[1];
        if !left_buf.data.is_null() {
            let dest = unsafe {
                std::slice::from_raw_parts_mut(
                    left_buf.data as *mut f32,
                    (left_buf.data_byte_size as usize) / 4,
                )
            };
            dest.fill(0.0);
            for ((left, _), stereo) in &mapped {
                let ch = (*left).max(0) as usize;
                if ch != 0 {
                    continue;
                }
                for (i, (sl, _)) in stereo.iter().enumerate() {
                    if let Some(slot) = dest.get_mut(i) {
                        *slot += sl.clamp(-1.0, 1.0);
                    }
                }
            }
        }
        if !right_buf.data.is_null() {
            let dest = unsafe {
                std::slice::from_raw_parts_mut(
                    right_buf.data as *mut f32,
                    (right_buf.data_byte_size as usize) / 4,
                )
            };
            dest.fill(0.0);
            for ((_, right), stereo) in &mapped {
                let ch = (*right).max(0) as usize;
                if ch != 1 {
                    continue;
                }
                for (i, (_, sr)) in stereo.iter().enumerate() {
                    if let Some(slot) = dest.get_mut(i) {
                        *slot += sr.clamp(-1.0, 1.0);
                    }
                }
            }
        }
        return NO_ERR;
    }
    let buf = &list.buffers[0];
    if buf.data.is_null() {
        return NO_ERR;
    }
    let samples = (buf.data_byte_size as usize) / 4;
    let dest = unsafe { std::slice::from_raw_parts_mut(buf.data as *mut f32, samples) };
    dest.fill(0.0);
    for ((left, right), stereo) in mapped {
        let l = left.max(0) as usize;
        let r = right.max(0) as usize;
        for (i, (sl, sr)) in stereo.iter().enumerate() {
            if l < 2 {
                if let Some(slot) = dest.get_mut(i * 2 + l) {
                    *slot += sl.clamp(-1.0, 1.0);
                }
            }
            if r < 2 && r != l {
                if let Some(slot) = dest.get_mut(i * 2 + r) {
                    *slot += sr.clamp(-1.0, 1.0);
                }
            }
        }
    }
    NO_ERR
}

pub fn enumerate(dest: &mut [AudioDeviceInfo]) -> usize {
    let mut n = 0usize;
    for (id, uid, name, channels) in output_devices() {
        if n >= dest.len() {
            break;
        }
        dest[n] = AudioDeviceInfo {
            kind: DEVICE_COREAUDIO,
            channels,
            id: write_fixed(&uid),
            name: write_fixed(&name),
        };
        let _ = id;
        n += 1;
    }
    n
}

pub fn channel_count(device_id: &str) -> i32 {
    output_devices()
        .into_iter()
        .find(|(_, uid, _, _)| uid == device_id)
        .map(|(_, _, _, ch)| ch as i32)
        .unwrap_or(0)
}

fn output_devices() -> Vec<(AudioDeviceID, String, String, u32)> {
    let mut out = Vec::new();
    let address = AudioObjectPropertyAddress {
        selector: K_AUDIO_HARDWARE_PROPERTY_DEVICES,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut size = 0u32;
    if unsafe {
        AudioObjectGetPropertyDataSize(K_AUDIO_OBJECT_SYSTEM_OBJECT, &address, 0, ptr::null(), &mut size)
    } != NO_ERR
        || size == 0
    {
        return out;
    }
    let count = (size as usize) / 4;
    let mut ids = vec![0u32; count];
    if unsafe {
        AudioObjectGetPropertyData(
            K_AUDIO_OBJECT_SYSTEM_OBJECT,
            &address,
            0,
            ptr::null(),
            &mut size,
            ids.as_mut_ptr() as *mut c_void,
        )
    } != NO_ERR
    {
        return out;
    }
    for id in ids {
        let channels = output_channels(id);
        if channels == 0 {
            continue;
        }
        let uid = cf_string_prop(id, K_AUDIO_DEVICE_PROPERTY_DEVICE_UID).unwrap_or_default();
        let name = cf_string_prop(id, K_AUDIO_OBJECT_PROPERTY_NAME).unwrap_or_else(|| uid.clone());
        if uid.is_empty() {
            continue;
        }
        out.push((id, uid, name, channels));
    }
    out
}

fn resolve_device(device_id: &str) -> Result<AudioDeviceID, String> {
    if device_id.is_empty() {
        return default_output();
    }
    output_devices()
        .into_iter()
        .find(|(_, uid, _, _)| uid == device_id)
        .map(|(id, _, _, _)| id)
        .ok_or_else(|| format!("audio device not found: {device_id}"))
        .or_else(|_| default_output())
}

fn default_output() -> Result<AudioDeviceID, String> {
    let address = AudioObjectPropertyAddress {
        selector: K_AUDIO_HARDWARE_PROPERTY_DEFAULT_OUTPUT_DEVICE,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut id = 0u32;
    let mut size = 4u32;
    status(
        unsafe {
            AudioObjectGetPropertyData(
                K_AUDIO_OBJECT_SYSTEM_OBJECT,
                &address,
                0,
                ptr::null(),
                &mut size,
                &mut id as *mut u32 as *mut c_void,
            )
        },
        "default output",
    )?;
    if id == 0 {
        return Err("no default output".into());
    }
    Ok(id)
}

fn device_rate(id: AudioDeviceID) -> f64 {
    let address = AudioObjectPropertyAddress {
        selector: K_AUDIO_DEVICE_PROPERTY_NOMINAL_SAMPLE_RATE,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut rate = 48_000.0f64;
    let mut size = 8u32;
    let _ = unsafe {
        AudioObjectGetPropertyData(
            id,
            &address,
            0,
            ptr::null(),
            &mut size,
            &mut rate as *mut f64 as *mut c_void,
        )
    };
    if rate < 8000.0 {
        48_000.0
    } else {
        rate
    }
}

fn output_channels(id: AudioDeviceID) -> u32 {
    let address = AudioObjectPropertyAddress {
        selector: K_AUDIO_DEVICE_PROPERTY_STREAM_CONFIGURATION,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_OUTPUT,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut size = 0u32;
    if unsafe { AudioObjectGetPropertyDataSize(id, &address, 0, ptr::null(), &mut size) } != NO_ERR
        || size == 0
    {
        return 0;
    }
    let mut raw = vec![0u8; size as usize];
    if unsafe {
        AudioObjectGetPropertyData(
            id,
            &address,
            0,
            ptr::null(),
            &mut size,
            raw.as_mut_ptr() as *mut c_void,
        )
    } != NO_ERR
    {
        return 0;
    }
    if raw.len() < 4 {
        return 0;
    }
    let nbuf = u32::from_ne_bytes(raw[0..4].try_into().unwrap_or([0; 4]));
    let mut channels = 0u32;
    // AudioBuffer is 4+4+ptr (16 on 64-bit, 12 on 32-bit).
    let stride = std::mem::size_of::<AudioBuffer>();
    for i in 0..nbuf as usize {
        let off = 4 + i * stride;
        if off + 4 > raw.len() {
            break;
        }
        channels += u32::from_ne_bytes(raw[off..off + 4].try_into().unwrap_or([0; 4]));
    }
    channels
}

fn cf_string_prop(id: AudioObjectID, selector: u32) -> Option<String> {
    let address = AudioObjectPropertyAddress {
        selector,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut cf: CFStringRef = ptr::null();
    let mut size = std::mem::size_of::<CFStringRef>() as u32;
    if unsafe {
        AudioObjectGetPropertyData(
            id,
            &address,
            0,
            ptr::null(),
            &mut size,
            &mut cf as *mut CFStringRef as *mut c_void,
        )
    } != NO_ERR
        || cf.is_null()
    {
        return None;
    }
    let mut buf = [0i8; 256];
    let ok = unsafe { CFStringGetCString(cf, buf.as_mut_ptr(), buf.len() as isize, K_CF_STRING_ENCODING_UTF8) };
    unsafe { CFRelease(cf) };
    if ok == 0 {
        return None;
    }
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_str()
        .ok()
        .map(|s| s.to_string())
}

fn write_fixed(text: &str) -> [u8; 256] {
    let mut buf = [0u8; 256];
    let bytes = text.as_bytes();
    let n = bytes.len().min(255);
    buf[..n].copy_from_slice(&bytes[..n]);
    buf
}

fn status(code: OSStatus, what: &str) -> Result<(), String> {
    if code == NO_ERR {
        Ok(())
    } else {
        Err(format!("{what}: {code}"))
    }
}
