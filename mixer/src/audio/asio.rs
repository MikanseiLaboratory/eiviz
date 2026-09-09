use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::AudioCaptureSpec;
use super::graph::{BusRing, DEVICE_ASIO};
use super::info::{
    CAPTURE_MODE_ENDPOINT_LOOPBACK, CAPTURE_MODE_MIC, CAPTURE_MODE_PROCESS_LOOPBACK,
};
use super::pop_stereo_rate;
use crate::upload::AudioInputStore;

struct AsioCapture {
    id: u64,
    map_left: i32,
    map_right: i32,
    uploads: Arc<Mutex<AudioInputStore>>,
    pts: AtomicI64,
}

struct AsioShared {
    outputs: Vec<(Arc<BusRing>, i32, i32)>,
    captures: HashMap<u64, Arc<AsioCapture>>,
    ins: i32,
    outs: i32,
    error: Option<String>,
    ready: bool,
}

struct AsioDevice {
    shared: Arc<Mutex<AsioShared>>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct AsioHub {
    devices: HashMap<String, AsioDevice>,
}

static HUB: std::sync::OnceLock<Mutex<AsioHub>> = std::sync::OnceLock::new();
static IO_CACHE: std::sync::OnceLock<Mutex<HashMap<String, (i32, i32)>>> = std::sync::OnceLock::new();

fn hub() -> &'static Mutex<AsioHub> {
    HUB.get_or_init(|| {
        Mutex::new(AsioHub {
            devices: HashMap::new(),
        })
    })
}

fn io_cache() -> &'static Mutex<HashMap<String, (i32, i32)>> {
    IO_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remember_io(key: &str, ins: i32, outs: i32) {
    if key.is_empty() || (ins <= 0 && outs <= 0) {
        return;
    }
    if let Ok(mut cache) = io_cache().lock() {
        cache.insert(key.to_string(), (ins, outs));
    }
}

fn cached_io(key: &str) -> Option<(i32, i32)> {
    io_cache().lock().ok()?.get(key).copied()
}

pub fn listed_io(name: &str, clsid: &str) -> (i32, i32) {
    let io = cpal_asio_io(clsid).or_else(|| cpal_asio_io(name));
    let Some(io) = io else {
        return (0, 0);
    };
    remember_io(&norm(clsid), io.0, io.1);
    remember_io(&norm(name), io.0, io.1);
    io
}

fn cpal_asio_io(device_id: &str) -> Option<(i32, i32)> {
    let names = asio_lookup_names(device_id);
    let table = cpal_asio_table(false);
    lookup_table(&table, &names)
}

fn lookup_table(table: &HashMap<String, (i32, i32)>, names: &[String]) -> Option<(i32, i32)> {
    for name in names {
        if let Some(io) = table.get(&norm(name)) {
            if io.0 > 0 || io.1 > 0 {
                return Some(*io);
            }
        }
    }
    None
}

fn cpal_asio_table(force: bool) -> HashMap<String, (i32, i32)> {
    static TABLE: std::sync::OnceLock<Mutex<HashMap<String, (i32, i32)>>> = std::sync::OnceLock::new();
    let slot = TABLE.get_or_init(|| Mutex::new(HashMap::new()));
    if !force {
        if let Ok(table) = slot.lock() {
            if !table.is_empty() {
                return table.clone();
            }
        }
    }
    let scanned = scan_cpal_asio();
    if let Ok(mut table) = slot.lock() {
        if !scanned.is_empty() {
            *table = scanned.clone();
        }
    }
    scanned
}

fn scan_cpal_asio() -> HashMap<String, (i32, i32)> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let mut table = HashMap::new();
    let Ok(host) = cpal::host_from_id(cpal::HostId::Asio) else {
        crate::diag::warn("asio: cpal ASIO host unavailable");
        return table;
    };
    let Ok(devices) = host.devices() else {
        crate::diag::warn("asio: cpal ASIO device list failed");
        return table;
    };
    for device in devices {
        let Ok(desc) = device.description() else {
            continue;
        };
        let name = desc.name().to_string();
        let ins = device
            .default_input_config()
            .map(|cfg| i32::from(cfg.channels()))
            .unwrap_or(0);
        let outs = device
            .default_output_config()
            .map(|cfg| i32::from(cfg.channels()))
            .unwrap_or(0);
        if ins > 0 || outs > 0 {
            crate::diag::info(&format!("asio cpal {name}: {ins} in / {outs} out"));
            table.insert(norm(&name), (ins, outs));
        }
    }
    table
}

