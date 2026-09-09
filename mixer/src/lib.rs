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
mod lifecycle;
#[cfg(windows)]
mod media;
mod output;
mod render;
#[cfg(windows)]
pub use media::enumerate_video_captures;
#[cfg(target_os = "macos")]
mod media_macos;
#[cfg(target_os = "macos")]
pub use media_macos::enumerate_video_captures;
#[cfg(target_os = "macos")]
mod main_thread;
mod native_ws;
#[cfg(any(windows, target_os = "macos"))]
mod ndi;
mod omt;
mod pool;
mod present;
mod readback;
mod rebar;
mod runtime;
mod save;
mod session;
pub mod simd;
mod snapshot;
mod tcp_listen_owner;
mod thumb;
mod upload;
#[cfg(any(windows, target_os = "linux"))]
mod vk_upload;
#[cfg(any(windows, target_os = "linux"))]
mod vk_video;
mod vmix_api;
mod vmix_tcp;
mod vmix_xml;

pub use crate::audio::{AudioBusInfo, AudioDeviceInfo};

pub use abi::{
    AudioPeak, BACKEND_AUTO, BACKEND_DX12, BACKEND_METAL, BACKEND_VULKAN, DURATION_FRAMES,
    DURATION_MS, EASING_IN, EASING_IN_OUT, EASING_LINEAR, EASING_OUT, EASING_SMOOTHSTEP,
    ERR_ALREADY_CREATED, ERR_BUFFER_TOO_SMALL, ERR_DEVICE, ERR_INVALID_ARGUMENT, ERR_IO,
    ERR_NOT_CREATED, GEN_BARS, GEN_SOLID, INCOMING_PREVIEW, INCOMING_PROGRAM, MULTIVIEW_BASE,
    MixerRebarInfo, MixerSourceStatus, MixerStats, MixerVideoInfo, NATIVE_APPKIT_NSVIEW,
    NATIVE_WIN32_HWND, OK, OUT_DECKLINK, OUT_NDI, OUT_OMT, OUTPUT_PREVIEW, OUTPUT_PROGRAM,
    OUTPUT_SOURCE, OverlayDesc, Rect, SAVE_FLAG_MULTIVIEW, SAVE_NOT_ON_PREVIEW_OR_PROGRAM,
    SCENE_BASE, SRC_BARS, SRC_BLACK, SRC_BLUE, SRC_COLOR, SRC_KIND_INPUT, SRC_KIND_MU_MULTIVIEW,
    SRC_KIND_MU_PREVIEW, SRC_KIND_MU_PROGRAM, SRC_KIND_SCENE, SourceUsage, TRANSITION_ADDITIVE,
    TRANSITION_BARN_DOOR, TRANSITION_BLINDS, TRANSITION_BLOOM, TRANSITION_CLOCK,
    TRANSITION_CROSS_ZOOM, TRANSITION_CUBE, TRANSITION_CUBE_ZOOM, TRANSITION_CUSTOM,
    TRANSITION_CUT, TRANSITION_DATAMOSH, TRANSITION_DIAMOND, TRANSITION_DIP, TRANSITION_DIR_DOWN,
    TRANSITION_DIR_LEFT, TRANSITION_DIR_RIGHT, TRANSITION_DIR_UP, TRANSITION_DISPLACE,
    TRANSITION_FADE, TRANSITION_FILM_BURN, TRANSITION_FLIP, TRANSITION_FLY_ROTATE,
    TRANSITION_GLITCH, TRANSITION_GRID_DISSOLVE, TRANSITION_HEART, TRANSITION_IRIS,
    TRANSITION_KALEIDOSCOPE, TRANSITION_LOREZ, TRANSITION_LUMA_MORPH, TRANSITION_METAMIX,
    TRANSITION_MULTITASK, TRANSITION_OPTICAL_FLOW, TRANSITION_PAGE_CURL, TRANSITION_PARTS,
    TRANSITION_PIXEL_SORT, TRANSITION_POLAR, TRANSITION_PUSH, TRANSITION_RIPPLE,
    TRANSITION_ROLLER_DOOR, TRANSITION_SHIFT_RGB, TRANSITION_SLIDE, TRANSITION_STAR,
    TRANSITION_STATIC, TRANSITION_STINGER, TRANSITION_SWIRL, TRANSITION_TILE,
    TRANSITION_VISUAL_DISSOLVE, TRANSITION_WIPE, TRANSITION_ZOOM, TRANSITION_ZOOM_BLUR, UnitSnap,
    UnitState, VideoCaptureInfo, VideoCaptureMode,
};
pub use eiviz_control::{ControlFacade, ControlService, RequestKey};
pub use runtime::ProcessMixer;

pub fn runtime_port() -> ProcessMixer {
    ProcessMixer
}

pub fn control_service() -> &'static std::sync::Mutex<ControlService> {
    runtime::control()
}

pub struct MixerFacade;

impl ControlFacade for MixerFacade {
    fn execute(
        &self,
        key: RequestKey,
        command: eiviz_control::Command,
    ) -> eiviz_control::ControlResult<eiviz_control::CommandOutcome> {
        runtime::control()
            .lock()
            .map_err(|_| eiviz_control::ControlError::internal("control lock"))?
            .execute(key, command)
    }

    fn snapshot(&self) -> eiviz_control::ControlResult<eiviz_control::Snapshot> {
        runtime::control()
            .lock()
            .map_err(|_| eiviz_control::ControlError::internal("control lock"))?
            .snapshot()
    }

    fn events_after(&self, after: u64) -> Vec<eiviz_control::Event> {
        runtime::control()
            .lock()
            .map(|svc| svc.events_after(after))
            .unwrap_or_default()
    }

    fn lifecycle(&self) -> eiviz_control::Lifecycle {
        runtime::control()
            .lock()
            .map(|svc| svc.lifecycle())
            .unwrap_or(eiviz_control::Lifecycle::Failed)
    }

    fn epoch(&self) -> String {
        runtime::control()
            .lock()
            .map(|svc| svc.epoch().to_string())
            .unwrap_or_default()
    }

    fn publish_meters(&self) {
        if let Ok(mut svc) = runtime::control().lock() {
            svc.publish_meters();
        }
    }
}

use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, c_char};
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use lifecycle::{
    MixerSlot, abort_mixer_create, commit_mixer_create, mixer_slot, reserve_mixer_create,
    with_mixer,
};
#[cfg(test)]
use output::{coalesce_latest_video, take_audio_first};
use output::{shutdown_output_worker, spawn_output_worker};
#[cfg(test)]
use render::{collect_frame_live_ids, collect_live_ids};
use render::{render_loop, unit_uses_mix_cycle};

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
use omt::{
    GpuSendStore, OmtGpu, OmtReceiver, ProgramSender, omt_gpu_for_send, omt_gpu_from_device,
};
use present::Presenters;
use readback::ReadbackStore;
use save::{LiveSave, collect_source_roles, want_full};
use upload::{AUDIO_RATE, AudioPacket, CpuFormat, GpuIngest, UploadStore};

pub(crate) struct AutoTransition {
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

pub(crate) struct OverlayAuto {
    desc: OverlayDesc,
    from: f32,
    to: f32,
    start: Instant,
    duration: Duration,
}

pub(crate) struct LiveUnit {
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

pub(crate) struct LiveOutput {
    source_kind: u32,
    source_id: u64,
    unit_id: u64,
    audio_bus_id: u64,
    video_sub: Arc<AtomicBool>,
    use_gpu: bool,
    skip_idle_encode: bool,
    tx: mpsc::Sender<SendCmd>,
}

#[derive(Clone)]
pub(crate) struct OutputSnap {
    output_id: u64,
    source_kind: u32,
    source_id: u64,
    unit_id: u64,
    audio_bus_id: u64,
    fps_n: u32,
    fps_d: u32,
    video_sub: Arc<AtomicBool>,
    use_gpu: bool,
    skip_idle_encode: bool,
    tx: mpsc::Sender<SendCmd>,
}

impl OutputSnap {
    fn visual_key(&self) -> u64 {
        pack_copy_key(self.source_kind, self.source_id, self.unit_id)
    }

