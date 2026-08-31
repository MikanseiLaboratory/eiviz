#![deny(unsafe_op_in_unsafe_fn)]

mod abi;
mod audio;
mod compose;
#[cfg(windows)]
mod convert;
mod delay;
mod device;
mod generator_audio;
#[cfg(windows)]
mod dxgi;
#[cfg(windows)]
mod media;
#[cfg(target_os = "macos")]
mod media_macos;
#[cfg(any(windows, target_os = "macos"))]
mod ndi;
#[cfg(target_os = "macos")]
mod main_thread;
mod omt;
mod pool;
mod present;
mod rebar;
mod readback;
mod save;
mod session;
mod upload;

pub use abi::{
    AudioPeak, ERR_ALREADY_CREATED, ERR_DEVICE, ERR_INVALID_ARGUMENT, ERR_IO, ERR_NOT_CREATED,
    GEN_BARS, GEN_SOLID, MixerRebarInfo, MixerStats, MixerVideoInfo, NATIVE_APPKIT_NSVIEW,
    NATIVE_WIN32_HWND, OK,
    OUT_DECKLINK, OUT_NDI, OUT_OMT, OUTPUT_MULTIVIEW, OUTPUT_PREVIEW, OUTPUT_PROGRAM, OverlayDesc,
    Rect, SAVE_FLAG_MULTIVIEW, SAVE_NOT_ON_PREVIEW_OR_PROGRAM, SCENE_BASE, SRC_BARS, SRC_BLACK,
    SRC_BLUE, SRC_COLOR, SRC_KIND_INPUT, SRC_KIND_MU_MULTIVIEW, SRC_KIND_MU_PREVIEW,
    SRC_KIND_MU_PROGRAM, SRC_KIND_SCENE, SourceUsage, UnitState,
};
pub use audio::{AudioBusInfo, AudioDeviceInfo};

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{CStr, c_char};
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use abi::NativeSurface;
use compose::{Composer, Generator};
use delay::FrameDelay;
use device::GpuDevice;
#[cfg(windows)]
use dxgi::GpuVideoContext;
#[cfg(windows)]
use media::VideoPump;
#[cfg(target_os = "macos")]
use media_macos::VideoPump;
#[cfg(any(windows, target_os = "macos"))]
use ndi::{NdiReceiver, NdiSender};
use omt::{GpuSendStore, OmtGpu, OmtReceiver, ProgramSender, omt_gpu_from_device};
use present::Presenters;
use readback::ReadbackStore;
use save::{LiveSave, collect_source_roles, want_full};
use upload::{AUDIO_RATE, AudioPacket, CpuFormat, UploadStore};

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
    use_gpu: bool,
}

#[derive(Clone)]
struct OutputSnap {
    output_id: u64,
    source_kind: u32,
    source_id: u64,
    unit_id: u64,
    fps_n: u32,
    fps_d: u32,
    video_sub: Arc<AtomicBool>,
    use_gpu: bool,
}

impl OutputSnap {
    fn cpu_video(&self) -> bool {
        !self.use_gpu && self.video_sub.load(Ordering::Relaxed)
    }

    fn gpu_video(&self) -> bool {
        self.use_gpu && self.video_sub.load(Ordering::Relaxed)
    }
}

struct Shared {
    master_fps_num: u32,
    master_fps_den: u32,
    units: HashMap<u64, LiveUnit>,
    scenes: HashMap<u64, (u32, u32, Arc<[crate::abi::OverlayDesc]>)>,
    uploads: Arc<Mutex<UploadStore>>,
    #[cfg(windows)]
    gpu_video: Option<GpuVideoContext>,
    omt_gpu: OmtGpu,
    receivers: HashMap<u64, LiveReceiver>,
    #[cfg(any(windows, target_os = "macos"))]
    videos: HashMap<u64, VideoPump>,
    outputs: HashMap<u64, LiveOutput>,
    generators: HashMap<u64, Generator>,
    tone_phase: HashMap<u64, f64>,
    live_save: HashMap<u64, LiveSave>,
    multiview_binds: HashMap<u64, (u64, u64)>,
    frame_buffer_frames: u32,
    rebar: crate::rebar::RebarSnapshot,
    rebar_optimization: bool,
    rebar_direct_sample: bool,
    audio: audio::AudioEngine,
}

/// Host-visible status that must not share the control lock with ingest or render.
struct Telemetry {
    last_error: String,
    last_render_ms: f32,
    follow_primed: bool,
    monitor_pcm: VecDeque<f32>,
}

enum LiveReceiver {
    Omt(OmtReceiver),
    #[cfg(any(windows, target_os = "macos"))]
    Ndi(NdiReceiver),
}

impl LiveReceiver {
    fn apply_save(&self, full: bool, on_program: bool, on_preview: bool) {
        match self {
            Self::Omt(receiver) => receiver.apply_save(full, on_program, on_preview),
            // NDI bandwidth save needs Advanced SDK; see NdiReceiver.
            #[cfg(any(windows, target_os = "macos"))]
            Self::Ndi(_) => {}
        }
    }
}

enum OutputHandle {
    Omt(ProgramSender),
    #[cfg(any(windows, target_os = "macos"))]
    Ndi(NdiSender),
}

impl OutputHandle {
    fn pump(&mut self) -> Result<bool, String> {
        match self {
            Self::Omt(sender) => {
                sender.pump()?;
                Ok(sender.video_subscribed())
            }
            #[cfg(any(windows, target_os = "macos"))]
            Self::Ndi(sender) => sender.pump(),
        }
    }

    fn send_video_uyvy(
        &mut self,
        width: u32,
        height: u32,
        stride: u32,
        pts: i64,
        data: Arc<[u8]>,
        fps_n: u32,
        fps_d: u32,
    ) -> Result<(), String> {
        match self {
            Self::Omt(sender) => {
                sender.send_video_uyvy(width, height, stride, pts, data, fps_n, fps_d)
            }
            #[cfg(any(windows, target_os = "macos"))]
            Self::Ndi(sender) => {
                sender.send_video_uyvy(width, height, stride, pts, &data, fps_n, fps_d)
            }
        }
    }

    fn send_video_texture(
        &mut self,
        omt_gpu: &OmtGpu,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
        pts: i64,
        fps_n: u32,
        fps_d: u32,
    ) -> Result<(), String> {
        match self {
            Self::Omt(sender) => {
                sender.send_video_texture(omt_gpu, texture, width, height, pts, fps_n, fps_d)
            }
            #[cfg(any(windows, target_os = "macos"))]
            Self::Ndi(_) => Ok(()),
        }
    }

    fn send_audio(&mut self, packet: &AudioPacket) -> Result<(), String> {
        match self {
            Self::Omt(sender) => sender.send_audio(packet),
            #[cfg(any(windows, target_os = "macos"))]
            Self::Ndi(sender) => sender.send_audio(packet),
        }
    }
}

