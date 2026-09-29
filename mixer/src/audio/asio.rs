use crate::guard::LockExt;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::AudioCaptureSpec;
use super::feed::CaptureFeed;
use super::graph::DEVICE_ASIO;
use super::info::{
    CAPTURE_MODE_ENDPOINT_LOOPBACK, CAPTURE_MODE_MIC, CAPTURE_MODE_PROCESS_LOOPBACK,
};
use super::pump::{OutputPump, Route};
use super::rt::{Published, PublishedReader, guard_callback};
use crate::upload::AudioInputStore;

struct AsioCapture {
    id: u64,
    map_left: usize,
    map_right: usize,
    uploads: Arc<Mutex<AudioInputStore>>,
    feed: CaptureFeed,
    pts: Mutex<i64>,
}

struct AsioShared {
    outputs: Vec<Route>,
    captures: HashMap<u64, Arc<AsioCapture>>,
    routes: Arc<Published<Vec<Route>>>,
    capture_list: Arc<Published<Vec<Arc<AsioCapture>>>>,
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
static IO_CACHE: std::sync::OnceLock<Mutex<HashMap<String, (i32, i32)>>> =
    std::sync::OnceLock::new();

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

/// Minimum time between full driver scans; scanning loads every installed driver.
const RESCAN_INTERVAL: Duration = Duration::from_secs(5);

fn cpal_asio_io(device_id: &str) -> Option<(i32, i32)> {
    let names = asio_lookup_names(device_id);
    if let Some(io) = lookup_table(&cpal_asio_table(false), &names) {
        return Some(io);
    }
    // A driver installed or plugged in after the first scan is not in the cached table.
    static LAST_RESCAN: std::sync::OnceLock<Mutex<Option<Instant>>> = std::sync::OnceLock::new();
    {
        let mut last = LAST_RESCAN
            .get_or_init(|| Mutex::new(None))
            .lock_or_recover();
        if last.is_some_and(|at| at.elapsed() < RESCAN_INTERVAL) {
            return None;
        }
        *last = Some(Instant::now());
    }
    lookup_table(&cpal_asio_table(true), &names)
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
    static TABLE: std::sync::OnceLock<Mutex<HashMap<String, (i32, i32)>>> =
        std::sync::OnceLock::new();
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

impl AsioShared {
    fn publish(&self) {
        self.routes.set(self.outputs.clone());
        self.capture_list
            .set(self.captures.values().cloned().collect());
    }
}

pub fn set_outputs(device_id: &str, maps: Vec<Route>) {
    let key = norm(device_id);
    if key.is_empty() {
        return;
    }
    let mut hub = hub().lock_or_recover();
    if maps.is_empty() {
        if let Some(device) = hub.devices.get_mut(&key) {
            let mut shared = device.shared.lock_or_recover();
            shared.outputs.clear();
            shared.publish();
            drop(shared);
            if device.is_idle() {
                stop_device(hub.devices.remove(&key));
            }
        }
        return;
    }
    let device = hub.ensure(&key, device_id);
    let mut shared = device.shared.lock_or_recover();
    shared.outputs = maps;
    shared.publish();
}

pub fn retain_outputs(keep: &HashSet<String>) {
    let keep: HashSet<String> = keep.iter().map(|id| norm(id)).collect();
    let mut hub = hub().lock_or_recover();
    let keys: Vec<String> = hub.devices.keys().cloned().collect();
    for key in keys {
        if keep.contains(&key) {
            continue;
        }
        if let Some(device) = hub.devices.get_mut(&key) {
            let mut shared = device.shared.lock_or_recover();
            shared.outputs.clear();
            shared.publish();
            drop(shared);
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
        let mut hub = hub().lock_or_recover();
        let device = hub.ensure(&key, &spec.device_id);
        let mut shared = device.shared.lock_or_recover();
        shared.error = None;
        shared.captures.insert(
            spec.id,
            Arc::new(AsioCapture {
                id: spec.id,
                map_left: spec.map_left as usize,
                map_right: spec.map_right as usize,
                uploads,
                feed: CaptureFeed::new(48_000),
                pts: Mutex::new(0),
            }),
        );
        shared.publish();
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
    let mut hub = hub().lock_or_recover();
    let keys: Vec<String> = hub.devices.keys().cloned().collect();
    for key in keys {
        let idle = if let Some(device) = hub.devices.get_mut(&key) {
            let mut shared = device.shared.lock_or_recover();
            shared.captures.remove(&id);
            shared.publish();
            drop(shared);
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
    let mut hub = hub().lock_or_recover();
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
                routes: Published::new(Vec::new()),
                capture_list: Published::new(Vec::new()),
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
        let shared = self.shared.lock_or_recover();
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

const REOPEN_BACKOFF_MIN: Duration = Duration::from_millis(250);
const REOPEN_BACKOFF_MAX: Duration = Duration::from_secs(2);

fn run_driver(
    device_id: &str,
    shared: Arc<Mutex<AsioShared>>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let name = registry_name_for_id(device_id)
        .ok_or_else(|| format!("ASIO driver name not found for {device_id}"))?;

    let mut input: Option<cpal::Stream> = None;
    let mut output: Option<cpal::Stream> = None;
    let mut opened = false;
    let mut ever_opened = false;
    let mut retry_at = Instant::now();
    let mut backoff = REOPEN_BACKOFF_MIN;
    while !stop.load(Ordering::Relaxed) {
        let (want_in, want_out) = {
            let guard = shared.lock_or_recover();
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
            // A driver reset or a removed device surfaces as a stream error. Tear down and
            // reopen instead of leaving both directions silent until the app restarts.
            let stream_error = shared.lock_or_recover().error.take();
            if let Some(error) = stream_error {
                crate::diag::error(&format!("asio {name}: {error}; reopening"));
                drop(output.take());
                drop(input.take());
                shared.lock_or_recover().ready = false;
                opened = false;
                retry_at = Instant::now() + backoff;
                backoff = (backoff * 2).min(REOPEN_BACKOFF_MAX);
                continue;
            }
            drain_captures(&shared);
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        if Instant::now() < retry_at {
            thread::sleep(Duration::from_millis(20));
            continue;
        }

        let device = match find_cpal_asio_device(&name) {
            Ok(device) => device,
            Err(error) if ever_opened => {
                crate::diag::warn(&format!("asio {name}: {error}; retrying"));
                retry_at = Instant::now() + backoff;
                backoff = (backoff * 2).min(REOPEN_BACKOFF_MAX);
                continue;
            }
            Err(error) => return Err(error),
        };
        let (ins, outs) = cpal_asio_io(device_id).unwrap_or((0, 0));
        {
            let mut guard = shared.lock_or_recover();
            guard.ins = ins;
            guard.outs = outs;
        }

        // ASIO allows one client. Open input, then output, so cpal creates one
        // duplex buffer set. A later bus route only updates the mix maps.
        let open_in = want_in || (want_out && ins > 0);
        let open_out = want_out || (want_in && outs > 0);
        let mut failure = None;
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
                        failure = Some(error);
                    } else {
                        crate::diag::warn(&format!("asio input {name}: {error}"));
                    }
                }
            }
        }
        if failure.is_none() && open_out {
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
                        failure = Some(error);
                    }
                }
            }
        }
        if failure.is_none() && input.is_none() && output.is_none() {
            failure = Some(format!("ASIO device '{name}' did not start"));
        }
        if let Some(error) = failure {
            drop(output.take());
            drop(input.take());
            if !ever_opened {
                shared.lock_or_recover().error = Some(error.clone());
                return Err(error);
            }
            crate::diag::warn(&format!("asio {name}: {error}; retrying"));
            retry_at = Instant::now() + backoff;
            backoff = (backoff * 2).min(REOPEN_BACKOFF_MAX);
            continue;
        }
        opened = true;
        ever_opened = true;
        backoff = REOPEN_BACKOFF_MIN;
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
    let config = supported.config();
    if config.sample_rate == 0 {
        return Err("ASIO driver reported a sample rate of 0 Hz".into());
    }
    if config.channels == 0 {
        return Err("ASIO driver reported 0 channels".into());
    }
    let channels = usize::from(config.channels);
    let rate = config.sample_rate;
    Ok((config, format, rate, channels))
}

fn drain_captures(shared: &Mutex<AsioShared>) {
    let captures: Vec<Arc<AsioCapture>> = shared
        .lock_or_recover()
        .captures
        .values()
        .cloned()
        .collect();
    for capture in captures {
        let mut pts = capture.pts.lock_or_recover();
        capture.feed.drain(&capture.uploads, capture.id, &mut pts);
    }
}

fn open_cpal_asio_input(
    device: &cpal::Device,
    shared: Arc<Mutex<AsioShared>>,
) -> Result<cpal::Stream, String> {
    use cpal::traits::StreamTrait;

    let (config, format, rate, channels) = asio_stream_config(device, false)?;
    let capture_list = Arc::clone(&shared.lock_or_recover().capture_list);
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
            build_asio_input::<f32>(device, config, capture_list, channels, rate, |s| s, err_cb)
        }
        cpal::SampleFormat::I16 => build_asio_input::<i16>(
            device,
            config,
            capture_list,
            channels,
            rate,
            |s| s as f32 / 32768.0,
            err_cb,
        ),
        cpal::SampleFormat::I32 => build_asio_input::<i32>(
            device,
            config,
            capture_list,
            channels,
            rate,
            |s| s as f32 / 2_147_483_648.0,
            err_cb,
        ),
        cpal::SampleFormat::I24 => build_asio_input::<cpal::I24>(
            device,
            config,
            capture_list,
            channels,
            rate,
            |s| s.inner() as f32 / 8_388_608.0,
            err_cb,
        ),
        other => return Err(format!("ASIO input format {other:?} is not supported")),
    }
    .map_err(|error| format!("ASIO input stream: {error}"))?;
    stream
        .play()
        .map_err(|error| format!("ASIO input play: {error}"))?;
    Ok(stream)
}