    fn wants_video(&self) -> bool {
        !self.skip_idle_encode || self.video_sub.load(Ordering::Relaxed)
    }

    fn cpu_video(&self) -> bool {
        !self.use_gpu && self.wants_video()
    }

    fn gpu_video(&self) -> bool {
        self.use_gpu && self.wants_video()
    }
}

pub(crate) fn pack_copy_key(source_kind: u32, source_id: u64, unit_id: u64) -> u64 {
    match source_kind {
        SRC_KIND_MU_PROGRAM => unit_id,
        SRC_KIND_MU_PREVIEW => 0x0200_0000_0000_0000 | unit_id,
        _ => 0x0100_0000_0000_0000 | source_id,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BusColors {
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

pub(crate) struct SceneSpec {
    width: u32,
    height: u32,
    layers: Arc<[crate::abi::OverlayDesc]>,
    labels: Arc<[String]>,
    mv_label: MvLabelStyle,
}

pub(crate) struct Shared {
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
    #[cfg(any(windows, target_os = "linux"))]
    vulkan_decode: Option<crate::vk_video::VulkanDecode>,
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
    audio_snap: Arc<Mutex<audio::AudioMixSnapshot>>,
    frame_buffer_frames: u32,
    rebar: crate::rebar::RebarSnapshot,
    rebar_optimization: bool,
    ndi_gpu_upload: bool,
    audio: audio::AudioEngine,
}

/// Host-visible status that must not share the control lock with ingest or render.
pub(crate) struct Telemetry {
    last_error: String,
    last_render_ms: f32,
    audio_monitor: audio::AudioMonitor,
    last_ram_bytes: u64,
    last_vram_bytes: u64,
    last_compose_vram: u64,
    last_delay_vram: u64,
    scene_usage: Vec<SourceUsage>,
}

pub(crate) enum LiveReceiver {
    Omt(OmtReceiver),
    /// Held so `Drop` joins the ingest thread. Bandwidth save needs Advanced SDK.
    #[cfg(any(windows, target_os = "macos"))]
    Ndi(#[allow(dead_code)] NdiReceiver),
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

    fn source_status(&self) -> (bool, bool, String) {
        match self {
            Self::Omt(receiver) => (
                receiver.session_live(),
                receiver.has_video(),
                receiver.last_error(),
            ),
            #[cfg(any(windows, target_os = "macos"))]
            Self::Ndi(_) => (true, false, String::new()),
        }
    }
}

pub(crate) enum OutputHandle {
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

    fn pump_accept(&mut self) -> Result<(), String> {
        match self {
            Self::Omt(sender) => sender.pump_accept(),
            #[cfg(any(windows, target_os = "macos"))]
            Self::Ndi(_) => Ok(()),
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
                sender.send_video_uyvy(width, height, stride, pts, data, fps_n, fps_d)
            }
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    fn last_ndi_send_ms(&self) -> Option<(f32, f32)> {
        match self {
            Self::Omt(_) => None,
            Self::Ndi(sender) => Some(sender.last_send_ms()),
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

    fn ndi_audio_first(&self) -> bool {
        match self {
            Self::Omt(_) => false,
            #[cfg(any(windows, target_os = "macos"))]
            Self::Ndi(_) => true,
        }
    }
}

pub(crate) enum SendCmd {
    Video {
        width: u32,
        height: u32,
        stride: u32,
        pts: i64,
        data: Arc<[u8]>,
        fps_n: u32,
        fps_d: u32,
    },
    GpuVideo {
        texture: wgpu::Texture,
        width: u32,
        height: u32,
        pts: i64,
        fps_n: u32,
        fps_d: u32,
        busy: Arc<AtomicBool>,
    },
    Audio {
        packet: AudioPacket,
    },
    Shutdown,
}

pub(crate) struct GpuEncodeCopy {
    tx: mpsc::Sender<SendCmd>,
    texture: wgpu::Texture,
    width: u32,
    height: u32,
    busy: Arc<AtomicBool>,
    fps_n: u32,
    fps_d: u32,
}

pub(crate) struct OutputWorker {
    tx: mpsc::Sender<SendCmd>,
    join: JoinHandle<()>,
}

pub(crate) enum GpuCmd {
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
        reply: mpsc::Sender<i32>,
    },
    DetachUnit {
        unit_id: u64,
        reply: mpsc::Sender<i32>,
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
    Snapshot {
        unit_id: u64,
        kind: u32,
        path: String,
        reply: mpsc::Sender<i32>,
    },
    Shutdown,
}

pub(crate) struct Mixer {
    shared: Arc<Mutex<Shared>>,
    uploads: Arc<Mutex<UploadStore>>,
    telemetry: Arc<Mutex<Telemetry>>,
    cmds: mpsc::Sender<GpuCmd>,
    send_workers: HashMap<u64, OutputWorker>,
    omt_gpu: OmtGpu,
    thumb_pixels: Arc<Mutex<HashMap<u64, crate::thumb::ThumbPixels>>>,
    render: Option<JoinHandle<()>>,
    audio_sched: Option<audio::AudioScheduler>,
    stop: Arc<AtomicBool>,
    backend: u32,
    #[cfg(target_os = "macos")]
    surface_gpu: present::SurfaceGpu,
}

pub(crate) fn live_snapshot() -> crate::vmix_xml::LiveSnapshot {
    with_mixer(|mixer| {
        let shared = mixer.shared.lock().expect("shared");
        let mut snap = crate::vmix_xml::LiveSnapshot::default();
        for (id, unit) in &shared.units {
            snap.units.insert(*id, live_unit_from(unit));
        }
        snap
    })
    .unwrap_or_default()
}

pub(crate) fn live_unit(unit_id: u64) -> Option<crate::vmix_xml::UnitLive> {
    with_mixer(|mixer| {
        let shared = mixer.shared.lock().expect("shared");
        shared.units.get(&unit_id).map(live_unit_from)
    })
    .ok()
    .flatten()
}

pub(crate) fn live_unit_from(unit: &LiveUnit) -> crate::vmix_xml::UnitLive {
    crate::vmix_xml::UnitLive {
        program_source: unit.state.program_source,
        preview_source: unit.state.preview_source,
        overlay_sources: unit
            .state
            .overlays
            .iter()
            .take(unit.state.overlay_count as usize)
            .map(|overlay| overlay.source_id)
            .collect(),
    }
}

pub(crate) fn stamp_live_scene_buses(doc: &mut eiviz_control::session::Document) {
    let snap = live_snapshot();
    let mut live = eiviz_control::LiveState::default();
    for (id, unit) in snap.units {
        live.units.insert(
            id,
            eiviz_control::UnitLiveState {
                program_source: unit.program_source,
                preview_source: unit.preview_source,
                ..Default::default()
            },
        );
    }
    eiviz_control::session::stamp_live_scene_buses(doc, &live);
}

pub(crate) fn remember_session_path(path: impl Into<std::path::PathBuf>) {
    if let Ok(mut svc) = crate::control_service().lock() {
        svc.set_session_path(Some(path.into()));
    }
}

pub(crate) fn session_revision() -> u64 {
    crate::control_service()
        .lock()
        .map(|svc| svc.revision())
        .unwrap_or(0)
}

pub(crate) fn report_io(error: impl Into<String>) -> i32 {
    let error = error.into();
    crate::diag::error(&error);
    let _ = with_mixer(|mixer| set_error(&mixer.telemetry, error));
    ERR_IO
}

pub(crate) fn insert_receiver(id: u64, receiver: LiveReceiver) -> i32 {
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
pub(crate) fn insert_video(id: u64, pump: VideoPump) -> i32 {
    with_mixer(|mixer| {
        mixer.shared.lock().expect("shared").videos.insert(id, pump);
        OK
    })
    .unwrap_or_else(|code| code)
}

pub(crate) struct DetachedSource {
    receiver: Option<LiveReceiver>,
    #[cfg(any(windows, target_os = "macos"))]
    video: Option<VideoPump>,
    uploads: Arc<Mutex<UploadStore>>,
}

pub(crate) fn detach_source(id: u64) -> Result<DetachedSource, i32> {
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
pub(crate) fn take_source_uploads(id: u64) -> Result<Arc<Mutex<UploadStore>>, i32> {
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
pub(crate) fn send_gpu_and_wait(send: impl FnOnce(&mut Mixer, mpsc::Sender<i32>) -> i32) -> i32 {
    send_gpu_and_wait_timeout(send, Duration::from_secs(30))
}

pub(crate) fn send_gpu_and_wait_timeout(
    send: impl FnOnce(&mut Mixer, mpsc::Sender<i32>) -> i32,
    timeout: Duration,
) -> i32 {
    let (reply_tx, reply_rx) = mpsc::channel();
    match with_mixer(|mixer| send(mixer, reply_tx)) {
        Ok(OK) => match reply_rx.recv_timeout(timeout) {
            Ok(code) => code,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                crate::diag::error("gpu command timed out");
                report_session_error("gpu command timed out");
                ERR_DEVICE
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => ERR_DEVICE,
        },
        Ok(code) => code,
        Err(code) => code,
    }
}

pub(crate) fn set_error(telemetry: &Mutex<Telemetry>, message: impl Into<String>) {
    telemetry.lock().expect("telemetry").last_error = message.into();
}

pub(crate) fn with_uploads<T>(mixer: &Mixer, f: impl FnOnce(&mut UploadStore) -> T) -> T {
    f(&mut mixer.uploads.lock().expect("uploads"))
}

pub(crate) fn session_error_slot() -> &'static Mutex<String> {
    static SLOT: OnceLock<Mutex<String>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(String::new()))
}

pub(crate) fn report_session_error(message: impl Into<String>) {
    let message = message.into();
    *session_error_slot().lock().expect("session error") = message.clone();
    let _ = with_mixer(|mixer| set_error(&mixer.telemetry, message));
}

pub(crate) fn copy_bytes(src: &[u8], out: *mut u8, cap: usize) -> i32 {
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
pub(crate) fn prepare_surface_off_slot(
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

/// Creates the OS-default wgpu device (DX12 on Windows, Metal on macOS, Vulkan on Linux).
#[unsafe(no_mangle)]
pub extern "C" fn mixer_create(adapter_luid: u64, fps_num: u32, fps_den: u32) -> i32 {
    mixer_create_with_backend(crate::abi::BACKEND_AUTO, adapter_luid, fps_num, fps_den)
}

/// Creates a mixer with an explicit GPU backend (`0=auto`, `1=dx12`, `2=vulkan`, `3=metal`).
#[unsafe(no_mangle)]
pub extern "C" fn mixer_create_with_backend(
    backend: u32,
    _adapter_luid: u64,
    fps_num: u32,
    fps_den: u32,
) -> i32 {
    crate::diag::init();
    crate::diag::info(&format!("mixer_create backend={backend}"));
    let _ = crate::diag::profile_send();
    if fps_num == 0 || fps_den == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    let Some(request) = crate::device::BackendRequest::from_abi(backend) else {
        report_session_error(format!("unknown GPU backend {backend}"));
        return ERR_INVALID_ARGUMENT;
    };
    if reserve_mixer_create() != OK {
        return ERR_ALREADY_CREATED;
    }
    crate::diag::reset_generation();
    reset_frame_caches();
    match start_mixer(request, fps_num, fps_den) {
        Ok(mixer) => {
            let code = commit_mixer_create(mixer);
            if code != OK {
                abort_mixer_create();
                return code;
            }
            #[cfg(any(windows, target_os = "macos"))]
            let _ = thread::Builder::new()
                .name("eiviz-ndi-find".into())
                .spawn(ndi::warm_finder);
            OK
        }
        Err(code) => {
            abort_mixer_create();
            code
        }
    }
}

pub(crate) fn start_mixer(
    request: crate::device::BackendRequest,
    fps_num: u32,
    fps_den: u32,
) -> Result<Mixer, i32> {
    let device = match GpuDevice::with_backend(request) {
        Ok(device) => device,
        Err(error) => {
            let message = format!("gpu device: {error}");
            crate::diag::error(&message);
            report_session_error(message);
            return Err(ERR_DEVICE);
        }
    };
    let backend = crate::device::abi_of_backend(device.adapter.get_info().backend);
    #[cfg(target_os = "macos")]
    let surface_gpu = present::SurfaceGpu {
        instance: device.instance.clone(),
        adapter: device.adapter.clone(),
        device: device.device.clone(),
    };
    #[cfg(windows)]
    let gpu_video = if device.adapter.get_info().backend == wgpu::Backend::Dx12 {
        match GpuVideoContext::new(&device) {
            Ok(ctx) => Some(ctx),
            Err(error) => {
                eprintln!("eiviz dxgi video: {error}");
                report_session_error(format!("dxgi video: {error}"));
                return Err(ERR_DEVICE);
            }
        }
    } else {
        None
    };
    #[cfg(any(windows, target_os = "linux"))]
    let vulkan_decode = device.vulkan.clone();
    let omt_recv_gpu = omt_gpu_from_device(&device);
    let omt_send_gpu = omt_gpu_for_send(&device);
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
        audio_monitor: audio::AudioMonitor::default(),
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
        #[cfg(any(windows, target_os = "linux"))]
        vulkan_decode,
        omt_gpu: omt_recv_gpu,
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
        audio_snap: Arc::new(Mutex::new(audio::AudioMixSnapshot::default())),
    }));
    let thumb_pixels = Arc::new(Mutex::new(HashMap::new()));
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let render_shared = Arc::clone(&shared);
    let render_uploads = Arc::clone(&uploads);
    let render_telemetry = Arc::clone(&telemetry);
    let render_stop = Arc::clone(&stop);
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
                render_stop,
            );
        })
        .expect("render thread");
    let audio_snap = Arc::clone(&shared.lock().expect("shared").audio_snap);
    let monitor = telemetry.lock().expect("telemetry").audio_monitor.clone();
    let audio_inputs = uploads.lock().expect("uploads").audio_store();
    let audio_sched = Some(audio::AudioScheduler::start(
        audio,
        audio_inputs,
        audio_snap,
        monitor.pcm,
        monitor.primed,
    ));
    Ok(Mixer {
        shared,
        uploads,
        telemetry,
        cmds: tx,
        send_workers: HashMap::new(),
        omt_gpu: omt_send_gpu,
        thumb_pixels,
        render: Some(render),
        audio_sched,
        stop,
        backend,
        #[cfg(target_os = "macos")]
        surface_gpu,
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_backend() -> u32 {
    with_mixer(|mixer| mixer.backend).unwrap_or(crate::abi::BACKEND_AUTO)
}

pub(crate) fn mixer_created() -> bool {
    mixer_slot()
        .lock()
        .map(|slot| matches!(*slot, MixerSlot::Running(_)))
        .unwrap_or(false)
}

pub(crate) fn all_live_state() -> eiviz_control::live::LiveState {
    use eiviz_control::live::{LivePeak, LiveState, UnitLiveState};
    with_mixer(|mixer| {
        let (units, master, buses, mix_peaks) = {
            let shared = mixer.shared.lock().expect("shared");
            let mut units = HashMap::new();
            for (id, unit) in &shared.units {
                units.insert(
                    *id,
                    UnitLiveState {
                        program_source: unit.state.program_source,
                        preview_source: unit.state.preview_source,
                        mix: unit.state.mix,
                        transitioning: unit.auto.is_some() || unit.state.mix > 0.001,
                        incoming_source: unit.state.incoming_source,
                        overlay_sources: unit
                            .state
                            .overlays
                            .iter()
                            .take(unit.state.overlay_count as usize)
                            .map(|overlay| overlay.source_id)
                            .collect(),
                    },
                );
            }
            (
                units,
                shared.audio.master_peak(),
                shared.audio.bus_peaks(),
                shared.audio.mix_input_peaks(),
            )
        };
        let audio_in = mixer.uploads.lock().expect("uploads").audio_store();
        let audio_in = audio_in.lock().expect("audio");
        let mut peaks = vec![LivePeak {
            id: 0,
            left: master.0,
            right: master.1,
        }];
        for (id, left, right) in buses {
            peaks.push(LivePeak {
                id: crate::abi::AUDIO_BUS_PEAK_BASE | id,
                left,
                right,
            });
        }
        for id in audio_in.ids() {
            if mix_peaks.iter().any(|(mix_id, ..)| *mix_id == id) {
                continue;
            }
            let Some((left, right)) = audio_in.peak(id) else {
                continue;
            };
            peaks.push(LivePeak { id, left, right });
        }
        for (id, left, right) in mix_peaks {
            peaks.push(LivePeak { id, left, right });
        }
        LiveState { units, peaks }
    })
    .unwrap_or_default()
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_destroy() {
    if let Ok(mut svc) = crate::runtime::control().try_lock() {
        svc.abandon();
    }
    mixer_destroy_inner();
}

pub(crate) fn mixer_destroy_inner() {
    crate::vmix_api::suspend();
    crate::diag::info("mixer_destroy begin");
    let mut mixer = {
        let mut slot = mixer_slot().lock().expect("mixer mutex poisoned");
        match std::mem::replace(&mut *slot, MixerSlot::Stopping) {
            MixerSlot::Running(mixer) => mixer,
            previous => {
                *slot = match previous {
                    MixerSlot::Initializing => MixerSlot::Empty,
                    other => other,
                };
                return;
            }
        }
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
    crate::diag::info("mixer_destroy audio");
    if let Some(mut sched) = mixer.audio_sched.take() {
        sched.stop();
    }
    audio.shutdown();
    crate::diag::info("mixer_destroy drop receivers");
    drop(receivers);
    drop(videos);
    let _ = mixer.cmds.send(GpuCmd::Shutdown);
    if let Some(join) = mixer.render.take() {
        if !crate::diag::join_timeout(join, Duration::from_secs(2), "render") {
            crate::diag::warn("render still running after join timeout");
        }
    }
    for worker in std::mem::take(&mut mixer.send_workers).into_values() {
        shutdown_output_worker(worker);
    }
    crate::diag::info("mixer_destroy drop");
    drop(mixer);
    *mixer_slot().lock().expect("mixer mutex poisoned") = MixerSlot::Empty;
    crate::diag::reset_generation();
    reset_frame_caches();
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
    send_gpu_and_wait_timeout(
        |mixer, reply| {
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
                if let Some(worker) = mixer.send_workers.remove(&output_id) {
                    shutdown_output_worker(worker);
                }
            }
            if mixer
                .cmds
                .send(GpuCmd::DetachUnit { unit_id, reply })
                .is_err()
            {
                return ERR_DEVICE;
            }
            OK
        },
        Duration::from_secs(2),
    )
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
        return attach_invalid(format!(
            "attach surface unit={unit_id} kind={kind}: width/height must be non-zero ({width}x{height})"
        ));
    }
    let Ok(surface) = NativeSurface::parse(native_kind, handle) else {
        return attach_invalid(format!(
            "attach surface unit={unit_id} kind={kind}: native kind={native_kind} handle={handle} is not valid on this OS"
        ));
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
            set_error(
                &mixer.telemetry,
                format!("attach surface: mixing unit {unit_id:#x} is not created"),
            );
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

pub(crate) fn attach_invalid(message: impl Into<String>) -> i32 {
    report_session_error(message);
    ERR_INVALID_ARGUMENT
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_unit_set_state(unit_id: u64, state: *const UnitState) -> i32 {
    if state.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let state = unsafe { *state };
    let code = unit_set_state_inner(unit_id, &state);
    if code == OK {
        crate::runtime::note_live("SetUnitState");
    }
    code
}

pub(crate) fn unit_set_state_inner(unit_id: u64, state: &UnitState) -> i32 {
    if state.overlay_count > state.overlays.len() as u32
        || state.mv_slot_count > state.mv_slots.len() as u32
        || !(0.0..=1.0).contains(&state.mix)
    {
        return ERR_INVALID_ARGUMENT;
    }
    let state = *state;
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

pub(crate) fn ease_mix(t: f32, kind: u32) -> f32 {
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

pub(crate) fn merge_overlay(state: &mut UnitState, desc: OverlayDesc) {
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

pub(crate) fn tick_unit_transitions(unit: &mut LiveUnit) {
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

pub(crate) fn live_incoming(unit: &LiveUnit) -> u64 {
    if let Some(auto) = &unit.auto
        && (auto.keep_preview || auto.incoming_locked)
    {
        return auto.frozen_preview;
    }
    unit.frozen_preview.unwrap_or(unit.state.preview_source)
}

pub(crate) fn resolve_incoming(requested: u64, preview: u64, program: u64) -> u64 {
    if requested == INCOMING_PREVIEW {
        preview
    } else if requested == INCOMING_PROGRAM {
        program
    } else {
        requested
    }
}

pub(crate) fn snapshot_mix_preview(unit: &LiveUnit) -> u64 {
    let incoming = live_incoming(unit);
    if incoming == unit.state.preview_source {
        0
    } else {
        incoming
    }
}

pub(crate) fn take_cut(unit: &mut LiveUnit, swap: bool) {
    take_cut_to(unit, swap, live_incoming(unit));
}

pub(crate) fn take_cut_to(unit: &mut LiveUnit, swap: bool, incoming: u64) {
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

/// GPU CUT. ControlService calls this; the C ABI entry goes through ControlService.
pub(crate) fn unit_cut_inner(unit_id: u64, swap: u32, incoming_source: u64) -> i32 {
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
pub extern "C" fn mixer_unit_cut(unit_id: u64, swap: u32, incoming_source: u64) -> i32 {
    crate::runtime::c_cut(unit_id, swap, incoming_source)
}

/// GPU AUTO. ControlService calls this; the C ABI entry goes through ControlService.
pub(crate) fn unit_auto_inner(
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
    crate::runtime::c_auto(
        unit_id,
        kind,
        duration_ms,
        swap,
        keep_preview,
        easing,
        direction,
        dip_r,
        dip_g,
        dip_b,
        dip_a,
        incoming_source,
        softness,
        param,
    )
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
    let code = overlay_auto_inner(unit_id, target_enabled, duration_ms, desc);
    if code == OK {
        crate::runtime::note_live("OverlayAuto");
    }
    code
}

pub(crate) fn overlay_auto_inner(
    unit_id: u64,
    target_enabled: u32,
    duration_ms: u32,
    desc: OverlayDesc,
) -> i32 {
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
    send_gpu_and_wait_timeout(
        |mixer, reply| {
            if mixer
                .cmds
                .send(GpuCmd::Detach {
                    unit_id,
                    kind,
                    surface,
                    reply,
                })
                .is_err()
            {
                return ERR_DEVICE;
            }
            OK
        },
        Duration::from_secs(2),
    )
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
        let audio = mixer.uploads.lock().expect("uploads").audio_store();
        audio.lock().expect("audio").ingest_audio(
            id,
            crate::upload::AudioPacket {
                timestamp: pts,
                sample_rate,
                channels,
                samples_per_channel: frames as i32,
                pcm_planar_f32: samples.to_vec(),
            },
        );
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

pub(crate) fn take_snapshot(unit_id: u64, kind: u32, path: &str) -> i32 {
    let path = path.to_string();
    send_gpu_and_wait(|mixer, reply| {
        if mixer
            .cmds
            .send(GpuCmd::Snapshot {
                unit_id,
                kind,
                path,
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
pub unsafe extern "C" fn mixer_snapshot(unit_id: u64, kind: u32, path: *const c_char) -> i32 {
    if path.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    // SAFETY: path is a NUL-terminated UTF-8 C string.
    let path = unsafe { CStr::from_ptr(path) }.to_str().unwrap_or_default();
    if path.is_empty() {
        return ERR_INVALID_ARGUMENT;
    }
    take_snapshot(unit_id, kind, path)
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
        let (uploads, gpu, ingest, vulkan, depth, previous_video, previous_recv) =
            match with_mixer(|mixer| {
                let mut shared = mixer.shared.lock().expect("shared");
                let previous_video = shared.videos.remove(&id);
                let previous_recv = shared.receivers.remove(&id);
                let uploads = shared.uploads.clone();
                let gpu = shared.gpu_video.clone();
                let ingest = shared.gpu_ingest.clone();
                let vulkan = shared.vulkan_decode.clone();
                let session = shared.frame_buffer_frames.clamp(1, 8);
                let depth = if frame_buffer_frames == 0 {
                    session
                } else {
                    frame_buffer_frames.clamp(1, 8)
                };
                (
                    uploads,
                    gpu,
                    ingest,
                    vulkan,
                    depth,
                    previous_video,
                    previous_recv,
                )
            }) {
                Ok(value) => value,
                Err(code) => return code,
            };
        drop(previous_recv);
        drop(previous_video);
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
            ingest,
            vulkan,
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
    crate::diag::info(&format!(
        "omt_connect id={id} gpu={} addr={address}",
        use_gpu != 0
    ));
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
    unsafe {
        mixer_output_add(
            unit_id,
            OUT_OMT,
            name,
            SRC_KIND_MU_PROGRAM,
            0,
            unit_id,
            0,
            0,
            1,
        )
    }
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
    audio_bus_id: u64,
    skip_idle_encode: u32,
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
    let skip_idle_encode = transport == OUT_OMT && skip_idle_encode != 0;
    let source_id = crate::abi::resolve_output_source_id(source_kind, source_id);
    let audio_bus_id = if source_kind == SRC_KIND_MU_MULTIVIEW {
        0
    } else {
        audio_bus_id
    };
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
                Ok(Ok(mut sender)) => {
                    // Render posts video then PCM. Immediate audio follows that
                    // video; holding until the next video slipped A/V by a frame.
                    sender.set_pair_after_video(false);
                    OutputHandle::Omt(sender)
                }
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
        mixer
            .shared
            .lock()
            .expect("shared")
            .outputs
            .remove(&output_id);
        if let Some(old) = mixer.send_workers.remove(&output_id) {
            shutdown_output_worker(old);
        }
        let video_sub = Arc::new(AtomicBool::new(false));
        let worker = spawn_output_worker(
            output_id,
            handle,
            Arc::clone(&video_sub),
            mixer.omt_gpu.clone(),
            Arc::clone(&mixer.stop),
        );
        mixer.shared.lock().expect("shared").outputs.insert(
            output_id,
            LiveOutput {
                source_kind,
                source_id,
                unit_id,
                audio_bus_id,
                video_sub,
                use_gpu,
                skip_idle_encode,
                tx: worker.tx.clone(),
            },
        );
        mixer.send_workers.insert(output_id, worker);
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
        if let Some(worker) = mixer.send_workers.remove(&output_id) {
            shutdown_output_worker(worker);
        }
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
pub unsafe extern "C" fn mixer_session_has_assets(path: *const c_char) -> i32 {
    if path.is_null() {
        return -ERR_INVALID_ARGUMENT;
    }
    match session::has_embedded_assets(&read_cstr(path)) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(error) => {
            report_session_error(error);
            -ERR_INVALID_ARGUMENT
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_session_import(
    export_path: *const c_char,
    session_dest: *const c_char,
    media_dir: *const c_char,
    out: *mut u8,
    cap: usize,
) -> i32 {
    if export_path.is_null()
        || session_dest.is_null()
        || media_dir.is_null()
        || out.is_null()
        || cap == 0
    {
        return -ERR_INVALID_ARGUMENT;
    }
    let export_path = read_cstr(export_path);
    let session_dest = read_cstr(session_dest);
    let media_dir = read_cstr(media_dir);
    if session_dest.is_empty() || media_dir.is_empty() {
        report_session_error("import needs a session path and a media directory");
        return -ERR_INVALID_ARGUMENT;
    }
    match session::import_exported_session(&export_path, &session_dest, &media_dir)
        .and_then(|document| session::to_vec(&document))
    {
        Ok(canonical) => {
            remember_session_path(&session_dest);
            copy_bytes(&canonical, out, cap)
        }
        Err(error) => {
            report_session_error(error);
            -ERR_INVALID_ARGUMENT
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_session_current_path(out: *mut u8, cap: usize) -> i32 {
    if out.is_null() || cap == 0 {
        return -ERR_INVALID_ARGUMENT;
    }
    let path = crate::control_service()
        .lock()
        .ok()
        .and_then(|svc| {
            svc.session_path()
                .map(|path| path.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    copy_bytes(path.as_bytes(), out, cap)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_session_load(path: *const c_char, out: *mut u8, cap: usize) -> i32 {
    if path.is_null() || out.is_null() || cap == 0 {
        return -ERR_INVALID_ARGUMENT;
    }
    let path = read_cstr(path);
    if let Err(error) = std::fs::read(&path) {
        report_session_error(error.to_string());
        return -ERR_IO;
    }
    match session::read_document(&path).and_then(|document| session::to_vec(&document)) {
        Ok(canonical) => {
            remember_session_path(&path);
            copy_bytes(&canonical, out, cap)
        }
        Err(error) => {
            report_session_error(error);
            -ERR_INVALID_ARGUMENT
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
    let path = read_cstr(path);
    let bytes = unsafe { std::slice::from_raw_parts(json, len) };
    let revision = session_revision();
    match session::parse(bytes).and_then(|mut document| {
        stamp_live_scene_buses(&mut document);
        session::write_document_rev(&path, &document, revision)
    }) {
        Ok(_) => {
            remember_session_path(&path);
            OK
        }
        Err(error) => {
            report_session_error(error);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_session_export(
    path: *const c_char,
    json: *const u8,
    len: usize,
) -> i32 {
    if path.is_null() || json.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let bytes = unsafe { std::slice::from_raw_parts(json, len) };
    match session::parse(bytes).and_then(|mut document| {
        stamp_live_scene_buses(&mut document);
        session::export_document(&read_cstr(path), &document)
    }) {
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
pub extern "C" fn mixer_session_clear_current() -> i32 {
    if let Ok(mut svc) = crate::control_service().lock() {
        svc.set_session_path(None);
    }
    OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_session_history(
    path: *const c_char,
    out: *mut u8,
    cap: usize,
) -> i32 {
    if path.is_null() || out.is_null() || cap == 0 {
        return -ERR_INVALID_ARGUMENT;
    }
    let path = read_cstr(path);
    match session::read_history(&path)
        .and_then(|history| serde_json::to_vec(&history).map_err(|error| error.to_string()))
    {
        Ok(json) => copy_bytes(&json, out, cap),
        Err(error) => {
            report_session_error(error);
            -ERR_INVALID_ARGUMENT
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_session_load_rev(
    path: *const c_char,
    index: u32,
    out: *mut u8,
    cap: usize,
) -> i32 {
    if path.is_null() || out.is_null() || cap == 0 {
        return -ERR_INVALID_ARGUMENT;
    }
    let path = read_cstr(path);
    match session::extract_history(&path, index).and_then(|document| session::to_vec(&document)) {
        Ok(canonical) => {
            remember_session_path(&path);
            copy_bytes(&canonical, out, cap)
        }
        Err(error) => {
            report_session_error(error);
            -ERR_INVALID_ARGUMENT
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_session_publish(json: *const u8, len: usize) -> i32 {
    unsafe { crate::vmix_api::publish_c(json, len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_session_replace(
    json: *const u8,
    len: usize,
    expected_revision: u64,
) -> i32 {
    if json.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let bytes = unsafe { std::slice::from_raw_parts(json, len) };
    crate::runtime::replace_session_bytes(bytes, expected_revision)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_poll_events(after: u64, out: *mut u8, cap: usize) -> i32 {
    if out.is_null() || cap == 0 {
        return -ERR_INVALID_ARGUMENT;
    }
    let buf = unsafe { std::slice::from_raw_parts_mut(out, cap) };
    crate::runtime::poll_event(after, buf)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_copy_snapshot(out: *mut u8, cap: usize) -> i32 {
    if out.is_null() || cap == 0 {
        return -ERR_INVALID_ARGUMENT;
    }
    let buf = unsafe { std::slice::from_raw_parts_mut(out, cap) };
    crate::runtime::copy_snapshot_bytes(buf)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_api_configure(
    enabled: u32,
    port: u32,
    user: *const c_char,
    pass: *const c_char,
) -> i32 {
    unsafe { crate::vmix_api::configure_c(enabled, port, user, pass) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_api_listen_owner(out: *mut u8, cap: usize) -> i32 {
    unsafe { crate::vmix_api::listen_owner_c(out, cap) }
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_tcp_configure(enabled: u32) -> i32 {
    crate::vmix_tcp::configure(enabled != 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_tcp_listen_owner(out: *mut u8, cap: usize) -> i32 {
    unsafe { crate::vmix_tcp::listen_owner_c(out, cap) }
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_ws_configure(enabled: u32, port: u32) -> i32 {
    crate::native_ws::configure(enabled != 0, port)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_ws_configure_bind(
    enabled: u32,
    host: *const c_char,
    port: u32,
) -> i32 {
    let host = if host.is_null() {
        "127.0.0.1"
    } else {
        unsafe { CStr::from_ptr(host) }
            .to_str()
            .unwrap_or("127.0.0.1")
    };
    crate::native_ws::configure_bind(enabled != 0, host, port)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_ws_configure_owned(
    enabled: u32,
    host: *const c_char,
    port: u32,
    token: *const c_char,
    max_role: *const c_char,
    media_directory: *const c_char,
) -> i32 {
    let host = if host.is_null() {
        "127.0.0.1"
    } else {
        unsafe { CStr::from_ptr(host) }
            .to_str()
            .unwrap_or("127.0.0.1")
    };
    let token = if token.is_null() {
        ""
    } else {
        unsafe { CStr::from_ptr(token) }.to_str().unwrap_or("")
    };
    let max_role = if max_role.is_null() {
        ""
    } else {
        unsafe { CStr::from_ptr(max_role) }.to_str().unwrap_or("")
    };
    let media = if media_directory.is_null() {
        ""
    } else {
        unsafe { CStr::from_ptr(media_directory) }
            .to_str()
            .unwrap_or("")
    };
    crate::native_ws::configure_owned(enabled != 0, host, port, token, max_role, media)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_ws_listen_owner(out: *mut u8, cap: usize) -> i32 {
    unsafe { crate::native_ws::listen_owner_c(out, cap) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_source_status(id: u64, out: *mut MixerSourceStatus) -> i32 {
    if out.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    with_mixer(|mixer| {
        let (connected, has_video, uploads) = {
            let shared = mixer.shared.lock().expect("shared");
            let (connected, has_video) = shared
                .receivers
                .get(&id)
                .map(LiveReceiver::source_status)
                .map(|(connected, has_video, _)| (connected, has_video))
                .unwrap_or((false, false));
            (connected, has_video, Arc::clone(&shared.uploads))
        };
        let store = uploads.lock().expect("uploads");
        unsafe {
            *out = MixerSourceStatus {
                connected: u32::from(connected),
                has_video: u32::from(has_video || store.has_video_frame(id)),
            };
        }
        OK
    })
    .unwrap_or_else(|code| code)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_source_copy_error(id: u64, out: *mut u8, cap: usize) -> i32 {
    if out.is_null() || cap == 0 {
        return ERR_INVALID_ARGUMENT;
    }
    let error = with_mixer(|mixer| {
        mixer
            .shared
            .lock()
            .expect("shared")
            .receivers
            .get(&id)
            .map(LiveReceiver::source_status)
            .map(|(_, _, error)| error)
            .unwrap_or_default()
    })
    .unwrap_or_default();
    let n = error.len().min(cap);
    if n > 0 {
        unsafe { std::ptr::copy_nonoverlapping(error.as_ptr(), out, n) };
    }
    n as i32
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
        let audio = mixer.uploads.lock().expect("uploads").audio_store();
        audio.lock().expect("audio").flush_audio(id);
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

pub(crate) fn read_cstr(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .unwrap_or_default()
        .to_string()
}

pub(crate) fn write_fixed<const N: usize>(text: &str) -> [u8; N] {
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
        let monitor = mixer
            .telemetry
            .lock()
            .expect("telemetry")
            .audio_monitor
            .clone();
        if !monitor.primed.load(Ordering::Relaxed) {
            dest.fill(0.0);
            return 0;
        }
        let mut pcm = monitor.pcm.lock().expect("monitor pcm");
        let n = dest.len().min(pcm.len());
        for slot in dest.iter_mut().take(n) {
            *slot = pcm.pop_front().unwrap_or(0.0);
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
        let audio_in = uploads.lock().expect("uploads").audio_store();
        let audio_in = audio_in.lock().expect("audio");
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
        for id in audio_in.ids() {
            if n >= cap {
                break;
            }
            if mix_peaks.iter().any(|(mix_id, ..)| *mix_id == id) {
                continue;
            }
            let Some((left, right)) = audio_in.peak(id) else {
                continue;
            };
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

pub(crate) fn copy_c_label(ptr: *const c_char) -> String {
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
pub(crate) struct Acquired {
    data: Arc<[u8]>,
    stride: u32,
    pts: i64,
}

static LAST_FRAME: OnceLock<Mutex<HashMap<u64, Acquired>>> = OnceLock::new();
static ACQUIRED: OnceLock<Mutex<HashMap<u64, Acquired>>> = OnceLock::new();

pub(crate) fn last_frames() -> &'static Mutex<HashMap<u64, Acquired>> {
    LAST_FRAME.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn acquired() -> &'static Mutex<HashMap<u64, Acquired>> {
    ACQUIRED.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn reset_frame_caches() {
    match last_frames().try_lock() {
        Ok(mut slot) => slot.clear(),
        Err(_) => crate::diag::warn("last_frames lock busy; skip reset"),
    }
    match acquired().try_lock() {
        Ok(mut slot) => slot.clear(),
        Err(_) => crate::diag::warn("acquired lock busy; skip reset"),
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
    fn skip_idle_encode_gates_video() {
        let (tx, _) = mpsc::channel();
        let idle = OutputSnap {
            output_id: 1,
            source_kind: SRC_KIND_MU_PROGRAM,
            source_id: 1,
            unit_id: 1,
            audio_bus_id: 1,
            fps_n: 60,
            fps_d: 1,
            video_sub: Arc::new(AtomicBool::new(false)),
            use_gpu: false,
            skip_idle_encode: true,
            tx: tx.clone(),
        };
        assert!(!idle.wants_video());
        assert!(!idle.cpu_video());
        let always = OutputSnap {
            skip_idle_encode: false,
            ..idle
        };
        assert!(always.wants_video());
        assert!(always.cpu_video());
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
    fn collect_live_ids_uploads_remote_omt_monitor() {
        let remote_prv = 0x0005_0001;
        assert!(!crate::abi::is_scene(remote_prv));
        let (_, uploads) = collect_live_ids(&[], &[], &[remote_prv], &[], &HashMap::new());
        assert!(uploads.contains(&remote_prv));
    }

    #[test]
    fn collect_live_ids_keeps_multiview_output() {
        let mv = MULTIVIEW_BASE | 1;
        let layers: std::sync::Arc<[crate::abi::OverlayDesc]> =
            std::sync::Arc::from([crate::abi::OverlayDesc {
                source_id: SRC_COLOR,
                ..crate::abi::OverlayDesc::default()
            }]);
        let specs = [(mv, 1920, 1080, layers, MvLabelStyle::default())];
        let outputs = [OutputSnap {
            output_id: 1,
            source_kind: SRC_KIND_MU_MULTIVIEW,
            source_id: mv,
            unit_id: 1,
            audio_bus_id: 1,
            fps_n: 60,
            fps_d: 1,
            video_sub: Arc::new(AtomicBool::new(true)),
            use_gpu: false,
            skip_idle_encode: true,
            tx: mpsc::channel().0,
        }];
        let (scenes, uploads) = collect_live_ids(&specs, &[], &[], &outputs, &HashMap::new());
        assert!(scenes.contains(&mv));
        assert!(uploads.contains(&SRC_COLOR));
    }

    #[test]
    fn mix_source_cycles_self_but_not_mutual() {
        let mix_a = crate::abi::MixInputSpec::new(1, SRC_KIND_MU_PROGRAM, 1, 0).unwrap();
        let mix_b = crate::abi::MixInputSpec::new(2, SRC_KIND_MU_PROGRAM, 1, 0).unwrap();
        let mix_inputs = HashMap::from([(20, mix_a), (21, mix_b)]);
        let scene_a = SceneSpec {
            width: 320,
            height: 180,
            layers: std::sync::Arc::from([crate::abi::OverlayDesc {
                source_id: 20,
                ..crate::abi::OverlayDesc::default()
            }]),
            labels: std::sync::Arc::from([]),
            mv_label: MvLabelStyle::default(),
        };
        let scene_b = SceneSpec {
            width: 320,
            height: 180,
            layers: std::sync::Arc::from([crate::abi::OverlayDesc {
                source_id: 21,
                ..crate::abi::OverlayDesc::default()
            }]),
            labels: std::sync::Arc::from([]),
            mv_label: MvLabelStyle::default(),
        };
        let scenes = HashMap::from([(SCENE_BASE | 2, scene_a), (SCENE_BASE | 3, scene_b)]);

        let self_on_a = UnitState {
            program_source: SCENE_BASE | 2,
            preview_source: SRC_BARS,
            ..UnitState::default()
        };
        assert!(unit_uses_mix_cycle(1, &self_on_a, &mix_inputs, &scenes));

        let nest_b = UnitState {
            program_source: SCENE_BASE | 2,
            preview_source: SRC_BARS,
            ..UnitState::default()
        };
        assert!(!unit_uses_mix_cycle(2, &nest_b, &mix_inputs, &scenes));

        let mutual_a = UnitState {
            program_source: SCENE_BASE | 3,
            preview_source: SRC_COLOR,
            ..UnitState::default()
        };
        let mutual_b = UnitState {
            program_source: SCENE_BASE | 2,
            preview_source: SRC_BLUE,
            ..UnitState::default()
        };
        assert!(!unit_uses_mix_cycle(1, &mutual_a, &mix_inputs, &scenes));
        assert!(!unit_uses_mix_cycle(2, &mutual_b, &mix_inputs, &scenes));
    }

    #[test]
    fn mix_input_spec_promotes_raw_multiview() {
        let spec = crate::abi::MixInputSpec::new(1, SRC_KIND_MU_MULTIVIEW, 1, 0).unwrap();
        assert_eq!(spec.target_id, MULTIVIEW_BASE | 1);
        let already =
            crate::abi::MixInputSpec::new(MULTIVIEW_BASE | 1, SRC_KIND_MU_MULTIVIEW, 1, 0).unwrap();
        assert_eq!(already.target_id, MULTIVIEW_BASE | 1);
        let program = crate::abi::MixInputSpec::new(1, SRC_KIND_MU_PROGRAM, 1, 0).unwrap();
        assert_eq!(program.target_id, 1);
    }

    #[test]
    fn resolve_output_source_id_promotes_raw_multiview() {
        assert_eq!(
            crate::abi::resolve_output_source_id(SRC_KIND_MU_MULTIVIEW, 1),
            MULTIVIEW_BASE | 1
        );
        assert_eq!(
            crate::abi::resolve_output_source_id(SRC_KIND_MU_MULTIVIEW, MULTIVIEW_BASE | 1),
            MULTIVIEW_BASE | 1
        );
        assert_eq!(
            crate::abi::resolve_output_source_id(SRC_KIND_SCENE, 2),
            SCENE_BASE | 2
        );
        assert_eq!(
            crate::abi::resolve_output_source_id(SRC_KIND_MU_PROGRAM, 0),
            0
        );
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

    fn last_error_text() -> String {
        let mut buf = vec![0u8; 512];
        let n = unsafe { mixer_last_error(buf.as_mut_ptr(), buf.len()) };
        if n <= 0 {
            return String::new();
        }
        String::from_utf8_lossy(&buf[..n as usize]).into_owned()
    }

    #[cfg(windows)]
    #[test]
    fn attach_missing_remote_unit_reports_error() {
        mixer_destroy();
        assert_eq!(mixer_create(0, 60, 1), OK);
        const REMOTE_UNIT: u64 = 0x0007_0001;
        assert_eq!(
            mixer_unit_attach_output(REMOTE_UNIT, 1, 1920, 1080, OUTPUT_PREVIEW),
            ERR_INVALID_ARGUMENT
        );
        let error = last_error_text();
        assert!(
            error.contains("0x70001") || error.contains("not created"),
            "{error}"
        );
        assert_eq!(mixer_create_unit(REMOTE_UNIT, 1920, 1080), OK);
        assert_eq!(
            mixer_unit_attach_native(
                REMOTE_UNIT,
                OUTPUT_PREVIEW,
                NATIVE_WIN32_HWND,
                0,
                1920,
                1080
            ),
            ERR_INVALID_ARGUMENT
        );
        let hwnd_error = last_error_text();
        assert!(
            hwnd_error.contains("handle") || hwnd_error.contains("not valid"),
            "{hwnd_error}"
        );
        assert!(!hwnd_error.contains("not created"), "{hwnd_error}");
        let preview = mixer_unit_attach_native(
            REMOTE_UNIT,
            OUTPUT_PREVIEW,
            NATIVE_WIN32_HWND,
            1,
            1920,
            1080,
        );
        let program = mixer_unit_attach_output(REMOTE_UNIT, 1, 1920, 1080, OUTPUT_PROGRAM);
        assert_ne!(
            preview, ERR_INVALID_ARGUMENT,
            "unit exists; preview attach must not fail the unit-missing check"
        );
        assert_ne!(
            program, ERR_INVALID_ARGUMENT,
            "unit exists; program attach must not fail the unit-missing check"
        );
        mixer_destroy();
    }

    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    #[test]
    fn source_status_is_empty_without_receiver() {
        mixer_destroy();
        let mut status = MixerSourceStatus {
            connected: 1,
            has_video: 1,
        };
        assert_eq!(
            unsafe { mixer_source_status(0x0005_0001, &mut status) },
            ERR_NOT_CREATED
        );
        assert_eq!(mixer_create(0, 60, 1), OK);
        assert_eq!(unsafe { mixer_source_status(0x0005_0001, &mut status) }, OK);
        assert_eq!(status.connected, 0);
        assert_eq!(status.has_video, 0);
        let mut buf = vec![0u8; 64];
        assert_eq!(
            unsafe { mixer_source_copy_error(0x0005_0001, buf.as_mut_ptr(), buf.len()) },
            0
        );
        mixer_destroy();
    }

    fn dummy_video() -> SendCmd {
        SendCmd::Video {
            width: 2,
            height: 2,
            stride: 4,
            pts: 0,
            data: Arc::from([0u8; 8]),
            fps_n: 60,
            fps_d: 1,
        }
    }

    fn dummy_audio() -> SendCmd {
        SendCmd::Audio {
            packet: AudioPacket {
                timestamp: 0,
                sample_rate: AUDIO_RATE,
                channels: 2,
                samples_per_channel: 1,
                pcm_planar_f32: vec![0.0, 0.0],
            },
        }
    }

    fn dummy_gpu_video(busy: Arc<AtomicBool>, pts: i64, device: &GpuDevice) -> SendCmd {
        let texture = device.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("eiviz coalesce test"),
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        SendCmd::GpuVideo {
            texture,
            width: 16,
            height: 16,
            pts,
            fps_n: 60,
            fps_d: 1,
            busy,
        }
    }

    fn send_kind(cmd: &SendCmd) -> u8 {
        match cmd {
            SendCmd::Audio { .. } => 0,
            SendCmd::Video { .. } => 1,
            SendCmd::GpuVideo { .. } => 2,
            SendCmd::Shutdown => 3,
        }
    }

    #[test]
    fn take_audio_first_sends_audio_before_video() {
        let out = take_audio_first(vec![
            dummy_video(),
            dummy_audio(),
            dummy_video(),
            dummy_audio(),
        ]);
        assert_eq!(
            out.iter().map(send_kind).collect::<Vec<_>>(),
            vec![0, 0, 1, 1]
        );
    }

    #[test]
    fn coalesce_latest_video_releases_stale_gpu_busy() {
        let device = GpuDevice::with_backend(crate::device::BackendRequest::Auto).expect("gpu");
        let stale = Arc::new(AtomicBool::new(true));
        let latest = Arc::new(AtomicBool::new(true));
        let out = coalesce_latest_video(vec![
            dummy_audio(),
            dummy_gpu_video(Arc::clone(&stale), 1, &device),
            dummy_video(),
            dummy_gpu_video(Arc::clone(&latest), 2, &device),
        ]);
        assert!(
            !stale.load(Ordering::Acquire),
            "stale GPU frame must free the send ring"
        );
        assert!(
            latest.load(Ordering::Acquire),
            "kept GPU frame stays busy until sent"
        );
        assert_eq!(out.iter().map(send_kind).collect::<Vec<_>>(), vec![0, 2]);
    }

    #[test]
    fn pack_copy_key_shares_same_picture() {
        assert_eq!(
            pack_copy_key(SRC_KIND_MU_PROGRAM, 0, 1),
            pack_copy_key(SRC_KIND_MU_PROGRAM, 99, 1)
        );
        assert_ne!(
            pack_copy_key(SRC_KIND_MU_PROGRAM, 0, 1),
            pack_copy_key(SRC_KIND_MU_PREVIEW, 0, 1)
        );
        assert_eq!(
            pack_copy_key(SRC_KIND_MU_PREVIEW, 0, 2),
            pack_copy_key(SRC_KIND_MU_PREVIEW, 7, 2)
        );
        assert_eq!(
            pack_copy_key(SRC_KIND_INPUT, 40, 1),
            pack_copy_key(SRC_KIND_INPUT, 40, 2)
        );
        assert_ne!(
            pack_copy_key(SRC_KIND_INPUT, 40, 1),
            pack_copy_key(SRC_KIND_INPUT, 41, 1)
        );
    }

    #[test]
    fn session_file_codec_keeps_json_abi() {
        use std::ffi::CString;
        let eivz = include_bytes!("../../headless/tests/fixtures/bars.eivz");
        let dir = std::env::temp_dir().join(format!("eiviz-session-abi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src_path = dir.join("bars.eivz");
        let saved_path = dir.join("saved.eivz");
        let json_path = dir.join("legacy.json");
        std::fs::write(&src_path, eivz).unwrap();
        std::fs::write(&json_path, br#"{"version":2}"#).unwrap();
        let src_c = CString::new(src_path.to_string_lossy().as_bytes()).unwrap();
        let saved_c = CString::new(saved_path.to_string_lossy().as_bytes()).unwrap();
        let json_c = CString::new(json_path.to_string_lossy().as_bytes()).unwrap();
        let mut buf = vec![0u8; 1 << 20];
        let rejected = unsafe { mixer_session_load(json_c.as_ptr(), buf.as_mut_ptr(), buf.len()) };
        assert!(rejected < 0, "legacy json must be rejected {rejected}");
        assert_eq!(unsafe { mixer_session_has_assets(src_c.as_ptr()) }, 0);
        let n = unsafe { mixer_session_load(src_c.as_ptr(), buf.as_mut_ptr(), buf.len()) };
        assert!(n > 0, "load eivz {n}");
        let loaded = buf[..n as usize].to_vec();
        assert_eq!(loaded.first().copied(), Some(b'{'));
        let save = unsafe { mixer_session_save(saved_c.as_ptr(), loaded.as_ptr(), loaded.len()) };
        assert_eq!(save, OK);
        let saved_bytes = std::fs::read(&saved_path).unwrap();
        assert_eq!(&saved_bytes[..4], b"EIVZ");
        let exported_path = dir.join("exported.eivz");
        let exported_c = CString::new(exported_path.to_string_lossy().as_bytes()).unwrap();
        let exported =
            unsafe { mixer_session_export(exported_c.as_ptr(), loaded.as_ptr(), loaded.len()) };
        assert_eq!(exported, OK);
        assert_eq!(&std::fs::read(&exported_path).unwrap()[..4], b"EIVZ");
        let hist_n =
            unsafe { mixer_session_history(saved_c.as_ptr(), buf.as_mut_ptr(), buf.len()) };
        assert!(hist_n >= 0, "history {hist_n}");
        assert_eq!(&buf[..hist_n as usize], b"[]");
        let mut value: serde_json::Value = serde_json::from_slice(&loaded).unwrap();
        value["inputs"][0]["name"] = serde_json::Value::String("changed".into());
        let edited = serde_json::to_vec(&value).unwrap();
        let save2 = unsafe { mixer_session_save(saved_c.as_ptr(), edited.as_ptr(), edited.len()) };
        assert_eq!(save2, OK);
        let hist_n =
            unsafe { mixer_session_history(saved_c.as_ptr(), buf.as_mut_ptr(), buf.len()) };
        assert!(hist_n > 2, "history after overwrite {hist_n}");
        let history_json = String::from_utf8(buf[..hist_n as usize].to_vec()).unwrap();
        assert!(history_json.contains("\"index\":0"), "{history_json}");
        let n_rev =
            unsafe { mixer_session_load_rev(saved_c.as_ptr(), 0, buf.as_mut_ptr(), buf.len()) };
        assert_eq!(n_rev, n);
        assert_eq!(&buf[..n_rev as usize], loaded.as_slice());
        mixer_session_clear_current();
        assert_eq!(
            unsafe { mixer_session_current_path(buf.as_mut_ptr(), buf.len()) },
            0
        );
        assert!(
            crate::control_service()
                .lock()
                .unwrap()
                .session_path()
                .is_none()
        );
        let n2 = unsafe { mixer_session_load(saved_c.as_ptr(), buf.as_mut_ptr(), buf.len()) };
        assert!(n2 > 0, "load saved current {n2}");
        let current: serde_json::Value = serde_json::from_slice(&buf[..n2 as usize]).unwrap();
        assert_eq!(current["inputs"][0]["name"], "changed");
        let n3 = unsafe {
            mixer_session_canonicalize(loaded.as_ptr(), loaded.len(), buf.as_mut_ptr(), buf.len())
        };
        assert_eq!(n3, n);
        assert_eq!(&buf[..n3 as usize], loaded.as_slice());
        let too_small = unsafe { mixer_session_load(saved_c.as_ptr(), buf.as_mut_ptr(), 16) };
        assert_eq!(too_small, -1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