enum SendCmd {
    Add {
        output_id: u64,
        sender: OutputHandle,
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
    GpuVideo {
        output_id: u64,
        texture: wgpu::Texture,
        width: u32,
        height: u32,
        pts: i64,
        fps_n: u32,
        fps_d: u32,
        busy: Arc<AtomicBool>,
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
        surface: NativeSurface,
        width: u32,
        height: u32,
        prepared: Option<present::PreparedSurface>,
        reply: mpsc::Sender<i32>,
    },
    Resize {
        unit_id: u64,
        kind: u32,
        surface: NativeSurface,
        width: u32,
        height: u32,
    },
    Detach {
        unit_id: u64,
        kind: u32,
        surface: NativeSurface,
    },
    DetachUnit {
        unit_id: u64,
    },
    AttachMonitor {
        monitor_id: u64,
        source_id: u64,
        surface: NativeSurface,
        width: u32,
        height: u32,
        prepared: Option<present::PreparedSurface>,
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
    SetMonitorInterval {
        monitor_id: u64,
        frames: u32,
    },
    Shutdown,
}

struct Mixer {
    shared: Arc<Mutex<Shared>>,
    uploads: Arc<Mutex<UploadStore>>,
    telemetry: Arc<Mutex<Telemetry>>,
    cmds: mpsc::Sender<GpuCmd>,
    send_tx: mpsc::Sender<SendCmd>,
    render: Option<JoinHandle<()>>,
    send: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    #[cfg(target_os = "macos")]
    surface_gpu: present::SurfaceGpu,
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

/// Send a GPU command that replies, without holding the mixer slot while waiting.
/// Holding the slot across `recv` deadlocks if the render thread needs the host.
fn send_gpu_and_wait(send: impl FnOnce(&Mixer, mpsc::Sender<i32>) -> i32) -> i32 {
    let (reply_tx, reply_rx) = mpsc::channel();
    match with_mixer(|mixer| send(mixer, reply_tx)) {
        Ok(OK) => reply_rx.recv().unwrap_or(ERR_DEVICE),
        Ok(code) => code,
        Err(code) => code,
    }
}

fn set_error(telemetry: &Mutex<Telemetry>, message: impl Into<String>) {
    telemetry.lock().expect("telemetry").last_error = message.into();
}

fn with_uploads<T>(mixer: &Mixer, f: impl FnOnce(&mut UploadStore) -> T) -> T {
    f(&mut mixer.uploads.lock().expect("uploads"))
}

fn session_error_slot() -> &'static Mutex<String> {
    static SLOT: OnceLock<Mutex<String>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(String::new()))
}

fn report_session_error(message: impl Into<String>) {
    let message = message.into();
    *session_error_slot().lock().expect("session error") = message.clone();
    let _ = with_mixer(|mixer| set_error(&mixer.telemetry, message));
}

fn copy_bytes(src: &[u8], out: *mut u8, cap: usize) -> i32 {
    if src.len() > cap {
        report_session_error(format!("session buffer too small (need {} bytes)", src.len()));
        return -1;
    }
    if !src.is_empty() {
        unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), out, src.len()) };
    }
    src.len() as i32
}

#[cfg(target_os = "macos")]
fn prepare_surface_off_slot(
    surface: NativeSurface,
    width: u32,
    height: u32,
) -> Result<present::PreparedSurface, i32> {
    let gpu = match with_mixer(|mixer| mixer.surface_gpu.clone()) {
        Ok(gpu) => gpu,
        Err(code) => return Err(code),
    };
    crate::main_thread::run_on_main(move || {
        present::prepare_surface(
            &gpu.instance,
            &gpu.adapter,
            &gpu.device,
            surface,
            width,
            height,
        )
    })
    .map_err(|error| {
        let _ = with_mixer(|mixer| set_error(&mixer.telemetry, error));
        ERR_DEVICE
    })
}

