use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STGM_READ,
};
use windows::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW,
    RegOpenKeyExW,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::PCWSTR;

use super::graph::{DEVICE_ASIO, DEVICE_WASAPI};
use super::info::AudioDeviceInfo;

pub fn enumerate(kind: u32, dest: &mut [AudioDeviceInfo]) -> usize {
    let mut n = 0usize;
    if kind == 0 || kind == DEVICE_WASAPI {
        n += enumerate_wasapi(&mut dest[n..]);
    }
    if kind == 0 || kind == DEVICE_ASIO {
        n += enumerate_asio_registry(&mut dest[n..]);
    }
    n
}

pub fn channel_count(kind: u32, device_id: &str) -> i32 {
    if kind == DEVICE_ASIO {
        return 2;
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(enumerator) =
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            return 2;
        };
        let device = if device_id.is_empty() {
            enumerator.GetDefaultAudioEndpoint(eRender, eConsole)
        } else {
            let wide: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
            enumerator.GetDevice(PCWSTR(wide.as_ptr()))
        };
        let Ok(device) = device else {
            return 2;
        };
        mix_channels(&device).unwrap_or(2) as i32
    }
}

fn enumerate_wasapi(dest: &mut [AudioDeviceInfo]) -> usize {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(enumerator) =
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            return 0;
        };
        let Ok(collection) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else {
            return 0;
        };
        let Ok(count) = collection.GetCount() else {
            return 0;
        };
        let mut n = 0usize;
        for i in 0..count {
            if n >= dest.len() {
                break;
            }
            let Ok(device) = collection.Item(i) else {
                continue;
            };
            let id = device_id_string(&device);
            let name = wasapi_friendly_name(&device).unwrap_or_else(|| id.clone());
            let channels = mix_channels(&device).unwrap_or(2);
            dest[n] = AudioDeviceInfo {
                kind: DEVICE_WASAPI,
                channels,
                id: cbuf(&id),
                name: cbuf(&name),
            };
            n += 1;
        }
        n
    }
}

pub fn enumerate_asio_registry(dest: &mut [AudioDeviceInfo]) -> usize {
    unsafe {
        let mut key = windows::Win32::System::Registry::HKEY::default();
        let path: Vec<u16> = "SOFTWARE\\ASIO"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(path.as_ptr()),
            Some(0),
            KEY_READ,
            &mut key,
        )
        .is_err()
        {
            return 0;
        }
        let mut n = 0usize;
        for index in 0..64u32 {
            if n >= dest.len() {
                break;
            }
            let mut name = [0u16; 256];
            let mut name_len = name.len() as u32;
            if RegEnumKeyExW(
                key,
                index,
                Some(windows::core::PWSTR(name.as_mut_ptr())),
                &mut name_len,
                None,
                None,
                None,
                None,
            )
            .is_err()
            {
                break;
            }
            let driver = String::from_utf16_lossy(&name[..name_len as usize]);
            let mut sub = windows::Win32::System::Registry::HKEY::default();
            let sub_path: Vec<u16> = driver.encode_utf16().chain(std::iter::once(0)).collect();
            if RegOpenKeyExW(key, PCWSTR(sub_path.as_ptr()), Some(0), KEY_READ, &mut sub).is_err() {
                continue;
            }
            let clsid = read_reg_sz(sub, "CLSID").unwrap_or_default();
            let _ = RegCloseKey(sub);
            if clsid.is_empty() {
                continue;
            }
            dest[n] = AudioDeviceInfo {
                kind: DEVICE_ASIO,
                channels: 2,
                id: cbuf(&clsid),
                name: cbuf(&driver),
            };
            n += 1;
        }
        let _ = RegCloseKey(key);
        n
    }
}

fn cbuf(text: &str) -> [u8; 256] {
    let mut buf = [0u8; 256];
    let bytes = text.as_bytes();
    let n = bytes.len().min(255);
    buf[..n].copy_from_slice(&bytes[..n]);
    buf
}

fn wasapi_friendly_name(device: &IMMDevice) -> Option<String> {
    unsafe {
        let store: IPropertyStore = device.OpenPropertyStore(STGM_READ).ok()?;
        let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let text = value.to_string();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

fn device_id_string(device: &IMMDevice) -> String {
    unsafe {
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

fn mix_channels(device: &IMMDevice) -> Option<u32> {
    unsafe {
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None).ok()?;
        let fmt = client.GetMixFormat().ok()?;
        if fmt.is_null() {
            return None;
        }
        let channels = (*fmt).nChannels as u32;
        CoTaskMemFree(Some(fmt.cast()));
        Some(channels.max(1))
    }
}

fn read_reg_sz(key: windows::Win32::System::Registry::HKEY, name: &str) -> Option<String> {
    unsafe {
        let name_w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut buf = [0u16; 256];
        let mut size = (buf.len() * 2) as u32;
        if RegGetValueW(
            key,
            PCWSTR::null(),
            PCWSTR(name_w.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
        .is_err()
        {
            return None;
        }
        let chars = (size as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buf[..chars.min(buf.len())]))
    }
}
