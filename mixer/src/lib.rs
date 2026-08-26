#![deny(unsafe_op_in_unsafe_fn)]

mod abi;
mod compose;
mod device;
mod omt;
mod present;
mod readback;
mod upload;

pub use abi::{
    AudioPeak, MixerStats, OverlayDesc, Rect, UnitState, ERR_ALREADY_CREATED, ERR_DEVICE,
    ERR_INVALID_ARGUMENT, ERR_IO, ERR_NOT_CREATED, GEN_BARS, GEN_SOLID, OK, OUTPUT_MULTIVIEW,
    OUTPUT_PREVIEW, OUTPUT_PROGRAM, OUT_DECKLINK, OUT_NDI, OUT_OMT, SCENE_BASE, SRC_BARS, SRC_BLACK,
    SRC_BLUE, SRC_COLOR, SRC_KIND_INPUT, SRC_KIND_MU_MULTIVIEW, SRC_KIND_MU_PREVIEW,
    SRC_KIND_MU_PROGRAM, SRC_KIND_SCENE,
};

use std::collections::HashMap;
use std::ffi::{c_char, CStr};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use compose::{Composer, Generator};
use device::GpuDevice;
use omt::{OmtReceiver, ProgramSender};
use present::Presenters;
use readback::ReadbackStore;
use upload::{CpuFormat, UploadStore};

struct AutoTransition {
    from: f32,
    to: f32,
    start: Instant,
    duration: Duration,
    swap: bool,
}

struct LiveUnit {
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    state: UnitState,
    auto: Option<AutoTransition>,
}

struct LiveOutput {
    transport: u32,
    source_kind: u32,
    source_id: u64,
    unit_id: u64,
    sender: ProgramSender,
}

struct Shared {
    master_fps_num: u32,
    master_fps_den: u32,
    units: HashMap<u64, LiveUnit>,
    scenes: HashMap<u64, (u32, u32, Vec<crate::abi::OverlayDesc>)>,
    uploads: Arc<Mutex<UploadStore>>,
    receivers: HashMap<u64, OmtReceiver>,
    outputs: HashMap<u64, LiveOutput>,
    generators: HashMap<u64, Generator>,
    last_error: String,
    last_render_ms: f32,
    frame_buffer_frames: u32,
}

enum GpuCmd {
    Attach {
        unit_id: u64,
        kind: u32,
        hwnd: isize,
        width: u32,
        height: u32,
        reply: mpsc::Sender<i32>,
    },
    Resize {
        unit_id: u64,
        kind: u32,
        hwnd: isize,
        width: u32,
        height: u32,
    },
    Detach {
        unit_id: u64,
        kind: u32,
        hwnd: isize,
    },
    DetachUnit {
        unit_id: u64,
    },
    AttachMonitor {
        monitor_id: u64,
        source_id: u64,
        hwnd: isize,
        width: u32,
        height: u32,
        reply: mpsc::Sender<i32>,
    },
    ResizeMonitor {
        monitor_id: u64,
        width: u32,
        height: u32,
    },
    DetachMonitor {
        monitor_id: u64,
    },
    SetMonitorSource {
        monitor_id: u64,
        source_id: u64,
    },
    Shutdown,
}

struct Mixer {
    shared: Arc<Mutex<Shared>>,
    cmds: mpsc::Sender<GpuCmd>,
    render: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

static MIXER: OnceLock<Mutex<Option<Mixer>>> = OnceLock::new();

fn mixer_slot() -> &'static Mutex<Option<Mixer>> {
    MIXER.get_or_init(|| Mutex::new(None))
}

fn with_mixer<T>(f: impl FnOnce(&Mixer) -> T) -> Result<T, i32> {
    let slot = mixer_slot().lock().expect("mixer mutex poisoned");
    match slot.as_ref() {
        Some(mixer) => Ok(f(mixer)),
        None => Err(ERR_NOT_CREATED),
    }
}

