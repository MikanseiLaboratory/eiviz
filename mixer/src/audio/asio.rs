use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::core::{IUnknown, Interface, GUID, HRESULT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};

use super::graph::{BusRing, DEVICE_ASIO};
use super::info::{
    CAPTURE_MODE_ENDPOINT_LOOPBACK, CAPTURE_MODE_MIC, CAPTURE_MODE_PROCESS_LOOPBACK,
};
use super::pop_stereo_rate;
use super::AudioCaptureSpec;
use crate::upload::{AudioInputStore, AudioPacket};

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

struct AsioCapture {
    id: u64,
    map_left: i32,
    map_right: i32,
    uploads: Arc<Mutex<AudioInputStore>>,
    pts: AtomicI64,
}

struct AsioRt {
    maps: Vec<(Arc<BusRing>, i32, i32)>,
    captures: Vec<Arc<AsioCapture>>,
    infos: Vec<AsioBufferInfo>,
    sample_types: Vec<i32>,
    buffer_size: i32,
    rate: f64,
    asio: usize,
    output_ready: Option<unsafe extern "system" fn(*mut Iasio) -> i32>,
}

struct AsioShared {
    outputs: Vec<(Arc<BusRing>, i32, i32)>,
    captures: HashMap<u64, Arc<AsioCapture>>,
    ins: i32,
    outs: i32,
    error: Option<String>,
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
static SYS_HANDLE: AtomicIsize = AtomicIsize::new(0);

fn hub() -> &'static Mutex<AsioHub> {
    HUB.get_or_init(|| {
        Mutex::new(AsioHub {
            devices: HashMap::new(),
        })
    })
}

pub fn remember_sys_handle(handle: isize) {
    if handle == 0 {
        return;
    }
    let root = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetAncestor(
            windows::Win32::Foundation::HWND(handle as *mut core::ffi::c_void),
            windows::Win32::UI::WindowsAndMessaging::GA_ROOT,
        )
    };
    let value = if root.0.is_null() {
        handle
    } else {
        root.0 as isize
    };
    SYS_HANDLE.store(value, Ordering::Relaxed);
}