/// Creates the OS-fixed wgpu device (DX12 on Windows, Metal on macOS).
#[unsafe(no_mangle)]
pub extern "C" fn mixer_create(_adapter_luid: u64, fps_num: u32, fps_den: u32) -> i32 {
    if fps_num == 0 || fps_den == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    let mut slot = mixer_slot().lock().expect("mixer mutex poisoned");
    if slot.is_some() {
        return ERR_ALREADY_CREATED;
    }
    let device = match GpuDevice::new() {
        Ok(device) => device,
        Err(_) => return ERR_DEVICE,
    };
    #[cfg(target_os = "macos")]
    let surface_gpu = present::SurfaceGpu {
        instance: device.instance.clone(),
        adapter: device.adapter.clone(),
        device: device.device.clone(),
    };
    #[cfg(windows)]
    let gpu_video = match GpuVideoContext::new(&device) {
        Ok(ctx) => Some(ctx),
        Err(error) => {
            eprintln!("eiviz dxgi video: {error}");
            return ERR_DEVICE;
        }
    };
    let omt_gpu = omt_gpu_from_device(&device);
    let rebar = crate::rebar::probe(&device);
    let uploads = Arc::new(Mutex::new(UploadStore::default()));
    let telemetry = Arc::new(Mutex::new(Telemetry {
        last_error: String::new(),
        last_render_ms: 0.0,
        follow_primed: false,
        monitor_pcm: VecDeque::new(),
    }));
    let audio = audio::AudioEngine::new();
    let shared = Arc::new(Mutex::new(Shared {
        master_fps_num: fps_num,
        master_fps_den: fps_den,
        units: HashMap::new(),
        scenes: HashMap::new(),
        uploads: Arc::clone(&uploads),
        #[cfg(windows)]
        gpu_video,
        omt_gpu: omt_gpu.clone(),
        receivers: HashMap::new(),
        #[cfg(any(windows, target_os = "macos"))]
        videos: HashMap::new(),
        outputs: HashMap::new(),
        generators: HashMap::new(),
        tone_phase: HashMap::new(),
        live_save: HashMap::new(),
        frame_buffer_frames: 3,
        rebar,
        rebar_optimization: true,
        rebar_direct_sample: false,
        audio: audio.clone(),
        multiview_binds: HashMap::new(),
    }));
    let (tx, rx) = mpsc::channel();
    let (send_tx, send_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let render_shared = Arc::clone(&shared);
    let render_uploads = Arc::clone(&uploads);
    let render_telemetry = Arc::clone(&telemetry);
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
                render_uploads,
                render_telemetry,
                rx,
                render_send,
                render_stop,
            );
        })
        .expect("render thread");
    let send_stop = Arc::clone(&stop);
    let send_gpu = omt_gpu;
    let send = thread::Builder::new()
        .name("eiviz-omt-send".into())
        .spawn(move || send_loop(send_rx, send_stop, send_gpu))
        .expect("omt send thread");
    *slot = Some(Mixer {
        shared,
        uploads,
        telemetry,
        cmds: tx,
        send_tx,
        render: Some(render),
        send: Some(send),
        stop,
        #[cfg(target_os = "macos")]
        surface_gpu,
    });
    #[cfg(any(windows, target_os = "macos"))]
    let _ = thread::Builder::new()
        .name("eiviz-ndi-find".into())
        .spawn(ndi::warm_finder);
    OK
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_destroy() {
    let Some(mut mixer) = mixer_slot().lock().expect("mixer mutex poisoned").take() else {
        return;
    };
    mixer.stop.store(true, Ordering::Relaxed);
    let audio = mixer.shared.lock().expect("shared").audio.clone();
    audio.shutdown();
    let _ = mixer.cmds.send(GpuCmd::Shutdown);
    if let Some(join) = mixer.render.take() {
        let _ = join.join();
    }
    let _ = mixer.send_tx.send(SendCmd::Shutdown);
    if let Some(join) = mixer.send.take() {
        let _ = join.join();
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
        shared
            .audio
            .set_unit_link(unit_id, audio::MASTER_BUS, audio::LINK_FOLLOW);
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
        mixer
            .shared
            .lock()
            .expect("shared")
            .scenes
            .remove(&scene_id);
        mixer
            .shared
            .lock()
            .expect("shared")
            .multiview_binds
            .remove(&scene_id);
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
        let mut shared = mixer.shared.lock().expect("shared");
        let previous = shared.generators.get(&id).copied();
        shared.generators.insert(
            id,
            Generator {
                kind,
                color: [r, g, b, a],
                scroll: scroll != 0,
                tone_hz: previous.map(|item| item.tone_hz).unwrap_or(0.0),
                tone_level_dbfs: previous.map(|item| item.tone_level_dbfs).unwrap_or(-20.0),
            },
        );
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_generator_set_tone(id: u64, hz: f32, level_dbfs: f32) -> i32 {
    with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        let kind = if id == SRC_BARS { GEN_BARS } else { GEN_SOLID };
        let entry = shared.generators.entry(id).or_insert_with(|| Generator {
            kind,
            ..Generator::default()
        });
        entry.tone_hz = hz.max(0.0);
        entry.tone_level_dbfs = level_dbfs.clamp(-120.0, 0.0);
        if hz <= 0.0 {
            shared.tone_phase.remove(&id);
        }
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
    mixer_unit_attach_native(unit_id, kind, NATIVE_WIN32_HWND, hwnd, width, height)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_unit_attach_native(
    unit_id: u64,
    kind: u32,
    native_kind: u32,
    handle: isize,
    width: u32,
    height: u32,
) -> i32 {
    if width == 0 || height == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    let Ok(surface) = NativeSurface::parse(native_kind, handle) else {
        return ERR_INVALID_ARGUMENT;
    };
    #[cfg(target_os = "macos")]
    let prepared = match prepare_surface_off_slot(surface, width, height) {
        Ok(prepared) => Some(prepared),
        Err(code) => return code,
    };
    #[cfg(not(target_os = "macos"))]
    let prepared = None;
    send_gpu_and_wait(|mixer, reply| {
        if !mixer
            .shared
            .lock()
            .expect("shared")
            .units
            .contains_key(&unit_id)
        {
            return ERR_INVALID_ARGUMENT;
        }
        if mixer
            .cmds
            .send(GpuCmd::Attach {
                unit_id,
                kind,
                surface,
                width,
                height,
                prepared,
                reply,
            })
            .is_err()
        {
            return ERR_DEVICE;
        }
        OK
    })
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
        std::mem::swap(
            &mut unit.state.program_source,
            &mut unit.state.preview_source,
        );
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
    mixer_unit_resize_native(unit_id, kind, NATIVE_WIN32_HWND, hwnd, width, height)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_unit_resize_native(
    unit_id: u64,
    kind: u32,
    native_kind: u32,
    handle: isize,
    width: u32,
    height: u32,
) -> i32 {
    if width == 0 || height == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    let Ok(surface) = NativeSurface::parse(native_kind, handle) else {
        return ERR_INVALID_ARGUMENT;
    };
    with_mixer(|mixer| {
        if mixer
            .cmds
            .send(GpuCmd::Resize {
                unit_id,
                kind,
                surface,
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
    mixer_unit_detach_native(unit_id, kind, NATIVE_WIN32_HWND, hwnd)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_unit_detach_native(
    unit_id: u64,
    kind: u32,
    native_kind: u32,
    handle: isize,
) -> i32 {
    let Ok(surface) = NativeSurface::parse(native_kind, handle) else {
        return ERR_INVALID_ARGUMENT;
    };
    with_mixer(|mixer| {
        let _ = mixer.cmds.send(GpuCmd::Detach {
            unit_id,
            kind,
            surface,
        });
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
    mixer_attach_monitor_native(
        monitor_id,
        source_id,
        NATIVE_WIN32_HWND,
        hwnd,
        width,
        height,
    )
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_attach_monitor_native(
    monitor_id: u64,
    source_id: u64,
    native_kind: u32,
    handle: isize,
    width: u32,
    height: u32,
) -> i32 {
    if width == 0 || height == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    let Ok(surface) = NativeSurface::parse(native_kind, handle) else {
        return ERR_INVALID_ARGUMENT;
    };
    #[cfg(target_os = "macos")]
    let prepared = match prepare_surface_off_slot(surface, width, height) {
        Ok(prepared) => Some(prepared),
        Err(code) => return code,
    };
    #[cfg(not(target_os = "macos"))]
    let prepared = None;
    send_gpu_and_wait(|mixer, reply| {
        if mixer
            .cmds
            .send(GpuCmd::AttachMonitor {
                monitor_id,
                source_id,
                surface,
                width,
                height,
                prepared,
                reply,
            })
            .is_err()
        {
            return ERR_DEVICE;
        }
        OK
    })
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
        with_uploads(mixer, |uploads| uploads.register(id, width, height, format));
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
        match with_uploads(mixer, |uploads| uploads.push(id, src, stride as usize, pts)) {
            Ok(()) => OK,
            Err(error) => {
                set_error(&mixer.telemetry, error);
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
        with_uploads(mixer, |uploads| {
            uploads.push_audio(id, sample_rate, channels, frames, pts, samples);
        });
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
    let path = unsafe { CStr::from_ptr(path) }.to_str().unwrap_or_default();
    let image = match image::open(Path::new(path)) {
        Ok(image) => image.to_rgba8(),
        Err(error) => {
            let _ = with_mixer(|mixer| {
                set_error(&mixer.telemetry, error.to_string());
            });
            return ERR_IO;
        }
    };
    let (width, height) = image.dimensions();
    with_mixer(|mixer| {
        with_uploads(mixer, |uploads| {
            uploads.register(id, width, height, CpuFormat::Rgba);
            match uploads.push(id, &image, width as usize * 4, 0) {
                Ok(()) => OK,
                Err(_) => ERR_IO,
            }
        })
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
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (id, path, capture, format);
        return with_mixer(|mixer| {
            set_error(&mixer.telemetry, "Video ingest is not available");
            ERR_IO
        })
        .unwrap_or_else(|code| code);
    }
    #[cfg(target_os = "macos")]
    {
        let _ = format;
        return with_mixer(|mixer| {
            let uploads = {
                let mut shared = mixer.shared.lock().expect("shared");
                let previous = shared.videos.remove(&id);
                drop(shared.receivers.remove(&id));
                let uploads = shared.uploads.clone();
                drop(shared);
                drop(previous);
                uploads
            };
            match VideoPump::start(id, path, capture != 0, uploads) {
                Ok(pump) => {
                    mixer.shared.lock().expect("shared").videos.insert(id, pump);
                    OK
                }
                Err(error) => {
                    set_error(&mixer.telemetry, error);
                    ERR_IO
                }
            }
        })
        .unwrap_or_else(|code| code);
    }
    #[cfg(windows)]
    with_mixer(|mixer| {
        let (uploads, gpu) = {
            let mut shared = mixer.shared.lock().expect("shared");
            let previous = shared.videos.remove(&id);
            drop(shared.receivers.remove(&id));
            let uploads = shared.uploads.clone();
            let Some(gpu) = shared.gpu_video.clone() else {
                return ERR_DEVICE;
            };
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
                set_error(&mixer.telemetry, error);
                ERR_IO
            }
        }
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_video_set_playing(id: u64, playing: u32) -> i32 {
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (id, playing);
        return with_mixer(|_| ERR_INVALID_ARGUMENT).unwrap_or_else(|code| code);
    }
    #[cfg(any(windows, target_os = "macos"))]
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
pub extern "C" fn mixer_video_set_loop(id: u64, looping: u32) -> i32 {
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (id, looping);
        return with_mixer(|_| ERR_INVALID_ARGUMENT).unwrap_or_else(|code| code);
    }
    #[cfg(any(windows, target_os = "macos"))]
    with_mixer(|mixer| {
        let shared = mixer.shared.lock().expect("shared");
        let Some(pump) = shared.videos.get(&id) else {
            return ERR_INVALID_ARGUMENT;
        };
        pump.set_looping(looping != 0);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_video_seek(id: u64, hns: i64) -> i32 {
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (id, hns);
        return with_mixer(|_| ERR_INVALID_ARGUMENT).unwrap_or_else(|code| code);
    }
    #[cfg(any(windows, target_os = "macos"))]
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
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = id;
        return with_mixer(|_| ERR_INVALID_ARGUMENT).unwrap_or_else(|code| code);
    }
    #[cfg(any(windows, target_os = "macos"))]
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
pub unsafe extern "C" fn mixer_omt_connect(
    id: u64,
    address: *const c_char,
    use_gpu: u32,
    frame_buffer_frames: u32,
    quality: u32,
) -> i32 {
    if address.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let address = unsafe { CStr::from_ptr(address) }
        .to_str()
        .unwrap_or_default()
        .to_string();
    let depth = frame_buffer_frames.clamp(1, 8);
    with_mixer(|mixer| {
        let (uploads, gpu) = {
            let mut shared = mixer.shared.lock().expect("shared");
            #[cfg(any(windows, target_os = "macos"))]
            let previous = shared.videos.remove(&id);
            drop(shared.receivers.remove(&id));
            let uploads = shared.uploads.clone();
            let gpu = if use_gpu != 0 {
                Some(shared.omt_gpu.clone())
            } else {
                None
            };
            drop(shared);
            #[cfg(any(windows, target_os = "macos"))]
            drop(previous);
            (uploads, gpu)
        };
        match OmtReceiver::start(id, address, uploads, gpu, depth, quality) {
            Ok(receiver) => {
                mixer
                    .shared
                    .lock()
                    .expect("shared")
                    .receivers
                    .insert(id, LiveReceiver::Omt(receiver));
                OK
            }
            Err(error) => {
                set_error(&mixer.telemetry, error);
                ERR_IO
            }
        }
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_ndi_connect(
    id: u64,
    address: *const c_char,
    frame_buffer_frames: u32,
    low_bandwidth: u32,
) -> i32 {
    if address.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let address = unsafe { CStr::from_ptr(address) }
        .to_str()
        .unwrap_or_default()
        .to_string();
    let depth = frame_buffer_frames.clamp(1, 8);
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (id, address, depth, low_bandwidth);
        return with_mixer(|mixer| {
            set_error(&mixer.telemetry, "NDI is not available");
            ERR_IO
        })
        .unwrap_or_else(|code| code);
    }
    #[cfg(any(windows, target_os = "macos"))]
    with_mixer(|mixer| {
        let uploads = {
            let mut shared = mixer.shared.lock().expect("shared");
            #[cfg(any(windows, target_os = "macos"))]
            let previous = shared.videos.remove(&id);
            drop(shared.receivers.remove(&id));
            let uploads = shared.uploads.clone();
            drop(shared);
            #[cfg(any(windows, target_os = "macos"))]
            drop(previous);
            uploads
        };
        match NdiReceiver::start(id, address, uploads, depth, low_bandwidth) {
            Ok(receiver) => {
                mixer
                    .shared
                    .lock()
                    .expect("shared")
                    .receivers
                    .insert(id, LiveReceiver::Ndi(receiver));
                OK
            }
            Err(error) => {
                set_error(&mixer.telemetry, error);
                ERR_IO
            }
        }
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_set_live_save(id: u64, mode: u32, flags: u32) -> i32 {
    if id == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        mixer.shared.lock().expect("shared").live_save.insert(
            id,
            LiveSave {
                mode,
                flags: flags & SAVE_FLAG_MULTIVIEW,
            },
        );
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_omt_set_quality(id: u64, quality: u32) -> i32 {
    if id == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let shared = mixer.shared.lock().expect("shared");
        if let Some(LiveReceiver::Omt(receiver)) = shared.receivers.get(&id) {
            receiver.set_quality(quality);
        }
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_omt_start_send(unit_id: u64, name: *const c_char) -> i32 {
    if name.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    unsafe { mixer_output_add(unit_id, OUT_OMT, name, SRC_KIND_MU_PROGRAM, 0, unit_id, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_output_add(
    output_id: u64,
    transport: u32,
    name: *const c_char,
    source_kind: u32,
    source_id: u64,
    unit_id: u64,
    use_gpu: u32,
) -> i32 {
    if name.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let name = unsafe { CStr::from_ptr(name) }
        .to_str()
        .unwrap_or_default()
        .to_string();
    if transport == OUT_DECKLINK {
        let _ = with_mixer(|mixer| {
            set_error(&mixer.telemetry, "DeckLink output is not linked in this build");
        });
        return ERR_IO;
    }
    let use_gpu = transport == OUT_OMT && use_gpu != 0;
    let handle = match transport {
        OUT_NDI => {
            #[cfg(not(any(windows, target_os = "macos")))]
            {
                let _ = with_mixer(|mixer| set_error(&mixer.telemetry, "NDI is not available"));
                return ERR_IO;
            }
            #[cfg(any(windows, target_os = "macos"))]
            {
                let started = panic::catch_unwind(AssertUnwindSafe(|| NdiSender::start(&name)));
                match started {
                    Ok(Ok(sender)) => OutputHandle::Ndi(sender),
                    Ok(Err(error)) => {
                        let _ = with_mixer(|mixer| set_error(&mixer.telemetry, error));
                        return ERR_IO;
                    }
                    Err(_) => {
                        let _ = with_mixer(|mixer| {
                            set_error(&mixer.telemetry, "NDI sender panicked during create")
                        });
                        return ERR_IO;
                    }
                }
            }
        }
        OUT_OMT => {
            let started = panic::catch_unwind(AssertUnwindSafe(|| ProgramSender::start(&name)));
            match started {
                Ok(Ok(sender)) => OutputHandle::Omt(sender),
                Ok(Err(error)) => {
                    let _ = with_mixer(|mixer| set_error(&mixer.telemetry, error));
                    return ERR_IO;
                }
                Err(_) => {
                    let _ = with_mixer(|mixer| {
                        set_error(&mixer.telemetry, "OMT sender panicked during create")
                    });
                    return ERR_IO;
                }
            }
        }
        _ => return ERR_INVALID_ARGUMENT,
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
                use_gpu,
            },
        );
        let _ = mixer.send_tx.send(SendCmd::Add {
            output_id,
            sender: handle,
            video_sub,
        });
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_output_remove(output_id: u64) -> i32 {
    with_mixer(|mixer| {
        mixer
            .shared
            .lock()
            .expect("shared")
            .outputs
            .remove(&output_id);
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
pub unsafe extern "C" fn mixer_ndi_discover(out: *mut u8, cap: usize) -> i32 {
    if out.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (out, cap);
        let _ = with_mixer(|mixer| set_error(&mixer.telemetry, "NDI is not available"));
        return 0;
    }
    #[cfg(any(windows, target_os = "macos"))]
    match ndi::discover_sources() {
        Ok(addresses) => {
            let text = addresses.join("\n");
            let bytes = text.as_bytes();
            let n = bytes.len().min(cap);
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, n) };
            n as i32
        }
        Err(error) => {
            let _ = with_mixer(|mixer| set_error(&mixer.telemetry, error));
            0
        }
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
    let error = match with_mixer(|mixer| mixer.telemetry.lock().expect("telemetry").last_error.clone()) {
        Ok(error) => error,
        Err(code) => return code,
    };
    let n = error.len().min(cap);
    unsafe { std::ptr::copy_nonoverlapping(error.as_ptr(), out, n) };
    n as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_session_load(path: *const c_char, out: *mut u8, cap: usize) -> i32 {
    if path.is_null() || out.is_null() || cap == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    match session::load_file(&read_cstr(path)) {
        Ok(bytes) => copy_bytes(&bytes, out, cap),
        Err(error) => {
            report_session_error(error);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_session_save(
    path: *const c_char,
    json: *const u8,
    len: usize,
) -> i32 {
    if path.is_null() || json.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let bytes = unsafe { std::slice::from_raw_parts(json, len) };
    match session::save_file(&read_cstr(path), bytes) {
        Ok(()) => OK,
        Err(error) => {
            report_session_error(error);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_session_canonicalize(
    json: *const u8,
    len: usize,
    out: *mut u8,
    cap: usize,
) -> i32 {
    if json.is_null() || out.is_null() || cap == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    let bytes = unsafe { std::slice::from_raw_parts(json, len) };
    match session::canonicalize_bytes(bytes) {
        Ok(canonical) => copy_bytes(&canonical, out, cap),
        Err(error) => {
            report_session_error(error);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_destroy_source(id: u64) -> i32 {
    with_mixer(|mixer| {
        let uploads = {
            let mut shared = mixer.shared.lock().expect("shared");
            #[cfg(any(windows, target_os = "macos"))]
            let previous = shared.videos.remove(&id);
            shared.receivers.remove(&id);
            shared.generators.remove(&id);
            shared.tone_phase.remove(&id);
            shared.live_save.remove(&id);
            let uploads = shared.uploads.clone();
            drop(shared);
            #[cfg(any(windows, target_os = "macos"))]
            drop(previous);
            uploads
        };
        uploads.lock().expect("uploads").unregister(id);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_flush_audio(id: u64) -> i32 {
    with_mixer(|mixer| {
        with_uploads(mixer, |uploads| uploads.flush_audio(id));
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_audio_bus_upsert(
    id: u64,
    name: *const c_char,
    role: u32,
    device_kind: u32,
    device_id: *const c_char,
    map_left: i32,
    map_right: i32,
    exclusive: u32,
) -> i32 {
    if id == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    let name = read_cstr(name);
    let device_id = read_cstr(device_id);
    with_mixer(|mixer| {
        let audio = mixer.shared.lock().expect("shared").audio.clone();
        audio.upsert_bus(
            id,
            &name,
            role,
            device_kind,
            &device_id,
            map_left,
            map_right,
            exclusive,
        );
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_audio_bus_remove(id: u64) -> i32 {
    with_mixer(|mixer| {
        let audio = mixer.shared.lock().expect("shared").audio.clone();
        audio.remove_bus(id);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_audio_bus_count() -> i32 {
    with_mixer(|mixer| {
        mixer
            .shared
            .lock()
            .expect("shared")
            .audio
            .graph()
            .lock()
            .expect("audio")
            .buses
            .len() as i32
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_audio_bus_get(index: u32, out: *mut AudioBusInfo) -> i32 {
    if out.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let graph = mixer.shared.lock().expect("shared").audio.graph();
        let graph = graph.lock().expect("audio");
        let Some(bus) = graph.buses.get(index as usize) else {
            return ERR_INVALID_ARGUMENT;
        };
        unsafe {
            *out = AudioBusInfo {
                id: bus.id,
                role: bus.role,
                device_kind: bus.device_kind,
                map_left: bus.map_left,
                map_right: bus.map_right,
                exclusive: u32::from(bus.exclusive),
                bit: bus.bit,
                name: write_fixed::<64>(&bus.name),
                device_id: write_fixed::<256>(&bus.device_id),
            };
        }
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_audio_set_input(id: u64, bus_mask: u32, gain: f32, mute: u32) -> i32 {
    with_mixer(|mixer| {
        mixer
            .shared
            .lock()
            .expect("shared")
            .audio
            .set_input(id, bus_mask, gain, mute);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_audio_set_bus_gain(id: u64, gain: f32, mute: u32) -> i32 {
    with_mixer(|mixer| {
        mixer
            .shared
            .lock()
            .expect("shared")
            .audio
            .set_bus_gain(id, gain, mute);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_audio_set_unit_link(unit_id: u64, bus_id: u64, mode: u32) -> i32 {
    with_mixer(|mixer| {
        mixer
            .shared
            .lock()
            .expect("shared")
            .audio
            .set_unit_link(unit_id, bus_id, mode);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_audio_set_headphone_cue(unit_id: u64) -> i32 {
    with_mixer(|mixer| {
        mixer
            .shared
            .lock()
            .expect("shared")
            .audio
            .set_headphone_cue(unit_id);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_audio_set_headphone_copy_master(enabled: u32) -> i32 {
    with_mixer(|mixer| {
        mixer
            .shared
            .lock()
            .expect("shared")
            .audio
            .set_headphone_copy_master(enabled);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_audio_enum_devices(
    kind: u32,
    out: *mut AudioDeviceInfo,
    cap: u32,
) -> i32 {
    if out.is_null() || cap == 0 {
        return 0;
    }
    let dest = unsafe { std::slice::from_raw_parts_mut(out, cap as usize) };
    audio::enumerate_devices(kind, dest) as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_audio_device_channels(kind: u32, device_id: *const c_char) -> i32 {
    audio::device_channels(kind, &read_cstr(device_id))
}

fn read_cstr(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .unwrap_or_default()
        .to_string()
}

fn write_fixed<const N: usize>(text: &str) -> [u8; N] {
    let mut buf = [0u8; N];
    let bytes = text.as_bytes();
    let n = bytes.len().min(N.saturating_sub(1));
    buf[..n].copy_from_slice(&bytes[..n]);
    buf
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
        let mut telemetry = mixer.telemetry.lock().expect("telemetry");
        if !telemetry.follow_primed {
            dest.fill(0.0);
            return 0;
        }
        let n = dest.len().min(telemetry.monitor_pcm.len());
        for slot in dest.iter_mut().take(n) {
            *slot = telemetry.monitor_pcm.pop_front().unwrap_or(0.0);
        }
        let hold = dest
            .get(n.saturating_sub(1).min(dest.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0.0);
        for slot in dest.iter_mut().skip(n) {
            *slot = hold;
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
        let (master, buses, uploads) = {
            let shared = mixer.shared.lock().expect("shared");
            let master = shared.audio.master_peak();
            let buses = shared.audio.bus_peaks();
            let uploads = Arc::clone(&mixer.uploads);
            drop(shared);
            (master, buses, uploads)
        };
        let uploads = uploads.lock().expect("uploads");
        let mut n = 0u32;
        if n < cap {
            let (left, right) = master;
            unsafe {
                *out.add(n as usize) = AudioPeak {
                    source_id: 0,
                    left,
                    right,
                };
            }
            n += 1;
        }
        for (id, left, right) in buses {
            if n >= cap {
                break;
            }
            unsafe {
                *out.add(n as usize) = AudioPeak {
                    source_id: crate::abi::AUDIO_BUS_PEAK_BASE | id,
                    left,
                    right,
                };
            }
            n += 1;
        }
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
            let (width, height) = if id == SRC_BARS { (1920u32, 1080u32) } else { (128, 72) };
            unsafe {
                *out.add(n as usize) = SourceUsage {
                    source_id: id,
                    width,
                    height,
                    ram_bytes: 0,
                    vram_bytes: u64::from(width) * u64::from(height) * 4,
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
        let (num, den) = {
            let shared = mixer.shared.lock().expect("shared");
            (shared.master_fps_num, shared.master_fps_den)
        };
        let render_ms = mixer.telemetry.lock().expect("telemetry").last_render_ms;
        let budget = 1000.0 * den as f32 / num.max(1) as f32;
        unsafe {
            *out = MixerStats {
                render_ms,
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_copy_rebar_info(out: *mut MixerRebarInfo) -> i32 {
    if out.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let shared = mixer.shared.lock().expect("shared");
        let snap = shared.rebar;
        let active = snap.available && shared.rebar_optimization;
        unsafe {
            *out = MixerRebarInfo {
                available: u32::from(snap.available),
                active: u32::from(active),
                uma: u32::from(snap.uma),
                gpu_upload_heaps: u32::from(snap.gpu_upload_heaps),
                bar_bytes: snap.bar_bytes,
                vram_bytes: snap.vram_bytes,
                adapter: snap.adapter,
            };
        }
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_set_rebar_optimization(enabled: u32) -> i32 {
    with_mixer(|mixer| {
        mixer.shared.lock().expect("shared").rebar_optimization = enabled != 0;
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_set_rebar_direct_sample(enabled: u32) -> i32 {
    with_mixer(|mixer| {
        mixer.shared.lock().expect("shared").rebar_direct_sample = enabled != 0;
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_set_monitor_present_interval(monitor_id: u64, frames: u32) -> i32 {
    let frames = frames.clamp(1, 8);
    with_mixer(|mixer| {
        let _ = mixer
            .cmds
            .send(GpuCmd::SetMonitorInterval { monitor_id, frames });
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
    telemetry: Arc<Mutex<Telemetry>>,
    cmds: mpsc::Receiver<GpuCmd>,
    send_tx: mpsc::Sender<SendCmd>,
    stop: Arc<AtomicBool>,
) {
    let mut composer = match Composer::new(&device) {
        Ok(composer) => composer,
        Err(error) => {
            set_error(&telemetry, error);
            return;
        }
    };
    let mut presenters = Presenters::default();
    let mut readbacks = ReadbackStore::default();
    let mut gpu_sends = GpuSendStore::default();
    let mut frame_delay = FrameDelay::new(3);
    let frame_dt = Duration::from_nanos(1_000_000_000u64 * u64::from(fps_den) / u64::from(fps_num));
    let mut next = Instant::now();
    let clock_start = Instant::now();
    let mut frame_i = 0u64;
    let mut audio_produced = 0u64;
    let mut audio_carry = 0u64;
    while !stop.load(Ordering::Relaxed) {
        while let Ok(cmd) = cmds.try_recv() {
            match cmd {
                GpuCmd::Attach {
                    unit_id,
                    kind,
                    surface,
                    width,
                    height,
                    prepared,
                    reply,
                } => {
                    let code = match presenters.attach(
                        &device,
                        unit_id,
                        kind,
                        surface,
                        width,
                        height,
                        prepared,
                    ) {
                        Ok(()) => OK,
                        Err(error) => {
                            set_error(&telemetry, error);
                            ERR_DEVICE
                        }
                    };
                    let _ = reply.send(code);
                }
                GpuCmd::Resize {
                    unit_id,
                    kind,
                    surface,
                    width,
                    height,
                } => presenters.resize(&device, unit_id, kind, surface, width, height),
                GpuCmd::Detach {
                    unit_id,
                    kind,
                    surface,
                } => presenters.detach(unit_id, kind, surface),
                GpuCmd::DetachUnit { unit_id } => presenters.detach_unit(unit_id),
                GpuCmd::AttachMonitor {
                    monitor_id,
                    source_id,
                    surface,
                    width,
                    height,
                    prepared,
                    reply,
                } => {
                    let code = match presenters.attach_monitor(
                        &device,
                        monitor_id,
                        source_id,
                        surface,
                        width,
                        height,
                        prepared,
                    ) {
                        Ok(()) => OK,
                        Err(error) => {
                            set_error(&telemetry, error);
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
                GpuCmd::SetMonitorInterval { monitor_id, frames } => {
                    presenters.set_monitor_interval(monitor_id, frames)
                }
                GpuCmd::Shutdown => return,
            }
        }
        presenters.reconfigure_pending(&device);
        let (buffer_frames, use_rebar, direct_sample) = {
            let guard = shared.lock().expect("shared");
            let use_rebar = guard.rebar.available && guard.rebar_optimization;
            let direct_sample = use_rebar && (cfg!(target_os = "macos") || guard.rebar_direct_sample);
            (
                guard.frame_buffer_frames.clamp(1, 8),
                use_rebar,
                direct_sample,
            )
        };
        frame_delay.set_depth(buffer_frames);
        shared
            .lock()
            .expect("shared")
            .audio
            .set_video_delay(buffer_frames, fps_num, fps_den);
        let now = Instant::now();
        if next + frame_dt.saturating_mul(buffer_frames) < now {
            let target =
                (clock_start.elapsed().as_secs_f64() * f64::from(AUDIO_RATE)).floor() as u64;
            let skip = target.saturating_sub(audio_produced) as usize;
            if skip > 0 {
                let audio = shared.lock().expect("shared").audio.clone();
                uploads.lock().expect("uploads").skip_audio_frames(skip);
                audio.skip_bus_frames(skip);
            }
            audio_produced = target;
            next = now;
        }
        if next > Instant::now() {
            thread::sleep(next.saturating_duration_since(Instant::now()));
        }
        let overdue = Instant::now().saturating_duration_since(next);
        let skip_compose = overdue > frame_dt.saturating_mul(buffer_frames.saturating_sub(1));
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
            let outputs_snap: Vec<OutputSnap> = guard
                .outputs
                .iter()
                .map(|(id, output)| OutputSnap {
                    output_id: *id,
                    source_kind: output.source_kind,
                    source_id: output.source_id,
                    unit_id: output.unit_id,
                    fps_n: guard
                        .units
                        .get(&output.unit_id)
                        .map(|u| u.fps_num)
                        .unwrap_or(fps_num),
                    fps_d: guard
                        .units
                        .get(&output.unit_id)
                        .map(|u| u.fps_den)
                        .unwrap_or(fps_den),
                    video_sub: Arc::clone(&output.video_sub),
                    use_gpu: output.use_gpu,
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
            frame_i = frame_i.wrapping_add(1);
            let monitor_sources = presenters.attached_monitor_sources();
            let (used_scenes, used_uploads) =
                collect_live_ids(&scene_specs, &snapshot, &monitor_sources, &outputs_snap);
            let output_refs: Vec<(u32, u64)> = outputs_snap
                .iter()
                .map(|item| (item.source_kind, item.source_id))
                .collect();
            let roles =
                collect_source_roles(&scene_specs, &snapshot, &monitor_sources, &output_refs);
            {
                let guard = shared.lock().expect("shared");
                for (id, receiver) in &guard.receivers {
                    let save = guard.live_save.get(id).copied().unwrap_or_default();
                    let role = roles.get(id).copied().unwrap_or_default();
                    receiver.apply_save(want_full(save, role), role.on_program, role.on_preview);
                }
            }
            let mut upload_guard = uploads.lock().expect("uploads");
            let frame_begin = Instant::now();
            let pts = (clock_start.elapsed().as_nanos() / 100) as i64;
            let secs = frame_i as f64 * f64::from(fps_den) / f64::from(fps_num.max(1));
            let phase = ((secs * 0.12) % 1.0) as f32;
            let phase_y = ((secs * 0.07) % 1.0) as f32;
            composer.begin_frame();
            composer.ensure_builtins(&device);
            composer.sync_generators(&generators, phase, phase_y);
            if !skip_compose {
                upload_guard.advance_playout(&used_uploads);
                composer.upload_sources(&device, &upload_guard, &used_uploads, use_rebar, direct_sample);
            }
            drop(upload_guard);
            let need_prv = outputs_snap
                .iter()
                .any(|item| item.source_kind == SRC_KIND_MU_PREVIEW && item.cpu_video());
            let need_mv = snapshot
                .iter()
                .any(|(unit_id, ..)| presenters.has_kind(*unit_id, OUTPUT_MULTIVIEW));
            let present_epoch = composer.gpu_epoch() ^ frame_delay.epoch().rotate_left(8);
            if let Err(error) =
                presenters.present_unit_buses(&device, present_epoch, |unit_id, kind| {
                    frame_delay
                        .view(unit_id, kind)
                        .or_else(|| composer.unit_view(unit_id, kind))
                })
            {
                set_error(&telemetry, error);
            }
            {
                let mut encoder =
                    device
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("eiviz delay out"),
                        });
                let mut packed_copies: Vec<(u64, u32, u32)> = Vec::new();
                let mut gpu_copies: Vec<(u64, wgpu::Texture, u32, u32, Arc<AtomicBool>, u32, u32)> =
                    Vec::new();
                for (unit_id, ..) in &snapshot {
                    let pack_pgm = outputs_snap.iter().any(|item| {
                        item.unit_id == *unit_id
                            && item.source_kind == SRC_KIND_MU_PROGRAM
                            && item.cpu_video()
                    });
                    if pack_pgm {
                        if let Some(packed) = frame_delay.packed(*unit_id, OUTPUT_PROGRAM) {
                            let width = packed.size().width.saturating_mul(2).max(2);
                            let height = packed.size().height.max(1);
                            let rb = readbacks.ensure(&device, *unit_id, width, height);
                            rb.copy_from(&mut encoder, packed);
                            packed_copies.push((*unit_id, width, height));
                        }
                    }
                }
                for output in &outputs_snap {
                    if output.source_kind == SRC_KIND_MU_PROGRAM || !output.cpu_video() {
                        continue;
                    }
                    let packed = match output.source_kind {
                        SRC_KIND_MU_PREVIEW => frame_delay.packed(output.unit_id, OUTPUT_PREVIEW),
                        SRC_KIND_MU_MULTIVIEW => {
                            frame_delay.packed(output.unit_id, OUTPUT_MULTIVIEW)
                        }
                        _ => None,
                    };
                    if let Some(texture) = packed {
                        let size = texture.size();
                        let w = size.width.saturating_mul(2).max(2);
                        let h = size.height.max(1);
                        let key = 0x0100_0000_0000_0000 | output.output_id;
                        let rb = readbacks.ensure(&device, key, w, h);
                        rb.copy_from(&mut encoder, texture);
                        packed_copies.push((key, w, h));
                    }
                }
                for output in &outputs_snap {
                    if !output.gpu_video() {
                        continue;
                    }
                    let rgba = match output.source_kind {
                        SRC_KIND_MU_PROGRAM => frame_delay.rgba(output.unit_id, OUTPUT_PROGRAM),
                        SRC_KIND_MU_PREVIEW => frame_delay.rgba(output.unit_id, OUTPUT_PREVIEW),
                        SRC_KIND_MU_MULTIVIEW => frame_delay.rgba(output.unit_id, OUTPUT_MULTIVIEW),
                        _ => None,
                    };
                    if let Some(src) = rgba
                        && let Some((texture, w, h, busy)) =
                            gpu_sends.copy(&device, &mut encoder, output.output_id, src)
                    {
                        gpu_copies.push((
                            output.output_id,
                            texture,
                            w,
                            h,
                            busy,
                            output.fps_n,
                            output.fps_d,
                        ));
                    }
                }
                device.submit(Some(encoder.finish()));
                emit_packed(
                    &mut readbacks,
                    &device,
                    &packed_copies,
                    &outputs_snap,
                    &send_tx,
                    pts,
                );
                emit_gpu(&gpu_copies, &send_tx, pts);
            }
            frame_delay.consume_display(skip_compose);
            if !skip_compose {
                composer.sync_scenes(&device, &scene_specs);
                let mut encoder =
                    device
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("eiviz compose"),
                        });
                if let Err(error) =
                    composer.render_scenes(&device, &used_scenes, &mut encoder, &tallies)
                {
                    set_error(&telemetry, error);
                }
                let mut packed_copies: Vec<(u64, u32, u32)> = Vec::new();
                let mut gpu_copies: Vec<(u64, wgpu::Texture, u32, u32, Arc<AtomicBool>, u32, u32)> =
                    Vec::new();
                for (unit_id, width, height, _, _, state) in &snapshot {
                    composer.ensure_unit(&device, *unit_id, *width, *height);
                    let pack_pgm = outputs_snap.iter().any(|item| {
                        item.unit_id == *unit_id
                            && item.source_kind == SRC_KIND_MU_PROGRAM
                            && item.cpu_video()
                    });
                    if let Err(error) = composer.render_unit(
                        &device,
                        *unit_id,
                        state,
                        &mut encoder,
                        need_mv,
                        pack_pgm,
                    ) {
                        set_error(&telemetry, error);
                    }
                    composer.pack_aux(&device, &mut encoder, *unit_id, need_prv, false);
                }
                for output in &outputs_snap {
                    if output.source_kind == SRC_KIND_MU_PROGRAM
                        || output.source_kind == SRC_KIND_MU_PREVIEW
                        || output.source_kind == SRC_KIND_MU_MULTIVIEW
                        || !output.cpu_video()
                    {
                        continue;
                    }
                    let packed = match output.source_kind {
                        SRC_KIND_INPUT => {
                            let (w, h) = snapshot
                                .iter()
                                .find(|(id, ..)| *id == output.unit_id)
                                .map(|(_, w, h, ..)| (*w, *h))
                                .unwrap_or((1920, 1080));
                            composer.pack_source(&device, &mut encoder, output.source_id, w, h)
                        }
                        SRC_KIND_SCENE => {
                            composer.pack_scene(&device, &mut encoder, output.source_id)
                        }
                        _ => None,
                    };
                    if let Some(texture) = packed {
                        let size = texture.size();
                        let w = size.width.saturating_mul(2).max(2);
                        let h = size.height.max(1);
                        let key = 0x0100_0000_0000_0000 | output.output_id;
                        let rb = readbacks.ensure(&device, key, w, h);
                        rb.copy_from(&mut encoder, texture);
                        packed_copies.push((key, w, h));
                    }
                }
                for output in &outputs_snap {
                    if output.source_kind == SRC_KIND_MU_PROGRAM
                        || output.source_kind == SRC_KIND_MU_PREVIEW
                        || output.source_kind == SRC_KIND_MU_MULTIVIEW
                        || !output.gpu_video()
                    {
                        continue;
                    }
                    let src = match output.source_kind {
                        SRC_KIND_INPUT => composer.source_texture(output.source_id),
                        SRC_KIND_SCENE => composer.scene_texture(output.source_id),
                        _ => None,
                    };
                    if let Some(src) = src
                        && let Some((texture, w, h, busy)) =
                            gpu_sends.copy(&device, &mut encoder, output.output_id, src)
                    {
                        gpu_copies.push((
                            output.output_id,
                            texture,
                            w,
                            h,
                            busy,
                            output.fps_n,
                            output.fps_d,
                        ));
                    }
                }
                frame_delay.capture(
                    &device,
                    &mut encoder,
                    &composer,
                    snapshot.iter().map(|(id, ..)| *id),
                );
                device.submit(Some(encoder.finish()));
                emit_packed(
                    &mut readbacks,
                    &device,
                    &packed_copies,
                    &outputs_snap,
                    &send_tx,
                    pts,
                );
                emit_gpu(&gpu_copies, &send_tx, pts);
            }
            if presenters.any_monitor_due(frame_i) {
                if let Err(error) =
                    presenters.present_monitors(&device, present_epoch, frame_i, |source_id| {
                        frame_delay
                            .view_for_source(source_id)
                            .or_else(|| composer.view_for_source(source_id))
                            .map(|view| (view, composer.source_is_packed(source_id)))
                    })
                {
                    set_error(&telemetry, error);
                }
            }
            audio_carry += AUDIO_RATE as u64 * u64::from(fps_den);
            let audio_frames = (audio_carry / u64::from(fps_num.max(1))) as usize;
            audio_carry %= u64::from(fps_num.max(1));
            audio_produced = audio_produced.saturating_add(audio_frames as u64);
            let audio = {
                let guard = shared.lock().expect("shared");
                guard.audio.clone()
            };
            let mut upload_guard = uploads.lock().expect("uploads");
            if audio_frames > 0 {
                let mut guard = shared.lock().expect("shared");
                for (id, spec) in &generators {
                    if spec.tone_hz <= 0.0 {
                        continue;
                    }
                    let phase = guard.tone_phase.entry(*id).or_insert(0.0);
                    let packet = generator_audio::sine_packet(
                        phase,
                        spec.tone_hz,
                        spec.tone_level_dbfs,
                        audio_frames,
                        pts,
                    );
                    upload_guard.ingest_audio(*id, packet);
                }
            }
            let mixed = audio.mix(
                &mut upload_guard,
                &snapshot,
                &scene_specs,
                audio_frames,
                !skip_compose,
            );
            drop(upload_guard);
            {
                let mut guard = telemetry.lock().expect("telemetry");
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
                for output in &outputs_snap {
                    let _ = send_tx.send(SendCmd::Audio {
                        output_id: output.output_id,
                        packet: packet.clone(),
                    });
                }
            }
        }
        next += frame_dt;
        if skip_compose && next < Instant::now() {
            next = Instant::now();
        }
    }
}

fn emit_packed(
    readbacks: &mut ReadbackStore,
    device: &GpuDevice,
    packed_copies: &[(u64, u32, u32)],
    outputs_snap: &[OutputSnap],
    send_tx: &mpsc::Sender<SendCmd>,
    pts: i64,
) {
    for (key, width, height) in packed_copies {
        if let Some(rb) = readbacks.get_mut(*key) {
            rb.advance(device);
            if let Some(packed) = rb.latest() {
                let data: Arc<[u8]> = packed.to_vec().into();
                last_frames().lock().expect("frames").insert(
                    *key,
                    Acquired {
                        data: Arc::clone(&data),
                        stride: width * 2,
                        pts,
                    },
                );
                for output in outputs_snap {
                    if !output.cpu_video() {
                        continue;
                    }
                    let out_key = if output.source_kind == SRC_KIND_MU_PROGRAM {
                        output.unit_id
                    } else {
                        0x0100_0000_0000_0000 | output.output_id
                    };
                    if out_key != *key {
                        continue;
                    }
                    let _ = send_tx.send(SendCmd::Video {
                        output_id: output.output_id,
                        width: *width,
                        height: *height,
                        stride: width * 2,
                        pts,
                        data: Arc::clone(&data),
                        fps_n: output.fps_n,
                        fps_d: output.fps_d,
                    });
                }
            }
        }
    }
}

fn emit_gpu(
    copies: &[(u64, wgpu::Texture, u32, u32, Arc<AtomicBool>, u32, u32)],
    send_tx: &mpsc::Sender<SendCmd>,
    pts: i64,
) {
    for (output_id, texture, width, height, busy, fps_n, fps_d) in copies {
        let _ = send_tx.send(SendCmd::GpuVideo {
            output_id: *output_id,
            texture: texture.clone(),
            width: *width,
            height: *height,
            pts,
            fps_n: *fps_n,
            fps_d: *fps_d,
            busy: Arc::clone(busy),
        });
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

#[allow(dead_code)]
fn follow_gains(
    snapshot: &[(u64, u32, u32, u32, u32, UnitState)],
    scenes: &[(u64, u32, u32, Arc<[OverlayDesc]>)],
    _uploads: &UploadStore,
) -> Vec<(u64, f32)> {
    let spec_map: HashMap<u64, &[OverlayDesc]> = scenes
        .iter()
        .map(|spec| (spec.0, spec.3.as_ref()))
        .collect();
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
                    add(
                        layer.source_id,
                        gain * layer.opacity.max(0.0),
                        spec_map,
                        gains,
                    );
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
            add(
                overlay.source_id,
                overlay.opacity.max(0.0),
                &spec_map,
                &mut gains,
            );
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
    monitor_sources: &[u64],
    outputs: &[OutputSnap],
) -> (HashSet<u64>, HashSet<u64>) {
    let spec_map: HashMap<u64, &[OverlayDesc]> = scene_specs
        .iter()
        .map(|spec| (spec.0, spec.3.as_ref()))
        .collect();
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
    for &id in monitor_sources {
        add(id, &spec_map, &mut scenes, &mut uploads);
    }
    for output in outputs {
        match output.source_kind {
            SRC_KIND_SCENE | SRC_KIND_MU_MULTIVIEW => {
                add(output.source_id, &spec_map, &mut scenes, &mut uploads)
            }
            SRC_KIND_INPUT => {
                uploads.insert(output.source_id);
            }
            _ => {}
        }
    }
    (scenes, uploads)
}

fn send_loop(rx: mpsc::Receiver<SendCmd>, stop: Arc<AtomicBool>, omt_gpu: OmtGpu) {
    let mut senders: HashMap<u64, (OutputHandle, Arc<AtomicBool>)> = HashMap::new();
    while !stop.load(Ordering::Relaxed) {
        loop {
            match rx.try_recv() {
                Ok(SendCmd::Shutdown) => return,
                Ok(cmd) => apply_send_cmd(&mut senders, cmd, &omt_gpu),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }
        let mut dead = Vec::new();
        for (&id, (sender, video_sub)) in senders.iter_mut() {
            let pumped = panic::catch_unwind(AssertUnwindSafe(|| sender.pump()));
            match pumped {
                Ok(Ok(subscribed)) => video_sub.store(subscribed, Ordering::Relaxed),
                Ok(Err(_)) | Err(_) => dead.push(id),
            }
        }
        for id in dead {
            senders.remove(&id);
        }
        match rx.recv_timeout(Duration::from_millis(2)) {
            Ok(SendCmd::Shutdown) => return,
            Ok(cmd) => apply_send_cmd(&mut senders, cmd, &omt_gpu),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn apply_send_cmd(
    senders: &mut HashMap<u64, (OutputHandle, Arc<AtomicBool>)>,
    cmd: SendCmd,
    omt_gpu: &OmtGpu,
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
        SendCmd::GpuVideo {
            output_id,
            texture,
            width,
            height,
            pts,
            fps_n,
            fps_d,
            busy,
        } => {
            if let Some((sender, _)) = senders.get_mut(&output_id) {
                let _ = panic::catch_unwind(AssertUnwindSafe(|| {
                    sender.send_video_texture(omt_gpu, &texture, width, height, pts, fps_n, fps_d)
                }));
            }
            busy.store(false, Ordering::Release);
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

    #[test]
    fn attach_rejects_null_hwnd_and_unknown_native_kind() {
        assert_eq!(
            mixer_unit_attach_output(1, 0, 1920, 1080, OUTPUT_PROGRAM),
            ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            mixer_unit_attach_native(1, OUTPUT_PROGRAM, 0, 1, 1920, 1080),
            ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            mixer_unit_attach_native(1, OUTPUT_PROGRAM, 99, 1, 1920, 1080),
            ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            mixer_attach_monitor_native(1, 1, 99, 1, 1920, 1080),
            ERR_INVALID_ARGUMENT
        );
        #[cfg(windows)]
        assert_eq!(
            mixer_unit_attach_native(1, OUTPUT_PROGRAM, NATIVE_APPKIT_NSVIEW, 1, 1920, 1080),
            ERR_INVALID_ARGUMENT
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            mixer_unit_attach_native(1, OUTPUT_PROGRAM, NATIVE_WIN32_HWND, 1, 1920, 1080),
            ERR_INVALID_ARGUMENT
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            mixer_unit_attach_native(1, OUTPUT_PROGRAM, NATIVE_APPKIT_NSVIEW, 1, 1920, 1080),
            ERR_NOT_CREATED
        );
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            assert_eq!(
                mixer_unit_attach_native(1, OUTPUT_PROGRAM, NATIVE_WIN32_HWND, 1, 1920, 1080),
                ERR_INVALID_ARGUMENT
            );
            assert_eq!(
                mixer_unit_attach_native(1, OUTPUT_PROGRAM, NATIVE_APPKIT_NSVIEW, 1, 1920, 1080),
                ERR_INVALID_ARGUMENT
            );
        }
    }
}