fn build_asio_input<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    capture_list: Arc<Published<Vec<Arc<AsioCapture>>>>,
    channels: usize,
    rate: u32,
    convert: fn(T) -> f32,
    err_cb: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + Send + 'static,
{
    use cpal::traits::DeviceTrait;

    let mut captures = PublishedReader::<Vec<Arc<AsioCapture>>>::new();
    let mut converted: Vec<f32> = Vec::with_capacity(16_384);
    let mut scratch: Vec<f32> = Vec::with_capacity(16_384);
    let panicked = AtomicBool::new(false);
    device.build_input_stream(
        config,
        move |data: &[T], _| {
            guard_callback("asio input", &panicked, || {
                let (list, _) = captures.get(&capture_list);
                if list.is_empty() {
                    return;
                }
                converted.clear();
                converted.extend(data.iter().map(|sample| convert(*sample)));
                for capture in list {
                    capture.feed.set_rate(rate);
                    capture.feed.push_mapped(
                        &converted,
                        channels,
                        capture.map_left,
                        capture.map_right,
                        &mut scratch,
                    );
                }
            });
        },
        err_cb,
        None,
    )
}

fn open_cpal_asio_output(
    device: &cpal::Device,
    shared: Arc<Mutex<AsioShared>>,
) -> Result<cpal::Stream, String> {
    use cpal::traits::StreamTrait;

    let (config, format, rate, channels) = asio_stream_config(device, true)?;
    let routes = Arc::clone(&shared.lock_or_recover().routes);
    let err_cb = {
        let shared = Arc::clone(&shared);
        move |error| {
            if let Ok(mut guard) = shared.lock() {
                guard.error = Some(format!("ASIO output stream: {error}"));
            }
        }
    };
    let stream = match format {
        cpal::SampleFormat::F32 => build_asio_output::<f32>(
            device,
            config,
            routes,
            channels,
            rate,
            |src, dst| dst.copy_from_slice(src),
            err_cb,
        ),
        cpal::SampleFormat::I16 => build_asio_output::<i16>(
            device,
            config,
            routes,
            channels,
            rate,
            super::pcm::f32_to_i16,
            err_cb,
        ),
        cpal::SampleFormat::I32 => build_asio_output::<i32>(
            device,
            config,
            routes,
            channels,
            rate,
            super::pcm::f32_to_i32,
            err_cb,
        ),
        cpal::SampleFormat::I24 => build_asio_output::<cpal::I24>(
            device,
            config,
            routes,
            channels,
            rate,
            |src, dst| {
                for (slot, sample) in dst.iter_mut().zip(src) {
                    let code = (sample.clamp(-1.0, 1.0) * 8_388_607.0) as i32;
                    *slot = cpal::I24::new_unchecked(code);
                }
            },
            err_cb,
        ),
        other => return Err(format!("ASIO output format {other:?} is not supported")),
    }
    .map_err(|error| format!("ASIO output stream: {error}"))?;
    stream
        .play()
        .map_err(|error| format!("ASIO output play: {error}"))?;
    Ok(stream)
}

fn build_asio_output<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    routes: Arc<Published<Vec<Route>>>,
    channels: usize,
    rate: u32,
    convert: fn(&[f32], &mut [T]),
    err_cb: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + Send + 'static,
{
    use cpal::traits::DeviceTrait;

    let mut pump = OutputPump::new();
    let mut mix: Vec<f32> = Vec::with_capacity(16_384);
    let panicked = AtomicBool::new(false);
    device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let rendered = guard_callback("asio output", &panicked, || {
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