fn sys_handle() -> *mut core::ffi::c_void {
    let stored = SYS_HANDLE.load(Ordering::Relaxed);
    if stored != 0 {
        return stored as *mut core::ffi::c_void;
    }
    unsafe {
        let foreground = windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
        if !foreground.0.is_null() {
            return foreground.0;
        }
        windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow().0
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
    let key = norm(&spec.device_id);
    {
        let mut hub = hub().lock().expect("asio hub");
        hub.ensure(&key, &spec.device_id);
    }
    let (ins, _) = match wait_io_channels(&spec.device_id) {
        Ok(io) => io,
        Err(error) => {
            reap_idle(&key);
            return Err(error);
        }
    };
    if spec.map_left < 0 || spec.map_right < 0 || spec.map_left >= ins || spec.map_right >= ins {
        reap_idle(&key);
        return Err(format!(
            "ASIO input L{} R{} is outside {ins} input channels",
            spec.map_left + 1,
            spec.map_right + 1
        ));
    }
    let mut hub = hub().lock().expect("asio hub");
    let device = hub.ensure(&key, &spec.device_id);
    device.shared.lock().expect("asio shared").captures.insert(
        spec.id,
        Arc::new(AsioCapture {
            id: spec.id,
            map_left: spec.map_left,
            map_right: spec.map_right,
            uploads,
            pts: AtomicI64::new(0),
        }),
    );
    Ok(())
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

pub fn io_channels(device_id: &str) -> Result<(i32, i32), String> {
    let key = norm(device_id);
    if key.is_empty() {
        return Err("ASIO device id is empty".into());
    }
    {
        let mut hub = hub().lock().expect("asio hub");
        hub.ensure(&key, device_id);
    }
    for _ in 0..50 {
        if let Some(error) = snapshot_error(&key) {
            return Err(error);
        }
        if let Some((ins, outs)) = snapshot_io(&key) {
            if ins > 0 || outs > 0 {
                return Ok((ins, outs));
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    if let Some(error) = snapshot_error(&key) {
        return Err(error);
    }
    if let Some((ins, outs)) = snapshot_io(&key) {
        if ins > 0 || outs > 0 {
            return Ok((ins, outs));
        }
    }
    Err("ASIO driver reported no channels".into())
}

fn wait_io_channels(device_id: &str) -> Result<(i32, i32), String> {
    io_channels(device_id)
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
            }))
        };
        let rt = Arc::new(Mutex::new(AsioRt {
            maps: Vec::new(),
            captures: Vec::new(),
            infos: Vec::new(),
            sample_types: Vec::new(),
            buffer_size: 0,
            rate: 48_000.0,
            asio: 0,
            output_ready: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let stop_t = Arc::clone(&stop);
        let shared_t = Arc::clone(&shared);
        let rt_t = Arc::clone(&rt);
        let id = device_id.to_string();
        let join = thread::Builder::new()
            .name(format!("eiviz-asio-{key}"))
            .spawn(move || run_device_thread(&id, shared_t, rt_t, &stop_t))
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

fn reap_idle(key: &str) {
    let mut hub = hub().lock().expect("asio hub");
    let idle = hub.devices.get(key).is_some_and(AsioDevice::is_idle);
    if idle {
        stop_device(hub.devices.remove(key));
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

fn run_device_thread(
    device_id: &str,
    shared: Arc<Mutex<AsioShared>>,
    rt: Arc<Mutex<AsioRt>>,
    stop: &AtomicBool,
) {
    if let Err(error) = run_driver(device_id, Arc::clone(&shared), rt, stop) {
        if let Ok(mut guard) = shared.lock() {
            guard.error = Some(error.clone());
        }
        crate::diag::error(&format!("asio {device_id}: {error}"));
    }
}

fn run_driver(
    device_id: &str,
    shared: Arc<Mutex<AsioShared>>,
    rt: Arc<Mutex<AsioRt>>,
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
        let (ins, outs) = match asio_init_channels(vtbl, asio) {
            Ok(io) => io,
            Err(error) => {
                let _ = (vtbl.release)(asio);
                return Err(error);
            }
        };
        {
            let mut guard = shared.lock().expect("asio shared");
            guard.ins = ins;
            guard.outs = outs;
            if !guard.outputs.is_empty() && outs <= 0 {
                let _ = (vtbl.release)(asio);
                return Err("ASIO has no outputs".into());
            }
            if !guard.captures.is_empty() && ins <= 0 {
                let _ = (vtbl.release)(asio);
                return Err("ASIO has no inputs".into());
            }
        }
        let _ = (vtbl.set_sample_rate)(asio, 48_000.0);
        let mut rate = 48_000.0f64;
        let _ = (vtbl.get_sample_rate)(asio, &mut rate);
        if rate < 1.0 {
            rate = 48_000.0;
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
        let mut infos = Vec::new();
        let mut types = Vec::new();
        append_buffers(vtbl, asio, 1, ins, &mut infos, &mut types);
        append_buffers(vtbl, asio, 0, outs, &mut infos, &mut types);
        if infos.is_empty() {
            let _ = (vtbl.release)(asio);
            return Err("ASIO has no channels".into());
        }
        {
            let mut slot = rt.lock().expect("asio rt");
            slot.infos.clear();
            slot.sample_types = types;
            slot.buffer_size = buffer_size;
            slot.rate = rate;
            slot.asio = asio as usize;
            slot.output_ready = Some(vtbl.output_ready);
        }
        *RT.lock().expect("asio rt slot") = Some(Arc::clone(&rt));
        let mut callbacks = AsioCallbacks {
            buffer_switch: Some(buffer_switch),
            sample_rate_did_change: Some(sample_rate_did_change),
            asio_message: Some(asio_message),
            buffer_switch_time_info: Some(buffer_switch_time_info),
        };
        if (vtbl.create_buffers)(
            asio,
            infos.as_mut_ptr(),
            infos.len() as i32,
            buffer_size,
            &mut callbacks,
        ) != ASIO_OK
        {
            *RT.lock().expect("asio rt slot") = None;
            let _ = (vtbl.release)(asio);
            return Err("ASIO createBuffers failed".into());
        }
        rt.lock().expect("asio rt").infos = infos;
        if (vtbl.start)(asio) != ASIO_OK {
            let _ = (vtbl.dispose_buffers)(asio);
            *RT.lock().expect("asio rt slot") = None;
            let _ = (vtbl.release)(asio);
            return Err("ASIO start failed".into());
        }
        while !stop.load(Ordering::Relaxed) {
            sync_rt(&shared, &rt);
            thread::sleep(Duration::from_millis(20));
        }
        let _ = (vtbl.stop)(asio);
        let _ = (vtbl.dispose_buffers)(asio);
        let _ = (vtbl.release)(asio);
        *RT.lock().expect("asio rt slot") = None;
        let _ = unk;
        Ok(())
    }
}

fn append_buffers(
    vtbl: &IasioVtbl,
    asio: *mut Iasio,
    is_input: i32,
    count: i32,
    infos: &mut Vec<AsioBufferInfo>,
    types: &mut Vec<i32>,
) {
    for ch in 0..count {
        infos.push(AsioBufferInfo {
            is_input,
            channel_num: ch,
            buffers: [std::ptr::null_mut(), std::ptr::null_mut()],
        });
        let mut info = AsioChannelInfo {
            channel: ch,
            is_input,
            is_active: 0,
            channel_group: 0,
            sample_type: ASIOST_FLOAT32_LSB,
            name: [0; 32],
        };
        unsafe {
            let _ = (vtbl.get_channel_info)(asio, &mut info);
        }
        types.push(info.sample_type);
    }
}

fn sync_rt(shared: &Arc<Mutex<AsioShared>>, rt: &Arc<Mutex<AsioRt>>) {
    let shared = shared.lock().expect("asio shared");
    let mut slot = rt.lock().expect("asio rt");
    slot.maps = shared.outputs.clone();
    slot.captures = shared.captures.values().cloned().collect();
}

static RT: Mutex<Option<Arc<Mutex<AsioRt>>>> = Mutex::new(None);

unsafe fn asio_init_channels(vtbl: &IasioVtbl, asio: *mut Iasio) -> Result<(i32, i32), String> {
    // IASIO::init is ASIOBool (1 = success). Some drivers return ASE_OK (0) on
    // success, so getChannels is the authority.
    unsafe {
        let _ = (vtbl.init)(asio, sys_handle());
        let mut ins = 0i32;
        let mut outs = 0i32;
        if (vtbl.get_channels)(asio, &mut ins, &mut outs) != ASIO_OK {
            return Err("ASIO init or getChannels failed".into());
        }
        Ok((ins, outs))
    }
}

unsafe extern "C" fn buffer_switch(index: i32, _direct: i32) {
    let Some(rt) = RT.lock().ok().and_then(|guard| guard.clone()) else {
        return;
    };
    let guard = rt.lock().expect("asio rt");
    let frames = guard.buffer_size.max(1) as usize;
    let rate = guard.rate.max(1.0) as u32;
    let mapped = pop_stereo_rate(&guard.maps, frames, rate);
    let infos = guard.infos.clone();
    let types = guard.sample_types.clone();
    let captures = guard.captures.clone();
    let asio = guard.asio as *mut Iasio;
    let output_ready = guard.output_ready;
    drop(guard);
    let idx = if index == 0 { 0usize } else { 1usize };
    let mut input_ptrs = HashMap::<i32, (*mut core::ffi::c_void, i32)>::new();
    for (slot, info) in infos.iter().enumerate() {
        let ptr = info.buffers[idx];
        if ptr.is_null() {
            continue;
        }
        let ty = types.get(slot).copied().unwrap_or(ASIOST_FLOAT32_LSB);
        if info.is_input != 0 {
            input_ptrs.insert(info.channel_num, (ptr, ty));
        } else {
            fill_asio_channel(ptr, ty, frames, &mapped, info.channel_num as usize);
        }
    }
    for capture in captures {
        let left = read_asio_channel(input_ptrs.get(&capture.map_left).copied(), frames);
        let right = read_asio_channel(input_ptrs.get(&capture.map_right).copied(), frames);
        let mut pcm = left;
        pcm.extend(right);
        let pts = capture.pts.load(Ordering::Relaxed);
        capture.uploads.lock().expect("audio").ingest_audio(
            capture.id,
            AudioPacket {
                timestamp: pts,
                sample_rate: rate as i32,
                channels: 2,
                samples_per_channel: frames as i32,
                pcm_planar_f32: pcm,
            },
        );
        capture.pts.store(
            pts.saturating_add(i64::from(frames as u32) * 10_000_000 / i64::from(rate.max(1))),
            Ordering::Relaxed,
        );
    }
    if let Some(output_ready) = output_ready {
        if !asio.is_null() {
            let _ = unsafe { output_ready(asio) };
        }
    }
}

fn read_asio_channel(src: Option<(*mut core::ffi::c_void, i32)>, frames: usize) -> Vec<f32> {
    let Some((ptr, ty)) = src else {
        return vec![0.0; frames];
    };
    if ptr.is_null() {
        return vec![0.0; frames];
    }
    unsafe {
        match ty {
            ASIOST_INT16_LSB => std::slice::from_raw_parts(ptr as *const i16, frames)
                .iter()
                .map(|sample| *sample as f32 / 32768.0)
                .collect(),
            ASIOST_INT24_LSB => {
                let bytes = std::slice::from_raw_parts(ptr as *const u8, frames * 3);
                (0..frames)
                    .map(|i| {
                        let value = i32::from_le_bytes([
                            bytes[i * 3],
                            bytes[i * 3 + 1],
                            bytes[i * 3 + 2],
                            0,
                        ]) << 8
                            >> 8;
                        value as f32 / 8_388_608.0
                    })
                    .collect()
            }
            ASIOST_INT32_LSB => std::slice::from_raw_parts(ptr as *const i32, frames)
                .iter()
                .map(|sample| *sample as f32 / 2_147_483_648.0)
                .collect(),
            ASIOST_FLOAT64_LSB => std::slice::from_raw_parts(ptr as *const f64, frames)
                .iter()
                .map(|sample| *sample as f32)
                .collect(),
            _ => std::slice::from_raw_parts(ptr as *const f32, frames).to_vec(),
        }
    }
}

fn fill_asio_channel(
    ptr: *mut core::ffi::c_void,
    ty: i32,
    frames: usize,
    mapped: &HashMap<(i32, i32), Vec<(f32, f32)>>,
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
    if let Some(rt) = RT.lock().ok().and_then(|guard| guard.clone()) {
        rt.lock().expect("asio rt").rate = rate;
    }
}

unsafe extern "C" fn asio_message(
    selector: i32,
    value: i32,
    _message: *mut core::ffi::c_void,
    _opt: *mut f64,
) -> i32 {
    match selector {
        1 => i32::from(matches!(value, 2 | 6 | 7)),
        2 => 2,
        6 | 7 => 1,
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

fn norm(id: &str) -> String {
    id.trim()
        .trim_matches('{')
        .trim_end_matches('}')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::parse_guid;

    #[test]
    fn parse_guid_accepts_braces() {
        let guid = parse_guid("{453661B3-88C3-45C4-8877-4C03B6490C33}").unwrap();
        assert_eq!(
            guid,
            windows::core::GUID::from_u128(0x4536_61b3_88c3_45c4_8877_4c03_b649_0c33)
        );
    }
}
