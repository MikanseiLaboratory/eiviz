#![deny(unsafe_op_in_unsafe_fn)]

mod abi;
mod audio;
mod compose;
#[cfg(windows)]
mod convert;
mod delay;
mod device;
mod diag;
#[cfg(windows)]
mod dxgi;
mod generator_audio;
mod labels;
#[cfg(windows)]
mod media;
#[cfg(windows)]
pub use media::enumerate_video_captures;
#[cfg(target_os = "macos")]
mod media_macos;
#[cfg(target_os = "macos")]
pub use media_macos::enumerate_video_captures;
#[cfg(target_os = "macos")]
mod main_thread;
#[cfg(any(windows, target_os = "macos"))]
mod ndi;
mod omt;
mod pool;
mod present;
mod readback;
mod rebar;
mod save;
mod session;
pub mod simd;
mod thumb;
mod upload;

pub use abi::{
    AudioPeak, DURATION_FRAMES, DURATION_MS, EASING_IN, EASING_IN_OUT, EASING_LINEAR, EASING_OUT,
    EASING_SMOOTHSTEP, ERR_ALREADY_CREATED, ERR_DEVICE, ERR_INVALID_ARGUMENT, ERR_IO,
    ERR_NOT_CREATED, GEN_BARS, GEN_SOLID, INCOMING_PREVIEW, INCOMING_PROGRAM, MixerRebarInfo,
    MixerStats, MixerVideoInfo, NATIVE_APPKIT_NSVIEW, NATIVE_WIN32_HWND, OK, OUT_DECKLINK, OUT_NDI,
    OUT_OMT, OUTPUT_PREVIEW, OUTPUT_PROGRAM, OverlayDesc, Rect, SAVE_FLAG_MULTIVIEW,
    SAVE_NOT_ON_PREVIEW_OR_PROGRAM, SCENE_BASE, SRC_BARS, SRC_BLACK, SRC_BLUE, SRC_COLOR,
    SRC_KIND_INPUT, SRC_KIND_MU_MULTIVIEW, SRC_KIND_MU_PREVIEW, SRC_KIND_MU_PROGRAM,
    SRC_KIND_SCENE, SourceUsage, TRANSITION_ADDITIVE, TRANSITION_BARN_DOOR, TRANSITION_BLINDS,
    TRANSITION_BLOOM, TRANSITION_CLOCK, TRANSITION_CROSS_ZOOM, TRANSITION_CUBE,
    TRANSITION_CUBE_ZOOM, TRANSITION_CUSTOM, TRANSITION_CUT, TRANSITION_DATAMOSH,
    TRANSITION_DIAMOND, TRANSITION_DIP, TRANSITION_DIR_DOWN, TRANSITION_DIR_LEFT,
    TRANSITION_DIR_RIGHT, TRANSITION_DIR_UP, TRANSITION_DISPLACE, TRANSITION_FADE,
    TRANSITION_FILM_BURN, TRANSITION_FLIP, TRANSITION_FLY_ROTATE, TRANSITION_GLITCH,
    TRANSITION_GRID_DISSOLVE, TRANSITION_HEART, TRANSITION_IRIS, TRANSITION_KALEIDOSCOPE,
    TRANSITION_LOREZ, TRANSITION_LUMA_MORPH, TRANSITION_METAMIX, TRANSITION_MULTITASK,
    TRANSITION_OPTICAL_FLOW, TRANSITION_PAGE_CURL, TRANSITION_PARTS, TRANSITION_PIXEL_SORT,
    TRANSITION_POLAR, TRANSITION_PUSH, TRANSITION_RIPPLE, TRANSITION_ROLLER_DOOR,
    TRANSITION_SHIFT_RGB, TRANSITION_SLIDE, TRANSITION_STAR, TRANSITION_STATIC, TRANSITION_STINGER,
    TRANSITION_SWIRL, TRANSITION_TILE, TRANSITION_VISUAL_DISSOLVE, TRANSITION_WIPE,
    TRANSITION_ZOOM, TRANSITION_ZOOM_BLUR, UnitSnap, UnitState, VideoCaptureInfo, VideoCaptureMode,
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

use abi::{MixInputSpec, NativeSurface};
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
use upload::{AUDIO_RATE, AudioPacket, CpuFormat, GpuIngest, UploadStore};

struct AutoTransition {
    from: f32,
    to: f32,
    start: Instant,
    duration: Duration,
    swap: bool,
    keep_preview: bool,
    incoming_locked: bool,
    frozen_preview: u64,
    easing: u32,
}

struct OverlayAuto {
    desc: OverlayDesc,
    from: f32,
    to: f32,
    start: Instant,
    duration: Duration,
}

struct LiveUnit {
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    state: UnitState,
    auto: Option<AutoTransition>,
    overlay_autos: Vec<OverlayAuto>,
    frozen_preview: Option<u64>,
    custom_wgsl: Option<String>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BusColors {
    preview: [u8; 3],
    program: [u8; 3],
    inactive: [u8; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MvLabelStyle {
    pub size: f32,
    pub percent: bool,
    pub top: bool,
}

impl Default for MvLabelStyle {
    fn default() -> Self {
        Self {
            size: 18.0,
            percent: false,
            top: false,
        }
    }
}

impl Default for BusColors {
    fn default() -> Self {
        Self {
            preview: [0, 255, 0],
            program: [255, 0, 0],
            inactive: [64, 64, 64],
        }
    }
}

struct SceneSpec {
    width: u32,
    height: u32,
    layers: Arc<[crate::abi::OverlayDesc]>,
    labels: Arc<[String]>,
    mv_label: MvLabelStyle,
}

struct Shared {
    master_fps_num: u32,
    master_fps_den: u32,
    units: HashMap<u64, LiveUnit>,
    scenes: HashMap<u64, SceneSpec>,
    bus_colors: BusColors,
    mv_label: MvLabelStyle,
    uploads: Arc<Mutex<UploadStore>>,
    gpu_ingest: GpuIngest,
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
    compose_dirty: bool,
    thumbs: HashMap<u64, crate::thumb::ThumbSub>,
    mix_inputs: HashMap<u64, MixInputSpec>,
    frame_buffer_frames: u32,
    rebar: crate::rebar::RebarSnapshot,
    rebar_optimization: bool,
    ndi_gpu_upload: bool,
    audio: audio::AudioEngine,
}

/// Host-visible status that must not share the control lock with ingest or render.
struct Telemetry {
    last_error: String,
    last_render_ms: f32,
    follow_primed: bool,
    monitor_pcm: VecDeque<f32>,
    last_ram_bytes: u64,
    last_vram_bytes: u64,
    last_compose_vram: u64,
    last_delay_vram: u64,
    scene_usage: Vec<SourceUsage>,
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
        reply: mpsc::Sender<i32>,
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
    thumb_pixels: Arc<Mutex<HashMap<u64, crate::thumb::ThumbPixels>>>,
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
    let start = Instant::now();
    let slot = mixer_slot().lock().expect("mixer mutex poisoned");
    let result = match slot.as_ref() {
        Some(mixer) => Ok(f(mixer)),
        None => Err(ERR_NOT_CREATED),
    };
    crate::diag::lock_held("mixer_slot", start, result)
}

fn report_io(error: impl Into<String>) -> i32 {
    let error = error.into();
    crate::diag::error(&error);
    let _ = with_mixer(|mixer| set_error(&mixer.telemetry, error));
    ERR_IO
}

fn insert_receiver(id: u64, receiver: LiveReceiver) -> i32 {
    with_mixer(|mixer| {
        mixer
            .shared
            .lock()
            .expect("shared")
            .receivers
            .insert(id, receiver);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[cfg(any(windows, target_os = "macos"))]
fn insert_video(id: u64, pump: VideoPump) -> i32 {
    with_mixer(|mixer| {
        mixer.shared.lock().expect("shared").videos.insert(id, pump);
        OK
    })
    .unwrap_or_else(|code| code)
}

struct DetachedSource {
    receiver: Option<LiveReceiver>,
    #[cfg(any(windows, target_os = "macos"))]
    video: Option<VideoPump>,
    uploads: Arc<Mutex<UploadStore>>,
}

fn detach_source(id: u64) -> Result<DetachedSource, i32> {
    with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        #[cfg(any(windows, target_os = "macos"))]
        let video = shared.videos.remove(&id);
        let receiver = shared.receivers.remove(&id);
        shared.generators.remove(&id);
        shared.tone_phase.remove(&id);
        shared.live_save.remove(&id);
        shared.mix_inputs.remove(&id);
        let uploads = shared.uploads.clone();
        DetachedSource {
            receiver,
            #[cfg(any(windows, target_os = "macos"))]
            video,
            uploads,
        }
    })
}

#[cfg(target_os = "macos")]
fn take_source_uploads(id: u64) -> Result<Arc<Mutex<UploadStore>>, i32> {
    let (video, receiver, uploads) = with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        let video = shared.videos.remove(&id);
        let receiver = shared.receivers.remove(&id);
        let uploads = shared.uploads.clone();
        (video, receiver, uploads)
    })?;
    drop(receiver);
    drop(video);
    Ok(uploads)
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
        report_session_error(format!(
            "session buffer too small (need {} bytes)",
            src.len()
        ));
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
    crate::diag::init();
    crate::diag::info("mixer_create");
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
    let gpu_ingest = GpuIngest {
        device: device.device.clone(),
        queue: device.queue.clone(),
        ndi_gpu: Arc::new(AtomicBool::new(true)),
        use_rebar: Arc::new(AtomicBool::new(rebar.available)),
        rebar_available: rebar.available,
    };
    let uploads = Arc::new(Mutex::new(UploadStore::default()));
    let telemetry = Arc::new(Mutex::new(Telemetry {
        last_error: String::new(),
        last_render_ms: 0.0,
        follow_primed: false,
        monitor_pcm: VecDeque::new(),
        last_ram_bytes: 0,
        last_vram_bytes: 0,
        last_compose_vram: 0,
        last_delay_vram: 0,
        scene_usage: Vec::new(),
    }));
    let audio = audio::AudioEngine::new();
    let shared = Arc::new(Mutex::new(Shared {
        master_fps_num: fps_num,
        master_fps_den: fps_den,
        units: HashMap::new(),
        scenes: HashMap::new(),
        bus_colors: BusColors::default(),
        mv_label: MvLabelStyle::default(),
        uploads: Arc::clone(&uploads),
        gpu_ingest,
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
        ndi_gpu_upload: true,
        audio: audio.clone(),
        multiview_binds: HashMap::new(),
        compose_dirty: false,
        thumbs: HashMap::new(),
        mix_inputs: HashMap::new(),
    }));
    let thumb_pixels = Arc::new(Mutex::new(HashMap::new()));
    let (tx, rx) = mpsc::channel();
    let (send_tx, send_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let render_shared = Arc::clone(&shared);
    let render_uploads = Arc::clone(&uploads);
    let render_telemetry = Arc::clone(&telemetry);
    let render_stop = Arc::clone(&stop);
    let render_send = send_tx.clone();
    let render_thumbs = Arc::clone(&thumb_pixels);
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
                render_thumbs,
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
        thumb_pixels,
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
    crate::diag::info("mixer_destroy begin");
    let Some(mut mixer) = mixer_slot().lock().expect("mixer mutex poisoned").take() else {
        return;
    };
    mixer.stop.store(true, Ordering::Relaxed);
    let (audio, receivers, videos) = {
        let mut shared = mixer.shared.lock().expect("shared");
        let audio = shared.audio.clone();
        let receivers = std::mem::take(&mut shared.receivers);
        #[cfg(any(windows, target_os = "macos"))]
        let videos = std::mem::take(&mut shared.videos);
        #[cfg(not(any(windows, target_os = "macos")))]
        let videos = ();
        (audio, receivers, videos)
    };
    audio.shutdown();
    drop(receivers);
    drop(videos);
    let _ = mixer.cmds.send(GpuCmd::Shutdown);
    if let Some(join) = mixer.render.take() {
        crate::diag::join_timeout(join, Duration::from_secs(2), "render");
    }
    let _ = mixer.send_tx.send(SendCmd::Shutdown);
    if let Some(join) = mixer.send.take() {
        crate::diag::join_timeout(join, Duration::from_secs(2), "send");
    }
    crate::diag::info("mixer_destroy end");
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_ping() -> u32 {
    crate::diag::init();
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
                overlay_autos: Vec::new(),
                frozen_preview: None,
                custom_wgsl: None,
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
    let (copied, labels) = if count == 0 {
        (Arc::from([]), Arc::from([]))
    } else {
        // SAFETY: caller keeps count OverlayDesc values readable for this call.
        let slice = unsafe { std::slice::from_raw_parts(layers, count as usize) };
        let mut descs = slice.to_vec();
        let mut texts = Vec::with_capacity(descs.len());
        for desc in &mut descs {
            texts.push(copy_c_label(desc.label));
            desc.label = std::ptr::null();
        }
        (Arc::from(descs), Arc::from(texts))
    };
    with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        let mv_label = shared
            .scenes
            .get(&scene_id)
            .map(|spec| spec.mv_label)
            .unwrap_or(shared.mv_label);
        shared.scenes.insert(
            scene_id,
            SceneSpec {
                width,
                height,
                layers: copied,
                labels,
                mv_label,
            },
        );
        shared.compose_dirty = true;
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_destroy_scene(scene_id: u64) -> i32 {
    with_mixer(|mixer| {
        {
            let mut shared = mixer.shared.lock().expect("shared");
            shared.scenes.remove(&scene_id);
            shared.multiview_binds.remove(&scene_id);
            shared.compose_dirty = true;
        }
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
        shared.compose_dirty = true;
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_define_mix_input(
    id: u64,
    target_id: u64,
    source_kind: u32,
    delay: u32,
    audio_bus_id: u64,
) -> i32 {
    let Some(spec) = MixInputSpec::new(target_id, source_kind, delay, audio_bus_id) else {
        return ERR_INVALID_ARGUMENT;
    };
    if id == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        let mut pending = shared.mix_inputs.clone();
        pending.insert(id, spec);
        if !spec.is_session_multiview() {
            for (unit_id, unit) in &shared.units {
                if unit_uses_mix_cycle(*unit_id, &unit.state, &pending, &shared.scenes) {
                    return ERR_INVALID_ARGUMENT;
                }
            }
        }
        shared.mix_inputs.insert(id, spec);
        shared.compose_dirty = true;
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
        if unit_uses_mix_cycle(unit_id, &state, &shared.mix_inputs, &shared.scenes) {
            return ERR_INVALID_ARGUMENT;
        }
        {
            let Some(unit) = shared.units.get_mut(&unit_id) else {
                return ERR_INVALID_ARGUMENT;
            };
            let keep = state.keep_preview != 0
                || unit
                    .auto
                    .as_ref()
                    .is_some_and(|auto| auto.keep_preview || auto.incoming_locked);
            if unit.auto.is_some() && keep {
                let mix = unit.state.mix;
                let program = unit.state.program_source;
                let keep_preview = unit.state.keep_preview;
                let dip = (
                    unit.state.dip_r,
                    unit.state.dip_g,
                    unit.state.dip_b,
                    unit.state.dip_a,
                );
                let look = (unit.state.softness, unit.state.param);
                let frozen = unit.frozen_preview;
                unit.state = state;
                unit.state.mix = mix;
                unit.state.program_source = program;
                unit.state.keep_preview = keep_preview;
                unit.state.dip_r = dip.0;
                unit.state.dip_g = dip.1;
                unit.state.dip_b = dip.2;
                unit.state.dip_a = dip.3;
                unit.state.softness = look.0;
                unit.state.param = look.1;
                unit.frozen_preview = frozen;
            } else {
                let mix_changed = (unit.state.mix - state.mix).abs() > 0.0001;
                unit.state = state;
                if mix_changed {
                    unit.auto = None;
                }
            }
            unit.state.incoming_source = 0;
            if unit
                .auto
                .as_ref()
                .is_some_and(|auto| auto.keep_preview || auto.incoming_locked)
            {
                unit.frozen_preview.get_or_insert(unit.state.preview_source);
            } else if unit.state.mix > 0.001 {
                if unit.state.keep_preview != 0 {
                    unit.frozen_preview.get_or_insert(unit.state.preview_source);
                } else {
                    unit.frozen_preview = None;
                }
            } else {
                unit.frozen_preview = None;
            }
        }
        shared.compose_dirty = true;
        OK
    })
    .unwrap_or_else(|code| code)
}

fn ease_mix(t: f32, kind: u32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match kind {
        1 => t * t * t,
        2 => 1.0 - (1.0 - t).powi(3),
        3 => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            }
        }
        4 => t * t * (3.0 - 2.0 * t),
        _ => t,
    }
}

fn merge_overlay(state: &mut UnitState, desc: OverlayDesc) {
    if let Some(existing) = state
        .overlays
        .iter_mut()
        .take(state.overlay_count as usize)
        .find(|item| item.source_id == desc.source_id)
    {
        *existing = desc;
        return;
    }
    if (state.overlay_count as usize) < state.overlays.len() {
        state.overlays[state.overlay_count as usize] = desc;
        state.overlay_count += 1;
    }
}

fn tick_unit_transitions(unit: &mut LiveUnit) {
    if let Some(auto) = unit.auto.take() {
        let t = auto.start.elapsed().as_secs_f32() / auto.duration.as_secs_f32();
        if t >= 1.0 {
            unit.state.mix = auto.to;
            if auto.to >= 1.0 {
                take_cut(unit, auto.swap);
            } else {
                unit.frozen_preview = None;
            }
        } else {
            let eased = ease_mix(t, auto.easing);
            unit.state.mix = auto.from + (auto.to - auto.from) * eased;
            unit.auto = Some(auto);
        }
    }
    let mut still = Vec::new();
    for mut item in unit.overlay_autos.drain(..) {
        let t = item.start.elapsed().as_secs_f32() / item.duration.as_secs_f32();
        if t >= 1.0 {
            item.desc.opacity = item.to;
            if item.to > 0.001 {
                merge_overlay(&mut unit.state, item.desc);
            }
        } else {
            item.desc.opacity = item.from + (item.to - item.from) * t;
            merge_overlay(&mut unit.state, item.desc);
            still.push(item);
        }
    }
    unit.overlay_autos = still;
}

fn live_incoming(unit: &LiveUnit) -> u64 {
    if let Some(auto) = &unit.auto
        && (auto.keep_preview || auto.incoming_locked)
    {
        return auto.frozen_preview;
    }
    unit.frozen_preview.unwrap_or(unit.state.preview_source)
}

fn resolve_incoming(requested: u64, preview: u64, program: u64) -> u64 {
    if requested == INCOMING_PREVIEW {
        preview
    } else if requested == INCOMING_PROGRAM {
        program
    } else {
        requested
    }
}

fn snapshot_mix_preview(unit: &LiveUnit) -> u64 {
    let incoming = live_incoming(unit);
    if incoming == unit.state.preview_source {
        0
    } else {
        incoming
    }
}

fn take_cut(unit: &mut LiveUnit, swap: bool) {
    take_cut_to(unit, swap, live_incoming(unit));
}

fn take_cut_to(unit: &mut LiveUnit, swap: bool, incoming: u64) {
    let preview = unit.state.preview_source;
    if swap && incoming == preview {
        unit.state.preview_source = unit.state.program_source;
        unit.state.program_source = incoming;
    } else {
        unit.state.program_source = incoming;
    }
    unit.state.mix = 0.0;
    unit.state.incoming_source = 0;
    unit.auto = None;
    unit.frozen_preview = None;
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_unit_cut(unit_id: u64, swap: u32, incoming_source: u64) -> i32 {
    with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        let Some(unit) = shared.units.get_mut(&unit_id) else {
            return ERR_INVALID_ARGUMENT;
        };
        let incoming = if incoming_source == INCOMING_PREVIEW {
            live_incoming(unit)
        } else {
            resolve_incoming(
                incoming_source,
                unit.state.preview_source,
                unit.state.program_source,
            )
        };
        take_cut_to(unit, swap != 0, incoming);
        shared.compose_dirty = true;
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_unit_auto(
    unit_id: u64,
    kind: u32,
    duration_ms: u32,
    swap: u32,
    keep_preview: u32,
    easing: u32,
    direction: u32,
    dip_r: f32,
    dip_g: f32,
    dip_b: f32,
    dip_a: f32,
    incoming_source: u64,
    softness: f32,
    param: f32,
) -> i32 {
    with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        let Some(unit) = shared.units.get_mut(&unit_id) else {
            return ERR_INVALID_ARGUMENT;
        };
        let kind = if kind == crate::abi::TRANSITION_STINGER {
            crate::abi::TRANSITION_FADE
        } else {
            kind
        };
        unit.state.transition_kind = kind;
        unit.state.transition_easing = easing;
        unit.state.transition_direction = direction;
        unit.state.keep_preview = keep_preview;
        unit.state.dip_r = dip_r;
        unit.state.dip_g = dip_g;
        unit.state.dip_b = dip_b;
        unit.state.dip_a = if dip_a <= 0.0 { 1.0 } else { dip_a };
        unit.state.softness = softness;
        unit.state.param = param;
        let keep = keep_preview != 0;
        let incoming = resolve_incoming(
            incoming_source,
            unit.state.preview_source,
            unit.state.program_source,
        );
        let incoming_locked = keep || incoming_source != INCOMING_PREVIEW;
        if incoming_locked {
            unit.frozen_preview = Some(incoming);
        } else {
            unit.frozen_preview = None;
        }
        unit.auto = Some(AutoTransition {
            from: unit.state.mix,
            to: if unit.state.mix < 0.5 { 1.0 } else { 0.0 },
            start: Instant::now(),
            duration: Duration::from_millis(u64::from(duration_ms.max(1))),
            swap: swap != 0,
            keep_preview: keep,
            incoming_locked,
            frozen_preview: incoming,
            easing,
        });
        shared.compose_dirty = true;
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_unit_overlay_auto(
    unit_id: u64,
    target_enabled: u32,
    duration_ms: u32,
    desc: *const OverlayDesc,
) -> i32 {
    if desc.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let desc = unsafe { *desc };
    with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        let Some(unit) = shared.units.get_mut(&unit_id) else {
            return ERR_INVALID_ARGUMENT;
        };
        let from = if target_enabled != 0 {
            0.0
        } else {
            desc.opacity.max(0.001)
        };
        let to = if target_enabled != 0 {
            desc.opacity.max(0.001)
        } else {
            0.0
        };
        unit.overlay_autos
            .retain(|item| item.desc.source_id != desc.source_id);
        unit.overlay_autos.push(OverlayAuto {
            desc,
            from,
            to,
            start: Instant::now(),
            duration: Duration::from_millis(u64::from(duration_ms.max(1))),
        });
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_validate_custom_wgsl(wgsl: *const c_char) -> i32 {
    let text = if wgsl.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(wgsl) }
            .to_str()
            .unwrap_or_default()
            .to_string()
    };
    match crate::compose::Composer::validate_custom_wgsl(&text) {
        Ok(()) => OK,
        Err(error) => {
            report_session_error(error);
            ERR_INVALID_ARGUMENT
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_unit_set_custom_wgsl(unit_id: u64, wgsl: *const c_char) -> i32 {
    let text = if wgsl.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(wgsl) }
            .to_str()
            .unwrap_or_default()
            .to_string()
    };
    with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        let Some(unit) = shared.units.get_mut(&unit_id) else {
            return ERR_INVALID_ARGUMENT;
        };
        unit.custom_wgsl = if text.trim().is_empty() {
            None
        } else {
            Some(text)
        };
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
        let mut shared = mixer.shared.lock().expect("shared");
        let Some(unit) = shared.units.get_mut(&unit_id) else {
            return ERR_INVALID_ARGUMENT;
        };
        tick_unit_transitions(unit);
        let mut state = unit.state;
        state.incoming_source = snapshot_mix_preview(unit);
        unsafe { *out = state };
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
    send_gpu_and_wait(|mixer, reply| {
        if mixer
            .cmds
            .send(GpuCmd::DetachMonitor { monitor_id, reply })
            .is_err()
        {
            return ERR_DEVICE;
        }
        OK
    })
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
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    frame_buffer_frames: u32,
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
        let _ = (
            id,
            path,
            capture,
            format,
            width,
            height,
            fps_num,
            fps_den,
            frame_buffer_frames,
        );
        return with_mixer(|mixer| {
            set_error(&mixer.telemetry, "Video ingest is not available");
            ERR_IO
        })
        .unwrap_or_else(|code| code);
    }
    crate::diag::info(&format!("video_start id={id}"));
    #[cfg(target_os = "macos")]
    {
        let _ = format;
        let depth = match with_mixer(|mixer| {
            let session = mixer
                .shared
                .lock()
                .expect("shared")
                .frame_buffer_frames
                .clamp(1, 8);
            if frame_buffer_frames == 0 {
                session
            } else {
                frame_buffer_frames.clamp(1, 8)
            }
        }) {
            Ok(depth) => depth,
            Err(code) => return code,
        };
        let uploads = match take_source_uploads(id) {
            Ok(uploads) => uploads,
            Err(code) => return code,
        };
        return match VideoPump::start(
            id,
            path,
            capture != 0,
            width,
            height,
            fps_num,
            fps_den,
            uploads,
            depth,
        ) {
            Ok(pump) => insert_video(id, pump),
            Err(error) => report_io(error),
        };
    }
    #[cfg(windows)]
    {
        let (uploads, gpu, depth, previous_video, previous_recv) = match with_mixer(|mixer| {
            let mut shared = mixer.shared.lock().expect("shared");
            let previous_video = shared.videos.remove(&id);
            let previous_recv = shared.receivers.remove(&id);
            let uploads = shared.uploads.clone();
            let gpu = shared.gpu_video.clone();
            let session = shared.frame_buffer_frames.clamp(1, 8);
            let depth = if frame_buffer_frames == 0 {
                session
            } else {
                frame_buffer_frames.clamp(1, 8)
            };
            (uploads, gpu, depth, previous_video, previous_recv)
        }) {
            Ok(value) => value,
            Err(code) => return code,
        };
        drop(previous_recv);
        drop(previous_video);
        let Some(gpu) = gpu else {
            return ERR_DEVICE;
        };
        return match VideoPump::start(
            id,
            path,
            capture != 0,
            format,
            width,
            height,
            fps_num,
            fps_den,
            uploads,
            gpu,
            depth,
        ) {
            Ok(pump) => insert_video(id, pump),
            Err(error) => report_io(error),
        };
    }
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
pub unsafe extern "C" fn mixer_video_enum_captures(out: *mut VideoCaptureInfo, cap: u32) -> i32 {
    if out.is_null() || cap == 0 {
        return 0;
    }
    let dest = unsafe { std::slice::from_raw_parts_mut(out, cap as usize) };
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = dest;
        return 0;
    }
    #[cfg(any(windows, target_os = "macos"))]
    {
        let devices = enumerate_video_captures();
        let n = devices.len().min(dest.len());
        for (slot, (name, id)) in dest.iter_mut().zip(devices).take(n) {
            *slot = VideoCaptureInfo {
                id: write_fixed(&id),
                name: write_fixed(&name),
            };
        }
        n as i32
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_video_enum_capture_modes(
    device_id: *const c_char,
    out: *mut VideoCaptureMode,
    cap: u32,
) -> i32 {
    if device_id.is_null() || out.is_null() || cap == 0 {
        return 0;
    }
    let id = unsafe { CStr::from_ptr(device_id) }
        .to_str()
        .unwrap_or_default();
    if id.is_empty() {
        return 0;
    }
    let dest = unsafe { std::slice::from_raw_parts_mut(out, cap as usize) };
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = dest;
        return 0;
    }
    #[cfg(windows)]
    {
        let modes = crate::media::enumerate_capture_modes(id);
        let n = modes.len().min(dest.len());
        dest[..n].copy_from_slice(&modes[..n]);
        return n as i32;
    }
    #[cfg(target_os = "macos")]
    {
        let modes = crate::media_macos::enumerate_capture_modes(id);
        let n = modes.len().min(dest.len());
        dest[..n].copy_from_slice(&modes[..n]);
        n as i32
    }
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
    crate::diag::info(&format!("omt_connect id={id}"));
    let taken = match with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        #[cfg(any(windows, target_os = "macos"))]
        let previous_video = shared.videos.remove(&id);
        let previous_recv = shared.receivers.remove(&id);
        let uploads = shared.uploads.clone();
        let gpu = if use_gpu != 0 {
            Some(shared.omt_gpu.clone())
        } else {
            None
        };
        #[cfg(any(windows, target_os = "macos"))]
        {
            (uploads, gpu, previous_recv, previous_video)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            (uploads, gpu, previous_recv)
        }
    }) {
        Ok(value) => value,
        Err(code) => return code,
    };
    #[cfg(any(windows, target_os = "macos"))]
    let (uploads, gpu, previous_recv, previous_video) = taken;
    #[cfg(not(any(windows, target_os = "macos")))]
    let (uploads, gpu, previous_recv) = taken;
    drop(previous_recv);
    #[cfg(any(windows, target_os = "macos"))]
    drop(previous_video);
    match OmtReceiver::start(id, address, uploads, gpu, depth, quality) {
        Ok(receiver) => insert_receiver(id, LiveReceiver::Omt(receiver)),
        Err(error) => report_io(error),
    }
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
    crate::diag::info(&format!("ndi_connect id={id}"));
    #[cfg(any(windows, target_os = "macos"))]
    {
        let (uploads, gpu, previous_video, previous_recv) = match with_mixer(|mixer| {
            let mut shared = mixer.shared.lock().expect("shared");
            let previous_video = shared.videos.remove(&id);
            let previous_recv = shared.receivers.remove(&id);
            let uploads = shared.uploads.clone();
            let gpu = shared.gpu_ingest.clone();
            (uploads, gpu, previous_video, previous_recv)
        }) {
            Ok(value) => value,
            Err(code) => return code,
        };
        drop(previous_recv);
        drop(previous_video);
        return match NdiReceiver::start(id, address, uploads, Some(gpu), depth, low_bandwidth) {
            Ok(receiver) => insert_receiver(id, LiveReceiver::Ndi(receiver)),
            Err(error) => report_io(error),
        };
    }
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
    crate::diag::info(&format!("output_add id={output_id} transport={transport}"));
    if transport == OUT_DECKLINK {
        let _ = with_mixer(|mixer| {
            set_error(
                &mixer.telemetry,
                "DeckLink output is not linked in this build",
            );
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
    let error = match with_mixer(|mixer| {
        mixer
            .telemetry
            .lock()
            .expect("telemetry")
            .last_error
            .clone()
    }) {
        Ok(error) if !error.is_empty() => error,
        _ => session_error_slot().lock().expect("session error").clone(),
    };
    let n = error.len().min(cap);
    unsafe { std::ptr::copy_nonoverlapping(error.as_ptr(), out, n) };
    n as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_take_fatal(out: *mut u8, cap: usize) -> i32 {
    if out.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let Some(error) = crate::diag::take_fatal() else {
        return 0;
    };
    let n = error.len().min(cap);
    unsafe { std::ptr::copy_nonoverlapping(error.as_ptr(), out, n) };
    n as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_session_load(path: *const c_char, out: *mut u8, cap: usize) -> i32 {
    if path.is_null() || out.is_null() || cap == 0 {
        return -ERR_INVALID_ARGUMENT;
    }
    match std::fs::read(read_cstr(path)) {
        Ok(bytes) => match session::canonicalize_bytes(&bytes) {
            Ok(canonical) => copy_bytes(&canonical, out, cap),
            Err(error) => {
                report_session_error(error);
                -ERR_INVALID_ARGUMENT
            }
        },
        Err(error) => {
            report_session_error(error.to_string());
            -ERR_IO
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
        return -ERR_INVALID_ARGUMENT;
    }
    let bytes = unsafe { std::slice::from_raw_parts(json, len) };
    match session::canonicalize_bytes(bytes) {
        Ok(canonical) => copy_bytes(&canonical, out, cap),
        Err(error) => {
            report_session_error(error);
            -ERR_INVALID_ARGUMENT
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_destroy_source(id: u64) -> i32 {
    crate::diag::info(&format!("destroy_source id={id}"));
    match detach_source(id) {
        Ok(taken) => {
            drop(taken.receiver);
            #[cfg(any(windows, target_os = "macos"))]
            drop(taken.video);
            taken.uploads.lock().expect("uploads").unregister(id);
            OK
        }
        Err(code) => code,
    }
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
        let (master, buses, mix_peaks, uploads) = {
            let shared = mixer.shared.lock().expect("shared");
            let master = shared.audio.master_peak();
            let buses = shared.audio.bus_peaks();
            let mix_peaks = shared.audio.mix_input_peaks();
            let uploads = Arc::clone(&mixer.uploads);
            drop(shared);
            (master, buses, mix_peaks, uploads)
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
            if mix_peaks.iter().any(|(mix_id, ..)| *mix_id == id) {
                continue;
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
        for (id, left, right) in mix_peaks {
            if n >= cap {
                break;
            }
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
        let (uploads_arc, generator_ids, scenes) = {
            let shared = mixer.shared.lock().expect("shared");
            let tel = mixer.telemetry.lock().expect("telemetry");
            (
                Arc::clone(&shared.uploads),
                shared.generators.keys().copied().collect::<Vec<_>>(),
                tel.scene_usage.clone(),
            )
        };
        let uploads = uploads_arc.lock().expect("uploads");
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
                    gpu_pct: 0.0,
                };
            }
            n += 1;
        }
        for id in generator_ids {
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
                    gpu_pct: 0.0,
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
            let (width, height) = if id == SRC_BARS {
                (1920u32, 1080u32)
            } else {
                (128, 72)
            };
            unsafe {
                *out.add(n as usize) = SourceUsage {
                    source_id: id,
                    width,
                    height,
                    ram_bytes: 0,
                    vram_bytes: u64::from(width) * u64::from(height) * 4,
                    gpu_pct: 0.0,
                };
            }
            n += 1;
        }
        for usage in scenes {
            if n >= cap {
                break;
            }
            if !seen.insert(usage.source_id) {
                continue;
            }
            unsafe {
                *out.add(n as usize) = usage;
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
        let tel = mixer.telemetry.lock().expect("telemetry");
        let render_ms = tel.last_render_ms;
        let budget = 1000.0 * den as f32 / num.max(1) as f32;
        unsafe {
            *out = MixerStats {
                render_ms,
                frame_budget_ms: budget,
                ram_bytes: tel.last_ram_bytes,
                vram_bytes: tel.last_vram_bytes,
                compose_vram_bytes: tel.last_compose_vram,
                delay_vram_bytes: tel.last_delay_vram,
                surface_lost: crate::diag::surface_lost(),
            };
        }
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_set_bus_colors(
    prv_r: u8,
    prv_g: u8,
    prv_b: u8,
    pgm_r: u8,
    pgm_g: u8,
    pgm_b: u8,
    in_r: u8,
    in_g: u8,
    in_b: u8,
) -> i32 {
    with_mixer(|mixer| {
        mixer.shared.lock().expect("shared").bus_colors = BusColors {
            preview: [prv_r, prv_g, prv_b],
            program: [pgm_r, pgm_g, pgm_b],
            inactive: [in_r, in_g, in_b],
        };
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_set_mv_label(scene_id: u64, size: f32, percent: u32, top: u32) -> i32 {
    with_mixer(|mixer| {
        let style = MvLabelStyle {
            size: crate::labels::clamp_size(size),
            percent: percent != 0,
            top: top != 0,
        };
        let mut shared = mixer.shared.lock().expect("shared");
        if scene_id == 0 {
            shared.mv_label = style;
        } else if let Some(spec) = shared.scenes.get_mut(&scene_id) {
            spec.mv_label = style;
        } else {
            shared.mv_label = style;
        }
        OK
    })
    .unwrap_or_else(|code| code)
}

fn copy_c_label(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: host keeps the UTF-8 C string readable for this FFI call.
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
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
        let mut shared = mixer.shared.lock().expect("shared");
        shared.rebar_optimization = enabled != 0;
        shared
            .gpu_ingest
            .use_rebar
            .store(enabled != 0 && shared.rebar.available, Ordering::Relaxed);
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_set_ndi_gpu_upload(enabled: u32) -> i32 {
    with_mixer(|mixer| {
        let mut shared = mixer.shared.lock().expect("shared");
        shared.ndi_gpu_upload = enabled != 0;
        shared
            .gpu_ingest
            .ndi_gpu
            .store(enabled != 0, Ordering::Relaxed);
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

#[unsafe(no_mangle)]
pub extern "C" fn mixer_thumb_set(source_id: u64, width: u32, height: u32, interval: u32) -> i32 {
    with_mixer(|mixer| {
        let mut guard = mixer.shared.lock().expect("shared");
        match crate::thumb::ThumbSub::clamp(width, height, interval) {
            Some(sub) => {
                guard.thumbs.insert(source_id, sub);
            }
            None => {
                guard.thumbs.remove(&source_id);
            }
        }
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_thumb_read(
    source_id: u64,
    buf: *mut u8,
    cap: usize,
    out_w: *mut u32,
    out_h: *mut u32,
    out_stride: *mut u32,
) -> i32 {
    if buf.is_null() || out_w.is_null() || out_h.is_null() || out_stride.is_null() {
        return 0;
    }
    with_mixer(|mixer| {
        let pixels = mixer.thumb_pixels.lock().expect("thumb pixels");
        let Some(frame) = pixels.get(&source_id) else {
            return 0;
        };
        if cap < frame.data.len() {
            return 0;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(frame.data.as_ptr(), buf, frame.data.len());
            *out_w = frame.width;
            *out_h = frame.height;
            *out_stride = frame.stride;
        }
        frame.data.len() as i32
    })
    .unwrap_or(0)
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
    thumb_pixels: Arc<Mutex<HashMap<u64, crate::thumb::ThumbPixels>>>,
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
    let mut thumbs = crate::thumb::ThumbStore::new(thumb_pixels);
    let mut readbacks = ReadbackStore::default();
    let mut gpu_sends = GpuSendStore::default();
    let mut frame_delay = FrameDelay::new(3);
    let frame_dt = Duration::from_nanos(1_000_000_000u64 * u64::from(fps_den) / u64::from(fps_num));
    let mut next = Instant::now();
    let clock_start = Instant::now();
    let mut frame_i = 0u64;
    let mut audio_produced = 0u64;
    let mut audio_carry = 0u64;
    let mut last_bus: HashMap<u64, (u64, u64, u32, u64)> = HashMap::new();
    let mut snapshot = Vec::new();
    let mut scene_specs = Vec::new();
    let mut scene_labels = HashMap::new();
    let mut generators = Vec::new();
    let mut outputs_snap = Vec::new();
    let mut cached_mem = (0u64, 0u64);
    let mut cached_adapter = 0u64;
    let mut mem_at = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    while !stop.load(Ordering::Relaxed) && !crate::diag::is_fatal() {
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
                    let code = match presenters
                        .attach(&device, unit_id, kind, surface, width, height, prepared)
                    {
                        Ok(()) => {
                            shared.lock().expect("shared").compose_dirty = true;
                            OK
                        }
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
                        &device, monitor_id, source_id, surface, width, height, prepared,
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
                GpuCmd::DetachMonitor { monitor_id, reply } => {
                    presenters.detach_monitor(monitor_id);
                    let _ = reply.send(OK);
                }
                GpuCmd::SetMonitorSource {
                    monitor_id,
                    source_id,
                } => presenters.set_monitor_source(monitor_id, source_id),
                GpuCmd::SetMonitorInterval { monitor_id, frames } => {
                    presenters.set_monitor_interval(monitor_id, frames)
                }
                GpuCmd::Shutdown => {
                    drop(presenters);
                    let _ = device.device.poll(wgpu::PollType::Wait {
                        submission_index: None,
                        timeout: Some(Duration::from_millis(200)),
                    });
                    return;
                }
            }
        }
        if crate::diag::take_gpu_fault() {
            crate::diag::mark_fatal("GPU device fault");
            set_error(&telemetry, "GPU device fault");
            break;
        }
        let frame = panic::catch_unwind(AssertUnwindSafe(|| {
            presenters.reconfigure_pending(&device);
        }));
        if frame.is_err() {
            crate::diag::error("presenter reconfigure panicked");
            set_error(&telemetry, "presenter reconfigure panicked");
            crate::diag::mark_fatal("presenter reconfigure panicked");
            break;
        }
        let (buffer_frames, use_rebar, direct_sample) = {
            let guard = shared.lock().expect("shared");
            let use_rebar = guard.rebar.available && guard.rebar_optimization;
            let direct_sample = use_rebar && cfg!(target_os = "macos");
            (
                {
                    let mix_max = guard
                        .mix_inputs
                        .values()
                        .map(|spec| spec.delay)
                        .max()
                        .unwrap_or(1);
                    guard.frame_buffer_frames.clamp(1, 8).max(mix_max)
                },
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
        {
            let mut guard = shared.lock().expect("shared");
            for unit in guard.units.values_mut() {
                tick_unit_transitions(unit);
            }
            snapshot.clear();
            snapshot.extend(guard.units.iter().map(|(id, unit)| {
                let mix_preview = snapshot_mix_preview(unit);
                let mut state = unit.state;
                state.incoming_source = mix_preview;
                (
                    *id,
                    unit.width,
                    unit.height,
                    unit.fps_num,
                    unit.fps_den,
                    state,
                    mix_preview,
                    unit.custom_wgsl.clone(),
                )
            }));
            scene_specs.clear();
            scene_specs.extend(guard.scenes.iter().map(|(id, spec)| {
                (
                    *id,
                    spec.width,
                    spec.height,
                    Arc::clone(&spec.layers),
                    spec.mv_label,
                )
            }));
            scene_labels.clear();
            scene_labels.extend(
                guard
                    .scenes
                    .iter()
                    .map(|(id, spec)| (*id, Arc::clone(&spec.labels))),
            );
            let bus_colors = guard.bus_colors;
            generators.clear();
            generators.extend(guard.generators.iter().map(|(id, spec)| (*id, *spec)));
            outputs_snap.clear();
            outputs_snap.extend(guard.outputs.iter().map(|(id, output)| {
                OutputSnap {
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
                }
            }));
            let compose_dirty = guard.compose_dirty;
            guard.compose_dirty = false;
            let thumbs_snap = guard.thumbs.clone();
            let mix_inputs = guard.mix_inputs.clone();
            drop(guard);
            let changed_units: Vec<u64> = snapshot
                .iter()
                .filter(|(id, _, _, _, _, state, mix_preview, _)| {
                    last_bus
                        .get(id)
                        .is_none_or(|(program, preview, mix, incoming)| {
                            *program != state.program_source
                                || *preview != state.preview_source
                                || *mix != state.mix.to_bits()
                                || *incoming != *mix_preview
                        })
                })
                .map(|(id, ..)| *id)
                .collect();
            if !changed_units.is_empty() {
                frame_delay.discard(changed_units);
            }
            let tallies: Vec<(u64, u64)> = snapshot
                .iter()
                .map(|item| (item.5.preview_source, item.5.program_source))
                .collect();
            frame_i = frame_i.wrapping_add(1);
            // Three lanes:
            // 1. On-air playout/upload — every master frame (FIFOs and live pixels).
            // 2. On-air compose (Preview/Program/outputs) — every master frame.
            // 3. Monitor compose and GUI-only upload — present_interval only.
            // Save roles still see every attached monitor/thumb so OMT quality
            // does not flap on skipped present ticks.
            let due_monitors = presenters.attached_monitor_sources_due(frame_i);
            let due_thumbs: Vec<u64> = thumbs_snap
                .iter()
                .filter(|(_, sub)| frame_i % u64::from(sub.interval) == 0)
                .map(|(id, _)| *id)
                .collect();
            let mut compose_sources = due_monitors;
            compose_sources.extend_from_slice(&due_thumbs);
            let (mut used_scenes, used_uploads) = collect_frame_live_ids(
                &scene_specs,
                &snapshot,
                &compose_sources,
                &outputs_snap,
                &mix_inputs,
                compose_dirty,
            );
            let mut role_sources = presenters.attached_monitor_sources();
            role_sources.extend(thumbs_snap.keys().copied());
            let output_refs: Vec<(u32, u64)> = outputs_snap
                .iter()
                .map(|item| (item.source_kind, item.source_id))
                .collect();
            let roles = collect_source_roles(&scene_specs, &snapshot, &role_sources, &output_refs);
            {
                let guard = shared.lock().expect("shared");
                for (id, receiver) in &guard.receivers {
                    let save = guard.live_save.get(id).copied().unwrap_or_default();
                    let role = roles.get(id).copied().unwrap_or_default();
                    receiver.apply_save(want_full(save, role), role.on_program, role.on_preview);
                }
            }
            let frame_begin = Instant::now();
            let pts = (clock_start.elapsed().as_nanos() / 100) as i64;
            let secs = frame_i as f64 * f64::from(fps_den) / f64::from(fps_num.max(1));
            let phase = ((secs * 0.12) % 1.0) as f32;
            let phase_y = ((secs * 0.07) % 1.0) as f32;
            let composed = panic::catch_unwind(AssertUnwindSafe(|| {
                composer.begin_frame();
                composer.ensure_builtins(&device);
                composer.sync_generators(&generators, phase, phase_y);
                let need_bake = composer.generators_need_rebake();
                if need_bake {
                    for (id, ..) in &scene_specs {
                        used_scenes.insert(*id);
                    }
                }
                let snaps = {
                    let mut upload_guard = uploads.lock().expect("uploads");
                    upload_guard.advance_playout(&used_uploads);
                    upload_guard.snapshot(&used_uploads)
                };
                composer.upload_sources(&device, &snaps, use_rebar, direct_sample);
                need_bake
            }));
            let need_gen_bake = match composed {
                Ok(need_bake) => need_bake,
                Err(_) => {
                    crate::diag::error("compose panicked");
                    set_error(&telemetry, "compose panicked");
                    crate::diag::mark_fatal("compose panicked");
                    false
                }
            };
            if need_gen_bake {
                frame_delay.discard(snapshot.iter().map(|(id, ..)| *id));
            }
            let need_prv = outputs_snap
                .iter()
                .any(|item| item.source_kind == SRC_KIND_MU_PREVIEW && item.cpu_video());
            let present_epoch = composer.gpu_epoch() ^ frame_delay.epoch().rotate_left(8);
            match panic::catch_unwind(AssertUnwindSafe(|| {
                presenters.present_unit_buses(&device, present_epoch, |unit_id, kind| {
                    frame_delay
                        .view(unit_id, kind)
                        .or_else(|| composer.unit_view(unit_id, kind))
                })
            })) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    set_error(&telemetry, error.clone());
                    if error.contains("unconfigured") {
                        crate::diag::mark_fatal(error);
                        break;
                    }
                }
                Err(_) => {
                    crate::diag::error("present unit buses panicked");
                    set_error(&telemetry, "present panicked");
                    crate::diag::mark_fatal("present unit buses panicked");
                    break;
                }
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
                        // Mix/T-bar ticks discard the delay ring so present can
                        // show the live compose. Program send must do the same
                        // or NDI/OMT freeze until mix is stable again.
                        if let Some(packed) = frame_delay
                            .packed(*unit_id, OUTPUT_PROGRAM)
                            .or_else(|| composer.packed_texture(*unit_id, OUTPUT_PROGRAM))
                        {
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
                        SRC_KIND_MU_PREVIEW => frame_delay
                            .packed(output.unit_id, OUTPUT_PREVIEW)
                            .or_else(|| composer.packed_texture(output.unit_id, OUTPUT_PREVIEW)),
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
                        SRC_KIND_MU_PROGRAM => frame_delay
                            .rgba(output.unit_id, OUTPUT_PROGRAM)
                            .or_else(|| composer.rgba_texture(output.unit_id, OUTPUT_PROGRAM)),
                        SRC_KIND_MU_PREVIEW => frame_delay
                            .rgba(output.unit_id, OUTPUT_PREVIEW)
                            .or_else(|| composer.rgba_texture(output.unit_id, OUTPUT_PREVIEW)),
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
            frame_delay.consume_display(false);
            composer.set_bus_colors(bus_colors.preview, bus_colors.program, bus_colors.inactive);
            composer.sync_scenes(&device, &scene_specs, &scene_labels);
            let mut encoder =
                device
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("eiviz compose"),
                    });
            {
                let mix_sources = resolve_mix_sources(&mix_inputs, &frame_delay);
                composer.stage_mix_inputs(
                    &device,
                    &mut encoder,
                    mix_inputs.keys().copied(),
                    &mix_sources,
                );
            }
            if let Err(error) =
                composer.render_scenes(&device, &used_scenes, &mut encoder, &tallies)
            {
                set_error(&telemetry, error);
            }
            let mut packed_copies: Vec<(u64, u32, u32)> = Vec::new();
            let mut gpu_copies: Vec<(u64, wgpu::Texture, u32, u32, Arc<AtomicBool>, u32, u32)> =
                Vec::new();
            for (unit_id, width, height, _, _, state, mix_preview, custom) in &snapshot {
                composer.ensure_unit(&device, *unit_id, *width, *height);
                if let Err(error) =
                    composer.set_custom_mix(&device, *unit_id, custom.as_deref().unwrap_or(""))
                {
                    crate::diag::error(&format!("custom wgsl: {error}"));
                }
                let pack_pgm = outputs_snap.iter().any(|item| {
                    item.unit_id == *unit_id
                        && item.source_kind == SRC_KIND_MU_PROGRAM
                        && item.cpu_video()
                });
                if let Err(error) = composer.render_unit(
                    &device,
                    *unit_id,
                    state,
                    *mix_preview,
                    &mut encoder,
                    pack_pgm,
                ) {
                    set_error(&telemetry, error);
                }
                composer.pack_aux(&device, &mut encoder, *unit_id, need_prv);
            }
            for output in &outputs_snap {
                if output.source_kind == SRC_KIND_MU_PROGRAM
                    || output.source_kind == SRC_KIND_MU_PREVIEW
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
                    SRC_KIND_SCENE | SRC_KIND_MU_MULTIVIEW => {
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
                    || !output.gpu_video()
                {
                    continue;
                }
                let src = match output.source_kind {
                    SRC_KIND_INPUT => composer
                        .mix_texture(output.source_id)
                        .or_else(|| composer.source_texture(output.source_id)),
                    SRC_KIND_SCENE | SRC_KIND_MU_MULTIVIEW => {
                        composer.scene_texture(output.source_id)
                    }
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
            frame_delay.capture_scenes(
                &device,
                &mut encoder,
                &composer,
                mix_inputs
                    .values()
                    .filter(|spec| spec.is_session_multiview())
                    .map(|spec| spec.target_id),
            );
            thumbs.capture(&device, &mut composer, &mut encoder, frame_i, &thumbs_snap);
            device.submit(Some(encoder.finish()));
            thumbs.advance(&device);
            emit_packed(
                &mut readbacks,
                &device,
                &packed_copies,
                &outputs_snap,
                &send_tx,
                pts,
            );
            emit_gpu(&gpu_copies, &send_tx, pts);
            for (id, _, _, _, _, state, mix_preview, ..) in &snapshot {
                last_bus.insert(
                    *id,
                    (
                        state.program_source,
                        state.preview_source,
                        state.mix.to_bits(),
                        *mix_preview,
                    ),
                );
            }
            if presenters.any_monitor_due(frame_i) {
                let monitor_epoch = composer.gpu_epoch() ^ frame_delay.epoch().rotate_left(8);
                if panic::catch_unwind(AssertUnwindSafe(|| {
                    if let Err(error) =
                        presenters.present_monitors(&device, monitor_epoch, frame_i, |source_id| {
                            frame_delay
                                .view_for_source(source_id)
                                .or_else(|| composer.view_for_source(source_id))
                                .map(|view| (view, composer.source_is_packed(source_id)))
                        })
                    {
                        set_error(&telemetry, error);
                    }
                }))
                .is_err()
                {
                    crate::diag::error("present monitors panicked");
                    set_error(&telemetry, "present monitors panicked");
                }
            }
            audio_carry += AUDIO_RATE as u64 * u64::from(fps_den);
            let audio_frames = (audio_carry / u64::from(fps_num.max(1))) as usize;
            audio_carry %= u64::from(fps_num.max(1));
            audio_produced = audio_produced.saturating_add(audio_frames as u64);
            let (audio, tone_packets) = {
                let mut guard = shared.lock().expect("shared");
                let audio = guard.audio.clone();
                let mut tone_packets = Vec::new();
                if audio_frames > 0 {
                    for (id, spec) in &generators {
                        if spec.tone_hz <= 0.0 {
                            continue;
                        }
                        let phase = guard.tone_phase.entry(*id).or_insert(0.0);
                        tone_packets.push((
                            *id,
                            generator_audio::sine_packet(
                                phase,
                                spec.tone_hz,
                                spec.tone_level_dbfs,
                                audio_frames,
                                pts,
                            ),
                        ));
                    }
                }
                (audio, tone_packets)
            };
            let mut upload_guard = uploads.lock().expect("uploads");
            for (id, packet) in tone_packets {
                upload_guard.ingest_audio(id, packet);
            }
            let mixed = audio.mix(
                &mut upload_guard,
                &snapshot,
                &scene_specs,
                audio_frames,
                true,
                &mix_inputs,
                fps_num,
                fps_den,
            );
            drop(upload_guard);
            let compose_vram = composer.vram_bytes();
            let delay_vram = frame_delay.vram_bytes();
            let send_vram = gpu_sends.vram_bytes();
            if mem_at.elapsed() >= Duration::from_millis(500) {
                cached_mem = uploads.lock().expect("uploads").memory_bytes();
                cached_adapter = crate::rebar::adapter_usage_bytes(&device.device);
                mem_at = Instant::now();
            }
            let (ram, source_vram) = cached_mem;
            let accounted = source_vram
                .saturating_add(compose_vram)
                .saturating_add(delay_vram)
                .saturating_add(send_vram);
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
                guard.last_ram_bytes = ram;
                guard.last_compose_vram = compose_vram;
                guard.last_delay_vram = delay_vram;
                guard.last_vram_bytes = cached_adapter.max(accounted);
                guard.scene_usage = composer.scene_usages();
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
    let mut planar = Vec::with_capacity(frames * 2);
    for sample in interleaved.iter().step_by(2) {
        planar.push(*sample);
    }
    for sample in interleaved.iter().skip(1).step_by(2) {
        planar.push(*sample);
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
    snapshot: &[UnitSnap],
    scenes: &[(u64, u32, u32, Arc<[OverlayDesc]>, MvLabelStyle)],
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
            gains
                .entry(id)
                .and_modify(|current| *current = (*current).max(gain))
                .or_insert(gain);
        }
    }
    for (_, _, _, _, _, state, mix_preview, _) in snapshot {
        let mix = state.mix.clamp(0.0, 1.0);
        let incoming = if *mix_preview != 0 {
            *mix_preview
        } else {
            state.mix_incoming()
        };
        add(state.program_source, 1.0 - mix, &spec_map, &mut gains);
        add(incoming, mix, &spec_map, &mut gains);
        for overlay in state.overlays.iter().take(state.overlay_count as usize) {
            if overlay.audio_follow == 0 {
                continue;
            }
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

#[allow(dead_code)]
fn audio_for_source(
    uploads: &UploadStore,
    scenes: &HashMap<u64, SceneSpec>,
    source_id: u64,
) -> Option<AudioPacket> {
    if let Some(ring) = uploads.get(source_id) {
        if ring.audio.is_some() {
            return ring.audio.clone();
        }
    }
    if crate::abi::is_scene(source_id)
        && let Some(spec) = scenes.get(&source_id)
    {
        for layer in spec.layers.iter() {
            if let Some(audio) = audio_for_source(uploads, scenes, layer.source_id) {
                return Some(audio);
            }
        }
    }
    None
}

/// Scenes and CPU/GPU uploads that must be current for the given buses.
/// Pass only *due* monitor ids for compose; pass every attached monitor when
/// collecting uploads so tile-only sources keep their FIFOs moving.
fn unit_uses_mix_cycle(
    unit_id: u64,
    state: &UnitState,
    mix_inputs: &HashMap<u64, MixInputSpec>,
    scenes: &HashMap<u64, SceneSpec>,
) -> bool {
    let mut seen = HashSet::new();
    let incoming = state.mix_incoming();
    mix_source_cycles(state.program_source, unit_id, mix_inputs, scenes, &mut seen)
        || mix_source_cycles(state.preview_source, unit_id, mix_inputs, scenes, &mut seen)
        || mix_source_cycles(incoming, unit_id, mix_inputs, scenes, &mut seen)
        || state
            .overlays
            .iter()
            .take(state.overlay_count as usize)
            .any(|overlay| {
                mix_source_cycles(overlay.source_id, unit_id, mix_inputs, scenes, &mut seen)
            })
}

fn mix_source_cycles(
    source_id: u64,
    unit_id: u64,
    mix_inputs: &HashMap<u64, MixInputSpec>,
    scenes: &HashMap<u64, SceneSpec>,
    seen: &mut HashSet<u64>,
) -> bool {
    if !seen.insert(source_id) {
        return false;
    }
    if let Some(spec) = mix_inputs.get(&source_id)
        && !spec.is_session_multiview()
        && spec.target_id == unit_id
    {
        return true;
    }
    if let Some(scene) = scenes.get(&source_id) {
        return scene
            .layers
            .iter()
            .any(|layer| mix_source_cycles(layer.source_id, unit_id, mix_inputs, scenes, seen));
    }
    false
}

fn resolve_mix_sources<'a>(
    mix_inputs: &HashMap<u64, MixInputSpec>,
    frame_delay: &'a FrameDelay,
) -> HashMap<u64, &'a wgpu::Texture> {
    let mut sources = HashMap::new();
    for id in mix_inputs.keys() {
        if let Some(texture) = mix_rgba_at(mix_inputs, frame_delay, *id) {
            sources.insert(*id, texture);
        }
    }
    sources
}

fn mix_rgba_at<'a>(
    mix_inputs: &HashMap<u64, MixInputSpec>,
    frame_delay: &'a FrameDelay,
    source_id: u64,
) -> Option<&'a wgpu::Texture> {
    let spec = mix_inputs.get(&source_id)?;
    if spec.is_session_multiview() {
        frame_delay.scene_rgba_at(spec.target_id, spec.delay)
    } else if let Some(bus) = spec.unit_bus() {
        frame_delay.rgba_at(spec.target_id, bus, spec.delay)
    } else {
        None
    }
}

fn collect_frame_live_ids(
    scene_specs: &[(u64, u32, u32, Arc<[OverlayDesc]>, MvLabelStyle)],
    snapshot: &[UnitSnap],
    due_gui: &[u64],
    outputs: &[OutputSnap],
    mix_inputs: &HashMap<u64, MixInputSpec>,
    compose_dirty: bool,
) -> (HashSet<u64>, HashSet<u64>) {
    let (mut scenes, mut uploads) =
        collect_live_ids(scene_specs, snapshot, due_gui, outputs, mix_inputs);
    if compose_dirty {
        let dirty: Vec<u64> = scene_specs.iter().map(|spec| spec.0).collect();
        scenes.extend(dirty.iter().copied());
        let (_, dirty_uploads) = collect_live_ids(scene_specs, &[], &dirty, &[], mix_inputs);
        uploads.extend(dirty_uploads);
    }
    (scenes, uploads)
}

fn collect_live_ids(
    scene_specs: &[(u64, u32, u32, Arc<[OverlayDesc]>, MvLabelStyle)],
    snapshot: &[UnitSnap],
    monitor_sources: &[u64],
    outputs: &[OutputSnap],
    mix_inputs: &HashMap<u64, MixInputSpec>,
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
        mix_inputs: &HashMap<u64, MixInputSpec>,
        scenes: &mut HashSet<u64>,
        uploads: &mut HashSet<u64>,
    ) {
        if let Some(spec) = mix_inputs.get(&id) {
            if spec.is_session_multiview() {
                add(spec.target_id, spec_map, mix_inputs, scenes, uploads);
            }
            return;
        }
        if crate::abi::is_scene(id) {
            if !scenes.insert(id) {
                return;
            }
            if let Some(layers) = spec_map.get(&id) {
                for layer in *layers {
                    add(layer.source_id, spec_map, mix_inputs, scenes, uploads);
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
    for (_, _, _, _, _, state, mix_preview, _) in snapshot {
        add(
            state.program_source,
            &spec_map,
            mix_inputs,
            &mut scenes,
            &mut uploads,
        );
        add(
            state.preview_source,
            &spec_map,
            mix_inputs,
            &mut scenes,
            &mut uploads,
        );
        let incoming = if *mix_preview != 0 {
            *mix_preview
        } else {
            state.mix_incoming()
        };
        add(incoming, &spec_map, mix_inputs, &mut scenes, &mut uploads);
        for overlay in state.overlays.iter().take(state.overlay_count as usize) {
            add(
                overlay.source_id,
                &spec_map,
                mix_inputs,
                &mut scenes,
                &mut uploads,
            );
        }
    }
    for &id in monitor_sources {
        add(id, &spec_map, mix_inputs, &mut scenes, &mut uploads);
    }
    for output in outputs {
        match output.source_kind {
            SRC_KIND_SCENE | SRC_KIND_MU_MULTIVIEW => add(
                output.source_id,
                &spec_map,
                mix_inputs,
                &mut scenes,
                &mut uploads,
            ),
            SRC_KIND_INPUT => add(
                output.source_id,
                &spec_map,
                mix_inputs,
                &mut scenes,
                &mut uploads,
            ),
            _ => {}
        }
    }
    (scenes, uploads)
}

fn send_loop(rx: mpsc::Receiver<SendCmd>, stop: Arc<AtomicBool>, omt_gpu: OmtGpu) {
    let mut senders: HashMap<u64, (OutputHandle, Arc<AtomicBool>)> = HashMap::new();
    while !stop.load(Ordering::Relaxed) && !crate::diag::is_fatal() {
        loop {
            match rx.try_recv() {
                Ok(SendCmd::Shutdown) => return,
                Ok(cmd) => {
                    if stop.load(Ordering::Relaxed) {
                        if let SendCmd::GpuVideo { busy, .. } = &cmd {
                            busy.store(false, Ordering::Release);
                        }
                        if matches!(cmd, SendCmd::Shutdown) {
                            return;
                        }
                        continue;
                    }
                    apply_send_cmd(&mut senders, cmd, &omt_gpu);
                }
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
                if panic::catch_unwind(AssertUnwindSafe(|| {
                    sender.send_video_uyvy(width, height, stride, pts, data, fps_n, fps_d)
                }))
                .is_err()
                {
                    crate::diag::mark_fatal("omt send video panicked");
                }
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
                match panic::catch_unwind(AssertUnwindSafe(|| {
                    sender.send_video_texture(omt_gpu, &texture, width, height, pts, fps_n, fps_d)
                })) {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => crate::diag::mark_fatal(format!("omt send texture: {error}")),
                    Err(_) => crate::diag::mark_fatal("omt send texture panicked"),
                }
            }
            busy.store(false, Ordering::Release);
        }
        SendCmd::Audio { output_id, packet } => {
            if let Some((sender, _)) = senders.get_mut(&output_id) {
                if panic::catch_unwind(AssertUnwindSafe(|| sender.send_audio(&packet))).is_err() {
                    crate::diag::mark_fatal("omt send audio panicked");
                }
            }
        }
        SendCmd::Shutdown => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn ping_is_stable() {
        assert_eq!(mixer_ping(), 0x4549_5649);
    }

    #[test]
    fn take_fatal_rejects_null() {
        assert_eq!(
            unsafe { mixer_take_fatal(std::ptr::null_mut(), 8) },
            ERR_INVALID_ARGUMENT
        );
    }

    #[test]
    fn collect_live_ids_keeps_input_monitors() {
        let (scenes, uploads) = collect_live_ids(&[], &[], &[SRC_COLOR, 20], &[], &HashMap::new());
        assert!(scenes.is_empty());
        assert!(uploads.contains(&SRC_COLOR));
        assert!(uploads.contains(&20));
    }

    #[test]
    fn collect_live_ids_on_air_skips_idle_monitor_scenes() {
        let empty: std::sync::Arc<[crate::abi::OverlayDesc]> = std::sync::Arc::from([]);
        let on_air = SCENE_BASE | 1;
        let idle = SCENE_BASE | 2;
        let specs = [
            (on_air, 1920, 1080, empty.clone(), MvLabelStyle::default()),
            (idle, 1920, 1080, empty, MvLabelStyle::default()),
        ];
        let snapshot = [(
            1,
            1920,
            1080,
            60_000,
            1_001,
            crate::abi::UnitState {
                program_source: on_air,
                preview_source: on_air,
                ..crate::abi::UnitState::default()
            },
            0,
            None,
        )];
        let (scenes, _) = collect_live_ids(&specs, &snapshot, &[], &[], &HashMap::new());
        assert!(scenes.contains(&on_air));
        assert!(!scenes.contains(&idle));
        let (scenes, _) = collect_live_ids(&specs, &snapshot, &[idle], &[], &HashMap::new());
        assert!(scenes.contains(&idle));
    }

    #[test]
    fn collect_live_ids_keeps_thumb_sources() {
        let empty: std::sync::Arc<[crate::abi::OverlayDesc]> = std::sync::Arc::from([]);
        let idle = SCENE_BASE | 3;
        let specs = [(idle, 1920, 1080, empty, MvLabelStyle::default())];
        let (scenes, _) = collect_live_ids(&specs, &[], &[idle], &[], &HashMap::new());
        assert!(scenes.contains(&idle));
    }

    #[test]
    fn collect_frame_live_ids_uploads_on_air_not_idle_gui() {
        let on_air = 20;
        let idle = 21;
        let snapshot = [(
            1,
            1920,
            1080,
            60_000,
            1_001,
            crate::abi::UnitState {
                program_source: on_air,
                preview_source: on_air,
                ..crate::abi::UnitState::default()
            },
            0,
            None,
        )];
        let (_, uploads) = collect_frame_live_ids(&[], &snapshot, &[], &[], &HashMap::new(), false);
        assert!(uploads.contains(&on_air));
        assert!(!uploads.contains(&idle));
        let (_, uploads) =
            collect_frame_live_ids(&[], &snapshot, &[idle], &[], &HashMap::new(), false);
        assert!(uploads.contains(&on_air));
        assert!(uploads.contains(&idle));
    }

    #[test]
    fn collect_frame_live_ids_dirty_uploads_idle_scene_layers() {
        let layer = 22;
        let idle = SCENE_BASE | 4;
        let layers: std::sync::Arc<[crate::abi::OverlayDesc]> =
            std::sync::Arc::from([crate::abi::OverlayDesc {
                source_id: layer,
                ..crate::abi::OverlayDesc::default()
            }]);
        let specs = [(idle, 1920, 1080, layers, MvLabelStyle::default())];
        let (scenes, uploads) =
            collect_frame_live_ids(&specs, &[], &[], &[], &HashMap::new(), false);
        assert!(!scenes.contains(&idle));
        assert!(!uploads.contains(&layer));
        let (scenes, uploads) =
            collect_frame_live_ids(&specs, &[], &[], &[], &HashMap::new(), true);
        assert!(scenes.contains(&idle));
        assert!(uploads.contains(&layer));
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
