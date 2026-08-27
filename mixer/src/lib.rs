#![deny(unsafe_op_in_unsafe_fn)]

mod abi;
mod compose;
mod convert;
mod device;
mod dxgi;
mod media;
mod omt;
mod pool;
mod present;
mod readback;
mod upload;

pub use abi::{
    AudioPeak, MixerStats, OverlayDesc, Rect, SourceUsage, UnitState, ERR_ALREADY_CREATED, ERR_DEVICE,
    ERR_INVALID_ARGUMENT, ERR_IO, ERR_NOT_CREATED, GEN_BARS, GEN_SOLID, OK, OUTPUT_MULTIVIEW,
    OUTPUT_PREVIEW, OUTPUT_PROGRAM, OUT_DECKLINK, OUT_NDI, OUT_OMT, SCENE_BASE, SRC_BARS, SRC_BLACK,
    SRC_BLUE, SRC_COLOR, SRC_KIND_INPUT, SRC_KIND_MU_MULTIVIEW, SRC_KIND_MU_PREVIEW,
    SRC_KIND_MU_PROGRAM, SRC_KIND_SCENE,
};
pub use media::MixerVideoInfo;

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{c_char, CStr};
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use compose::{Composer, Generator};
use device::GpuDevice;
use dxgi::GpuVideoContext;
use media::VideoPump;
use omt::{OmtReceiver, ProgramSender};
use present::Presenters;
use readback::ReadbackStore;
use upload::{AudioPacket, CpuFormat, UploadStore, AUDIO_RATE};

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
    video_sub: Arc<AtomicBool>,
}

struct Shared {
    master_fps_num: u32,
    master_fps_den: u32,
    units: HashMap<u64, LiveUnit>,
    scenes: HashMap<u64, (u32, u32, Arc<[crate::abi::OverlayDesc]>)>,
    uploads: Arc<Mutex<UploadStore>>,
    gpu_video: GpuVideoContext,
    receivers: HashMap<u64, OmtReceiver>,
    videos: HashMap<u64, VideoPump>,
    outputs: HashMap<u64, LiveOutput>,
    generators: HashMap<u64, Generator>,
    multiview_binds: HashMap<u64, (u64, u64)>,
    last_error: String,
    last_render_ms: f32,
    frame_buffer_frames: u32,
    follow_primed: bool,
    monitor_pcm: VecDeque<f32>,
}

