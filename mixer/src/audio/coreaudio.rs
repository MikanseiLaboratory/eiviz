//! Core Audio device listing for macOS. Playback and capture go through cpal.

use std::ffi::{c_void, CStr};
use std::ptr;

use super::graph::DEVICE_COREAUDIO;
use super::info::AudioDeviceInfo;

type OSStatus = i32;
type AudioObjectID = u32;
type AudioDeviceID = u32;
type CFStringRef = *const c_void;
type CFTypeRef = *const c_void;

const K_AUDIO_OBJECT_SYSTEM_OBJECT: AudioObjectID = 1;
const K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL: u32 = 0x676c6f62; // 'glob'
const K_AUDIO_OBJECT_PROPERTY_SCOPE_OUTPUT: u32 = 0x6f757470; // 'outp'
const K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN: u32 = 0;
const K_AUDIO_HARDWARE_PROPERTY_DEVICES: u32 = 0x64657623; // 'dev#'
const K_AUDIO_DEVICE_PROPERTY_DEVICE_UID: u32 = 0x75696420; // 'uid '
const K_AUDIO_OBJECT_PROPERTY_NAME: u32 = 0x6c6e616d; // 'lnam'
const K_AUDIO_DEVICE_PROPERTY_STREAM_CONFIGURATION: u32 = 0x736c6179; // 'slay'
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const NO_ERR: OSStatus = 0;

#[repr(C)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

#[repr(C)]
struct AudioBuffer {
    number_channels: u32,
    data_byte_size: u32,
    data: *mut c_void,
}

#[link(name = "CoreAudio", kind = "framework")]
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
    fn CFRelease(cf: CFTypeRef);
    fn CFStringGetCString(string: CFStringRef, buffer: *mut i8, size: isize, encoding: u32) -> u8;
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
            direction: super::info::AUDIO_DIR_RENDER,
            caps: 0,
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
        AudioObjectGetPropertyDataSize(
            K_AUDIO_OBJECT_SYSTEM_OBJECT,
            &address,
            0,
            ptr::null(),
            &mut size,
        )
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
    let ok = unsafe {
        CFStringGetCString(
            cf,
            buf.as_mut_ptr(),
            buf.len() as isize,
            K_CF_STRING_ENCODING_UTF8,
        )
    };
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