fn asio_lookup_names(device_id: &str) -> Vec<String> {
    let mut names = vec![device_id.trim().to_string()];
    if let Some(name) = registry_name_for_id(device_id) {
        names.push(name);
    }
    names
}

fn registry_name_for_id(device_id: &str) -> Option<String> {
    let want = norm(device_id);
    if want.is_empty() {
        return None;
    }
    unsafe {
        let mut key = windows::Win32::System::Registry::HKEY::default();
        let path: Vec<u16> = "SOFTWARE\\ASIO"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        if windows::Win32::System::Registry::RegOpenKeyExW(
            windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
            windows::core::PCWSTR(path.as_ptr()),
            Some(0),
            windows::Win32::System::Registry::KEY_READ,
            &mut key,
        )
        .is_err()
        {
            return None;
        }
        let mut found = None;
        for index in 0..64u32 {
            let mut name = [0u16; 256];
            let mut name_len = name.len() as u32;
            if windows::Win32::System::Registry::RegEnumKeyExW(
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
            if norm(&driver) == want {
                found = Some(driver);
                break;
            }
            let mut sub = windows::Win32::System::Registry::HKEY::default();
            let sub_path: Vec<u16> = driver.encode_utf16().chain(std::iter::once(0)).collect();
            if windows::Win32::System::Registry::RegOpenKeyExW(
                key,
                windows::core::PCWSTR(sub_path.as_ptr()),
                Some(0),
                windows::Win32::System::Registry::KEY_READ,
                &mut sub,
            )
            .is_err()
            {
                continue;
            }
            let clsid = read_reg_sz(sub, "CLSID").unwrap_or_default();
            let _ = windows::Win32::System::Registry::RegCloseKey(sub);
            if norm(&clsid) == want {
                found = Some(driver);
                break;
            }
        }
        let _ = windows::Win32::System::Registry::RegCloseKey(key);
        found
    }
}

fn read_reg_sz(key: windows::Win32::System::Registry::HKEY, name: &str) -> Option<String> {
    unsafe {
        let name_w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut buf = [0u16; 256];
        let mut size = (buf.len() * 2) as u32;
        if windows::Win32::System::Registry::RegGetValueW(
            key,
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR(name_w.as_ptr()),
            windows::Win32::System::Registry::RRF_RT_REG_SZ,
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

pub fn set_outputs(device_id: &str, maps: Vec<(Arc<BusRing>, i32, i32)>) {
    let key = norm(device_id);
    if key.is_empty() {
        return;
    }
    let mut hub = hub().lock().expect("asio hub");
    if maps.is_empty() {
        if let Some(device) = hub.devices.get_mut(&key) {
            device.shared.lock().expect("asio shared").outputs.clear();
            if device.is_idle() {
                stop_device(hub.devices.remove(&key));
            }
        }
        return;
    }
    let device = hub.ensure(&key, device_id);
    device.shared.lock().expect("asio shared").outputs = maps;
}

pub fn retain_outputs(keep: &HashSet<String>) {
    let keep: HashSet<String> = keep.iter().map(|id| norm(id)).collect();
    let mut hub = hub().lock().expect("asio hub");
    let keys: Vec<String> = hub.devices.keys().cloned().collect();
    for key in keys {
        if keep.contains(&key) {
            continue;
        }
        if let Some(device) = hub.devices.get_mut(&key) {
            device.shared.lock().expect("asio shared").outputs.clear();
            if device.is_idle() {
                stop_device(hub.devices.remove(&key));
            }
        }
    }
}

pub fn start_capture(
    spec: &AudioCaptureSpec,
    uploads: Arc<Mutex<AudioInputStore>>,
) -> Result<(), String> {
    if spec.kind != 0 && spec.kind != DEVICE_ASIO {
        return Err(format!("ASIO capture rejected backend {}", spec.kind));
    }
    if spec.mode == CAPTURE_MODE_PROCESS_LOOPBACK {
        return Err("ASIO input does not use process loopback".into());
    }
    if spec.mode == CAPTURE_MODE_ENDPOINT_LOOPBACK {
        return Err("ASIO input is device channels, not endpoint loopback".into());
    }
    if spec.mode != CAPTURE_MODE_MIC && spec.mode != 0 {
        return Err(format!("unknown ASIO capture mode {}", spec.mode));
    }
    if spec.device_id.trim().is_empty() {
        return Err("ASIO input needs a driver CLSID".into());
    }
    let (ins, _) = cpal_asio_io(&spec.device_id)
        .ok_or_else(|| "ASIO driver reported no input channels".to_string())?;
    if spec.map_left < 0 || spec.map_right < 0 || spec.map_left >= ins || spec.map_right >= ins {
        return Err(format!(
            "ASIO input L{} R{} is outside {ins} input channels",
            spec.map_left + 1,
            spec.map_right + 1
        ));
    }
    let key = norm(&spec.device_id);
    {
        let mut hub = hub().lock().expect("asio hub");
        let device = hub.ensure(&key, &spec.device_id);
        let mut shared = device.shared.lock().expect("asio shared");
        shared.error = None;
        shared.captures.insert(
            spec.id,
            Arc::new(AsioCapture {
                id: spec.id,
                map_left: spec.map_left,
                map_right: spec.map_right,
                uploads,
                pts: AtomicI64::new(0),
            }),
        );
    }
    for _ in 0..80 {
        if let Some(error) = snapshot_error(&key) {
            stop_capture(spec.id);
            return Err(error);
        }
        if snapshot_ready(&key) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
    if let Some(error) = snapshot_error(&key) {
        stop_capture(spec.id);
        return Err(error);
    }
    stop_capture(spec.id);
    Err("ASIO stream did not start".into())
}

pub fn stop_capture(id: u64) {
    let mut hub = hub().lock().expect("asio hub");
    let keys: Vec<String> = hub.devices.keys().cloned().collect();
    for key in keys {
        let idle = if let Some(device) = hub.devices.get_mut(&key) {
            device
                .shared
                .lock()
                .expect("asio shared")
                .captures
                .remove(&id);
            device.is_idle()
        } else {
            false
        };
        if idle {
            stop_device(hub.devices.remove(&key));
        }
    }
}

pub fn shutdown() {
    let mut hub = hub().lock().expect("asio hub");
    let devices: Vec<AsioDevice> = hub.devices.drain().map(|(_, device)| device).collect();
    drop(hub);
    for device in devices {
        stop_device(Some(device));
    }
}

pub fn probe_io_channels(device_id: &str) -> Result<(i32, i32), String> {
    let key = norm(device_id);
    if key.is_empty() {
        return Err("ASIO device id is empty".into());
    }
    if let Some(io) = live_or_cached_io(&key) {
        return Ok(io);
    }
    if let Some(io) = cpal_asio_io(device_id) {
        remember_io(&key, io.0, io.1);
        return Ok(io);
    }
    if hub_owns(&key) {
        return wait_live_io(&key);
    }
    Err("ASIO driver reported no channels".into())
}

fn snapshot_io(key: &str) -> Option<(i32, i32)> {
    let hub = hub().lock().ok()?;
    let device = hub.devices.get(key)?;
    let shared = device.shared.lock().ok()?;
    Some((shared.ins, shared.outs))
}

fn snapshot_error(key: &str) -> Option<String> {
    let hub = hub().lock().ok()?;
    let device = hub.devices.get(key)?;
    device.shared.lock().ok()?.error.clone()
}

fn snapshot_ready(key: &str) -> bool {
    let Ok(hub) = hub().lock() else {
        return false;
    };
    let Some(device) = hub.devices.get(key) else {
        return false;
    };
    device
        .shared
        .lock()
        .map(|guard| guard.ready)
        .unwrap_or(false)
}

fn live_or_cached_io(key: &str) -> Option<(i32, i32)> {
    if let Some((ins, outs)) = snapshot_io(key) {
        if ins > 0 || outs > 0 {
            return Some((ins, outs));
        }
    }
    cached_io(key)
}

fn hub_owns(key: &str) -> bool {
    hub()
        .lock()
        .ok()
        .and_then(|hub| hub.devices.get(key).map(AsioDevice::is_alive))
        .unwrap_or(false)
}

fn wait_live_io(key: &str) -> Result<(i32, i32), String> {
    for _ in 0..80 {
        if let Some(error) = snapshot_error(key) {
            return Err(error);
        }
        if let Some(io) = live_or_cached_io(key) {
            return Ok(io);
        }
        thread::sleep(Duration::from_millis(25));
    }
    if let Some(error) = snapshot_error(key) {
        return Err(error);
    }
    Err("ASIO driver reported no channels".into())
}

impl AsioHub {
    fn ensure(&mut self, key: &str, device_id: &str) -> &mut AsioDevice {
        if self.devices.get(key).is_some_and(AsioDevice::is_alive) {
            return self.devices.get_mut(key).expect("asio device");
        }
        let shared = if let Some(dead) = self.devices.remove(key) {
            let shared = Arc::clone(&dead.shared);
            if let Ok(mut guard) = shared.lock() {
                guard.ins = 0;
                guard.outs = 0;
                guard.error = None;
                guard.ready = false;
            }
            stop_device(Some(dead));
            shared
        } else {
            Arc::new(Mutex::new(AsioShared {
                outputs: Vec::new(),
                captures: HashMap::new(),
                ins: 0,
                outs: 0,
                error: None,
                ready: false,
            }))
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stop_t = Arc::clone(&stop);
        let shared_t = Arc::clone(&shared);
        let id = device_id.to_string();
        let join = thread::Builder::new()
            .name(format!("eiviz-asio-{key}"))
            .spawn(move || run_device_thread(&id, shared_t, &stop_t))
            .ok();
        self.devices
            .insert(key.to_string(), AsioDevice { shared, stop, join });
        self.devices.get_mut(key).expect("asio device")
    }
}

impl AsioDevice {
    fn is_alive(&self) -> bool {
        self.join.as_ref().is_some_and(|join| !join.is_finished())
    }

    fn is_idle(&self) -> bool {
        let shared = self.shared.lock().expect("asio shared");
        shared.outputs.is_empty() && shared.captures.is_empty()
    }
}

fn stop_device(device: Option<AsioDevice>) {
    let Some(mut device) = device else {
        return;
    };
    device.stop.store(true, Ordering::Relaxed);
    if let Some(join) = device.join.take() {
        crate::diag::join_timeout(join, Duration::from_secs(2), "asio");
    }
}

fn run_device_thread(device_id: &str, shared: Arc<Mutex<AsioShared>>, stop: &AtomicBool) {
    if let Err(error) = run_driver(device_id, Arc::clone(&shared), stop) {
        if let Ok(mut guard) = shared.lock() {
            guard.error = Some(error.clone());
        }
        crate::diag::error(&format!("asio {device_id}: {error}"));
    }
}

fn run_driver(
    device_id: &str,
    shared: Arc<Mutex<AsioShared>>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let name = registry_name_for_id(device_id)
        .ok_or_else(|| format!("ASIO driver name not found for {device_id}"))?;
    let device = find_cpal_asio_device(&name)?;
    let (ins, outs) = cpal_asio_io(device_id).unwrap_or((0, 0));
    if let Ok(mut guard) = shared.lock() {
        guard.ins = ins;
        guard.outs = outs;
    }

    let mut input: Option<cpal::Stream> = None;
    let mut output: Option<cpal::Stream> = None;
    let mut opened = false;
    while !stop.load(Ordering::Relaxed) {
        let (want_in, want_out) = {
            let guard = shared.lock().expect("asio shared");
            (!guard.captures.is_empty(), !guard.outputs.is_empty())
        };
        if !want_in && !want_out {
            drop(output.take());
            drop(input.take());
            opened = false;
            thread::sleep(Duration::from_millis(20));
            continue;
        }
        if opened {
            thread::sleep(Duration::from_millis(20));
            continue;
        }

        // ASIO allows one client. Open input, then output, so cpal creates one
        // duplex buffer set. A later bus route only updates the mix maps.
        let open_in = want_in || (want_out && ins > 0);
        let open_out = want_out || (want_in && outs > 0);
        if open_in {
            match open_cpal_asio_input(&device, Arc::clone(&shared)) {
                Ok(stream) => {
                    input = Some(stream);
                    if let Ok(mut guard) = shared.lock() {
                        guard.ready = true;
                        guard.error = None;
                    }
                }
                Err(error) => {
                    if want_in {
                        if let Ok(mut guard) = shared.lock() {
                            guard.error = Some(error.clone());
                        }
                        return Err(error);
                    }
                    crate::diag::warn(&format!("asio input {name}: {error}"));
                }
            }
        }
        if open_out {
            match open_cpal_asio_output(&device, Arc::clone(&shared)) {
                Ok(stream) => {
                    output = Some(stream);
                    crate::diag::info(&format!("asio output started: {name} ({outs} ch)"));
                    if let Ok(mut guard) = shared.lock() {
                        guard.ready = true;
                        guard.error = None;
                    }
                }
                Err(error) => {
                    crate::diag::error(&format!("asio output {name}: {error}"));
                    if want_out && input.is_none() {
                        if let Ok(mut guard) = shared.lock() {
                            guard.error = Some(error.clone());
                        }
                        return Err(error);
                    }
                }
            }
        }
        opened = input.is_some() || output.is_some();
        if !opened {
            return Err(format!("ASIO device '{name}' did not start"));
        }
        thread::sleep(Duration::from_millis(20));
    }
    drop(input);
    drop(output);
    Ok(())
}

fn find_cpal_asio_device(name: &str) -> Result<cpal::Device, String> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let host = cpal::host_from_id(cpal::HostId::Asio)
        .map_err(|error| format!("cpal ASIO host: {error}"))?;
    let devices = host
        .devices()
        .map_err(|error| format!("cpal ASIO devices: {error}"))?;
    let want = norm(name);
    for device in devices {
        let Ok(desc) = device.description() else {
            continue;
        };
        let driver = desc.driver().unwrap_or("").to_string();
        if norm(desc.name()) == want || norm(&driver) == want {
            return Ok(device);
        }
    }
    Err(format!("ASIO device '{name}' not found"))
}

fn asio_stream_config(
    device: &cpal::Device,
    output: bool,
) -> Result<(cpal::StreamConfig, cpal::SampleFormat, u32, usize), String> {
    use cpal::traits::DeviceTrait;

    let supported = if output {
        device
            .default_output_config()
            .map_err(|error| format!("ASIO output config: {error}"))?
    } else {
        device
            .default_input_config()
            .map_err(|error| format!("ASIO input config: {error}"))?
    };
    let format = supported.sample_format();
    let mut config = supported.config();
    if config.sample_rate == 0 {
        config.sample_rate = 48_000;
    }
    let channels = usize::from(config.channels.max(1));
    let rate = config.sample_rate.max(1);
    Ok((config, format, rate, channels))
}

fn open_cpal_asio_input(
    device: &cpal::Device,
    shared: Arc<Mutex<AsioShared>>,
) -> Result<cpal::Stream, String> {
    use cpal::traits::{DeviceTrait, StreamTrait};

    let (config, format, rate, channels) = asio_stream_config(device, false)?;
    let err_cb = {
        let shared = Arc::clone(&shared);
        move |error| {
            if let Ok(mut guard) = shared.lock() {
                guard.error = Some(format!("ASIO input stream: {error}"));
            }
        }
    };
    let stream = match format {
        cpal::SampleFormat::F32 => {
            let shared = Arc::clone(&shared);
            device.build_input_stream(
                config,
                move |data: &[f32], _| ingest_asio_input(&shared, data, channels, rate),
                err_cb,
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let shared = Arc::clone(&shared);
            device.build_input_stream(
                config,
                move |data: &[i16], _| {
                    let converted: Vec<f32> = data
                        .iter()
                        .map(|sample| *sample as f32 / 32768.0)
                        .collect();
                    ingest_asio_input(&shared, &converted, channels, rate);
                },
                err_cb,
                None,
            )
        }
        cpal::SampleFormat::I32 => {
            let shared = Arc::clone(&shared);
            device.build_input_stream(
                config,
                move |data: &[i32], _| {
                    let converted: Vec<f32> = data
                        .iter()
                        .map(|sample| *sample as f32 / 2_147_483_648.0)
                        .collect();
                    ingest_asio_input(&shared, &converted, channels, rate);
                },
                err_cb,
                None,
            )
        }
        cpal::SampleFormat::I24 => {
            let shared = Arc::clone(&shared);
            device.build_input_stream(
                config,
                move |data: &[cpal::I24], _| {
                    let converted: Vec<f32> = data
                        .iter()
                        .map(|sample| sample.inner() as f32 / 8_388_608.0)
                        .collect();
                    ingest_asio_input(&shared, &converted, channels, rate);
                },
                err_cb,
                None,
            )
        }
        other => return Err(format!("ASIO input format {other:?} is not supported")),
    }
    .map_err(|error| format!("ASIO input stream: {error}"))?;
    stream
        .play()
        .map_err(|error| format!("ASIO input play: {error}"))?;
    Ok(stream)
}

fn open_cpal_asio_output(
    device: &cpal::Device,
    shared: Arc<Mutex<AsioShared>>,
) -> Result<cpal::Stream, String> {
    use cpal::traits::{DeviceTrait, StreamTrait};

    let (config, format, rate, channels) = asio_stream_config(device, true)?;
    let err_cb = {
        let shared = Arc::clone(&shared);
        move |error| {
            if let Ok(mut guard) = shared.lock() {
                guard.error = Some(format!("ASIO output stream: {error}"));
            }
        }
    };
    let stream = match format {
        cpal::SampleFormat::F32 => {
            let shared = Arc::clone(&shared);
            device.build_output_stream(
                config,
                move |data: &mut [f32], _| render_asio_output(&shared, data, channels, rate),
                err_cb,
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let shared = Arc::clone(&shared);
            device.build_output_stream(
                config,
                move |data: &mut [i16], _| {
                    let mut mixed = vec![0.0f32; data.len()];
                    render_asio_output(&shared, &mut mixed, channels, rate);
                    super::pcm::f32_to_i16(&mixed, data);
                },
                err_cb,
                None,
            )
        }
        cpal::SampleFormat::I32 => {
            let shared = Arc::clone(&shared);
            device.build_output_stream(
                config,
                move |data: &mut [i32], _| {
                    let mut mixed = vec![0.0f32; data.len()];
                    render_asio_output(&shared, &mut mixed, channels, rate);
                    super::pcm::f32_to_i32(&mixed, data);
                },
                err_cb,
                None,
            )
        }
        cpal::SampleFormat::I24 => {
            let shared = Arc::clone(&shared);
            device.build_output_stream(
                config,
                move |data: &mut [cpal::I24], _| {
                    let mut mixed = vec![0.0f32; data.len()];
                    render_asio_output(&shared, &mut mixed, channels, rate);
                    for (slot, sample) in data.iter_mut().zip(mixed) {
                        let code = (sample.clamp(-1.0, 1.0) * 8_388_607.0) as i32;
                        *slot = cpal::I24::new_unchecked(code);
                    }
                },
                err_cb,
                None,
            )
        }
        other => return Err(format!("ASIO output format {other:?} is not supported")),
    }
    .map_err(|error| format!("ASIO output stream: {error}"))?;
    stream
        .play()
        .map_err(|error| format!("ASIO output play: {error}"))?;
    Ok(stream)
}

fn ingest_asio_input(shared: &Mutex<AsioShared>, interleaved: &[f32], channels: usize, rate: u32) {
    let captures = {
        let Ok(guard) = shared.lock() else {
            return;
        };
        guard.captures.values().cloned().collect::<Vec<_>>()
    };
    for capture in captures {
        let packet = super::pcm::interleaved_f32_packet(
            capture.pts.load(Ordering::Relaxed),
            rate as i32,
            interleaved,
            channels,
            capture.map_left.max(0) as usize,
            capture.map_right.max(0) as usize,
        );
        let frames = i64::from(packet.samples_per_channel.max(0));
        capture
            .uploads
            .lock()
            .expect("audio")
            .ingest_audio(capture.id, packet);
        capture.pts.store(
            capture
                .pts
                .load(Ordering::Relaxed)
                .saturating_add(frames * 10_000_000 / i64::from(rate.max(1))),
            Ordering::Relaxed,
        );
    }
}

fn render_asio_output(shared: &Mutex<AsioShared>, dest: &mut [f32], channels: usize, rate: u32) {
    let maps = {
        let Ok(guard) = shared.lock() else {
            dest.fill(0.0);
            return;
        };
        guard.outputs.clone()
    };
    let frames = dest.len() / channels.max(1);
    let mapped = pop_stereo_rate(&maps, frames, rate);
    super::pcm::mix_mapped_f32(dest, channels, &mapped);
}

fn norm(id: &str) -> String {
    id.trim()
        .trim_matches('{')
        .trim_end_matches('}')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::probe_io_channels;

    #[test]
    fn probe_empty_id_is_error() {
        assert!(probe_io_channels("").is_err());
        assert!(probe_io_channels("   ").is_err());
    }
}