enum SendCmd {
    Add {
        output_id: u64,
        sender: ProgramSender,
        video_sub: Arc<AtomicBool>,
    },
    Remove {
        output_id: u64,
    },
    Video {
        output_id: u64,
        width: u32,
        height: u32,
        stride: u32,
        pts: i64,
        data: Arc<[u8]>,
        fps_n: u32,
        fps_d: u32,
    },
    Audio {
        output_id: u64,
        packet: AudioPacket,
    },
    Shutdown,
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
    send_tx: mpsc::Sender<SendCmd>,
    render: Option<JoinHandle<()>>,
    send: Option<JoinHandle<()>>,
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
    let gpu_video = match GpuVideoContext::new(&device) {
        Ok(ctx) => ctx,
        Err(error) => {
            eprintln!("eiviz dxgi video: {error}");
            return ERR_DEVICE;
        }
    };
    let uploads = Arc::new(Mutex::new(UploadStore::default()));
    let shared = Arc::new(Mutex::new(Shared {
        master_fps_num: fps_num,
        master_fps_den: fps_den,
        units: HashMap::new(),
        scenes: HashMap::new(),
        uploads: Arc::clone(&uploads),
        gpu_video: gpu_video.clone(),
        receivers: HashMap::new(),
        videos: HashMap::new(),
        outputs: HashMap::new(),
        generators: HashMap::new(),
        last_error: String::new(),
        last_render_ms: 0.0,
        frame_buffer_frames: 3,
        follow_primed: false,
        monitor_pcm: VecDeque::new(),
        multiview_binds: HashMap::new(),
    }));
    let (tx, rx) = mpsc::channel();
    let (send_tx, send_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let render_shared = Arc::clone(&shared);
    let render_stop = Arc::clone(&stop);
    let render_send = send_tx.clone();
    let render = thread::Builder::new()
        .name("eiviz-render".into())
        .spawn(move || {
            render_loop(
                device,
                fps_num,
                fps_den,
                render_shared,
                uploads,
                rx,
                render_send,
                render_stop,
            );
        })
        .expect("render thread");
    let send_stop = Arc::clone(&stop);
    let send = thread::Builder::new()
        .name("eiviz-omt-send".into())
        .spawn(move || send_loop(send_rx, send_stop))
        .expect("omt send thread");
    *slot = Some(Mixer {
        shared,
        cmds: tx,
        send_tx,
        render: Some(render),
        send: Some(send),
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
        let _ = mixer.send_tx.send(SendCmd::Shutdown);
        if let Some(join) = mixer.send.take() {
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
    let copied: Arc<[OverlayDesc]> = if count == 0 {
        Arc::from([])
    } else {
        // SAFETY: caller keeps count OverlayDesc values readable.
        Arc::from(unsafe { std::slice::from_raw_parts(layers, count as usize) })
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
        mixer.shared.lock().expect("shared").multiview_binds.remove(&scene_id);
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
        let mut gone = Vec::new();
        shared.outputs.retain(|id, output| {
            if output.unit_id == unit_id {
                gone.push(*id);
                false
            } else {
                true
            }
        });
        drop(shared);
        for output_id in gone {
            let _ = mixer.send_tx.send(SendCmd::Remove { output_id });
        }
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
pub unsafe extern "C" fn mixer_push_audio(
    id: u64,
    sample_rate: i32,
    channels: i32,
    frames: u32,
    pts: i64,
    planar: *const f32,
) -> i32 {
    if planar.is_null() || channels <= 0 || frames == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    let count = channels as usize * frames as usize;
    // SAFETY: caller keeps planar readable for this call only.
    let samples = unsafe { std::slice::from_raw_parts(planar, count) };
    with_mixer(|mixer| {
        mixer
            .shared
            .lock()
            .expect("shared")
            .uploads
            .lock()
            .expect("uploads")
            .push_audio(id, sample_rate, channels, frames, pts, samples);
        OK
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
pub unsafe extern "C" fn mixer_video_start(
    id: u64,
    path: *const c_char,
    capture: u32,
    format: u32,
) -> i32 {
    if path.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let path = unsafe { CStr::from_ptr(path) }
        .to_str()
        .unwrap_or_default()
        .to_string();
    if path.is_empty() {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let (uploads, gpu) = {
            let mut shared = mixer.shared.lock().expect("shared");
            let previous = shared.videos.remove(&id);
            drop(shared.receivers.remove(&id));
            let uploads = shared.uploads.clone();
            let gpu = shared.gpu_video.clone();
            drop(shared);
            drop(previous);
            (uploads, gpu)
        };
        match VideoPump::start(id, path, capture != 0, format, uploads, gpu) {
            Ok(pump) => {
                mixer.shared.lock().expect("shared").videos.insert(id, pump);
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
pub extern "C" fn mixer_video_set_playing(id: u64, playing: u32) -> i32 {
    with_mixer(|mixer| {
        let shared = mixer.shared.lock().expect("shared");
        let Some(pump) = shared.videos.get(&id) else {
            return ERR_INVALID_ARGUMENT;
        };
        pump.set_playing(playing != 0);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_video_seek(id: u64, hns: i64) -> i32 {
    with_mixer(|mixer| {
        let shared = mixer.shared.lock().expect("shared");
        let Some(pump) = shared.videos.get(&id) else {
            return ERR_INVALID_ARGUMENT;
        };
        pump.seek(hns);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_video_copy_info(id: u64, out: *mut MixerVideoInfo) -> i32 {
    if out.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let shared = mixer.shared.lock().expect("shared");
        let Some(pump) = shared.videos.get(&id) else {
            return ERR_INVALID_ARGUMENT;
        };
        unsafe { *out = pump.info() };
        OK
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
        let uploads = {
            let mut shared = mixer.shared.lock().expect("shared");
            let previous = shared.videos.remove(&id);
            drop(shared.receivers.remove(&id));
            let uploads = shared.uploads.clone();
            drop(shared);
            drop(previous);
            uploads
        };
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
    let started = panic::catch_unwind(AssertUnwindSafe(|| ProgramSender::start(&name)));
    let sender = match started {
        Ok(Ok(sender)) => sender,
        Ok(Err(error)) => {
            let _ = with_mixer(|mixer| set_error(&mixer.shared, error));
            return ERR_IO;
        }
        Err(_) => {
            let _ = with_mixer(|mixer| {
                set_error(&mixer.shared, "OMT sender panicked during create")
            });
            return ERR_IO;
        }
    };
    with_mixer(|mixer| {
        let video_sub = Arc::new(AtomicBool::new(false));
        mixer.shared.lock().expect("shared").outputs.insert(
            output_id,
            LiveOutput {
                transport,
                source_kind,
                source_id,
                unit_id,
                video_sub: Arc::clone(&video_sub),
            },
        );
        let _ = mixer.send_tx.send(SendCmd::Add {
            output_id,
            sender,
            video_sub,
        });
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_output_remove(output_id: u64) -> i32 {
    with_mixer(|mixer| {
        mixer.shared.lock().expect("shared").outputs.remove(&output_id);
        let _ = mixer.send_tx.send(SendCmd::Remove { output_id });
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
pub extern "C" fn mixer_destroy_source(id: u64) -> i32 {
    with_mixer(|mixer| {
        let (previous, uploads) = {
            let mut shared = mixer.shared.lock().expect("shared");
            (
                shared.videos.remove(&id),
                {
                    shared.receivers.remove(&id);
                    shared.generators.remove(&id);
                    shared.uploads.clone()
                },
            )
        };
        drop(previous);
        uploads.lock().expect("uploads").unregister(id);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_flush_audio(id: u64) -> i32 {
    with_mixer(|mixer| {
        mixer
            .shared
            .lock()
            .expect("shared")
            .uploads
            .lock()
            .expect("uploads")
            .flush_audio(id);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_bind_multiview(scene_id: u64, preview_unit: u64, program_unit: u64) -> i32 {
    with_mixer(|mixer| {
        mixer
            .shared
            .lock()
            .expect("shared")
            .multiview_binds
            .insert(scene_id, (preview_unit, program_unit));
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_copy_follow_audio(out: *mut f32, cap: u32) -> i32 {
    if out.is_null() || cap == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let dest = unsafe { std::slice::from_raw_parts_mut(out, cap as usize) };
        let mut shared = mixer.shared.lock().expect("shared");
        if !shared.follow_primed {
            dest.fill(0.0);
            return 0;
        }
        let n = dest.len().min(shared.monitor_pcm.len());
        for slot in dest.iter_mut().take(n) {
            *slot = shared.monitor_pcm.pop_front().unwrap_or(0.0);
        }
        for slot in dest.iter_mut().skip(n) {
            *slot = 0.0;
        }
        if shared.monitor_pcm.is_empty() {
            shared.follow_primed = false;
        }
        n as i32
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_copy_monitor_audio(
    id: u64,
    out: *mut f32,
    cap: u32,
    sample_rate: *mut i32,
    channels: *mut i32,
) -> i32 {
    if out.is_null() || sample_rate.is_null() || channels.is_null() || cap == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    let n = unsafe { mixer_copy_follow_audio(out, cap) };
    unsafe {
        *sample_rate = AUDIO_RATE;
        *channels = 2;
    }
    let _ = id;
    n
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
pub unsafe extern "C" fn mixer_copy_source_usage(out: *mut SourceUsage, cap: u32) -> i32 {
    if out.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let shared = mixer.shared.lock().expect("shared");
        let uploads = shared.uploads.lock().expect("uploads");
        let mut n = 0u32;
        let mut seen = std::collections::HashSet::new();
        for id in uploads.ids() {
            if n >= cap {
                break;
            }
            let Some(ring) = uploads.get(id) else {
                continue;
            };
            seen.insert(id);
            unsafe {
                *out.add(n as usize) = SourceUsage {
                    source_id: id,
                    width: ring.width,
                    height: ring.height,
                    ram_bytes: ring.ram_bytes(),
                    vram_bytes: ring.vram_bytes(),
                };
            }
            n += 1;
        }
        for id in shared.generators.keys().copied() {
            if n >= cap {
                break;
            }
            if !seen.insert(id) {
                continue;
            }
            unsafe {
                *out.add(n as usize) = SourceUsage {
                    source_id: id,
                    width: 1920,
                    height: 1080,
                    ram_bytes: 0,
                    vram_bytes: 1920 * 1080 * 4,
                };
            }
            n += 1;
        }
        for id in [SRC_COLOR, SRC_BARS, SRC_BLACK, SRC_BLUE] {
            if n >= cap {
                break;
            }
            if !seen.insert(id) {
                continue;
            }
            unsafe {
                *out.add(n as usize) = SourceUsage {
                    source_id: id,
                    width: 128,
                    height: 72,
                    ram_bytes: 0,
                    vram_bytes: 128 * 72 * 4,
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
    data: Arc<[u8]>,
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
    send_tx: mpsc::Sender<SendCmd>,
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
    let mut frame_i = 0u64;
    let mut audio_acc = 0.0f64;
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
            let scene_specs: Vec<(u64, u32, u32, Arc<[OverlayDesc]>)> = guard
                .scenes
                .iter()
                .map(|(id, (w, h, layers))| (*id, *w, *h, Arc::clone(layers)))
                .collect();
            let generators: Vec<(u64, Generator)> = guard
                .generators
                .iter()
                .map(|(id, spec)| (*id, *spec))
                .collect();
            let outputs_snap: Vec<(u64, u32, u64, u64, u32, u32, Arc<AtomicBool>)> = guard
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
                        Arc::clone(&output.video_sub),
                    )
                })
                .collect();
            let binds: HashMap<u64, (u64, u64)> = guard.multiview_binds.clone();
            drop(guard);
            let mut tallies = HashMap::new();
            for (scene_id, (preview_unit, program_unit)) in &binds {
                let preview_source = snapshot
                    .iter()
                    .find(|(id, ..)| *id == *preview_unit)
                    .map(|item| item.5.preview_source)
                    .unwrap_or(0);
                let program_source = snapshot
                    .iter()
                    .find(|(id, ..)| *id == *program_unit)
                    .map(|item| item.5.program_source)
                    .unwrap_or(0);
                tallies.insert(*scene_id, (preview_source, program_source));
            }
            let (mut used_scenes, mut used_uploads) = collect_live_ids(
                &scene_specs,
                &snapshot,
                presenters.attached_monitor_sources(),
                &outputs_snap,
            );
            frame_i = frame_i.wrapping_add(1);
            if frame_i % 3 != 0 {
                let (hot_scenes, hot_uploads) =
                    collect_live_ids(&scene_specs, &snapshot, Vec::new(), &outputs_snap);
                used_scenes = hot_scenes;
                used_uploads = hot_uploads;
            }
            let upload_guard = uploads.lock().expect("uploads");
            let frame_begin = Instant::now();
            let pts = (clock_start.elapsed().as_nanos() / 100) as i64;
            let phase = (clock_start.elapsed().as_secs_f32() * 0.12) % 1.0;
            composer.begin_frame();
            composer.ensure_builtins(&device);
            composer.sync_generators(&generators, phase);
            if !skip_compose {
                composer.upload_sources(&device, &upload_guard, &used_uploads);
            }
            drop(upload_guard);
            let need_prv = outputs_snap.iter().any(|item| {
                item.1 == SRC_KIND_MU_PREVIEW && item.6.load(Ordering::Relaxed)
            });
            let need_mv = snapshot
                .iter()
                .any(|(unit_id, ..)| presenters.has_kind(*unit_id, OUTPUT_MULTIVIEW));
            if !skip_compose {
                composer.sync_scenes(&device, &scene_specs);
                let mut encoder = device.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("eiviz compose"),
                });
                if let Err(error) =
                    composer.render_scenes(&device, &used_scenes, &mut encoder, &tallies)
                {
                    shared.lock().expect("shared").last_error = error;
                }
                let mut packed_copies: Vec<(u64, u32, u32)> = Vec::new();
                for (unit_id, width, height, _, _, state) in &snapshot {
                    composer.ensure_unit(&device, *unit_id, *width, *height);
                    let pack_pgm = outputs_snap.iter().any(|item| {
                        item.3 == *unit_id
                            && item.1 == SRC_KIND_MU_PROGRAM
                            && item.6.load(Ordering::Relaxed)
                    });
                    if let Err(error) = composer.render_unit(
                        &device,
                        *unit_id,
                        state,
                        &mut encoder,
                        need_mv,
                        pack_pgm,
                    ) {
                        shared.lock().expect("shared").last_error = error;
                    }
                    composer.pack_aux(&device, &mut encoder, *unit_id, need_prv, false);
                    if pack_pgm {
                        if let Some(packed) = composer.packed_texture(*unit_id, OUTPUT_PROGRAM) {
                            let width = packed.size().width.saturating_mul(2).max(2);
                            let height = packed.size().height.max(1);
                            let rb = readbacks.ensure(&device, *unit_id, width, height);
                            rb.copy_from(&mut encoder, packed);
                            packed_copies.push((*unit_id, width, height));
                        }
                    }
                }
                for (output_id, source_kind, source_id, unit_id, _, _, video_sub) in &outputs_snap {
                    if *source_kind == SRC_KIND_MU_PROGRAM || !video_sub.load(Ordering::Relaxed) {
                        continue;
                    }
                    let packed = match *source_kind {
                        SRC_KIND_INPUT => {
                            let (w, h) = snapshot
                                .iter()
                                .find(|(id, ..)| *id == *unit_id)
                                .map(|(_, w, h, ..)| (*w, *h))
                                .unwrap_or((1920, 1080));
                            composer.pack_source(&device, &mut encoder, *source_id, w, h)
                        }
                        SRC_KIND_SCENE | SRC_KIND_MU_MULTIVIEW => {
                            composer.pack_scene(&device, &mut encoder, *source_id)
                        }
                        SRC_KIND_MU_PREVIEW => composer.packed_texture(*unit_id, OUTPUT_PREVIEW),
                        _ => None,
                    };
                    if let Some(texture) = packed {
                        let size = texture.size();
                        let w = size.width.saturating_mul(2).max(2);
                        let h = size.height.max(1);
                        let key = 0x0100_0000_0000_0000 | *output_id;
                        let rb = readbacks.ensure(&device, key, w, h);
                        rb.copy_from(&mut encoder, texture);
                        packed_copies.push((key, w, h));
                    }
                }
                device.queue.submit(Some(encoder.finish()));
                for (key, width, height) in packed_copies {
                    if let Some(rb) = readbacks.get_mut(key) {
                        rb.advance(&device);
                        if let Some(packed) = rb.latest() {
                            let data: Arc<[u8]> = packed.to_vec().into();
                            last_frames().lock().expect("frames").insert(
                                key,
                                Acquired {
                                    data: Arc::clone(&data),
                                    stride: width * 2,
                                    pts,
                                },
                            );
                            for (output_id, source_kind, _, unit_id, fps_n, fps_d, video_sub) in
                                &outputs_snap
                            {
                                if !video_sub.load(Ordering::Relaxed) {
                                    continue;
                                }
                                let out_key = if *source_kind == SRC_KIND_MU_PROGRAM {
                                    *unit_id
                                } else {
                                    0x0100_0000_0000_0000 | *output_id
                                };
                                if out_key != key {
                                    continue;
                                }
                                let _ = send_tx.send(SendCmd::Video {
                                    output_id: *output_id,
                                    width,
                                    height,
                                    stride: width * 2,
                                    pts,
                                    data: Arc::clone(&data),
                                    fps_n: *fps_n,
                                    fps_d: *fps_d,
                                });
                            }
                        }
                    }
                }
            }
            if !skip_compose {
                if let Err(error) = presenters.present_unit_buses(&device, composer.gpu_epoch(), |unit_id, kind| {
                    composer.unit_view(unit_id, kind)
                }) {
                    shared.lock().expect("shared").last_error = error;
                }
                if frame_i % 3 == 1 {
                    if let Err(error) = presenters.present_monitors(&device, composer.gpu_epoch(), |source_id| {
                        composer.view_for_source(source_id).map(|view| {
                            (view, composer.source_is_packed(source_id))
                        })
                    }) {
                        shared.lock().expect("shared").last_error = error;
                    }
                }
            }
            audio_acc += f64::from(AUDIO_RATE) * f64::from(fps_den) / f64::from(fps_num);
            let audio_frames = audio_acc.floor() as usize;
            audio_acc -= audio_frames as f64;
            let mut upload_guard = uploads.lock().expect("uploads");
            let gains = follow_gains(&snapshot, &scene_specs, &upload_guard);
            let mixed = upload_guard.mix_follow(&gains, audio_frames);
            drop(upload_guard);
            {
                let mut guard = shared.lock().expect("shared");
                guard.monitor_pcm.extend(mixed.iter().copied());
                let cap = AUDIO_RATE as usize;
                while guard.monitor_pcm.len() > cap {
                    guard.monitor_pcm.pop_front();
                }
                if guard.monitor_pcm.len() >= (AUDIO_RATE as usize) / 5 {
                    guard.follow_primed = true;
                }
                guard.last_render_ms = frame_begin.elapsed().as_secs_f32() * 1000.0;
            }
            if audio_frames > 0 {
                let packet = interleaved_to_packet(&mixed, pts);
                for (output_id, ..) in &outputs_snap {
                    let _ = send_tx.send(SendCmd::Audio {
                        output_id: *output_id,
                        packet: packet.clone(),
                    });
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

fn interleaved_to_packet(interleaved: &[f32], pts: i64) -> AudioPacket {
    let frames = interleaved.len() / 2;
    let mut planar = Vec::with_capacity(frames * 2 * 4);
    for sample in interleaved.iter().step_by(2) {
        planar.extend_from_slice(&sample.to_le_bytes());
    }
    for sample in interleaved.iter().skip(1).step_by(2) {
        planar.extend_from_slice(&sample.to_le_bytes());
    }
    AudioPacket {
        timestamp: pts,
        sample_rate: AUDIO_RATE,
        channels: 2,
        samples_per_channel: frames as i32,
        pcm_planar_f32: planar,
    }
}

fn follow_gains(
    snapshot: &[(u64, u32, u32, u32, u32, UnitState)],
    scenes: &[(u64, u32, u32, Arc<[OverlayDesc]>)],
    _uploads: &UploadStore,
) -> Vec<(u64, f32)> {
    let spec_map: HashMap<u64, &[OverlayDesc]> =
        scenes.iter().map(|spec| (spec.0, spec.3.as_ref())).collect();
    let mut gains = HashMap::<u64, f32>::new();
    fn add(
        id: u64,
        gain: f32,
        spec_map: &HashMap<u64, &[OverlayDesc]>,
        gains: &mut HashMap<u64, f32>,
    ) {
        if gain.abs() < 1e-4 {
            return;
        }
        if crate::abi::is_scene(id) {
            if let Some(layers) = spec_map.get(&id) {
                for layer in *layers {
                    if layer.audio_follow == 0 {
                        continue;
                    }
                    add(layer.source_id, gain * layer.opacity.max(0.0), spec_map, gains);
                }
            }
            return;
        }
        if crate::abi::mixing_unit_from_source(id).is_some() {
            return;
        }
        if id > 0 {
            *gains.entry(id).or_insert(0.0) += gain;
        }
    }
    for (_, _, _, _, _, state) in snapshot {
        let mix = state.mix.clamp(0.0, 1.0);
        add(state.program_source, 1.0 - mix, &spec_map, &mut gains);
        add(state.preview_source, mix, &spec_map, &mut gains);
        for overlay in state.overlays.iter().take(state.overlay_count as usize) {
            add(overlay.source_id, overlay.opacity.max(0.0), &spec_map, &mut gains);
        }
    }
    gains.into_iter().filter(|(_, gain)| *gain > 1e-4).collect()
}

fn audio_for_source(
    uploads: &UploadStore,
    scenes: &HashMap<u64, (u32, u32, Arc<[OverlayDesc]>)>,
    source_id: u64,
) -> Option<AudioPacket> {
    if let Some(ring) = uploads.get(source_id) {
        if ring.audio.is_some() {
            return ring.audio.clone();
        }
    }
    if crate::abi::is_scene(source_id)
        && let Some((_, _, layers)) = scenes.get(&source_id)
    {
        for layer in layers.iter() {
            if let Some(audio) = audio_for_source(uploads, scenes, layer.source_id) {
                return Some(audio);
            }
        }
    }
    None
}

fn collect_live_ids(
    scene_specs: &[(u64, u32, u32, Arc<[OverlayDesc]>)],
    snapshot: &[(u64, u32, u32, u32, u32, UnitState)],
    monitor_sources: Vec<u64>,
    outputs: &[(u64, u32, u64, u64, u32, u32, Arc<AtomicBool>)],
) -> (HashSet<u64>, HashSet<u64>) {
    let spec_map: HashMap<u64, &[OverlayDesc]> =
        scene_specs.iter().map(|spec| (spec.0, spec.3.as_ref())).collect();
    let mut scenes = HashSet::new();
    let mut uploads = HashSet::new();
    fn add(
        id: u64,
        spec_map: &HashMap<u64, &[OverlayDesc]>,
        scenes: &mut HashSet<u64>,
        uploads: &mut HashSet<u64>,
    ) {
        if crate::abi::is_scene(id) {
            if !scenes.insert(id) {
                return;
            }
            if let Some(layers) = spec_map.get(&id) {
                for layer in *layers {
                    add(layer.source_id, spec_map, scenes, uploads);
                }
            }
            return;
        }
        if crate::abi::mixing_unit_from_source(id).is_some() {
            return;
        }
        if id > 0 {
            uploads.insert(id);
        }
    }
    for (_, _, _, _, _, state) in snapshot {
        add(state.program_source, &spec_map, &mut scenes, &mut uploads);
        add(state.preview_source, &spec_map, &mut scenes, &mut uploads);
        for overlay in state.overlays.iter().take(state.overlay_count as usize) {
            add(overlay.source_id, &spec_map, &mut scenes, &mut uploads);
        }
        for slot in state.mv_slots.iter().take(state.mv_slot_count as usize) {
            add(*slot, &spec_map, &mut scenes, &mut uploads);
        }
    }
    for id in monitor_sources {
        add(id, &spec_map, &mut scenes, &mut uploads);
    }
    for (_, kind, source_id, ..) in outputs {
        match *kind {
            SRC_KIND_SCENE | SRC_KIND_MU_MULTIVIEW => add(*source_id, &spec_map, &mut scenes, &mut uploads),
            SRC_KIND_INPUT => {
                uploads.insert(*source_id);
            }
            _ => {}
        }
    }
    (scenes, uploads)
}

fn send_loop(rx: mpsc::Receiver<SendCmd>, stop: Arc<AtomicBool>) {
    let mut senders: HashMap<u64, (ProgramSender, Arc<AtomicBool>)> = HashMap::new();
    while !stop.load(Ordering::Relaxed) {
        loop {
            match rx.try_recv() {
                Ok(SendCmd::Shutdown) => return,
                Ok(cmd) => apply_send_cmd(&mut senders, cmd),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }
        let mut dead = Vec::new();
        for (&id, (sender, video_sub)) in senders.iter_mut() {
            let pumped = panic::catch_unwind(AssertUnwindSafe(|| sender.pump()));
            match pumped {
                Ok(_) => video_sub.store(sender.video_subscribed(), Ordering::Relaxed),
                Err(_) => dead.push(id),
            }
        }
        for id in dead {
            senders.remove(&id);
        }
        match rx.recv_timeout(Duration::from_millis(2)) {
            Ok(SendCmd::Shutdown) => return,
            Ok(cmd) => apply_send_cmd(&mut senders, cmd),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn apply_send_cmd(
    senders: &mut HashMap<u64, (ProgramSender, Arc<AtomicBool>)>,
    cmd: SendCmd,
) {
    match cmd {
        SendCmd::Add {
            output_id,
            sender,
            video_sub,
        } => {
            senders.insert(output_id, (sender, video_sub));
        }
        SendCmd::Remove { output_id } => {
            senders.remove(&output_id);
        }
        SendCmd::Video {
            output_id,
            width,
            height,
            stride,
            pts,
            data,
            fps_n,
            fps_d,
        } => {
            if let Some((sender, _)) = senders.get_mut(&output_id) {
                let _ = panic::catch_unwind(AssertUnwindSafe(|| {
                    sender.send_video_uyvy(width, height, stride, pts, data, fps_n, fps_d)
                }));
            }
        }
        SendCmd::Audio { output_id, packet } => {
            if let Some((sender, _)) = senders.get_mut(&output_id) {
                let _ = panic::catch_unwind(AssertUnwindSafe(|| sender.send_audio(&packet)));
            }
        }
        SendCmd::Shutdown => {}
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