fn set_error(shared: &Mutex<Shared>, message: impl Into<String>) {
    shared.lock().expect("shared").last_error = message.into();
}

/// Creates the DX12-only wgpu device. No other backend is ever attempted.
#[unsafe(no_mangle)]
pub extern "C" fn mixer_create(_adapter_luid: u64, fps_num: u32, fps_den: u32) -> i32 {
    if fps_num == 0 || fps_den == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    let mut slot = mixer_slot().lock().expect("mixer mutex poisoned");
    if slot.is_some() {
        return ERR_ALREADY_CREATED;
    }
    let device = match GpuDevice::new_dx12_only() {
        Ok(device) => device,
        Err(_) => return ERR_DEVICE,
    };
    let uploads = Arc::new(Mutex::new(UploadStore::default()));
    let shared = Arc::new(Mutex::new(Shared {
        master_fps_num: fps_num,
        master_fps_den: fps_den,
        units: HashMap::new(),
        scenes: HashMap::new(),
        uploads: Arc::clone(&uploads),
        receivers: HashMap::new(),
        outputs: HashMap::new(),
        generators: HashMap::new(),
        last_error: String::new(),
        last_render_ms: 0.0,
        frame_buffer_frames: 3,
    }));
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let render_shared = Arc::clone(&shared);
    let render_stop = Arc::clone(&stop);
    let render = thread::Builder::new()
        .name("eiviz-render".into())
        .spawn(move || {
            render_loop(device, fps_num, fps_den, render_shared, uploads, rx, render_stop);
        })
        .expect("render thread");
    *slot = Some(Mixer {
        shared,
        cmds: tx,
        render: Some(render),
        stop,
    });
    OK
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_destroy() {
    let mut slot = mixer_slot().lock().expect("mixer mutex poisoned");
    if let Some(mut mixer) = slot.take() {
        mixer.stop.store(true, Ordering::Relaxed);
        let _ = mixer.cmds.send(GpuCmd::Shutdown);
        if let Some(join) = mixer.render.take() {
            let _ = join.join();
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_ping() -> u32 {
    0x4549_5649
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_create_unit(unit_id: u64, width: u32, height: u32) -> i32 {
    if width == 0 || height == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        let fps_num = shared.master_fps_num;
        let fps_den = shared.master_fps_den;
        shared.units.insert(
            unit_id,
            LiveUnit {
                width,
                height,
                fps_num,
                fps_den,
                state: UnitState {
                    program_source: SRC_BLACK,
                    preview_source: SRC_BARS,
                    ..UnitState::default()
                },
                auto: None,
            },
        );
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_define_scene(
    scene_id: u64,
    width: u32,
    height: u32,
    count: u32,
    layers: *const OverlayDesc,
) -> i32 {
    if width == 0 || height == 0 || count > 64 {
        return ERR_INVALID_ARGUMENT;
    }
    if count > 0 && layers.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let copied = if count == 0 {
        Vec::new()
    } else {
        // SAFETY: caller keeps count OverlayDesc values readable.
        unsafe { std::slice::from_raw_parts(layers, count as usize) }.to_vec()
    };
    with_mixer(|mixer| {
        mixer
            .shared
            .lock()
            .expect("shared")
            .scenes
            .insert(scene_id, (width, height, copied));
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_destroy_scene(scene_id: u64) -> i32 {
    with_mixer(|mixer| {
        mixer.shared.lock().expect("shared").scenes.remove(&scene_id);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_define_generator(
    id: u64,
    kind: u32,
    r: f32,
    g: f32,
    b: f32,
    a: f32,
    scroll: u32,
) -> i32 {
    if kind != GEN_SOLID && kind != GEN_BARS {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        mixer.shared.lock().expect("shared").generators.insert(
            id,
            Generator {
                kind,
                color: [r, g, b, a],
                scroll: scroll != 0,
            },
        );
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_destroy_unit(unit_id: u64) -> i32 {
    with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        shared.units.remove(&unit_id);
        shared.outputs.retain(|_, output| output.unit_id != unit_id);
        drop(shared);
        let _ = mixer.cmds.send(GpuCmd::DetachUnit { unit_id });
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_unit_attach_output(
    unit_id: u64,
    hwnd: isize,
    width: u32,
    height: u32,
    kind: u32,
) -> i32 {
    if hwnd == 0 || width == 0 || height == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    match with_mixer(|mixer| {
        if !mixer.shared.lock().expect("shared").units.contains_key(&unit_id) {
            return ERR_INVALID_ARGUMENT;
        }
        let (reply_tx, reply_rx) = mpsc::channel();
        if mixer
            .cmds
            .send(GpuCmd::Attach {
                unit_id,
                kind,
                hwnd,
                width,
                height,
                reply: reply_tx,
            })
            .is_err()
        {
            return ERR_DEVICE;
        }
        reply_rx.recv().unwrap_or(ERR_DEVICE)
    }) {
        Ok(code) => code,
        Err(code) => code,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_unit_set_state(unit_id: u64, state: *const UnitState) -> i32 {
    if state.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    // SAFETY: caller keeps UnitState valid for this call.
    let state = unsafe { *state };
    if state.overlay_count > state.overlays.len() as u32
        || state.mv_slot_count > state.mv_slots.len() as u32
        || !(0.0..=1.0).contains(&state.mix)
    {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        let Some(unit) = shared.units.get_mut(&unit_id) else {
            return ERR_INVALID_ARGUMENT;
        };
        unit.state = state;
        unit.auto = None;
        OK
    })
    .unwrap_or_else(|code| code)
}

fn take_cut(unit: &mut LiveUnit, swap: bool) {
    if swap {
        std::mem::swap(&mut unit.state.program_source, &mut unit.state.preview_source);
    } else {
        unit.state.program_source = unit.state.preview_source;
    }
    unit.state.mix = 0.0;
    unit.auto = None;
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_unit_cut(unit_id: u64, swap: u32) -> i32 {
    with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        let Some(unit) = shared.units.get_mut(&unit_id) else {
            return ERR_INVALID_ARGUMENT;
        };
        take_cut(unit, swap != 0);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_unit_auto(unit_id: u64, duration_ms: u32, swap: u32) -> i32 {
    with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        let Some(unit) = shared.units.get_mut(&unit_id) else {
            return ERR_INVALID_ARGUMENT;
        };
        unit.auto = Some(AutoTransition {
            from: unit.state.mix,
            to: if unit.state.mix < 0.5 { 1.0 } else { 0.0 },
            start: Instant::now(),
            duration: Duration::from_millis(u64::from(duration_ms.max(1))),
            swap: swap != 0,
        });
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_unit_configure(
    unit_id: u64,
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
) -> i32 {
    if width == 0 || height == 0 || fps_num == 0 || fps_den == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        let Some(unit) = shared.units.get_mut(&unit_id) else {
            return ERR_INVALID_ARGUMENT;
        };
        unit.width = width;
        unit.height = height;
        unit.fps_num = fps_num;
        unit.fps_den = fps_den;
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_unit_get_state(unit_id: u64, out: *mut UnitState) -> i32 {
    if out.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let shared = mixer.shared.lock().expect("shared");
        let Some(unit) = shared.units.get(&unit_id) else {
            return ERR_INVALID_ARGUMENT;
        };
        unsafe { *out = unit.state };
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_unit_resize_output(
    unit_id: u64,
    kind: u32,
    hwnd: isize,
    width: u32,
    height: u32,
) -> i32 {
    if width == 0 || height == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        if mixer
            .cmds
            .send(GpuCmd::Resize {
                unit_id,
                kind,
                hwnd,
                width,
                height,
            })
            .is_err()
        {
            return ERR_DEVICE;
        }
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_unit_detach_output(unit_id: u64, kind: u32, hwnd: isize) -> i32 {
    with_mixer(|mixer| {
        let _ = mixer.cmds.send(GpuCmd::Detach { unit_id, kind, hwnd });
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_attach_monitor(
    monitor_id: u64,
    source_id: u64,
    hwnd: isize,
    width: u32,
    height: u32,
) -> i32 {
    if hwnd == 0 || width == 0 || height == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    match with_mixer(|mixer| {
        let (reply_tx, reply_rx) = mpsc::channel();
        if mixer
            .cmds
            .send(GpuCmd::AttachMonitor {
                monitor_id,
                source_id,
                hwnd,
                width,
                height,
                reply: reply_tx,
            })
            .is_err()
        {
            return ERR_DEVICE;
        }
        reply_rx.recv().unwrap_or(ERR_DEVICE)
    }) {
        Ok(code) => code,
        Err(code) => code,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_resize_monitor(monitor_id: u64, width: u32, height: u32) -> i32 {
    if width == 0 || height == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let _ = mixer.cmds.send(GpuCmd::ResizeMonitor {
            monitor_id,
            width,
            height,
        });
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_detach_monitor(monitor_id: u64) -> i32 {
    with_mixer(|mixer| {
        let _ = mixer.cmds.send(GpuCmd::DetachMonitor { monitor_id });
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_monitor_set_source(monitor_id: u64, source_id: u64) -> i32 {
    with_mixer(|mixer| {
        let _ = mixer.cmds.send(GpuCmd::SetMonitorSource {
            monitor_id,
            source_id,
        });
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_register_source(id: u64, width: u32, height: u32, format: u32) -> i32 {
    let Some(format) = CpuFormat::from_abi(format) else {
        return ERR_INVALID_ARGUMENT;
    };
    with_mixer(|mixer| {
        mixer
            .shared
            .lock()
            .expect("shared")
            .uploads
            .lock()
            .expect("uploads")
            .register(id, width, height, format);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_push_frame(
    id: u64,
    ptr: *const u8,
    stride: u32,
    height: u32,
    pts: i64,
) -> i32 {
    if ptr.is_null() || stride == 0 || height == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    let len = stride as usize * height as usize;
    // SAFETY: caller keeps the frame readable for this call only.
    let src = unsafe { std::slice::from_raw_parts(ptr, len) };
    with_mixer(|mixer| {
        match mixer
            .shared
            .lock()
            .expect("shared")
            .uploads
            .lock()
            .expect("uploads")
            .push(id, src, stride as usize, pts)
        {
            Ok(()) => OK,
            Err(error) => {
                set_error(&mixer.shared, error);
                ERR_INVALID_ARGUMENT
            }
        }
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_load_still(id: u64, path: *const c_char) -> i32 {
    if path.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    // SAFETY: path is a NUL-terminated UTF-8 C string.
    let path = unsafe { CStr::from_ptr(path) }
        .to_str()
        .unwrap_or_default();
    let image = match image::open(Path::new(path)) {
        Ok(image) => image.to_rgba8(),
        Err(error) => {
            let _ = with_mixer(|mixer| {
                set_error(&mixer.shared, error.to_string());
            });
            return ERR_IO;
        }
    };
    let (width, height) = image.dimensions();
    with_mixer(|mixer| {
        let shared = mixer.shared.lock().expect("shared");
        let mut uploads = shared.uploads.lock().expect("uploads");
        uploads.register(id, width, height, CpuFormat::Rgba);
        match uploads.push(id, &image, width as usize * 4, 0) {
            Ok(()) => OK,
            Err(_) => ERR_IO,
        }
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_omt_connect(id: u64, address: *const c_char) -> i32 {
    if address.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let address = unsafe { CStr::from_ptr(address) }
        .to_str()
        .unwrap_or_default()
        .to_string();
    with_mixer(|mixer| {
        let uploads = mixer.shared.lock().expect("shared").uploads.clone();
        match OmtReceiver::start(id, address, uploads) {
            Ok(receiver) => {
                mixer.shared.lock().expect("shared").receivers.insert(id, receiver);
                OK
            }
            Err(error) => {
                set_error(&mixer.shared, error);
                ERR_IO
            }
        }
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_omt_start_send(unit_id: u64, name: *const c_char) -> i32 {
    if name.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    unsafe { mixer_output_add(unit_id, OUT_OMT, name, SRC_KIND_MU_PROGRAM, 0, unit_id) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_output_add(
    output_id: u64,
    transport: u32,
    name: *const c_char,
    source_kind: u32,
    source_id: u64,
    unit_id: u64,
) -> i32 {
    if name.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let name = unsafe { CStr::from_ptr(name) }
        .to_str()
        .unwrap_or_default()
        .to_string();
    match transport {
        OUT_NDI => {
            let _ = with_mixer(|mixer| {
                set_error(&mixer.shared, "NDI output is not linked in this build");
            });
            return ERR_IO;
        }
        OUT_DECKLINK => {
            let _ = with_mixer(|mixer| {
                set_error(&mixer.shared, "DeckLink output is not linked in this build");
            });
            return ERR_IO;
        }
        OUT_OMT => {}
        _ => return ERR_INVALID_ARGUMENT,
    }
    with_mixer(|mixer| match ProgramSender::start(&name) {
        Ok(mut sender) => {
            sender.unit_id = unit_id;
            mixer.shared.lock().expect("shared").outputs.insert(
                output_id,
                LiveOutput {
                    transport,
                    source_kind,
                    source_id,
                    unit_id,
                    sender,
                },
            );
            OK
        }
        Err(error) => {
            set_error(&mixer.shared, error);
            ERR_IO
        }
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_output_remove(output_id: u64) -> i32 {
    with_mixer(|mixer| {
        mixer.shared.lock().expect("shared").outputs.remove(&output_id);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_omt_discover(out: *mut u8, cap: usize) -> i32 {
    if out.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    match omt::discover_addresses() {
        Ok(addresses) => {
            let text = addresses.join("\n");
            let bytes = text.as_bytes();
            let n = bytes.len().min(cap);
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, n) };
            n as i32
        }
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_unit_acquire_frame(
    unit_id: u64,
    ptr: *mut *const u8,
    stride: *mut u32,
    pts: *mut i64,
    length: *mut u32,
) -> i32 {
    if ptr.is_null() || stride.is_null() || pts.is_null() || length.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    if with_mixer(|_| ()).is_err() {
        return ERR_NOT_CREATED;
    }
    // The latest packed frame is stored on the render-thread readback cache and
    // copied into a process-wide acquire buffer so the pointer stays stable.
    let Some(frame) = last_frames().lock().expect("frame").get(&unit_id).cloned() else {
        return ERR_IO;
    };
    unsafe {
        *ptr = frame.data.as_ptr();
        *stride = frame.stride;
        *pts = frame.pts;
        *length = frame.data.len() as u32;
    }
    acquired().lock().expect("acq").insert(unit_id, frame);
    OK
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_unit_release_frame(unit_id: u64) -> i32 {
    acquired().lock().expect("acq").remove(&unit_id);
    OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_last_error(out: *mut u8, cap: usize) -> i32 {
    if out.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let error = match with_mixer(|mixer| mixer.shared.lock().expect("shared").last_error.clone()) {
        Ok(error) => error,
        Err(code) => return code,
    };
    let n = error.len().min(cap);
    unsafe { std::ptr::copy_nonoverlapping(error.as_ptr(), out, n) };
    n as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_copy_audio_peaks(out: *mut AudioPeak, cap: u32) -> i32 {
    if out.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let shared = mixer.shared.lock().expect("shared");
        let uploads = shared.uploads.lock().expect("uploads");
        let mut n = 0u32;
        for id in uploads.ids() {
            if n >= cap {
                break;
            }
            let Some(ring) = uploads.get(id) else {
                continue;
            };
            let (left, right) = ring.peak();
            unsafe {
                *out.add(n as usize) = AudioPeak {
                    source_id: id,
                    left,
                    right,
                };
            }
            n += 1;
        }
        n as i32
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_copy_stats(out: *mut MixerStats) -> i32 {
    if out.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let shared = mixer.shared.lock().expect("shared");
        let budget = 1000.0 * shared.master_fps_den as f32 / shared.master_fps_num.max(1) as f32;
        unsafe {
            *out = MixerStats {
                render_ms: shared.last_render_ms,
                frame_budget_ms: budget,
            };
        }
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_set_frame_buffer(frames: u32) -> i32 {
    let frames = frames.clamp(1, 8);
    with_mixer(|mixer| {
        mixer.shared.lock().expect("shared").frame_buffer_frames = frames;
        OK
    })
    .unwrap_or_else(|code| code)
}

#[derive(Clone)]
struct Acquired {
    data: Vec<u8>,
    stride: u32,
    pts: i64,
}

static LAST_FRAME: OnceLock<Mutex<HashMap<u64, Acquired>>> = OnceLock::new();
static ACQUIRED: OnceLock<Mutex<HashMap<u64, Acquired>>> = OnceLock::new();

fn last_frames() -> &'static Mutex<HashMap<u64, Acquired>> {
    LAST_FRAME.get_or_init(|| Mutex::new(HashMap::new()))
}

fn acquired() -> &'static Mutex<HashMap<u64, Acquired>> {
    ACQUIRED.get_or_init(|| Mutex::new(HashMap::new()))
}

fn render_loop(
    device: GpuDevice,
    fps_num: u32,
    fps_den: u32,
    shared: Arc<Mutex<Shared>>,
    uploads: Arc<Mutex<UploadStore>>,
    cmds: mpsc::Receiver<GpuCmd>,
    stop: Arc<AtomicBool>,
) {
    let mut composer = match Composer::new(&device) {
        Ok(composer) => composer,
        Err(error) => {
            shared.lock().expect("shared").last_error = error;
            return;
        }
    };
    let mut presenters = Presenters::default();
    let mut readbacks = ReadbackStore::default();
    let frame_dt = Duration::from_nanos(1_000_000_000u64 * u64::from(fps_den) / u64::from(fps_num));
    let mut next = Instant::now();
    let clock_start = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        while let Ok(cmd) = cmds.try_recv() {
            match cmd {
                GpuCmd::Attach {
                    unit_id,
                    kind,
                    hwnd,
                    width,
                    height,
                    reply,
                } => {
                    let code = match presenters.attach(&device, unit_id, kind, hwnd, width, height) {
                        Ok(()) => OK,
                        Err(error) => {
                            shared.lock().expect("shared").last_error = error;
                            ERR_DEVICE
                        }
                    };
                    let _ = reply.send(code);
                }
                GpuCmd::Resize {
                    unit_id,
                    kind,
                    hwnd,
                    width,
                    height,
                } => presenters.resize(&device, unit_id, kind, hwnd, width, height),
                GpuCmd::Detach { unit_id, kind, hwnd } => presenters.detach(unit_id, kind, hwnd),
                GpuCmd::DetachUnit { unit_id } => presenters.detach_unit(unit_id),
                GpuCmd::AttachMonitor {
                    monitor_id,
                    source_id,
                    hwnd,
                    width,
                    height,
                    reply,
                } => {
                    let code = match presenters.attach_monitor(
                        &device, monitor_id, source_id, hwnd, width, height,
                    ) {
                        Ok(()) => OK,
                        Err(error) => {
                            shared.lock().expect("shared").last_error = error;
                            ERR_DEVICE
                        }
                    };
                    let _ = reply.send(code);
                }
                GpuCmd::ResizeMonitor {
                    monitor_id,
                    width,
                    height,
                } => presenters.resize_monitor(&device, monitor_id, width, height),
                GpuCmd::DetachMonitor { monitor_id } => presenters.detach_monitor(monitor_id),
                GpuCmd::SetMonitorSource {
                    monitor_id,
                    source_id,
                } => presenters.set_monitor_source(monitor_id, source_id),
                GpuCmd::Shutdown => return,
            }
        }
        let buffer_frames = shared
            .lock()
            .expect("shared")
            .frame_buffer_frames
            .clamp(1, 8);
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        }
        let skip_compose = Instant::now().saturating_duration_since(next)
            > frame_dt.saturating_mul(buffer_frames.saturating_sub(1));
        {
            let mut guard = shared.lock().expect("shared");
            for unit in guard.units.values_mut() {
                if let Some(auto) = unit.auto.take() {
                    let t = auto.start.elapsed().as_secs_f32() / auto.duration.as_secs_f32();
                    if t >= 1.0 {
                        unit.state.mix = auto.to;
                        if auto.to >= 1.0 {
                            take_cut(unit, auto.swap);
                        }
                    } else {
                        unit.state.mix = auto.from + (auto.to - auto.from) * t;
                        unit.auto = Some(auto);
                    }
                }
            }
            let snapshot: Vec<(u64, u32, u32, u32, u32, UnitState)> = guard
                .units
                .iter()
                .map(|(id, unit)| {
                    (
                        *id,
                        unit.width,
                        unit.height,
                        unit.fps_num,
                        unit.fps_den,
                        unit.state,
                    )
                })
                .collect();
            let scene_specs: Vec<(u64, u32, u32, Vec<OverlayDesc>)> = guard
                .scenes
                .iter()
                .map(|(id, (w, h, layers))| (*id, *w, *h, layers.clone()))
                .collect();
            let generators: Vec<(u64, Generator)> = guard
                .generators
                .iter()
                .map(|(id, spec)| (*id, *spec))
                .collect();
            let outputs_snap: Vec<(u64, u32, u64, u64, u32, u32)> = guard
                .outputs
                .iter()
                .map(|(id, output)| {
                    (
                        *id,
                        output.source_kind,
                        output.source_id,
                        output.unit_id,
                        guard
                            .units
                            .get(&output.unit_id)
                            .map(|u| u.fps_num)
                            .unwrap_or(fps_num),
                        guard
                            .units
                            .get(&output.unit_id)
                            .map(|u| u.fps_den)
                            .unwrap_or(fps_den),
                    )
                })
                .collect();
            drop(guard);
            let upload_guard = uploads.lock().expect("uploads");
            let frame_begin = Instant::now();
            let pts = (clock_start.elapsed().as_nanos() / 100) as i64;
            let phase = (clock_start.elapsed().as_secs_f32() * 0.12) % 1.0;
            composer.ensure_builtins(&device);
            composer.sync_generators(&generators, phase);
            composer.upload_sources(&device, &upload_guard);
            if !skip_compose {
                composer.sync_scenes(&device, &scene_specs);
                if let Err(error) = composer.render_scenes(&device) {
                    shared.lock().expect("shared").last_error = error;
                }
                for (unit_id, width, height, _, _, state) in &snapshot {
                    composer.ensure_unit(&device, *unit_id, *width, *height);
                    if let Err(error) = composer.render_unit(
                        &device,
                        *unit_id,
                        state,
                        &presenters,
                        &mut readbacks,
                        &upload_guard,
                    ) {
                        shared.lock().expect("shared").last_error = error;
                    }
                    composer.pack_aux(&device, *unit_id);
                    if let Some(packed) =
                        readbacks.get(*unit_id).and_then(readback::UnitReadback::latest)
                    {
                        last_frames().lock().expect("frames").insert(
                            *unit_id,
                            Acquired {
                                data: packed.to_vec(),
                                stride: *width * 2,
                                pts,
                            },
                        );
                    }
                }
            }
            if let Err(error) = presenters.present_monitors(&device, |source_id| {
                composer.view_for_source(source_id)
            }) {
                shared.lock().expect("shared").last_error = error;
            }
            if !skip_compose {
            for (output_id, source_kind, source_id, unit_id, _, _) in &outputs_snap {
                let packed = match *source_kind {
                    SRC_KIND_INPUT => {
                        let (w, h) = snapshot
                            .iter()
                            .find(|(id, ..)| *id == *unit_id)
                            .map(|(_, w, h, ..)| (*w, *h))
                            .unwrap_or((1920, 1080));
                        composer.pack_source(&device, *source_id, w, h)
                    }
                    SRC_KIND_SCENE => composer.pack_scene(&device, *source_id),
                    SRC_KIND_MU_PREVIEW => composer.packed_texture(*unit_id, OUTPUT_PREVIEW),
                    SRC_KIND_MU_MULTIVIEW => composer.packed_texture(*unit_id, OUTPUT_MULTIVIEW),
                    _ => composer.packed_texture(*unit_id, OUTPUT_PROGRAM),
                };
                if let Some(texture) = packed {
                    let (w, h) = snapshot
                        .iter()
                        .find(|(id, ..)| *id == *unit_id)
                        .map(|(_, w, h, ..)| (*w, *h))
                        .or_else(|| {
                            scene_specs
                                .iter()
                                .find(|(id, ..)| *id == *source_id)
                                .map(|(_, w, h, _)| (*w, *h))
                        })
                        .unwrap_or((1920, 1080));
                    let rb = readbacks.ensure(&device, 0x0100_0000_0000_0000 | *output_id, w, h);
                    let mut encoder = device.device.create_command_encoder(&Default::default());
                    rb.copy_from(&mut encoder, texture);
                    device.queue.submit(Some(encoder.finish()));
                    rb.advance(&device);
                    if let Some(data) = rb.latest() {
                        last_frames().lock().expect("frames").insert(
                            0x0100_0000_0000_0000 | *output_id,
                            Acquired {
                                data: data.to_vec(),
                                stride: w * 2,
                                pts,
                            },
                        );
                    }
                }
            }
            }
            drop(upload_guard);
            shared.lock().expect("shared").last_render_ms =
                frame_begin.elapsed().as_secs_f32() * 1000.0;
            let mut guard = shared.lock().expect("shared");
            let frames = last_frames().lock().expect("frames").clone();
            let output_ids: Vec<u64> = guard.outputs.keys().copied().collect();
            for output_id in output_ids {
                let (unit_id, source_kind, fps_n, fps_d) = {
                    let Some(output) = guard.outputs.get(&output_id) else {
                        continue;
                    };
                    let (fps_n, fps_d) = guard
                        .units
                        .get(&output.unit_id)
                        .map(|unit| (unit.fps_num, unit.fps_den))
                        .unwrap_or((fps_num, fps_den));
                    (output.unit_id, output.source_kind, fps_n, fps_d)
                };
                let Some(output) = guard.outputs.get_mut(&output_id) else {
                    continue;
                };
                let _ = output.sender.pump();
                let key = if source_kind == SRC_KIND_MU_PROGRAM {
                    unit_id
                } else {
                    0x0100_0000_0000_0000 | output_id
                };
                if let Some(frame) = frames.get(&key).or_else(|| frames.get(&unit_id)) {
                    let width = readbacks
                        .get(key)
                        .or_else(|| readbacks.get(unit_id))
                        .map(|r| r.width)
                        .unwrap_or(1920);
                    let height = readbacks
                        .get(key)
                        .or_else(|| readbacks.get(unit_id))
                        .map(|r| r.height)
                        .unwrap_or(1080);
                    let _ = output.sender.send_video_uyvy(
                        width,
                        height,
                        frame.stride,
                        frame.pts,
                        &frame.data,
                        fps_n,
                        fps_d,
                    );
                }
            }
        }
        next += frame_dt;
        let now = Instant::now();
        if next + frame_dt.saturating_mul(buffer_frames) < now {
            next = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_is_stable() {
        assert_eq!(mixer_ping(), 0x4549_5649);
    }

    #[test]
    fn rejects_invalid_framerate() {
        assert_eq!(mixer_create(0, 60_000, 0), ERR_INVALID_ARGUMENT);
    }
}
