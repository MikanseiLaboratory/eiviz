use eiviz_command::{Command, CommandEnvelope};
use eiviz_core::{
    AacEncoderProfile, AsrcProfile, AudioFollowPolicy, AudioResamplingPolicy, AudioRoute,
    AuxiliaryLoadSheddingPolicy, CompositorBackend, DistributionProfile, H264EncoderProfile, Input,
    InputId, InputSource, MixTap, MixingUnit, MixingUnitId, Multiview, MultiviewId,
    MultiviewSource, MultiviewTile, Output, OutputColorFormat, OutputId, OutputKind,
    OutputVideoSource, OverlayId, OverlaySlot, Project, ReconnectProfile, RouteMode, Scene,
    SceneId, SceneItem, SceneItemId, ToneMapPolicy, Transform2D, TransitionStyle, TransportProfile,
    VideoFormat,
};
#[cfg(any(feature = "decklink", feature = "audio-cpal"))]
use eiviz_core::{DeviceBinding, DeviceBindingId};
use eiviz_engine::Engine;
use eiviz_operations::{CapabilityEntry, EvidenceState};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputSelectKind {
    Video,
    Image,
    NdiOmt,
    MixFeed,
    ColorBars,
    Color,
}

impl InputSelectKind {
    const ALL: [Self; 6] = [
        Self::Video,
        Self::Image,
        Self::NdiOmt,
        Self::MixFeed,
        Self::ColorBars,
        Self::Color,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Image => "Image",
            Self::NdiOmt => "NDI / OMT",
            Self::MixFeed => "MixFeed",
            Self::ColorBars => "Color Bars",
            Self::Color => "Colour",
        }
    }
}

fn parse_env<T>(name: &str, default: T) -> Result<T, std::io::Error>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match std::env::var(name) {
        Ok(value) => value.parse().map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid {name}: {error}"),
            )
        }),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid {name}: {error}"),
        )),
    }
}

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1600.0, 900.0])
            .with_min_inner_size([1100.0, 700.0])
            .with_title("eiviz"),
        ..Default::default()
    };
    eframe::run_native(
        "eiviz",
        native,
        Box::new(|cc| Ok(Box::new(DesktopApp::new(cc)?))),
    )
}

#[cfg(feature = "wgpu-backend")]
struct WgpuPreviewBridge {
    render_state: eframe::egui_wgpu::RenderState,
    textures: std::collections::HashMap<String, egui::TextureId>,
}

#[cfg(feature = "wgpu-backend")]
impl WgpuPreviewBridge {
    fn new(render_state: eframe::egui_wgpu::RenderState) -> Self {
        Self {
            render_state,
            textures: std::collections::HashMap::new(),
        }
    }

    fn native_texture_id(
        &mut self,
        key: &str,
        frame: eiviz_gpu::WgpuTextureFrame,
    ) -> egui::TextureId {
        if let Some(texture_id) = self.textures.get(key).copied() {
            self.render_state
                .renderer
                .write()
                .update_egui_texture_from_wgpu_texture(
                    &self.render_state.device,
                    frame.view(),
                    eframe::wgpu::FilterMode::Linear,
                    texture_id,
                );
            texture_id
        } else {
            let texture_id = self.render_state.renderer.write().register_native_texture(
                &self.render_state.device,
                frame.view(),
                eframe::wgpu::FilterMode::Linear,
            );
            self.textures.insert(key.to_owned(), texture_id);
            texture_id
        }
    }

    fn show(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        frame: eiviz_gpu::WgpuTextureFrame,
        fill: bool,
    ) -> egui::Response {
        let size = if fill {
            ui.available_size()
        } else {
            fit_monitor_size(ui.available_size())
        };
        self.show_at(ui, key, frame, size)
    }

    fn show_at(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        frame: eiviz_gpu::WgpuTextureFrame,
        size: egui::Vec2,
    ) -> egui::Response {
        let texture_id = self.native_texture_id(key, frame);
        ui.image((texture_id, size))
    }
}

fn fit_monitor_size(available: egui::Vec2) -> egui::Vec2 {
    let width = available.x.max(16.0);
    let height = available.y.max(9.0);
    let aspect = 16.0 / 9.0;
    if width / height > aspect {
        egui::vec2(height * aspect, height)
    } else {
        egui::vec2(width, width / aspect)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum OutputWindow {
    Preview(MixingUnitId),
    Program(MixingUnitId),
    Multiview(MultiviewId),
    Input(InputId),
}

struct DesktopApp {
    engine: Arc<Engine>,
    media_stop: Arc<AtomicBool>,
    media_error: Arc<Mutex<Option<String>>>,
    media_thread: Option<std::thread::JoinHandle<()>>,
    status: String,
    selected_unit: MixingUnitId,
    selected_scene: Option<SceneId>,
    cut_duration_frames: u32,
    cut_swap: bool,
    mix_duration_frames: u32,
    mix_swap: bool,
    overlay_name_draft: String,
    mixfeed_source: Option<MixingUnitId>,
    mixfeed_preview_tap: bool,
    save_path: String,
    asset_root: String,
    image_path: String,
    video_path: String,
    openh264_path: String,
    fdk_aac_path: String,
    portable_path: String,
    capability_report_path: String,
    diagnostics_export_path: String,
    crash_report_path: String,
    asset_diagnostics: Vec<eiviz_project::AssetDiagnostic>,
    recovery: Option<RecoveryPrompt>,
    rtmp_url: String,
    srt_url: String,
    recording_path: String,
    output_source: OutputVideoSource,
    omt_address: String,
    omt_output_name: String,
    omt_output_uyvy: bool,
    omt_discovered: Vec<String>,
    omt_connections: Vec<(InputId, Arc<eiviz_io_omt::OmtSource>)>,
    omt_outputs: Vec<(OutputId, Arc<eiviz_io_omt::OmtSink>)>,
    #[cfg(feature = "decklink")]
    decklink_devices: Vec<eiviz_io_decklink::DeviceInfo>,
    #[cfg(feature = "decklink")]
    decklink_capture_selected: Option<usize>,
    #[cfg(feature = "decklink")]
    decklink_playback_selected: Option<usize>,
    #[cfg(feature = "decklink")]
    decklink_sources: Vec<Arc<eiviz_io_decklink::DeckLinkSource>>,
    #[cfg(feature = "decklink")]
    decklink_outputs: Vec<(OutputId, Arc<eiviz_io_decklink::DeckLinkSink>)>,
    #[cfg(feature = "decklink")]
    decklink_capability: eiviz_media::Capability,
    #[cfg(feature = "ndi")]
    ndi_discovered: Vec<eiviz_io_ndi::NdiSourceInfo>,
    #[cfg(feature = "ndi")]
    ndi_selected: Option<usize>,
    #[cfg(feature = "ndi")]
    ndi_connections: Vec<(InputId, Arc<eiviz_io_ndi::NdiSource>)>,
    #[cfg(feature = "ndi")]
    ndi_outputs: Vec<(OutputId, Arc<eiviz_io_ndi::NdiSink>)>,
    #[cfg(feature = "ndi")]
    ndi_output_name: String,
    #[cfg(feature = "ndi")]
    ndi_output_nv12: bool,
    #[cfg(feature = "ndi")]
    ndi_capability: eiviz_media::Capability,
    #[cfg(feature = "audio-cpal")]
    audio_backends: Vec<eiviz_io_audio::AudioBackend>,
    #[cfg(feature = "audio-cpal")]
    audio_backend_selected: Option<usize>,
    #[cfg(feature = "audio-cpal")]
    audio_devices: Vec<eiviz_io_audio::AudioDeviceInfo>,
    #[cfg(feature = "audio-cpal")]
    audio_input_selected: Option<usize>,
    #[cfg(feature = "audio-cpal")]
    audio_output_selected: Option<usize>,
    #[cfg(feature = "audio-cpal")]
    audio_inputs: Vec<(InputId, Arc<eiviz_io_audio::CpalInput>)>,
    #[cfg(feature = "audio-cpal")]
    audio_outputs: Vec<(OutputId, Arc<eiviz_io_audio::CpalOutput>)>,
    control_stop: Arc<AtomicBool>,
    control_ports: Option<eiviz_control::ControlPorts>,
    control_token_required: bool,
    control_rate: u32,
    control_queue_capacity: usize,
    #[cfg(feature = "midi")]
    midi_devices: Vec<eiviz_control::MidiDeviceInfo>,
    #[cfg(feature = "midi")]
    midi_selected: Option<usize>,
    #[cfg(feature = "midi")]
    midi_channel: u8,
    #[cfg(feature = "midi")]
    midi_take_note: u8,
    #[cfg(feature = "midi")]
    midi_handle: Option<eiviz_control::MidiHandle>,
    drag_item: Option<(SceneId, SceneItemId, f32, f32, Transform2D)>,
    #[cfg(feature = "wgpu-backend")]
    wgpu_preview: WgpuPreviewBridge,
    settings_open: bool,
    logs_open: bool,
    switcher_windows: BTreeSet<MixingUnitId>,
    output_windows: BTreeSet<OutputWindow>,
    layout_audit: LayoutAudit,
    inputs_open: bool,
    outputs_open: bool,
    scenes_open: bool,
    input_kind: InputSelectKind,
    file_browser_open: bool,
    file_browser_dir: PathBuf,
    recent_media: Vec<String>,
    color_draft: [u8; 4],
    scene_name_draft: String,
    scene_add_input: Option<InputId>,
    fps_window_start: std::time::Instant,
    fps_window_frame: u64,
    fps_window_misses: u64,
    measured_fps: f32,
    fps_warning: bool,
}

struct LayoutAudit {
    enabled: bool,
    frames: u32,
    screenshot_requested: bool,
    written: bool,
    bottom: Option<egui::Rect>,
    input_pane: Option<egui::Rect>,
    audio_pane: Option<egui::Rect>,
    tile: Option<egui::Rect>,
    cut: Option<egui::Rect>,
    mix: Option<egui::Rect>,
    meter_labels: Vec<String>,
}

impl LayoutAudit {
    fn from_env() -> Self {
        Self {
            enabled: std::env::var("EIVIZ_LAYOUT_AUDIT").is_ok(),
            frames: 0,
            screenshot_requested: false,
            written: false,
            bottom: None,
            input_pane: None,
            audio_pane: None,
            tile: None,
            cut: None,
            mix: None,
            meter_labels: Vec::new(),
        }
    }

    fn failures(&self) -> Vec<String> {
        let mut fail = Vec::new();
        match self.tile {
            Some(tile) => {
                if tile.height() > 240.0 {
                    fail.push(format!(
                        "input tile height {:.0}px > 240 (wrapped row stretched to pane)",
                        tile.height()
                    ));
                }
                if tile.height() < 140.0 {
                    fail.push(format!(
                        "input tile height {:.0}px < 140 (collapsed)",
                        tile.height()
                    ));
                }
                if !(200.0..=280.0).contains(&tile.width()) {
                    fail.push(format!(
                        "input tile width {:.0}px outside 200-280",
                        tile.width()
                    ));
                }
            }
            None => fail.push("input tile rect missing".into()),
        }
        match (self.cut, self.mix) {
            (Some(cut), Some(mix)) => {
                if mix.top() + 1.0 < cut.bottom() {
                    fail.push("CUT and MIX overlap vertically; expected stacked cards".into());
                }
                if (cut.left() - mix.left()).abs() > 24.0 {
                    fail.push("CUT and MIX are not in the same column".into());
                }
            }
            _ => fail.push("transition card rects missing".into()),
        }
        match (self.input_pane, self.audio_pane, self.bottom) {
            (Some(input), Some(audio), Some(bottom)) => {
                if input.height() + 16.0 < bottom.height() {
                    fail.push("input pane does not fill bottom row height".into());
                }
                if audio.width() < 140.0 {
                    fail.push(format!(
                        "audio mixer width {:.0}px is too narrow",
                        audio.width()
                    ));
                }
                if input.right() > audio.left() + 8.0 && input.left() < audio.right() {
                    fail.push("input list and audio mixer overlap".into());
                }
            }
            _ => fail.push("bottom pane rects missing".into()),
        }
        for label in &self.meter_labels {
            if label.len() >= 32 && label.contains('-') {
                fail.push(format!("meter label looks like a UUID: {label}"));
            }
        }
        fail
    }

    fn write_report(&self) -> std::io::Result<()> {
        let path = std::path::Path::new("target/eiviz-layout-audit.txt");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let fail = self.failures();
        let mut out = String::new();
        out.push_str(&format!("frames={}\n", self.frames));
        out.push_str(&format!("bottom={:?}\n", self.bottom));
        out.push_str(&format!("input_pane={:?}\n", self.input_pane));
        out.push_str(&format!("audio_pane={:?}\n", self.audio_pane));
        out.push_str(&format!("tile={:?}\n", self.tile));
        out.push_str(&format!("cut={:?}\n", self.cut));
        out.push_str(&format!("mix={:?}\n", self.mix));
        out.push_str(&format!("meter_labels={:?}\n", self.meter_labels));
        if fail.is_empty() {
            out.push_str("RESULT=PASS\n");
        } else {
            out.push_str("RESULT=FAIL\n");
            for item in fail {
                out.push_str(&format!("- {item}\n"));
            }
        }
        std::fs::write(path, out)
    }
}

enum RecoveryPrompt {
    Recoverable {
        path: std::path::PathBuf,
        project_hash: String,
        newer_than_project: bool,
    },
    Corrupt {
        path: std::path::PathBuf,
        error: String,
    },
}

fn desktop_distribution_output(
    transport_name: &str,
    endpoint: &str,
    owner: eiviz_core::MixingUnitId,
    video_source: OutputVideoSource,
) -> Result<Output, String> {
    let (name, kind, transport) = match transport_name {
        "rtmp" => (
            "RTMP output".to_owned(),
            OutputKind::Rtmp {
                url: endpoint.to_owned(),
            },
            TransportProfile::RtmpPublish {
                chunk_size: 4096,
                connect_timeout_ms: 5_000,
            },
        ),
        "srt" => (
            "SRT output".to_owned(),
            OutputKind::Srt {
                url: endpoint.to_owned(),
            },
            TransportProfile::SrtCallerMpegTs {
                latency_ms: 120,
                stream_id: None,
                connect_timeout_ms: 5_000,
            },
        ),
        "mp4" => (
            "Fragmented MP4 output".to_owned(),
            OutputKind::Mp4 {
                path: endpoint.to_owned(),
            },
            TransportProfile::FragmentedMp4 {
                recover_incomplete_tail: true,
            },
        ),
        _ => return Err(format!("unknown distribution transport {transport_name}")),
    };
    let color_format = OutputColorFormat::recommended_for(&kind);
    Ok(Output {
        id: OutputId::new(),
        name,
        owner,
        video_source,
        kind,
        enabled: false,
        color_format,
        distribution: Some(DistributionProfile {
            video: H264EncoderProfile::CiscoOpenH26426 {
                bitrate_bps: 8_000_000,
                keyframe_interval_frames: 120,
                level_idc: 42,
            },
            audio: AacEncoderProfile::FdkAacLc {
                bitrate_bps: 192_000,
                sample_rate: 48_000,
                channels: 2,
            },
            transport,
            queue_capacity: 256,
            reconnect: ReconnectProfile {
                initial_delay_ms: 250,
                max_delay_ms: 10_000,
                max_attempts: 0,
            },
        }),
    })
}

impl DesktopApp {
    fn selected_output_source(&self) -> OutputVideoSource {
        self.output_source
    }

    fn new(
        cc: &eframe::CreationContext<'_>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        #[cfg(not(feature = "wgpu-backend"))]
        let _ = cc;
        let mut project = Project::new("Untitled");
        project.compositor = match std::env::var("EIVIZ_COMPOSITOR") {
            Ok(value) if value == "wgpu" => CompositorBackend::Wgpu,
            Ok(value) if value == "cpu-reference" => CompositorBackend::CpuReference,
            Err(std::env::VarError::NotPresent) => CompositorBackend::CpuReference,
            Ok(other) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("unknown EIVIZ_COMPOSITOR={other}; expected wgpu or cpu-reference"),
                )
                .into());
            }
            Err(error) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    error.to_string(),
                )
                .into());
            }
        };
        #[cfg(feature = "wgpu-backend")]
        let render_state = cc.wgpu_render_state.clone().ok_or_else(|| {
            std::io::Error::other("wgpu-backend requires eframe CreationContext::wgpu_render_state")
        })?;
        #[cfg(feature = "wgpu-backend")]
        let wgpu_context = eiviz_gpu::SharedWgpuContext::new(
            render_state.adapter.clone(),
            render_state.device.clone(),
            render_state.queue.clone(),
        );
        #[cfg(feature = "wgpu-backend")]
        let engine = Engine::from_project_with_wgpu_context(project, wgpu_context)?.shared();
        #[cfg(not(feature = "wgpu-backend"))]
        let engine = Engine::from_project(project)?.shared();
        bootstrap(&engine);
        let save_path = "project.json".to_owned();
        let recovery = match eiviz_project::inspect_autosave(std::path::Path::new(&save_path)) {
            eiviz_project::AutosaveInspection::Recoverable(candidate) => {
                Some(RecoveryPrompt::Recoverable {
                    path: candidate.path,
                    project_hash: candidate.project_hash,
                    newer_than_project: candidate.newer_than_project,
                })
            }
            eiviz_project::AutosaveInspection::Corrupt { path, error } => {
                Some(RecoveryPrompt::Corrupt { path, error })
            }
            eiviz_project::AutosaveInspection::Missing
            | eiviz_project::AutosaveInspection::Current => None,
        };
        if recovery.is_none() {
            engine.set_autosave_path(&save_path);
        }
        install_crash_hook(
            engine.clone(),
            std::env::var("EIVIZ_CRASH_REPORT_PATH")
                .unwrap_or_else(|_| "eiviz-crash-report.json".into()),
        );
        let control_stop = Arc::new(AtomicBool::new(false));
        let control_enabled = std::env::var("EIVIZ_CONTROL").as_deref() != Ok("off");
        let control_rate = parse_env("EIVIZ_CONTROL_RATE", 60)?;
        let control_queue_capacity = parse_env("EIVIZ_CONTROL_QUEUE", 256)?;
        let control_token = std::env::var("EIVIZ_CONTROL_TOKEN")
            .ok()
            .filter(|token| !token.is_empty());
        let control_token_required = control_token.is_some();
        let control_ports = if control_enabled {
            Some(eiviz_control::spawn_control(
                engine.clone(),
                eiviz_control::ControlConfig {
                    http_bind: std::env::var("EIVIZ_HTTP_BIND")
                        .unwrap_or_else(|_| "127.0.0.1:8090".into()),
                    tcp_bind: std::env::var("EIVIZ_TCP_BIND")
                        .unwrap_or_else(|_| "127.0.0.1:8091".into()),
                    websocket_bind: std::env::var("EIVIZ_WS_BIND")
                        .unwrap_or_else(|_| "127.0.0.1:8092".into()),
                    require_token: control_token,
                    max_requests_per_sec: control_rate,
                    command_queue_capacity: control_queue_capacity,
                    event_queue_capacity: parse_env("EIVIZ_CONTROL_EVENT_QUEUE", 128)?,
                    max_connections: parse_env("EIVIZ_CONTROL_MAX_CONNECTIONS", 64)?,
                },
                control_stop.clone(),
            )?)
        } else {
            None
        };
        let selected_unit = engine.primary_unit();
        let mut app = Self {
            engine,
            media_stop: Arc::new(AtomicBool::new(false)),
            media_error: Arc::new(Mutex::new(None)),
            media_thread: None,
            status: "ready".into(),
            selected_unit,
            selected_scene: None,
            cut_duration_frames: 0,
            cut_swap: false,
            mix_duration_frames: 15,
            mix_swap: false,
            overlay_name_draft: "DSK 1".into(),
            mixfeed_source: None,
            mixfeed_preview_tap: false,
            save_path,
            asset_root: "eiviz-assets".into(),
            image_path: String::new(),
            video_path: String::new(),
            openh264_path: resolve_openh264_binary(),
            fdk_aac_path: std::env::var("EIVIZ_FDK_AAC_PATH").unwrap_or_default(),
            portable_path: "project.eiviz".into(),
            capability_report_path: "eiviz-capabilities.json".into(),
            diagnostics_export_path: "eiviz-flight-recorder.json".into(),
            crash_report_path: "eiviz-crash-report.json".into(),
            asset_diagnostics: Vec::new(),
            recovery,
            rtmp_url: "rtmp://127.0.0.1:1935/live/eiviz".into(),
            srt_url: "srt://127.0.0.1:9000".into(),
            recording_path: "recording.mp4".into(),
            output_source: OutputVideoSource::Program,
            omt_address: std::env::var("EIVIZ_OMT_SOURCE").unwrap_or_default(),
            omt_output_name: "eiviz".into(),
            omt_output_uyvy: true,
            omt_discovered: Vec::new(),
            omt_connections: Vec::new(),
            omt_outputs: Vec::new(),
            #[cfg(feature = "decklink")]
            decklink_devices: Vec::new(),
            #[cfg(feature = "decklink")]
            decklink_capture_selected: None,
            #[cfg(feature = "decklink")]
            decklink_playback_selected: None,
            #[cfg(feature = "decklink")]
            decklink_sources: Vec::new(),
            #[cfg(feature = "decklink")]
            decklink_outputs: Vec::new(),
            #[cfg(feature = "decklink")]
            decklink_capability: eiviz_io_decklink::probe(),
            #[cfg(feature = "ndi")]
            ndi_discovered: Vec::new(),
            #[cfg(feature = "ndi")]
            ndi_selected: None,
            #[cfg(feature = "ndi")]
            ndi_connections: Vec::new(),
            #[cfg(feature = "ndi")]
            ndi_outputs: Vec::new(),
            #[cfg(feature = "ndi")]
            ndi_output_name: "eiviz".into(),
            #[cfg(feature = "ndi")]
            ndi_output_nv12: true,
            #[cfg(feature = "ndi")]
            ndi_capability: eiviz_io_ndi::probe(),
            #[cfg(feature = "audio-cpal")]
            audio_backends: eiviz_io_audio::AudioBackend::compiled(),
            #[cfg(feature = "audio-cpal")]
            audio_backend_selected: None,
            #[cfg(feature = "audio-cpal")]
            audio_devices: Vec::new(),
            #[cfg(feature = "audio-cpal")]
            audio_input_selected: None,
            #[cfg(feature = "audio-cpal")]
            audio_output_selected: None,
            #[cfg(feature = "audio-cpal")]
            audio_inputs: Vec::new(),
            #[cfg(feature = "audio-cpal")]
            audio_outputs: Vec::new(),
            control_stop,
            control_ports,
            control_token_required,
            control_rate,
            control_queue_capacity,
            #[cfg(feature = "midi")]
            midi_devices: Vec::new(),
            #[cfg(feature = "midi")]
            midi_selected: None,
            #[cfg(feature = "midi")]
            midi_channel: 0,
            #[cfg(feature = "midi")]
            midi_take_note: 60,
            #[cfg(feature = "midi")]
            midi_handle: None,
            drag_item: None,
            #[cfg(feature = "wgpu-backend")]
            wgpu_preview: WgpuPreviewBridge::new(render_state),
            settings_open: false,
            logs_open: false,
            switcher_windows: BTreeSet::new(),
            output_windows: BTreeSet::new(),
            layout_audit: LayoutAudit::from_env(),
            inputs_open: false,
            outputs_open: false,
            scenes_open: false,
            input_kind: InputSelectKind::Video,
            file_browser_open: false,
            file_browser_dir: default_media_dir(),
            recent_media: Vec::new(),
            color_draft: [40, 40, 200, 255],
            scene_name_draft: "Scene".into(),
            scene_add_input: None,
            fps_window_start: std::time::Instant::now(),
            fps_window_frame: 0,
            fps_window_misses: 0,
            measured_fps: 0.0,
            fps_warning: false,
        };
        app.start_media_clock();
        if !app.omt_address.is_empty() {
            app.connect_omt();
        }
        app.link_openh264_at_startup();
        Ok(app)
    }

    fn start_media_clock(&mut self) {
        let engine = self.engine.clone();
        let stop = self.media_stop.clone();
        let error_slot = self.media_error.clone();
        self.media_thread = Some(
            std::thread::Builder::new()
                .name("eiviz-media".into())
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        let started = std::time::Instant::now();
                        if let Err(error) = engine.tick() {
                            if let Ok(mut slot) = error_slot.lock() {
                                *slot = Some(error.to_string());
                            }
                        }
                        let period = engine.frame_period();
                        let elapsed = started.elapsed();
                        if elapsed < period {
                            std::thread::sleep(period - elapsed);
                        }
                    }
                })
                .expect("media clock thread"),
        );
    }

    fn active_unit(&self) -> MixingUnitId {
        let project = self.engine.snapshot();
        if project.mixing_units.contains_key(&self.selected_unit) {
            self.selected_unit
        } else {
            self.engine.primary_unit()
        }
    }

    fn submit(&mut self, command: Command) {
        match self.engine.submit_payload(command) {
            Ok(_) => self.status = format!("rev {}", self.engine.revision()),
            Err(error) => self.status = error.to_string(),
        }
    }

    fn take_command(&self, style: TransitionStyle) -> Command {
        let (swap, duration_frames) = match style {
            TransitionStyle::Cut => (self.cut_swap, self.cut_duration_frames),
            TransitionStyle::Mix => (self.mix_swap, self.mix_duration_frames.max(1)),
        };
        Command::Take {
            unit: self.active_unit(),
            swap,
            style,
            duration_frames,
        }
    }

    fn add_mixing_unit(&mut self) {
        let index = self.engine.snapshot().mixing_units.len() + 1;
        let unit = MixingUnit::new(format!("Mix {index}"));
        let id = unit.id;
        self.submit(Command::AddMixingUnit { unit });
        if self.status.starts_with("rev ") {
            self.selected_unit = id;
        }
    }

    fn add_overlay(&mut self) {
        let unit = self.active_unit();
        let name = self.overlay_name_draft.trim();
        let name = if name.is_empty() {
            "Overlay".to_owned()
        } else {
            name.to_owned()
        };
        let overlay = OverlaySlot {
            id: OverlayId::new(),
            name,
            scene: self.selected_scene,
            enabled: false,
            z_order: self
                .engine
                .snapshot()
                .mixing_units
                .get(&unit)
                .map(|u| u.overlays.len() as i32)
                .unwrap_or(0),
        };
        self.submit(Command::AddOverlay { unit, overlay });
    }

    fn add_mix_feed(&mut self) {
        let dest = self.active_unit();
        let project = self.engine.snapshot();
        if self.mixfeed_source.is_none() {
            self.mixfeed_source = project
                .mixing_units
                .values()
                .find(|candidate| candidate.id != dest)
                .map(|candidate| candidate.id);
        }
        let Some(source) = self.mixfeed_source else {
            self.status = "add another mixing unit, then select it as MixFeed source".into();
            return;
        };
        if source == dest {
            self.status = "MixFeed cannot target the same mixing unit".into();
            return;
        }
        let Some(source_unit) = project.mixing_units.get(&source) else {
            self.status = "MixFeed source mixing unit is missing".into();
            return;
        };
        let tap = if self.mixfeed_preview_tap {
            MixTap::Preview
        } else {
            MixTap::Program
        };
        let tap_label = match tap {
            MixTap::Program => "PGM",
            MixTap::Preview => "PRV",
        };
        let input = Input {
            id: InputId::new(),
            name: format!("{} {tap_label}", source_unit.name),
            tags: vec!["mixfeed".into()],
            groups: vec![],
            source: InputSource::MixFeed { unit: source, tap },
        };
        let scene = Scene {
            id: SceneId::new(),
            name: format!("{} {tap_label}", source_unit.name),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: input.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        self.selected_scene = Some(scene.id);
        self.submit(Command::AddInput { input });
        if !self.status.starts_with("rev ") {
            return;
        }
        self.submit(Command::AddScene {
            scene: scene.clone(),
        });
        if !self.status.starts_with("rev ") {
            return;
        }
        self.submit(Command::SetPreview {
            unit: dest,
            scene: Some(scene.id),
        });
    }

    fn start_omt_output(&mut self, owner: MixingUnitId, source: OutputVideoSource) {
        let project = self.engine.snapshot();
        let feed = output_feed_label(&project, source);
        let unit_name = project
            .mixing_units
            .get(&owner)
            .map(|unit| unit.name.as_str())
            .unwrap_or("Mix");
        let base = self.omt_output_name.trim();
        let name = if base.is_empty() {
            format!("eiviz {unit_name} {feed}")
        } else {
            format!("{base} {unit_name} {feed}")
        };
        let color_profile = match project.video.color {
            eiviz_core::ColorSpace::Bt709Sdr => Some(eiviz_io_omt::OmtColorProfile::Bt709Limited),
            eiviz_core::ColorSpace::Bt2020Pq | eiviz_core::ColorSpace::Bt2020Hlg => None,
        };
        let Some(color_profile) = color_profile else {
            self.status = "OMT output supports only an explicit Bt709Sdr project profile".into();
            return;
        };
        let config = eiviz_io_omt::OmtOutputConfig {
            pixel_format: if self.omt_output_uyvy {
                eiviz_io_omt::OmtOutputPixelFormat::Uyvy
            } else {
                eiviz_io_omt::OmtOutputPixelFormat::Bgra
            },
            color_profile,
            send_queue_depth: 4,
        };
        let sink = match eiviz_io_omt::OmtSink::create_for_video_format(&name, &project.video, config)
        {
            Ok(sink) => Arc::new(sink),
            Err(error) => {
                self.status = format!("OMT output: {error}");
                return;
            }
        };
        let output = Output {
            id: OutputId::new(),
            name: name.clone(),
            owner,
            video_source: source,
            kind: OutputKind::Omt { url: name.clone() },
            enabled: true,
            color_format: Some(if self.omt_output_uyvy {
                OutputColorFormat::Uyvy
            } else {
                OutputColorFormat::Bgra8
            }),
            distribution: None,
        };
        if let Err(error) = self.engine.submit_payload(Command::AddOutput {
            output: output.clone(),
        }) {
            self.status = format!("OMT output project: {error}");
            return;
        }
        if let Err(error) = self.engine.attach_output_sink(output.id, sink.clone()) {
            self.status = format!("OMT output route: {error}");
            return;
        }
        self.omt_outputs.push((output.id, sink));
        self.status = format!("OMT output started: {name} ({feed})");
    }

    fn remove_managed_input(&mut self, input_id: InputId) {
        let project = self.engine.snapshot();
        for route in project
            .audio_matrix
            .routes
            .iter()
            .filter(|route| route.input == input_id)
        {
            let _ = self.engine.submit_payload(Command::ClearAudioRoute {
                input: input_id,
                bus: route.bus,
            });
        }
        let mut scenes_to_remove = Vec::new();
        for scene in project.scenes.values() {
            let uses = scene.items.iter().any(|item| item.input == input_id);
            if !uses {
                continue;
            }
            if scene.items.iter().all(|item| item.input == input_id) {
                scenes_to_remove.push(scene.id);
            } else {
                for item in scene.items.iter().filter(|item| item.input == input_id) {
                    let _ = self.engine.submit_payload(Command::RemoveSceneItem {
                        scene: scene.id,
                        item: item.id,
                    });
                }
            }
        }
        for unit in project.mixing_units.values() {
            if unit
                .preview
                .scene
                .is_some_and(|scene| scenes_to_remove.contains(&scene))
            {
                let _ = self.engine.submit_payload(Command::SetPreview {
                    unit: unit.id,
                    scene: None,
                });
            }
            if unit
                .program
                .scene
                .is_some_and(|scene| scenes_to_remove.contains(&scene))
            {
                let _ = self.engine.submit_payload(Command::SetProgram {
                    unit: unit.id,
                    scene: None,
                });
            }
            for overlay in &unit.overlays {
                if overlay
                    .scene
                    .is_some_and(|scene| scenes_to_remove.contains(&scene))
                {
                    let _ = self.engine.submit_payload(Command::SetOverlayScene {
                        unit: unit.id,
                        overlay: overlay.id,
                        scene: None,
                    });
                }
            }
        }
        for scene in scenes_to_remove {
            let _ = self.engine.submit_payload(Command::RemoveScene { id: scene });
        }
        if let Err(error) = self.engine.submit_payload(Command::RemoveInput { id: input_id }) {
            self.status = format!("remove input: {error}");
            return;
        }
        self.engine.detach_source(input_id);
        self.omt_connections.retain(|(id, _)| *id != input_id);
        #[cfg(feature = "ndi")]
        self.ndi_connections.retain(|(id, _)| *id != input_id);
        #[cfg(feature = "audio-cpal")]
        self.audio_inputs.retain(|(id, _)| *id != input_id);
        self.status = "input removed".into();
    }

    fn preview_input(&mut self, unit: MixingUnitId, input_id: InputId) {
        let project = self.engine.snapshot();
        let Some(input) = project.inputs.get(&input_id) else {
            self.status = "preview: unknown input".into();
            return;
        };
        let scene_id = match scene_id_for_input(&project, input_id) {
            Some(id) => id,
            None => {
                let scene = Scene {
                    id: SceneId::new(),
                    name: input.name.clone(),
                    items: vec![SceneItem {
                        id: SceneItemId::new(),
                        input: input_id,
                        transform: Transform2D::fullscreen(),
                        z_order: 0,
                        playback: Default::default(),
                    }],
                };
                let id = scene.id;
                if let Err(error) = self.engine.submit_payload(Command::AddScene { scene }) {
                    self.status = format!("preview scene: {error}");
                    return;
                }
                id
            }
        };
        self.selected_scene = Some(scene_id);
        self.submit(Command::SetPreview {
            unit,
            scene: Some(scene_id),
        });
    }

    fn link_openh264_at_startup(&mut self) {
        if self.openh264_path.is_empty() {
            self.status = "OpenH264 2.6.0 bundle was not found next to the executable".into();
            return;
        }
        match eiviz_codec_software::OpenH264Decoder::new(std::path::Path::new(&self.openh264_path)) {
            Ok(_) => {
                self.status = format!(
                    "ready; OpenH264 2.6.0 linked ({})",
                    std::path::Path::new(&self.openh264_path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(&self.openh264_path)
                );
            }
            Err(error) => {
                self.status = format!("OpenH264 link failed: {error}");
            }
        }
    }

    fn sample_fps(&mut self, project: &Project) {
        let metrics = self.engine.metrics();
        let now = std::time::Instant::now();
        let elapsed = now.saturating_duration_since(self.fps_window_start);
        if metrics.deadline_misses > self.fps_window_misses || metrics.deadline_slack_nanos < 0 {
            self.fps_warning = true;
        }
        if elapsed >= std::time::Duration::from_millis(1000) {
            let frames = metrics.frame.saturating_sub(self.fps_window_frame);
            let misses = metrics.deadline_misses.saturating_sub(self.fps_window_misses);
            let seconds = elapsed.as_secs_f32().max(0.001);
            self.measured_fps = frames as f32 / seconds;
            let expected = project.video.frame_rate.numerator() as f32
                / project.video.frame_rate.denominator() as f32;
            self.fps_warning = misses > 0
                || metrics.deadline_slack_nanos < 0
                || self.measured_fps + 0.75 < expected * 0.95;
            self.fps_window_start = now;
            self.fps_window_frame = metrics.frame;
            self.fps_window_misses = metrics.deadline_misses;
        }
    }

    fn draw_fps_badge(&self, ui: &mut egui::Ui, project: &Project) {
        let expected = project.video.frame_rate;
        let metrics = self.engine.metrics();
        let tick_ms = metrics.processing_nanos as f32 / 1_000_000.0;
        let gpu_ms = metrics.gpu_frame_nanos as f32 / 1_000_000.0;
        let readback_ms = metrics.gpu_readback_nanos as f32 / 1_000_000.0;
        let slack_ms = metrics.deadline_slack_nanos as f32 / 1_000_000.0;
        let detail = format!(
            "tick {tick_ms:.1}ms gpu {gpu_ms:.1}ms rb {readback_ms:.1}ms n={} slack {slack_ms:.1}ms",
            metrics.gpu_readbacks
        );
        if self.fps_warning {
            ui.colored_label(
                egui::Color32::from_rgb(220, 80, 60),
                format!(
                    "{} not held (meas. {:.1})  {detail}",
                    expected, self.measured_fps
                ),
            );
        } else {
            ui.label(format!("{expected}  {detail}"));
        }
    }

    fn draw_input_manager(
        &mut self,
        ui: &mut egui::Ui,
        project: &Project,
        unit_id: Option<MixingUnitId>,
    ) {
        ui.heading("Input Select");
        ui.label(&self.status);
        ui.separator();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(188.0);
                ui.label("Input");
                for kind in InputSelectKind::ALL {
                    if ui
                        .selectable_label(self.input_kind == kind, kind.label())
                        .clicked()
                    {
                        self.input_kind = kind;
                        self.file_browser_open = false;
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_min_width(520.0);
                self.draw_input_select_body(ui, project, unit_id);
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("OK").clicked() {
                self.confirm_input_select();
            }
            if ui.button("Cancel").clicked() {
                self.inputs_open = false;
            }
        });
        ui.separator();
        ui.heading("Existing inputs");
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut remove = None;
            for input in project.inputs.values() {
                ui.push_id(input.id, |ui| {
                    ui.horizontal(|ui| {
                        let clicked = ui
                            .selectable_label(
                                false,
                                format!(
                                    "{}  [{}]  {}",
                                    input.name,
                                    input_source_label(&input.source),
                                    input_source_detail(project, &input.source)
                                ),
                            )
                            .clicked();
                        if clicked {
                            if let Some(unit) = unit_id {
                                self.preview_input(unit, input.id);
                            } else {
                                self.status = "select a mixing unit before previewing an input".into();
                            }
                        }
                        if ui.button("Remove").clicked() {
                            remove = Some(input.id);
                        }
                    });
                });
            }
            if let Some(id) = remove {
                self.remove_managed_input(id);
            }
        });
    }

    fn draw_input_select_body(
        &mut self,
        ui: &mut egui::Ui,
        project: &Project,
        unit_id: Option<MixingUnitId>,
    ) {
        match self.input_kind {
            InputSelectKind::Video => {
                ui.label("Select the Video file to open.");
                ui.label(openh264_link_label(&self.openh264_path));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.video_path).desired_width(420.0));
                    if ui.button("Browse").clicked() {
                        self.file_browser_open = true;
                        self.file_browser_dir = default_media_dir();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("FDK AAC (optional)");
                    ui.text_edit_singleline(&mut self.fdk_aac_path);
                });
                if self.file_browser_open {
                    if let Some(path) = self.draw_file_browser(ui, &["mp4", "mov", "m4v", "avi"]) {
                        self.video_path = path;
                    }
                }
                self.draw_recent_media(ui, true);
            }
            InputSelectKind::Image => {
                ui.label("Select the Image file to open.");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.image_path).desired_width(420.0));
                    if ui.button("Browse").clicked() {
                        self.file_browser_open = true;
                        self.file_browser_dir = default_pictures_dir();
                    }
                });
                if self.file_browser_open {
                    if let Some(path) =
                        self.draw_file_browser(ui, &["png", "jpg", "jpeg", "bmp", "webp"])
                    {
                        self.image_path = path;
                    }
                }
                self.draw_recent_media(ui, false);
            }
            InputSelectKind::NdiOmt => {
                ui.heading("OMT");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.omt_address).desired_width(360.0));
                    if ui.button("Discover").clicked() {
                        match eiviz_io_omt::discover_sources() {
                            Ok(sources) => {
                                self.omt_discovered = sources;
                                self.status =
                                    format!("OMT: {} source(s)", self.omt_discovered.len());
                            }
                            Err(error) => self.status = format!("OMT discovery: {error}"),
                        }
                    }
                });
                for address in self.omt_discovered.clone() {
                    if ui
                        .selectable_label(self.omt_address == address, &address)
                        .clicked()
                    {
                        self.omt_address = address;
                    }
                }
                ui.separator();
                ui.heading("NDI");
                #[cfg(feature = "ndi")]
                {
                    if ui.button("Discover NDI").clicked() {
                        match eiviz_io_ndi::discover_sources(std::time::Duration::from_secs(2)) {
                            Ok(sources) => {
                                self.ndi_discovered = sources;
                                self.status =
                                    format!("NDI: {} source(s)", self.ndi_discovered.len());
                            }
                            Err(error) => self.status = format!("NDI discovery: {error}"),
                        }
                    }
                    for (index, source) in self.ndi_discovered.clone().iter().enumerate() {
                        if ui
                            .selectable_label(self.ndi_selected == Some(index), source.label())
                            .clicked()
                        {
                            self.ndi_selected = Some(index);
                        }
                    }
                }
                #[cfg(not(feature = "ndi"))]
                ui.label("Build with `--features ndi` after installing the NDI 6 SDK/runtime.");
            }
            InputSelectKind::MixFeed => {
                ui.label("Route another mixing unit's Program or Preview into this unit.");
                let others: Vec<_> = project
                    .mixing_units
                    .values()
                    .filter(|candidate| unit_id != Some(candidate.id))
                    .cloned()
                    .collect();
                if others.is_empty() {
                    ui.label("Add another Mixing Unit first, then select it here.");
                    if ui.button("Add Mixing Unit").clicked() {
                        let dest = self.active_unit();
                        let index = self.engine.snapshot().mixing_units.len() + 1;
                        let unit = MixingUnit::new(format!("Mix {index}"));
                        let source = unit.id;
                        self.submit(Command::AddMixingUnit { unit });
                        self.selected_unit = dest;
                        self.mixfeed_source = Some(source);
                    }
                } else {
                    if self.mixfeed_source.is_none() {
                        self.mixfeed_source = others.first().map(|candidate| candidate.id);
                    }
                    for candidate in &others {
                        if ui
                            .selectable_label(
                                self.mixfeed_source == Some(candidate.id),
                                &candidate.name,
                            )
                            .clicked()
                        {
                            self.mixfeed_source = Some(candidate.id);
                        }
                    }
                    ui.checkbox(&mut self.mixfeed_preview_tap, "Use source Preview");
                }
            }
            InputSelectKind::ColorBars => {
                ui.label("Add a Color Bars generator.");
            }
            InputSelectKind::Color => {
                ui.label("Add a solid colour source.");
                ui.horizontal(|ui| {
                    ui.label("R");
                    ui.add(egui::DragValue::new(&mut self.color_draft[0]).range(0..=255));
                    ui.label("G");
                    ui.add(egui::DragValue::new(&mut self.color_draft[1]).range(0..=255));
                    ui.label("B");
                    ui.add(egui::DragValue::new(&mut self.color_draft[2]).range(0..=255));
                    ui.label("A");
                    ui.add(egui::DragValue::new(&mut self.color_draft[3]).range(0..=255));
                });
            }
        }
    }

    fn draw_recent_media(&mut self, ui: &mut egui::Ui, video: bool) {
        ui.horizontal(|ui| {
            ui.label("Recent");
            if !self.recent_media.is_empty() && ui.button("Clear").clicked() {
                self.recent_media.clear();
            }
        });
        if self.recent_media.is_empty() {
            ui.label("No recent files. Use Browse, then OK.");
            return;
        }
        egui::ScrollArea::vertical()
            .max_height(160.0)
            .show(ui, |ui| {
                for path in self.recent_media.clone() {
                    let lower = path.to_ascii_lowercase();
                    let is_video = [".mp4", ".mov", ".m4v", ".avi"]
                        .iter()
                        .any(|ext| lower.ends_with(ext));
                    if video != is_video {
                        continue;
                    }
                    if ui.selectable_label(false, &path).clicked() {
                        if video {
                            self.video_path = path;
                        } else {
                            self.image_path = path;
                        }
                    }
                }
            });
    }

    fn draw_file_browser(&mut self, ui: &mut egui::Ui, exts: &[&str]) -> Option<String> {
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Up").clicked() {
                if let Some(parent) = self.file_browser_dir.parent() {
                    self.file_browser_dir = parent.to_path_buf();
                }
            }
            ui.label(self.file_browser_dir.display().to_string());
            if ui.button("Close browser").clicked() {
                self.file_browser_open = false;
            }
        });
        let mut chosen = None;
        let mut enter = None;
        let (dirs, files) = list_dir_entries(&self.file_browser_dir, exts);
        egui::ScrollArea::vertical()
            .max_height(240.0)
            .show(ui, |ui| {
                for dir in dirs {
                    let name = format!(
                        "{}/",
                        dir.file_name().and_then(|n| n.to_str()).unwrap_or("..")
                    );
                    if ui.selectable_label(false, name).clicked() {
                        enter = Some(dir);
                    }
                }
                for file in files {
                    let name = file
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default()
                        .to_owned();
                    if ui.selectable_label(false, name).clicked() {
                        chosen = Some(file.display().to_string());
                    }
                }
            });
        if let Some(dir) = enter {
            self.file_browser_dir = dir;
        }
        if chosen.is_some() {
            self.file_browser_open = false;
        }
        chosen
    }

    fn confirm_input_select(&mut self) {
        match self.input_kind {
            InputSelectKind::Video => {
                if self.video_path.trim().is_empty() {
                    self.status = "select a video file, then OK".into();
                    return;
                }
                self.remember_media(&self.video_path.clone());
                self.add_video();
            }
            InputSelectKind::Image => {
                if self.image_path.trim().is_empty() {
                    self.status = "select an image file, then OK".into();
                    return;
                }
                self.remember_media(&self.image_path.clone());
                self.add_image();
            }
            InputSelectKind::NdiOmt => {
                if !self.omt_address.trim().is_empty() {
                    self.connect_omt();
                    return;
                }
                #[cfg(feature = "ndi")]
                self.connect_ndi();
                #[cfg(not(feature = "ndi"))]
                {
                    self.status = "select an OMT source, then OK".into();
                }
            }
            InputSelectKind::MixFeed => self.add_mix_feed(),
            InputSelectKind::ColorBars => self.add_color_bars(),
            InputSelectKind::Color => self.add_solid_color(),
        }
    }

    fn remember_media(&mut self, path: &str) {
        let path = path.trim();
        if path.is_empty() {
            return;
        }
        self.recent_media.retain(|item| item != path);
        self.recent_media.insert(0, path.to_owned());
        self.recent_media.truncate(24);
    }

    fn add_generated_input(&mut self, input: Input) {
        let scene = Scene {
            id: SceneId::new(),
            name: input.name.clone(),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: input.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        if let Err(error) = self.engine.submit_payload(Command::AddInput { input }) {
            self.status = format!("input: {error}");
            return;
        }
        if let Err(error) = self.engine.submit_payload(Command::AddScene {
            scene: scene.clone(),
        }) {
            self.status = format!("scene: {error}");
            return;
        }
        let unit = self.active_unit();
        if let Err(error) = self.engine.submit_payload(Command::SetPreview {
            unit,
            scene: Some(scene.id),
        }) {
            self.status = format!("preview: {error}");
            return;
        }
        self.selected_scene = Some(scene.id);
        self.status = format!("ready: {}", scene.name);
    }

    fn add_color_bars(&mut self) {
        self.add_generated_input(Input {
            id: InputId::new(),
            name: "Color Bars".into(),
            tags: vec!["synth".into()],
            groups: vec![],
            source: InputSource::ColorBars,
        });
    }

    fn add_solid_color(&mut self) {
        let [r, g, b, a] = self.color_draft;
        self.add_generated_input(Input {
            id: InputId::new(),
            name: format!("Colour {r},{g},{b}"),
            tags: vec!["synth".into()],
            groups: vec![],
            source: InputSource::SolidColor { r, g, b, a },
        });
    }

    fn add_empty_scene(&mut self) {
        let name = self.scene_name_draft.trim();
        let name = if name.is_empty() {
            format!("Scene {}", self.engine.snapshot().scenes.len() + 1)
        } else {
            name.to_owned()
        };
        let scene = Scene {
            id: SceneId::new(),
            name,
            items: vec![],
        };
        self.selected_scene = Some(scene.id);
        self.submit(Command::AddScene { scene });
    }

    fn add_item_to_selected_scene(&mut self) {
        let Some(scene) = self.selected_scene else {
            self.status = "select a scene first".into();
            return;
        };
        let Some(input) = self.scene_add_input else {
            self.status = "select an input to add to the scene".into();
            return;
        };
        self.submit(Command::AddSceneItem {
            scene,
            item: SceneItem {
                id: SceneItemId::new(),
                input,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            },
        });
    }

    fn remove_scene(&mut self, scene_id: SceneId) {
        let project = self.engine.snapshot();
        for unit in project.mixing_units.values() {
            if unit.preview.scene == Some(scene_id) {
                let _ = self.engine.submit_payload(Command::SetPreview {
                    unit: unit.id,
                    scene: None,
                });
            }
            if unit.program.scene == Some(scene_id) {
                let _ = self.engine.submit_payload(Command::SetProgram {
                    unit: unit.id,
                    scene: None,
                });
            }
            for overlay in &unit.overlays {
                if overlay.scene == Some(scene_id) {
                    let _ = self.engine.submit_payload(Command::SetOverlayScene {
                        unit: unit.id,
                        overlay: overlay.id,
                        scene: None,
                    });
                }
            }
        }
        if let Err(error) = self.engine.submit_payload(Command::RemoveScene { id: scene_id }) {
            self.status = format!("remove scene: {error}");
            return;
        }
        if self.selected_scene == Some(scene_id) {
            self.selected_scene = None;
        }
        self.status = "scene removed".into();
    }

    fn draw_scene_manager(
        &mut self,
        ui: &mut egui::Ui,
        project: &Project,
        unit_id: Option<MixingUnitId>,
    ) {
        ui.heading("Scenes");
        ui.label(&self.status);
        ui.separator();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(260.0);
                ui.label("Scene list");
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for scene in project.scenes.values() {
                            let mut badges = Vec::new();
                            for unit in project.mixing_units.values() {
                                if unit.preview.scene == Some(scene.id) {
                                    badges.push(format!("{} PRV", unit.name));
                                }
                                if unit.program.scene == Some(scene.id) {
                                    badges.push(format!("{} PGM", unit.name));
                                }
                            }
                            let text = if badges.is_empty() {
                                scene.name.clone()
                            } else {
                                format!("{}  [{}]", scene.name, badges.join(" / "))
                            };
                            if ui
                                .selectable_label(self.selected_scene == Some(scene.id), text)
                                .clicked()
                            {
                                self.selected_scene = Some(scene.id);
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.scene_name_draft);
                    if ui.button("New").clicked() {
                        self.add_empty_scene();
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                let Some(scene_id) = self.selected_scene else {
                    ui.label("Select a scene.");
                    return;
                };
                let Some(scene) = project.scenes.get(&scene_id).cloned() else {
                    ui.label("Selected scene is missing.");
                    return;
                };
                ui.label(format!("Scene: {}", scene.name));
                ui.label(format!("{} item(s)", scene.items.len()));
                egui::ScrollArea::vertical()
                    .max_height(280.0)
                    .show(ui, |ui| {
                        let mut remove_item = None;
                        for item in &scene.items {
                            ui.push_id(item.id, |ui| {
                                ui.horizontal(|ui| {
                                    let input_name = project
                                        .inputs
                                        .get(&item.input)
                                        .map(|input| input.name.as_str())
                                        .unwrap_or("missing input");
                                    ui.label(format!("z={}  {}", item.z_order, input_name));
                                    if ui.button("Remove item").clicked() {
                                        remove_item = Some(item.id);
                                    }
                                });
                            });
                        }
                        if let Some(item) = remove_item {
                            self.submit(Command::RemoveSceneItem {
                                scene: scene_id,
                                item,
                            });
                        }
                    });
                ui.horizontal(|ui| {
                    let selected = self
                        .scene_add_input
                        .and_then(|id| project.inputs.get(&id).map(|input| input.name.clone()))
                        .unwrap_or_else(|| "(select input)".into());
                    egui::ComboBox::from_id_salt("scene-add-input")
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for input in project.inputs.values() {
                                ui.selectable_value(
                                    &mut self.scene_add_input,
                                    Some(input.id),
                                    &input.name,
                                );
                            }
                        });
                    if ui.button("Add item").clicked() {
                        self.add_item_to_selected_scene();
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Preview").clicked() {
                        if let Some(unit) = unit_id {
                            self.submit(Command::SetPreview {
                                unit,
                                scene: Some(scene_id),
                            });
                        } else {
                            self.status = "select a mixing unit first".into();
                        }
                    }
                    if ui.button("Program").clicked() {
                        if let Some(unit) = unit_id {
                            self.submit(Command::SetProgram {
                                unit,
                                scene: Some(scene_id),
                            });
                        } else {
                            self.status = "select a mixing unit first".into();
                        }
                    }
                    if ui.button("Delete scene").clicked() {
                        self.remove_scene(scene_id);
                    }
                });
            });
        });
    }

    fn draw_output_manager(
        &mut self,
        ui: &mut egui::Ui,
        project: &Project,
        unit_id: Option<MixingUnitId>,
    ) {
        ui.heading("NDI / OMT outputs");
        ui.label("Each Mixing Unit can emit Preview, Program, and each Multiview as separate NDI/OMT channels.");
        let Some(unit_id) = unit_id else {
            ui.label("Select a mixing unit first.");
            return;
        };
        let unit_name = project
            .mixing_units
            .get(&unit_id)
            .map(|unit| unit.name.as_str())
            .unwrap_or("Mix");
        ui.label(format!("Mixing Unit: {unit_name}"));
        ui.separator();
        #[cfg(feature = "ndi")]
        {
            ui.heading("NDI");
            ui.horizontal(|ui| {
                ui.label("name prefix");
                ui.text_edit_singleline(&mut self.ndi_output_name);
            });
            ui.checkbox(
                &mut self.ndi_output_nv12,
                "NV12 BT.709 limited (otherwise RGBA)",
            );
            ui.horizontal(|ui| {
                if ui.button("Start PGM").clicked() {
                    self.start_ndi_output(unit_id, OutputVideoSource::Program);
                }
                if ui.button("Start PRV").clicked() {
                    self.start_ndi_output(unit_id, OutputVideoSource::Preview);
                }
                for view in project
                    .multiviews
                    .values()
                    .filter(|view| view.owner == unit_id)
                {
                    if ui.button(format!("Start MV {}", view.name)).clicked() {
                        self.start_ndi_output(unit_id, OutputVideoSource::Multiview(view.id));
                    }
                }
            });
        }
        #[cfg(not(feature = "ndi"))]
        ui.label("NDI output requires `--features ndi`.");
        ui.heading("OMT");
        ui.horizontal(|ui| {
            ui.label("name prefix");
            ui.text_edit_singleline(&mut self.omt_output_name);
        });
        ui.checkbox(&mut self.omt_output_uyvy, "UYVY 4:2:2 (otherwise BGRA)");
        ui.horizontal(|ui| {
            if ui.button("Start PGM").clicked() {
                self.start_omt_output(unit_id, OutputVideoSource::Program);
            }
            if ui.button("Start PRV").clicked() {
                self.start_omt_output(unit_id, OutputVideoSource::Preview);
            }
            for view in project
                .multiviews
                .values()
                .filter(|view| view.owner == unit_id)
            {
                if ui.button(format!("Start MV {}", view.name)).clicked() {
                    self.start_omt_output(unit_id, OutputVideoSource::Multiview(view.id));
                }
            }
        });
        ui.separator();
        ui.heading("Active outputs");
        let mut stop_ndi = None;
        let mut stop_omt = None;
        for output in project.outputs.values().filter(|output| {
            matches!(
                output.kind,
                OutputKind::Ndi { .. } | OutputKind::Omt { .. }
            )
        }) {
            ui.push_id(output.id, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{} → {} {}",
                        output.name,
                        project
                            .mixing_units
                            .get(&output.owner)
                            .map(|unit| unit.name.as_str())
                            .unwrap_or("Mix"),
                        output_feed_label(project, output.video_source)
                    ));
                    if let Some(command) = output_color_format_combo(ui, output) {
                        if let Err(error) = self.engine.submit_payload(command) {
                            self.status = format!("output color format: {error}");
                        }
                    }
                    if ui.button("Stop").clicked() {
                        match output.kind {
                            OutputKind::Ndi { .. } => stop_ndi = Some(output.id),
                            OutputKind::Omt { .. } => stop_omt = Some(output.id),
                            _ => {}
                        }
                    }
                });
            });
        }
        if let Some(id) = stop_ndi {
            let _ = self.engine.submit_payload(Command::RemoveOutput { id });
            self.engine.detach_output_sink(id);
            #[cfg(feature = "ndi")]
            self.ndi_outputs.retain(|(output, _)| *output != id);
            self.status = "NDI output stopped".into();
        }
        if let Some(id) = stop_omt {
            let _ = self.engine.submit_payload(Command::RemoveOutput { id });
            self.engine.detach_output_sink(id);
            self.omt_outputs.retain(|(output, _)| *output != id);
            self.status = "OMT output stopped".into();
        }
    }

    #[cfg(feature = "decklink")]
    fn refresh_decklink(&mut self) {
        match eiviz_io_decklink::enumerate_devices() {
            Ok(devices) => {
                self.decklink_devices = devices;
                self.decklink_capture_selected = None;
                self.decklink_playback_selected = None;
                self.decklink_capability = eiviz_io_decklink::probe();
                self.status = format!(
                    "DeckLink: {} physical device(s)",
                    self.decklink_devices.len()
                );
            }
            Err(error) => {
                self.decklink_devices.clear();
                self.decklink_capability = eiviz_io_decklink::probe();
                self.status = format!("DeckLink enumeration: {error}");
            }
        }
    }

    #[cfg(any(feature = "decklink", feature = "audio-cpal"))]
    fn reassign_device_binding(
        &mut self,
        binding_id: DeviceBindingId,
        hardware_id: String,
        logical_name: String,
    ) {
        let project = self.engine.staged_snapshot();
        let Some(binding) = project.device_bindings.get(&binding_id) else {
            self.status = format!("Device reassignment: unknown binding {binding_id}");
            return;
        };
        let mut envelope = CommandEnvelope::new(
            self.engine.client(),
            Command::UpdateDeviceBinding {
                id: binding_id,
                expected_hardware_id: binding.last_seen_hardware_id.clone(),
                hardware_id: hardware_id.clone(),
                logical_name,
            },
        );
        envelope.expected_revision = Some(self.engine.revision());
        match self.engine.submit(envelope) {
            Ok(ack) => {
                self.status = format!(
                    "Device binding {binding_id} staged at revision {} with exact hardware ID {hardware_id}; restart any active endpoint to apply it",
                    ack.revision
                );
            }
            Err(error) => self.status = format!("Device reassignment rejected: {error}"),
        }
    }

    fn refresh_asset_diagnostics(&mut self) {
        self.asset_diagnostics = eiviz_project::inspect_assets(
            &self.engine.staged_snapshot(),
            Some(std::path::Path::new(self.asset_root.trim())),
        );
    }

    #[cfg(feature = "decklink")]
    fn connect_decklink_capture(&mut self) {
        if let Err(error) = eiviz_io_decklink::validate_video_format(&self.engine.snapshot().video)
        {
            self.status = format!("DeckLink capture profile unsupported: {error}");
            return;
        }
        let Some(device) = self
            .decklink_capture_selected
            .and_then(|index| self.decklink_devices.get(index))
            .cloned()
        else {
            self.status = "DeckLink capture: refresh and select an input".into();
            return;
        };
        let binding = DeviceBinding {
            id: DeviceBindingId::new(),
            kind: "decklink".into(),
            logical_name: device.display_name.clone(),
            last_seen_hardware_id: Some(device.persistent_id.clone()),
        };
        let input = Input {
            id: InputId::new(),
            name: format!("DeckLink {}", device.display_name),
            tags: vec!["decklink".into(), "sdi".into(), "live".into()],
            groups: vec![],
            source: InputSource::DeckLink {
                binding: binding.id,
            },
        };
        let source = match eiviz_io_decklink::DeckLinkSource::open(
            input.id,
            &binding,
            eiviz_io_decklink::DeckLinkConfig::default(),
        ) {
            Ok(source) => Arc::new(source),
            Err(error) => {
                self.status = format!("DeckLink capture open: {error}");
                return;
            }
        };
        let scene = Scene {
            id: SceneId::new(),
            name: input.name.clone(),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: input.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        for command in [
            Command::AddDeviceBinding {
                binding: binding.clone(),
            },
            Command::AddInput {
                input: input.clone(),
            },
            Command::AddScene {
                scene: scene.clone(),
            },
        ] {
            if let Err(error) = self.engine.submit_payload(command) {
                self.status = format!("DeckLink project binding: {error}");
                return;
            }
        }
        let unit = self.active_unit();
        if let Some(bus) = self.engine.snapshot().audio_matrix.buses.first() {
            let route = AudioRoute {
                input: input.id,
                bus: bus.id,
                mode: RouteMode::Follow { unit },
                gain_db: 0.0,
                muted: false,
                solo: false,
                delay_ms: 0.0,
                pan: 0.0,
            };
            if let Err(error) = self.engine.submit_payload(Command::SetAudioRoute { route }) {
                self.status = format!("DeckLink audio route: {error}");
                return;
            }
        }
        self.engine.attach_source(
            source.clone(),
            eiviz_engine::SourceClockPolicy::Bounded {
                config: Default::default(),
                unlocked: eiviz_engine::UnlockedBehavior::Fail,
            },
        );
        self.decklink_sources.push(source);
        if let Err(error) = self.engine.submit_payload(Command::SetPreview {
            unit,
            scene: Some(scene.id),
        }) {
            self.status = format!("DeckLink preview: {error}");
            return;
        }
        self.selected_scene = Some(scene.id);
        self.status = format!("DeckLink capture started: {}", device.display_name);
    }

    #[cfg(feature = "decklink")]
    fn start_decklink_output(&mut self, owner: eiviz_core::MixingUnitId) {
        if let Err(error) = eiviz_io_decklink::validate_video_format(&self.engine.snapshot().video)
        {
            self.status = format!("DeckLink output profile unsupported: {error}");
            return;
        }
        let Some(device) = self
            .decklink_playback_selected
            .and_then(|index| self.decklink_devices.get(index))
            .cloned()
        else {
            self.status = "DeckLink output: refresh and select an output".into();
            return;
        };
        let binding = DeviceBinding {
            id: DeviceBindingId::new(),
            kind: "decklink".into(),
            logical_name: device.display_name.clone(),
            last_seen_hardware_id: Some(device.persistent_id.clone()),
        };
        let name = format!("DeckLink {}", device.display_name);
        let sink = match eiviz_io_decklink::DeckLinkSink::create(
            name.clone(),
            &binding,
            eiviz_io_decklink::DeckLinkConfig::default(),
        ) {
            Ok(sink) => Arc::new(sink),
            Err(error) => {
                self.status = format!("DeckLink output open: {error}");
                return;
            }
        };
        if let Err(error) = self.engine.submit_payload(Command::AddDeviceBinding {
            binding: binding.clone(),
        }) {
            self.status = format!("DeckLink output binding: {error}");
            return;
        }
        let output = Output {
            id: OutputId::new(),
            name: name.clone(),
            owner,
            video_source: self.selected_output_source(),
            kind: OutputKind::DeckLink {
                binding: binding.id,
            },
            enabled: true,
            color_format: Some(OutputColorFormat::Bgra8),
            distribution: None,
        };
        if let Err(error) = self.engine.submit_payload(Command::AddOutput {
            output: output.clone(),
        }) {
            self.status = format!("DeckLink output project: {error}");
            return;
        }
        if let Err(error) = self.engine.attach_output_sink(output.id, sink.clone()) {
            self.status = format!("DeckLink output route: {error}");
            return;
        }
        self.decklink_outputs.push((output.id, sink));
        self.status = format!("DeckLink scheduled output started: {name}");
    }

    fn connect_omt(&mut self) {
        let address = self.omt_address.trim().to_owned();
        if address.is_empty() {
            self.status = "OMT: enter a source address".into();
            return;
        }
        let input = Input {
            id: InputId::new(),
            name: format!("OMT {address}"),
            tags: vec!["omt".into(), "live".into()],
            groups: vec![],
            source: InputSource::Omt {
                url: address.clone(),
            },
        };
        let source = match eiviz_io_omt::OmtSource::connect(input.id, address.clone()) {
            Ok(source) => Arc::new(source),
            Err(error) => {
                self.status = format!("OMT connect: {error}");
                return;
            }
        };
        let scene = Scene {
            id: SceneId::new(),
            name: format!("OMT {address}"),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: input.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        if let Err(error) = self.engine.submit_payload(Command::AddInput {
            input: input.clone(),
        }) {
            self.status = format!("OMT input: {error}");
            return;
        }
        if let Err(error) = self.engine.submit_payload(Command::AddScene {
            scene: scene.clone(),
        }) {
            self.status = format!("OMT scene: {error}");
            return;
        }
        self.engine.attach_source(
            source.clone(),
            eiviz_engine::SourceClockPolicy::Bounded {
                config: Default::default(),
                unlocked: eiviz_engine::UnlockedBehavior::Fail,
            },
        );
        self.omt_connections.push((input.id, source));
        let unit = self.active_unit();
        if let Err(error) = self.engine.submit_payload(Command::SetPreview {
            unit,
            scene: Some(scene.id),
        }) {
            self.status = format!("OMT preview: {error}");
            return;
        }
        self.selected_scene = Some(scene.id);
        self.status = format!("OMT connected: {address}");
    }

    fn clear_stale_live_io(&mut self) {
        self.omt_connections.clear();
        self.omt_outputs.clear();
        #[cfg(feature = "ndi")]
        {
            self.ndi_connections.clear();
            self.ndi_outputs.clear();
        }
        #[cfg(feature = "decklink")]
        {
            self.decklink_sources.clear();
            self.decklink_outputs.clear();
        }
        #[cfg(feature = "audio-cpal")]
        {
            self.audio_inputs.clear();
            self.audio_outputs.clear();
        }
    }

    fn rebind_declared_live_sources(&mut self, action: &str) {
        self.clear_stale_live_io();
        let mut failures = Vec::new();
        let mut bound = 0usize;
        for (id, url) in self.engine.declared_omt_sources() {
            match self.engine.bind_omt_source(id, &url) {
                Ok(source) => {
                    self.omt_connections.push((id, source));
                    bound += 1;
                }
                Err(error) => failures.push(format!("{url}: {error}")),
            }
        }
        for (id, source_name) in self.engine.declared_ndi_sources() {
            #[cfg(feature = "ndi")]
            match self.engine.bind_ndi_source(id, &source_name) {
                Ok(source) => {
                    self.ndi_connections.push((id, source));
                    bound += 1;
                }
                Err(error) => failures.push(format!("{source_name}: {error}")),
            }
            #[cfg(not(feature = "ndi"))]
            {
                let _ = id;
                failures.push(format!(
                    "{source_name}: NDI adapter is not enabled in this build; no substitute"
                ));
            }
        }
        for (id, binding_id) in self.engine.declared_decklink_sources() {
            #[cfg(feature = "decklink")]
            match self.engine.bind_decklink_source(id, binding_id) {
                Ok(source) => {
                    self.decklink_sources.push(source);
                    bound += 1;
                }
                Err(error) => failures.push(format!("DeckLink {binding_id}: {error}")),
            }
            #[cfg(not(feature = "decklink"))]
            match self.engine.bind_decklink_source(id, binding_id) {
                Ok(()) => failures.push(format!(
                    "DeckLink {binding_id}: adapter claimed success without an endpoint; no substitute"
                )),
                Err(error) => failures.push(format!("DeckLink {binding_id}: {error}")),
            }
        }
        for (id, binding_id) in self.engine.declared_audio_device_sources() {
            #[cfg(feature = "audio-cpal")]
            match self.engine.bind_audio_device_source(id, binding_id) {
                Ok(source) => {
                    self.audio_inputs.push((id, source));
                    bound += 1;
                }
                Err(error) => failures.push(format!("audio {binding_id}: {error}")),
            }
            #[cfg(not(feature = "audio-cpal"))]
            match self.engine.bind_audio_device_source(id, binding_id) {
                Ok(()) => failures.push(format!(
                    "audio {binding_id}: adapter claimed success without an endpoint; no substitute"
                )),
                Err(error) => failures.push(format!("audio {binding_id}: {error}")),
            }
        }
        let declared_distribution = self.engine.declared_distribution_outputs().len();
        self.status = if failures.is_empty() {
            if bound == 0 && declared_distribution == 0 {
                action.to_owned()
            } else if declared_distribution == 0 {
                format!("{action}; rebound {bound} live source(s)")
            } else {
                format!(
                    "{action}; rebound {bound} live source(s); {declared_distribution} distribution mapping(s) remain stopped until Start with explicit encoder binaries"
                )
            }
        } else {
            format!(
                "{action}; live bind failures (no implicit substitute): {}",
                failures.join("; ")
            )
        };
    }

    fn add_image(&mut self) {
        let path = std::path::Path::new(self.image_path.trim());
        let root = std::path::Path::new(self.asset_root.trim());
        let asset = match self.engine.ingest_asset(path, root) {
            Ok(asset) => asset,
            Err(error) => {
                self.status = format!("image ingest: {error}");
                return;
            }
        };
        let input = Input {
            id: InputId::new(),
            name: asset.original_name.clone(),
            tags: vec!["image".into()],
            groups: vec![],
            source: InputSource::Image { asset: asset.id },
        };
        let scene = Scene {
            id: SceneId::new(),
            name: asset.original_name,
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: input.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        if let Err(error) = self.engine.submit_payload(Command::AddInput { input }) {
            self.status = format!("image input: {error}");
            return;
        }
        if let Err(error) = self.engine.submit_payload(Command::AddScene {
            scene: scene.clone(),
        }) {
            self.status = format!("image scene: {error}");
            return;
        }
        let unit = self.active_unit();
        if let Err(error) = self.engine.submit_payload(Command::SetPreview {
            unit,
            scene: Some(scene.id),
        }) {
            self.status = format!("image preview: {error}");
            return;
        }
        self.selected_scene = Some(scene.id);
        self.status = format!("image ready: {}", scene.name);
    }

    fn add_video(&mut self) {
        if self.openh264_path.trim().is_empty() {
            self.status = "video ingest: enter an explicit Cisco OpenH264 2.6.0 binary path".into();
            return;
        }
        let fdk_path = (!self.fdk_aac_path.trim().is_empty())
            .then(|| std::path::Path::new(self.fdk_aac_path.trim()));
        let ingest = match self.engine.ingest_file(
            std::path::Path::new(self.video_path.trim()),
            std::path::Path::new(self.asset_root.trim()),
            std::path::Path::new(self.openh264_path.trim()),
            fdk_path,
            Default::default(),
        ) {
            Ok(ingest) => ingest,
            Err(error) => {
                self.status = format!("file ingest: {error}");
                return;
            }
        };
        let input = ingest.input;
        let scene = Scene {
            id: SceneId::new(),
            name: input.name.clone(),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: input.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        if let Err(error) = self.engine.submit_payload(Command::AddScene {
            scene: scene.clone(),
        }) {
            self.status = format!("video scene: {error}");
            return;
        }
        let unit = self.active_unit();
        if let Err(error) = self.engine.submit_payload(Command::SetPreview {
            unit,
            scene: Some(scene.id),
        }) {
            self.status = format!("video preview: {error}");
            return;
        }
        if matches!(
            &ingest.status,
            eiviz_io_file::FileMediaStatus::AudioVideo { .. }
        ) && let Some(bus) = self.engine.snapshot().audio_matrix.buses.first()
        {
            let route = AudioRoute {
                input: input.id,
                bus: bus.id,
                mode: RouteMode::Follow { unit },
                gain_db: 0.0,
                muted: false,
                solo: false,
                delay_ms: 0.0,
                pan: 0.0,
            };
            if let Err(error) = self.engine.submit_payload(Command::SetAudioRoute { route }) {
                self.status = format!("file AAC route: {error}");
                return;
            }
        }
        self.selected_scene = Some(scene.id);
        self.status = format!("file ready: {} — {}", scene.name, ingest.status);
    }

    #[cfg(feature = "ndi")]
    fn connect_ndi(&mut self) {
        let Some(index) = self.ndi_selected else {
            self.status = "NDI: discover and select a source".into();
            return;
        };
        let Some(discovered) = self.ndi_discovered.get(index).cloned() else {
            self.status = "NDI: selected source is no longer in the discovery list".into();
            return;
        };
        let input = Input {
            id: InputId::new(),
            name: discovered.name().to_owned(),
            tags: vec!["ndi".into(), "live".into()],
            groups: vec![],
            source: InputSource::Ndi {
                source_name: discovered.name().to_owned(),
            },
        };
        let source = match eiviz_io_ndi::NdiSource::connect(
            input.id,
            &discovered,
            eiviz_io_ndi::NdiConfig::default(),
        ) {
            Ok(source) => Arc::new(source),
            Err(error) => {
                self.status = format!("NDI connect: {error}");
                return;
            }
        };
        let scene = Scene {
            id: SceneId::new(),
            name: format!("NDI {}", discovered.name()),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: input.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        if let Err(error) = self.engine.submit_payload(Command::AddInput {
            input: input.clone(),
        }) {
            self.status = format!("NDI input: {error}");
            return;
        }
        if let Err(error) = self.engine.submit_payload(Command::AddScene {
            scene: scene.clone(),
        }) {
            self.status = format!("NDI scene: {error}");
            return;
        }
        let unit = self.active_unit();
        if let Some(bus) = self.engine.snapshot().audio_matrix.buses.first() {
            let route = AudioRoute {
                input: input.id,
                bus: bus.id,
                mode: RouteMode::Follow { unit },
                gain_db: 0.0,
                muted: false,
                solo: false,
                delay_ms: 0.0,
                pan: 0.0,
            };
            if let Err(error) = self.engine.submit_payload(Command::SetAudioRoute { route }) {
                self.status = format!("NDI audio route: {error}");
                return;
            }
        }
        self.engine.attach_source(
            source.clone(),
            eiviz_engine::SourceClockPolicy::Bounded {
                config: Default::default(),
                unlocked: eiviz_engine::UnlockedBehavior::Fail,
            },
        );
        self.ndi_connections.push((input.id, source));
        if let Err(error) = self.engine.submit_payload(Command::SetPreview {
            unit,
            scene: Some(scene.id),
        }) {
            self.status = format!("NDI preview: {error}");
            return;
        }
        self.selected_scene = Some(scene.id);
        self.status = format!("NDI connected: {}", discovered.label());
    }

    #[cfg(feature = "ndi")]
    fn start_ndi_output(
        &mut self,
        owner: eiviz_core::MixingUnitId,
        source: OutputVideoSource,
    ) {
        let project = self.engine.snapshot();
        let feed = output_feed_label(&project, source);
        let unit_name = project
            .mixing_units
            .get(&owner)
            .map(|unit| unit.name.as_str())
            .unwrap_or("Mix");
        let base = self.ndi_output_name.trim();
        let name = if base.is_empty() {
            format!("eiviz {unit_name} {feed}")
        } else {
            format!("{base} {unit_name} {feed}")
        };
        let mut config = match eiviz_io_ndi::NdiConfig::for_output(&project.video) {
            Ok(config) => config,
            Err(error) => {
                self.status = format!("NDI output profile unsupported: {error}");
                return;
            }
        };
        if self.ndi_output_nv12 {
            if project.video.color != eiviz_core::ColorSpace::Bt709Sdr {
                self.status =
                    "NDI NV12 output requires an explicit Bt709Sdr project color profile".into();
                return;
            }
            config.output_pixel_format = eiviz_io_ndi::NdiOutputPixelFormat::Nv12;
            config.output_color_profile = Some(eiviz_io_ndi::NdiColorProfile::Bt709Limited);
        }
        let sink = match eiviz_io_ndi::NdiSink::create(&name, project.video.frame_rate, config) {
            Ok(sink) => Arc::new(sink),
            Err(error) => {
                self.status = format!("NDI output: {error}");
                return;
            }
        };
        let output = Output {
            id: OutputId::new(),
            name: name.clone(),
            owner,
            video_source: source,
            kind: OutputKind::Ndi { name: name.clone() },
            enabled: true,
            color_format: Some(if self.ndi_output_nv12 {
                OutputColorFormat::Nv12
            } else {
                OutputColorFormat::Rgba8
            }),
            distribution: None,
        };
        if let Err(error) = self.engine.submit_payload(Command::AddOutput {
            output: output.clone(),
        }) {
            self.status = format!("NDI output project: {error}");
            return;
        }
        if let Err(error) = self.engine.attach_output_sink(output.id, sink.clone()) {
            self.status = format!("NDI output route: {error}");
            return;
        }
        self.ndi_outputs.push((output.id, sink));
        self.status = format!("NDI output started: {name} ({feed})");
    }

    #[cfg(feature = "audio-cpal")]
    fn refresh_audio_devices(&mut self) {
        let Some(backend) = self
            .audio_backend_selected
            .and_then(|index| self.audio_backends.get(index))
            .copied()
        else {
            self.status = "Audio: select an explicit backend".into();
            return;
        };
        match eiviz_io_audio::enumerate_devices(backend) {
            Ok(devices) => {
                self.audio_devices = devices;
                self.audio_input_selected = None;
                self.audio_output_selected = None;
                self.status = format!(
                    "Audio {backend}: {} physical/virtual device(s)",
                    self.audio_devices.len()
                );
            }
            Err(error) => {
                self.audio_devices.clear();
                self.status = format!("Audio {backend} enumeration: {error}");
            }
        }
    }

    #[cfg(feature = "audio-cpal")]
    fn start_audio_input(&mut self) {
        let Some(backend) = self
            .audio_backend_selected
            .and_then(|index| self.audio_backends.get(index))
            .copied()
        else {
            self.status = "Audio input: select a backend".into();
            return;
        };
        let Some(device) = self
            .audio_input_selected
            .and_then(|index| self.audio_devices.get(index))
            .cloned()
        else {
            self.status = "Audio input: refresh and select a device".into();
            return;
        };
        let binding = DeviceBinding {
            id: DeviceBindingId::new(),
            kind: backend.binding_kind(),
            logical_name: device.display_name.clone(),
            last_seen_hardware_id: Some(device.persistent_id.clone()),
        };
        let input = Input {
            id: InputId::new(),
            name: format!("Audio {}", device.display_name),
            tags: vec!["audio".into(), backend.id().into(), "live".into()],
            groups: vec![],
            source: InputSource::AudioDevice {
                binding: binding.id,
            },
        };
        let project = self.engine.snapshot();
        let config = eiviz_io_audio::AudioStreamConfig {
            sample_rate: project.audio.sample_rate,
            channels: project.audio.channels,
            ..Default::default()
        };
        let source = match eiviz_io_audio::CpalInput::open_with_policy(
            input.id,
            &binding,
            backend,
            config,
            project.audio.resampling,
        ) {
            Ok(source) => Arc::new(source),
            Err(error) => {
                self.status = format!("Audio input open: {error}");
                return;
            }
        };
        for command in [
            Command::AddDeviceBinding {
                binding: binding.clone(),
            },
            Command::AddInput {
                input: input.clone(),
            },
        ] {
            if let Err(error) = self.engine.submit_payload(command) {
                self.status = format!("Audio input project binding: {error}");
                return;
            }
        }
        let Some(bus) = self.engine.snapshot().audio_matrix.buses.first().cloned() else {
            self.status = "Audio input: project has no audio bus".into();
            return;
        };
        if let Err(error) = self.engine.submit_payload(Command::SetAudioRoute {
            route: AudioRoute {
                input: input.id,
                bus: bus.id,
                mode: RouteMode::Manual,
                gain_db: 0.0,
                muted: false,
                solo: false,
                delay_ms: 0.0,
                pan: 0.0,
            },
        }) {
            self.status = format!("Audio input route: {error}");
            return;
        }
        self.engine.attach_source(
            source.clone(),
            eiviz_engine::SourceClockPolicy::Bounded {
                config: Default::default(),
                unlocked: eiviz_engine::UnlockedBehavior::Fail,
            },
        );
        let device_rate = source.stream_config().sample_rate;
        self.audio_inputs.push((input.id, source));
        self.status = format!(
            "Audio input started: {} at {} Hz (project {} Hz, {:?})",
            device.display_name, device_rate, project.audio.sample_rate, project.audio.resampling
        );
    }

    #[cfg(feature = "audio-cpal")]
    fn start_audio_output(&mut self, owner: eiviz_core::MixingUnitId) {
        let Some(backend) = self
            .audio_backend_selected
            .and_then(|index| self.audio_backends.get(index))
            .copied()
        else {
            self.status = "Audio output: select a backend".into();
            return;
        };
        let Some(device) = self
            .audio_output_selected
            .and_then(|index| self.audio_devices.get(index))
            .cloned()
        else {
            self.status = "Audio output: refresh and select a device".into();
            return;
        };
        let binding = DeviceBinding {
            id: DeviceBindingId::new(),
            kind: backend.binding_kind(),
            logical_name: device.display_name.clone(),
            last_seen_hardware_id: Some(device.persistent_id.clone()),
        };
        let project = self.engine.snapshot();
        let config = eiviz_io_audio::AudioStreamConfig {
            sample_rate: project.audio.sample_rate,
            channels: project.audio.channels,
            ..Default::default()
        };
        let name = format!("Audio {}", device.display_name);
        let sink = match eiviz_io_audio::CpalOutput::open_with_policy(
            &name,
            &binding,
            backend,
            config,
            project.audio.resampling,
        ) {
            Ok(sink) => Arc::new(sink),
            Err(error) => {
                self.status = format!("Audio output open: {error}");
                return;
            }
        };
        if let Err(error) = self.engine.submit_payload(Command::AddDeviceBinding {
            binding: binding.clone(),
        }) {
            self.status = format!("Audio output binding: {error}");
            return;
        }
        let output = Output {
            id: OutputId::new(),
            name: name.clone(),
            owner,
            video_source: OutputVideoSource::Program,
            kind: OutputKind::AudioDevice {
                binding: binding.id,
            },
            enabled: true,
            color_format: None,
            distribution: None,
        };
        if let Err(error) = self.engine.submit_payload(Command::AddOutput {
            output: output.clone(),
        }) {
            self.status = format!("Audio output project: {error}");
            return;
        }
        if let Err(error) = self.engine.attach_audio_output(output.id, sink.clone()) {
            self.status = format!("Audio output attachment: {error}");
            return;
        }
        let device_rate = sink.stream_config().sample_rate;
        self.audio_outputs.push((output.id, sink));
        self.status = format!(
            "Audio output started: {} at {} Hz (project {} Hz, {:?})",
            device.display_name, device_rate, project.audio.sample_rate, project.audio.resampling
        );
    }

    #[cfg(feature = "midi")]
    fn refresh_midi_inputs(&mut self) {
        match eiviz_control::list_midi_inputs() {
            Ok(devices) => {
                self.midi_devices = devices;
                self.midi_selected = None;
                self.status = format!("MIDI: {} input port(s)", self.midi_devices.len());
            }
            Err(error) => {
                self.midi_devices.clear();
                self.midi_selected = None;
                self.status = format!("MIDI enumeration: {error}");
            }
        }
    }

    #[cfg(feature = "midi")]
    fn start_midi_take(&mut self) {
        let Some(device) = self
            .midi_selected
            .and_then(|index| self.midi_devices.get(index))
            .cloned()
        else {
            self.status = "MIDI: refresh and select an explicit input port".into();
            return;
        };
        let config = eiviz_control::MidiConfig {
            device_id: device.id.clone(),
            mappings: vec![eiviz_control::MidiMapping {
                trigger: eiviz_control::MidiTrigger::NoteOn {
                    channel: self.midi_channel,
                    note: self.midi_take_note,
                    minimum_velocity: 1,
                },
                command: self.take_command(TransitionStyle::Cut),
            }],
            queue_capacity: 128,
        };
        match eiviz_control::spawn_midi(self.engine.clone(), config, self.control_stop.clone()) {
            Ok(handle) => {
                self.midi_handle = Some(handle);
                self.status = format!(
                    "MIDI TAKE active: {} channel {} note {}",
                    device.name,
                    self.midi_channel + 1,
                    self.midi_take_note
                );
            }
            Err(error) => self.status = format!("MIDI input: {error}"),
        }
    }

    fn configure_distribution(&mut self, transport_name: &str, owner: eiviz_core::MixingUnitId) {
        let endpoint = match transport_name {
            "rtmp" => self.rtmp_url.trim(),
            "srt" => self.srt_url.trim(),
            "mp4" => self.recording_path.trim(),
            _ => "",
        };
        let output = match desktop_distribution_output(
            transport_name,
            endpoint,
            owner,
            self.selected_output_source(),
        ) {
            Ok(output) => output,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        match self.engine.configure_distribution_output(output) {
            Ok(_) => {
                self.status = format!(
                    "{transport_name} mapping saved stopped; Start requires explicit Cisco OpenH264 2.6.0 and license-reviewed FDK AAC binaries"
                )
            }
            Err(error) => self.status = format!("{transport_name} mapping: {error}"),
        }
    }

    fn show_program_frame(
        &mut self,
        ui: &mut egui::Ui,
        unit: eiviz_core::MixingUnitId,
    ) -> Option<egui::Response> {
        self.show_program_frame_filled(ui, unit, "pgm", false)
    }

    fn show_program_frame_filled(
        &mut self,
        ui: &mut egui::Ui,
        unit: eiviz_core::MixingUnitId,
        key: &str,
        fill: bool,
    ) -> Option<egui::Response> {
        #[cfg(feature = "wgpu-backend")]
        if let Some(texture) = self.engine.last_program_texture(unit) {
            return Some(self.wgpu_preview.show(ui, key, texture, fill));
        }
        show_frame(ui, self.engine.last_program(unit), key, fill)
    }

    fn show_program_frame_at(
        &mut self,
        ui: &mut egui::Ui,
        unit: eiviz_core::MixingUnitId,
        key: &str,
        size: egui::Vec2,
    ) -> Option<egui::Response> {
        #[cfg(feature = "wgpu-backend")]
        if let Some(texture) = self.engine.last_program_texture(unit) {
            return Some(self.wgpu_preview.show_at(ui, key, texture, size));
        }
        show_frame_at(ui, self.engine.last_program(unit), key, size)
    }

    fn show_preview_frame(
        &mut self,
        ui: &mut egui::Ui,
        unit: eiviz_core::MixingUnitId,
    ) -> Option<egui::Response> {
        self.show_preview_frame_filled(ui, unit, "prv", false)
    }

    fn show_preview_frame_filled(
        &mut self,
        ui: &mut egui::Ui,
        unit: eiviz_core::MixingUnitId,
        key: &str,
        fill: bool,
    ) -> Option<egui::Response> {
        #[cfg(feature = "wgpu-backend")]
        if let Some(texture) = self.engine.last_preview_texture(unit) {
            return Some(self.wgpu_preview.show(ui, key, texture, fill));
        }
        show_frame(ui, self.engine.last_preview(unit), key, fill)
    }

    fn show_preview_frame_at(
        &mut self,
        ui: &mut egui::Ui,
        unit: eiviz_core::MixingUnitId,
        key: &str,
        size: egui::Vec2,
    ) -> Option<egui::Response> {
        #[cfg(feature = "wgpu-backend")]
        if let Some(texture) = self.engine.last_preview_texture(unit) {
            return Some(self.wgpu_preview.show_at(ui, key, texture, size));
        }
        show_frame_at(ui, self.engine.last_preview(unit), key, size)
    }

    fn show_multiview_frame_filled(
        &mut self,
        ui: &mut egui::Ui,
        view: MultiviewId,
        texture_id: &str,
        fill: bool,
    ) -> Option<egui::Response> {
        #[cfg(feature = "wgpu-backend")]
        if let Some(texture) = self.engine.last_multiview_texture(view) {
            return Some(self.wgpu_preview.show(ui, texture_id, texture, fill));
        }
        show_frame(ui, self.engine.last_multiview(view), texture_id, fill)
    }

    fn draw_monitor_chrome(
        ui: &mut egui::Ui,
        title: &str,
        color: egui::Color32,
        add_contents: impl FnOnce(&mut egui::Ui),
    ) {
        egui::Frame::group(ui.style())
            .inner_margin(4.0)
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.colored_label(color, title);
                add_contents(ui);
            });
    }

    fn draw_audio_meters(&mut self, ui: &mut egui::Ui, project: &Project) {
        ui.colored_label(egui::Color32::from_rgb(60, 160, 90), "Audio Mixer");
        let meters = self.engine.metrics().peak_meters;
        self.layout_audit.meter_labels = project
            .audio_matrix
            .buses
            .iter()
            .map(|bus| bus.name.clone())
            .collect();
        let strip_h = (ui.available_height() - 4.0).max(120.0);
        let (meter_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), strip_h),
            egui::Sense::hover(),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(meter_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Min)),
            |ui| {
                ui.set_clip_rect(meter_rect);
                egui::ScrollArea::horizontal()
                    .id_salt("audio-mixer-strips")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_height(strip_h);
                        ui.horizontal(|ui| {
                            ui.set_min_height(strip_h);
                            ui.spacing_mut().item_spacing.x = 10.0;
                            for bus in &project.audio_matrix.buses {
                                let peak = meters
                                    .iter()
                                    .find(|(name, _)| name == &bus.id.to_string())
                                    .map(|(_, peak)| *peak)
                                    .unwrap_or(0.0);
                                draw_peak_meter(ui, &bus.name, peak, strip_h);
                            }
                        });
                    });
            },
        );
    }

    fn draw_transition_column(&mut self, ui: &mut egui::Ui, unit: &eiviz_core::MixingUnit) {
        ui.heading("Transition");
        egui::ScrollArea::vertical()
            .id_salt("transition-column")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;

                let cut = egui::Frame::group(ui.style())
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical(|ui| {
                            if ui
                                .add_sized([ui.available_width(), 40.0], egui::Button::new("CUT"))
                                .clicked()
                            {
                                self.submit(self.take_command(TransitionStyle::Cut));
                            }
                            ui.horizontal(|ui| {
                                ui.label("Duration (frames)");
                                ui.add(
                                    egui::DragValue::new(&mut self.cut_duration_frames)
                                        .range(0..=300)
                                        .speed(1.0),
                                );
                            });
                            ui.checkbox(&mut self.cut_swap, "SWAP");
                        });
                    });
                self.layout_audit.cut = Some(cut.response.rect);

                let mix = egui::Frame::group(ui.style())
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical(|ui| {
                            if ui
                                .add_sized([ui.available_width(), 40.0], egui::Button::new("MIX"))
                                .clicked()
                            {
                                self.submit(self.take_command(TransitionStyle::Mix));
                            }
                            ui.horizontal(|ui| {
                                ui.label("Duration (frames)");
                                ui.add(
                                    egui::DragValue::new(&mut self.mix_duration_frames)
                                        .range(1..=300)
                                        .speed(1.0),
                                );
                            });
                            ui.checkbox(&mut self.mix_swap, "SWAP");
                        });
                    });
                self.layout_audit.mix = Some(mix.response.rect);

                ui.separator();
                ui.label("Overlays");
                for (index, overlay) in unit.overlays.iter().enumerate() {
                    let mut on = overlay.enabled;
                    if ui
                        .selectable_label(on, format!("{} {}", index + 1, overlay.name))
                        .clicked()
                    {
                        on = !overlay.enabled;
                        self.submit(Command::SetOverlayEnabled {
                            unit: unit.id,
                            overlay: overlay.id,
                            enabled: on,
                        });
                    }
                }
            });
    }

    fn draw_input_grid(
        &mut self,
        ui: &mut egui::Ui,
        project: &Project,
        unit_id: MixingUnitId,
        unit: Option<&eiviz_core::MixingUnit>,
    ) {
        ui.horizontal(|ui| {
            ui.heading("Inputs");
            if ui.button("Manage").clicked() {
                self.inputs_open = true;
            }
            ui.menu_button("Add", |ui| {
                if ui.button("Image / Video / NDI / OMT / MixFeed…").clicked() {
                    self.inputs_open = true;
                    ui.close();
                }
            });
        });
        const TILE_W: f32 = 232.0;
        const PREVIEW_H: f32 = 130.0;
        const TILE_H: f32 = PREVIEW_H + 62.0;
        egui::ScrollArea::vertical()
            .id_salt("input-grid")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                ui.horizontal_wrapped(|ui| {
                    for input in project.inputs.values() {
                        let scene_id = scene_id_for_input(project, input.id);
                        let on_preview = unit
                            .and_then(|u| u.preview.scene)
                            .is_some_and(|id| Some(id) == scene_id);
                        let on_program = unit
                            .and_then(|u| u.program.scene)
                            .is_some_and(|id| Some(id) == scene_id);
                        let desired = egui::vec2(TILE_W + 14.0, TILE_H);
                        let (rect, tile_resp) =
                            ui.allocate_exact_size(desired, egui::Sense::click());
                        let mut handled_click = false;
                        ui.scope_builder(
                            egui::UiBuilder::new()
                                .max_rect(rect)
                                .layout(egui::Layout::top_down(egui::Align::LEFT)),
                            |ui| {
                                ui.set_clip_rect(rect);
                                egui::Frame::group(ui.style())
                                    .inner_margin(6.0)
                                    .show(ui, |ui| {
                                        ui.set_width(TILE_W);
                                        ui.set_max_size(egui::vec2(TILE_W, TILE_H));
                                        ui.label(&input.name);
                                        ui.allocate_ui(egui::vec2(TILE_W, PREVIEW_H), |ui| {
                                            ui.set_min_size(egui::vec2(TILE_W, PREVIEW_H));
                                            ui.set_max_size(egui::vec2(TILE_W, PREVIEW_H));
                                            if on_program {
                                                let _ = self.show_program_frame_at(
                                                    ui,
                                                    unit_id,
                                                    &format!("tile-pgm-{}", input.id),
                                                    egui::vec2(TILE_W, PREVIEW_H),
                                                );
                                            } else if on_preview {
                                                let _ = self.show_preview_frame_at(
                                                    ui,
                                                    unit_id,
                                                    &format!("tile-prv-{}", input.id),
                                                    egui::vec2(TILE_W, PREVIEW_H),
                                                );
                                            } else {
                                                ui.weak("off-air");
                                            }
                                        });
                                        ui.horizontal(|ui| {
                                            if ui.button("PRV").clicked() {
                                                handled_click = true;
                                                self.preview_input(unit_id, input.id);
                                            }
                                            if ui
                                                .add_enabled(
                                                    scene_id.is_some(),
                                                    egui::Button::new("CUT"),
                                                )
                                                .clicked()
                                            {
                                                handled_click = true;
                                                if let Some(scene) = scene_id {
                                                    self.selected_scene = Some(scene);
                                                    self.submit(Command::SetProgram {
                                                        unit: unit_id,
                                                        scene: Some(scene),
                                                    });
                                                }
                                            }
                                            if ui.button("FS").clicked() {
                                                handled_click = true;
                                                self.output_windows
                                                    .insert(OutputWindow::Input(input.id));
                                            }
                                            if let InputSource::Video { playback, .. } =
                                                &input.source
                                            {
                                                let mut updated = playback.clone();
                                                if ui
                                                    .button(if playback.playing {
                                                        "Pause"
                                                    } else {
                                                        "Play"
                                                    })
                                                    .clicked()
                                                {
                                                    handled_click = true;
                                                    updated.playing = !playback.playing;
                                                    match self
                                                        .engine
                                                        .set_video_playback(input.id, updated)
                                                    {
                                                        Ok(_) => {
                                                            self.status = format!(
                                                                "video playback: {}",
                                                                input.name
                                                            )
                                                        }
                                                        Err(error) => {
                                                            self.status =
                                                                format!("video playback: {error}")
                                                        }
                                                    }
                                                }
                                            }
                                        });
                                    });
                            },
                        );
                        if tile_resp.clicked() && !handled_click {
                            self.preview_input(unit_id, input.id);
                        }
                        if self.layout_audit.tile.is_none() {
                            self.layout_audit.tile = Some(tile_resp.rect);
                        }
                    }
                });
            });
    }

    fn draw_logs(&self, ui: &mut egui::Ui) {
        ui.heading("Logs");
        ui.label(&self.status);
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for event in self.engine.flight_log().iter().rev().take(300).rev() {
                    ui.monospace(format!(
                        "{} {} {} {}",
                        event.sequence, event.subsystem, event.kind, event.monotonic_nanos
                    ));
                }
            });
    }

    fn draw_output_window(&mut self, ui: &mut egui::Ui, target: OutputWindow, project: &Project) {
        ui.set_min_size(ui.available_size());
        match target {
            OutputWindow::Preview(unit) => {
                let _ = self.show_preview_frame_filled(ui, unit, &format!("fs-prv-{unit}"), true);
            }
            OutputWindow::Program(unit) => {
                let _ = self.show_program_frame_filled(ui, unit, &format!("fs-pgm-{unit}"), true);
            }
            OutputWindow::Multiview(id) => {
                let _ = self.show_multiview_frame_filled(ui, id, &format!("fs-mv-{id}"), true);
            }
            OutputWindow::Input(id) => {
                if let Some(scene) = scene_id_for_input(project, id) {
                    if let Some(unit) = project
                        .mixing_units
                        .values()
                        .find(|u| u.program.scene == Some(scene) || u.preview.scene == Some(scene))
                    {
                        if unit.program.scene == Some(scene) {
                            let _ = self.show_program_frame_filled(
                                ui,
                                unit.id,
                                &format!("fs-input-pgm-{id}"),
                                true,
                            );
                        } else {
                            let _ = self.show_preview_frame_filled(
                                ui,
                                unit.id,
                                &format!("fs-input-prv-{id}"),
                                true,
                            );
                        }
                    } else {
                        ui.label("Input is not on Preview or Program of any mixing unit");
                    }
                } else {
                    ui.label("No scene references this input");
                }
            }
        }
    }

    fn show_aux_viewports(
        &mut self,
        ctx: &egui::Context,
        project: &Project,
        unit: Option<&eiviz_core::MixingUnit>,
    ) {
        if self.logs_open {
            let mut close = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("eiviz-logs"),
                egui::ViewportBuilder::default()
                    .with_title("eiviz Logs")
                    .with_inner_size([720.0, 480.0]),
                |ctx, _class| {
                    egui::CentralPanel::default().show(ctx, |ui| self.draw_logs(ui));
                    if ctx.input(|i| i.viewport().close_requested()) {
                        close = true;
                    }
                },
            );
            if close {
                self.logs_open = false;
            }
        }

        if self.inputs_open {
            let mut close = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("eiviz-inputs"),
                egui::ViewportBuilder::default()
                    .with_title("Input Select")
                    .with_inner_size([960.0, 720.0]),
                |ctx, _class| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        self.draw_input_manager(ui, project, unit.as_ref().map(|u| u.id));
                    });
                    if ctx.input(|i| i.viewport().close_requested()) {
                        close = true;
                    }
                },
            );
            if close {
                self.inputs_open = false;
            }
        }

        if self.outputs_open {
            let mut close = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("eiviz-outputs"),
                egui::ViewportBuilder::default()
                    .with_title("eiviz Outputs")
                    .with_inner_size([720.0, 520.0]),
                |ctx, _class| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        self.draw_output_manager(ui, project, unit.as_ref().map(|u| u.id));
                    });
                    if ctx.input(|i| i.viewport().close_requested()) {
                        close = true;
                    }
                },
            );
            if close {
                self.outputs_open = false;
            }
        }

        if self.scenes_open {
            let mut close = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("eiviz-scenes"),
                egui::ViewportBuilder::default()
                    .with_title("Scenes")
                    .with_inner_size([860.0, 560.0]),
                |ctx, _class| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        self.draw_scene_manager(ui, project, unit.as_ref().map(|u| u.id));
                    });
                    if ctx.input(|i| i.viewport().close_requested()) {
                        close = true;
                    }
                },
            );
            if close {
                self.scenes_open = false;
            }
        }

        let extra_units: Vec<MixingUnitId> = self.switcher_windows.iter().copied().collect();
        for extra in extra_units {
            let mut close = false;
            let title = project
                .mixing_units
                .get(&extra)
                .map(|u| format!("Switcher — {}", u.name))
                .unwrap_or_else(|| "Switcher".into());
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of(("eiviz-switcher", extra)),
                egui::ViewportBuilder::default()
                    .with_title(title)
                    .with_inner_size([1280.0, 800.0]),
                |ctx, _class| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        self.draw_switcher_surface(ui, project, extra);
                    });
                    if ctx.input(|i| i.viewport().close_requested()) {
                        close = true;
                    }
                },
            );
            if close {
                self.switcher_windows.remove(&extra);
            }
        }

        let outputs: Vec<OutputWindow> = self.output_windows.iter().copied().collect();
        for target in outputs {
            let mut close = false;
            let title = match target {
                OutputWindow::Preview(_) => "eiviz Preview",
                OutputWindow::Program(_) => "eiviz Program",
                OutputWindow::Multiview(_) => "eiviz Multiview",
                OutputWindow::Input(_) => "eiviz Input",
            };
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of(format!("{target:?}")),
                egui::ViewportBuilder::default()
                    .with_title(title)
                    .with_fullscreen(true)
                    .with_decorations(false),
                |ctx, _class| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            ui.expand_to_include_rect(ui.max_rect());
                            self.draw_output_window(ui, target, project);
                        });
                    if ctx.input(|i| {
                        i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)
                    }) {
                        close = true;
                    }
                },
            );
            if close {
                self.output_windows.remove(&target);
            }
        }

        let _ = unit;
    }

    fn draw_switcher_surface(
        &mut self,
        ui: &mut egui::Ui,
        project: &Project,
        unit_id: MixingUnitId,
    ) {
        let unit = project.mixing_units.get(&unit_id).cloned();
        let remaining = ui.available_size();
        let bottom_h = (remaining.y * 0.38).clamp(240.0, 380.0);
        let audio_w = (remaining.x * 0.22).clamp(168.0, 260.0);
        let trans_w = 180.0;

        egui::TopBottomPanel::bottom("switcher-bottom")
            .resizable(false)
            .exact_height(bottom_h)
            .show_inside(ui, |ui| {
                self.layout_audit.bottom = Some(ui.max_rect());
                egui::SidePanel::right("switcher-audio")
                    .resizable(false)
                    .exact_width(audio_w)
                    .show_inside(ui, |ui| {
                        self.layout_audit.audio_pane = Some(ui.max_rect());
                        self.draw_audio_meters(ui, project);
                    });
                egui::CentralPanel::default().show_inside(ui, |ui| {
                    self.layout_audit.input_pane = Some(ui.max_rect());
                    self.draw_input_grid(ui, project, unit_id, unit.as_ref());
                });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let top_w = ui.available_width();
            let monitor_w = ((top_w - trans_w) / 2.0).max(160.0);
            egui::SidePanel::left("switcher-preview")
                .resizable(false)
                .exact_width(monitor_w)
                .show_inside(ui, |ui| {
                    let title = unit
                        .as_ref()
                        .and_then(|u| {
                            u.preview
                                .scene
                                .and_then(|id| project.scenes.get(&id).map(|s| s.name.clone()))
                        })
                        .unwrap_or_default();
                    let header = if title.is_empty() {
                        "Preview".into()
                    } else {
                        format!("Preview  {title}")
                    };
                    Self::draw_monitor_chrome(
                        ui,
                        &header,
                        egui::Color32::from_rgb(210, 130, 40),
                        |ui| {
                            let preview = self.show_preview_frame(ui, unit_id);
                            if let (Some(resp), Some(u), Some(scene_id)) = (
                                preview,
                                unit.as_ref(),
                                unit.as_ref().and_then(|u| u.preview.scene),
                            ) {
                                if let Some(scene) = project.scenes.get(&scene_id) {
                                    self.handle_preview_pointer(ui, &resp, scene, unit_id);
                                }
                                let _ = u;
                            }
                        },
                    );
                });
            egui::SidePanel::right("switcher-program")
                .resizable(false)
                .exact_width(monitor_w)
                .show_inside(ui, |ui| {
                    let title = unit
                        .as_ref()
                        .and_then(|u| {
                            u.program
                                .scene
                                .and_then(|id| project.scenes.get(&id).map(|s| s.name.clone()))
                        })
                        .unwrap_or_default();
                    let header = if title.is_empty() {
                        "Program".into()
                    } else {
                        format!("Program  {title}")
                    };
                    Self::draw_monitor_chrome(
                        ui,
                        &header,
                        egui::Color32::from_rgb(50, 160, 80),
                        |ui| {
                            let _ = self.show_program_frame(ui, unit_id);
                        },
                    );
                });
            egui::CentralPanel::default().show_inside(ui, |ui| {
                if let Some(unit) = unit.as_ref() {
                    self.draw_transition_column(ui, unit);
                }
            });
        });
    }

    fn consume_layout_audit_events(&mut self, ctx: &egui::Context) {
        if !self.layout_audit.enabled {
            return;
        }
        let shots: Vec<std::sync::Arc<egui::ColorImage>> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
                .collect()
        });
        for image in shots {
            let path = std::path::Path::new("target/eiviz-layout-audit.png");
            match write_layout_png(&image, path) {
                Ok(()) => tracing::info!("layout audit screenshot {}", path.display()),
                Err(error) => tracing::error!("layout audit screenshot: {error}"),
            }
            if let Err(error) = self.layout_audit.write_report() {
                tracing::error!("layout audit report: {error}");
            }
            self.layout_audit.written = true;
        }
    }

    fn tick_layout_audit(&mut self, ctx: &egui::Context) {
        if !self.layout_audit.enabled {
            return;
        }
        self.layout_audit.frames += 1;
        if self.layout_audit.frames == 45 && !self.layout_audit.screenshot_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.layout_audit.screenshot_requested = true;
            if let Err(error) = self.layout_audit.write_report() {
                tracing::error!("layout audit report: {error}");
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use eiviz_codec_software::{
        EncoderDiagnostics, EncoderSessionRequest, ProgramEncoder, ProgramEncoderFactory,
    };
    use eiviz_io_stream::EncoderCapabilities;
    use eiviz_media::{
        AudioBuffer, EncodedAccessUnit, EncodedKind, EncodedStreamConfig, VideoFrame,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    struct SmokeFactory {
        encodes: Arc<AtomicU64>,
    }

    struct SmokeEncoder {
        config: EncodedStreamConfig,
        encodes: Arc<AtomicU64>,
    }

    impl ProgramEncoderFactory for SmokeFactory {
        fn create(
            &self,
            _request: &EncoderSessionRequest,
        ) -> eiviz_media::Result<Box<dyn ProgramEncoder>> {
            Ok(Box::new(SmokeEncoder {
                config: EncodedStreamConfig {
                    h264_sps: vec![0x67, 66, 0, 31].into(),
                    h264_pps: vec![0x68, 0].into(),
                    aac_audio_specific_config: vec![0x11, 0x90].into(),
                    video_width: 1920,
                    video_height: 1080,
                    video_timescale: 60_000,
                    video_sample_duration: 1001,
                    audio_sample_rate: 48_000,
                    audio_channels: 2,
                },
                encodes: self.encodes.clone(),
            }))
        }

        fn description(&self) -> String {
            "desktop smoke mock".into()
        }
    }

    impl ProgramEncoder for SmokeEncoder {
        fn stream_config(&self) -> &EncodedStreamConfig {
            &self.config
        }

        fn encode(
            &mut self,
            video: &VideoFrame,
            _audio: &AudioBuffer,
        ) -> eiviz_media::Result<Vec<Arc<EncodedAccessUnit>>> {
            self.encodes.fetch_add(1, Ordering::Relaxed);
            Ok(vec![
                Arc::new(EncodedAccessUnit {
                    pts: video.pts,
                    dts: Some(video.pts),
                    keyframe: true,
                    bytes: vec![0, 0, 0, 1, 0x65, 1].into(),
                    kind: EncodedKind::Avc,
                }),
                Arc::new(EncodedAccessUnit {
                    pts: video.pts,
                    dts: Some(video.pts),
                    keyframe: false,
                    bytes: vec![0x21, 0x10].into(),
                    kind: EncodedKind::Aac,
                }),
            ])
        }

        fn request_idr(&mut self) -> eiviz_media::Result<()> {
            Ok(())
        }

        fn diagnostics(&self) -> EncoderDiagnostics {
            EncoderDiagnostics {
                video_backend: "desktop-smoke-avc".into(),
                audio_backend: "desktop-smoke-aac".into(),
                video_frames: self.encodes.load(Ordering::Relaxed),
                keyframes: self.encodes.load(Ordering::Relaxed),
                audio_access_units: self.encodes.load(Ordering::Relaxed),
                ..Default::default()
            }
        }
    }

    #[test]
    fn desktop_mapping_reaches_engine_mock_encoder_and_two_fanout_sinks() {
        let root =
            std::env::temp_dir().join(format!("eiviz-desktop-fanout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let engine = Engine::new("desktop distribution smoke");
        let encodes = Arc::new(AtomicU64::new(0));
        engine
            .install_distribution_encoder_factory(
                Arc::new(SmokeFactory {
                    encodes: encodes.clone(),
                }),
                EncoderCapabilities::dynamic_openh264_fdk(),
            )
            .unwrap();
        let output_a = desktop_distribution_output(
            "mp4",
            root.join("a.mp4").to_str().unwrap(),
            engine.primary_unit(),
            OutputVideoSource::Program,
        )
        .unwrap();
        let output_b = desktop_distribution_output(
            "mp4",
            root.join("b.mp4").to_str().unwrap(),
            engine.primary_unit(),
            OutputVideoSource::Program,
        )
        .unwrap();
        for output in [&output_a, &output_b] {
            engine
                .configure_distribution_output(output.clone())
                .unwrap();
            engine.set_distribution_enabled(output.id, true).unwrap();
        }

        engine.tick().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if engine
                .metrics()
                .distribution_outputs
                .iter()
                .filter(|diagnostic| diagnostic.sent >= 2)
                .count()
                == 2
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(encodes.load(Ordering::Relaxed), 1);
        let diagnostics = engine.metrics().distribution_outputs;
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.sent >= 2)
                .count(),
            2,
            "fanout sinks did not send encoded AUs in time: {diagnostics:?}"
        );
        for output in [&output_a, &output_b] {
            engine.set_distribution_enabled(output.id, false).unwrap();
        }
        engine.tick().unwrap();
        assert!(root.join("a.mp4").metadata().unwrap().len() > 0);
        assert!(root.join("b.mp4").metadata().unwrap().len() > 0);
        let _ = std::fs::remove_dir_all(root);
    }
}

fn install_crash_hook(engine: Arc<Engine>, path: String) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = engine.export_crash_report(
            std::path::Path::new(&path),
            info.to_string(),
            desktop_capabilities(),
        );
        previous(info);
    }));
}

fn desktop_capabilities() -> Vec<CapabilityEntry> {
    let mut capabilities = Vec::new();
    capabilities.push(capability_entry(
        eiviz_io_omt::probe(),
        true,
        false,
        EvidenceState::HilPending,
    ));
    capabilities.push(capability_entry(
        eiviz_io_decklink::probe(),
        cfg!(feature = "decklink"),
        false,
        EvidenceState::HilPending,
    ));
    #[cfg(feature = "ndi")]
    capabilities.push(capability_entry(
        eiviz_io_ndi::probe(),
        true,
        false,
        EvidenceState::HilPending,
    ));
    #[cfg(not(feature = "ndi"))]
    capabilities.push(CapabilityEntry {
        id: "ndi".into(),
        compiled: false,
        available: false,
        active: false,
        detail: "not compiled; NDI 6 SDK/runtime and explicit `ndi` feature required".into(),
        evidence: EvidenceState::HilPending,
    });
    for capability in eiviz_io_audio::probe() {
        capabilities.push(capability_entry(
            capability,
            cfg!(feature = "audio-cpal"),
            false,
            EvidenceState::HilPending,
        ));
    }
    let midi = eiviz_control::midi_capability();
    capabilities.push(CapabilityEntry {
        id: "midi".into(),
        compiled: midi.compiled,
        available: midi.compiled,
        active: false,
        detail: midi.detail.into(),
        evidence: EvidenceState::HilPending,
    });
    capabilities
}

fn capability_entry(
    capability: eiviz_media::Capability,
    compiled: bool,
    active: bool,
    evidence: EvidenceState,
) -> CapabilityEntry {
    CapabilityEntry {
        id: capability.id,
        compiled,
        available: capability.available,
        active,
        detail: capability.detail,
        evidence,
    }
}

fn bootstrap(engine: &Engine) {
    let red = Input {
        id: InputId::new(),
        name: "Red".into(),
        tags: vec!["synth".into()],
        groups: vec![],
        source: InputSource::SolidColor {
            r: 200,
            g: 40,
            b: 40,
            a: 255,
        },
    };
    let bars = Input {
        id: InputId::new(),
        name: "Bars".into(),
        tags: vec!["synth".into()],
        groups: vec![],
        source: InputSource::ColorBars,
    };
    let scene_a = Scene {
        id: SceneId::new(),
        name: "Scene A".into(),
        items: vec![SceneItem {
            id: SceneItemId::new(),
            input: red.id,
            transform: Transform2D::fullscreen(),
            z_order: 0,
            playback: Default::default(),
        }],
    };
    let scene_b = Scene {
        id: SceneId::new(),
        name: "Scene B".into(),
        items: vec![SceneItem {
            id: SceneItemId::new(),
            input: bars.id,
            transform: Transform2D::fullscreen(),
            z_order: 0,
            playback: Default::default(),
        }],
    };
    let _ = engine.submit_payload(Command::AddInput { input: red });
    let _ = engine.submit_payload(Command::AddInput { input: bars });
    let _ = engine.submit_payload(Command::AddScene {
        scene: scene_a.clone(),
    });
    let _ = engine.submit_payload(Command::AddScene { scene: scene_b });
    let unit = engine.primary_unit();
    let _ = engine.submit_payload(Command::SetPreview {
        unit,
        scene: Some(scene_a.id),
    });
    let _ = engine.submit_payload(Command::AddMultiview {
        view: Multiview {
            id: MultiviewId::new(),
            name: "Mix 1 PRV / PGM".into(),
            owner: unit,
            columns: 2,
            rows: 1,
            tiles: vec![
                MultiviewTile {
                    column: 0,
                    row: 0,
                    source: MultiviewSource::Preview(unit),
                },
                MultiviewTile {
                    column: 1,
                    row: 0,
                    source: MultiviewSource::Program(unit),
                },
            ],
        },
    });
}

impl eframe::App for DesktopApp {
    #[allow(clippy::collapsible_if)]
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.consume_layout_audit_events(ctx);
        if let Some(error) = self
            .media_error
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
        {
            self.status = format!("Engine boundary failed: {error}");
            if matches!(
                self.engine.gpu_lifecycle_state(),
                eiviz_engine::GpuLifecycleState::Degraded { .. }
            ) {
                self.engine.require_gpu_restart(
                    "eframe 0.32 does not expose in-place Wgpu RenderState recreation; restart the desktop or inject a newly framework-owned compositor",
                );
            }
        }
        let project = self.engine.snapshot();
        self.sample_fps(&project);
        if !project.mixing_units.contains_key(&self.selected_unit) {
            self.selected_unit = self.engine.primary_unit();
        }
        let unit_id = self.selected_unit;
        let unit = project.mixing_units.get(&unit_id).cloned();
        #[cfg(any(feature = "decklink", feature = "audio-cpal"))]
        let mut device_reassignment = None;

        let mut recover = false;
        let mut discard = false;
        if let Some(prompt) = self.recovery.as_ref() {
            egui::Window::new("Autosave recovery")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| match prompt {
                    RecoveryPrompt::Recoverable {
                        path,
                        project_hash,
                        newer_than_project,
                    } => {
                        ui.label("A divergent autosave was detected.");
                        ui.label(format!("Path: {}", path.display()));
                        ui.label(format!("Project hash: {project_hash}"));
                        ui.label(if *newer_than_project {
                            "The autosave is at least as new as the saved project."
                        } else {
                            "The autosave is older than the saved project."
                        });
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            "The current project has not been replaced. Choose Recover or Discard.",
                        );
                        ui.horizontal(|ui| {
                            recover = ui.button("Recover autosave").clicked();
                            discard = ui.button("Discard autosave").clicked();
                        });
                    }
                    RecoveryPrompt::Corrupt { path, error } => {
                        ui.colored_label(egui::Color32::RED, "The autosave is corrupt.");
                        ui.label(format!("Path: {}", path.display()));
                        ui.label(error);
                        ui.label("The current project remains unchanged.");
                        discard = ui.button("Discard corrupt autosave").clicked();
                    }
                });
        }
        if recover {
            match self.engine.recover_autosave_into(
                std::path::Path::new(&self.save_path),
                Some(std::path::Path::new(&self.asset_root)),
            ) {
                Ok(true) => {
                    self.engine.set_autosave_path(&self.save_path);
                    self.selected_scene = None;
                    self.selected_unit = self.engine.primary_unit();
                    self.asset_diagnostics = self.engine.asset_diagnostics();
                    self.recovery = None;
                    self.rebind_declared_live_sources(
                        "autosave recovered in memory; Save explicitly to replace project.json",
                    );
                }
                Ok(false) => {
                    self.engine.set_autosave_path(&self.save_path);
                    self.recovery = None;
                    self.status = "autosave disappeared before recovery".into();
                }
                Err(error) => self.status = format!("autosave recovery: {error}"),
            }
        } else if discard {
            match self
                .engine
                .discard_autosave(std::path::Path::new(&self.save_path))
            {
                Ok(()) => {
                    self.engine.set_autosave_path(&self.save_path);
                    self.recovery = None;
                    self.status = "autosave discarded; current project unchanged".into();
                }
                Err(error) => self.status = format!("autosave discard: {error}"),
            }
        }

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("eiviz");
                ui.separator();
                ui.label("Mixing Unit");
                let unit_name = unit
                    .as_ref()
                    .map(|u| u.name.clone())
                    .unwrap_or_else(|| "(none)".into());
                egui::ComboBox::from_id_salt("header-mixing-unit")
                    .selected_text(unit_name)
                    .show_ui(ui, |ui| {
                        for candidate in project.mixing_units.values() {
                            ui.selectable_value(
                                &mut self.selected_unit,
                                candidate.id,
                                &candidate.name,
                            );
                        }
                    });
                if ui.button("Add Mixing Unit").clicked() {
                    self.add_mixing_unit();
                }
                if ui.button("Switcher window").clicked() {
                    self.switcher_windows.insert(unit_id);
                }
                ui.separator();
                ui.menu_button("Fullscreen", |ui| {
                    if ui.button("This unit Preview").clicked() {
                        self.output_windows.insert(OutputWindow::Preview(unit_id));
                        ui.close();
                    }
                    if ui.button("This unit Program").clicked() {
                        self.output_windows.insert(OutputWindow::Program(unit_id));
                        ui.close();
                    }
                    for view in project.multiviews.values() {
                        if ui.button(format!("Multiview {}", view.name)).clicked() {
                            self.output_windows.insert(OutputWindow::Multiview(view.id));
                            ui.close();
                        }
                    }
                    for input in project.inputs.values() {
                        if ui.button(format!("Input {}", input.name)).clicked() {
                            self.output_windows.insert(OutputWindow::Input(input.id));
                            ui.close();
                        }
                    }
                });
                ui.separator();
                ui.label(format!("{:?}", project.compositor));
                ui.separator();
                self.draw_fps_badge(ui, &project);
                ui.separator();
                if ui
                    .selectable_label(self.inputs_open, "Inputs")
                    .clicked()
                {
                    self.inputs_open = !self.inputs_open;
                }
                if ui
                    .selectable_label(self.outputs_open, "Outputs")
                    .clicked()
                {
                    self.outputs_open = !self.outputs_open;
                }
                if ui
                    .selectable_label(self.scenes_open, "Scenes")
                    .clicked()
                {
                    self.scenes_open = !self.scenes_open;
                }
                ui.label("path");
                ui.add(egui::TextEdit::singleline(&mut self.save_path).desired_width(180.0));
                if ui.button("Save").clicked() {
                    match self.engine.save(std::path::Path::new(&self.save_path)) {
                        Ok(()) => {
                            self.engine.set_autosave_path(&self.save_path);
                            self.status = "saved".into();
                        }
                        Err(e) => self.status = format!("save: {e}"),
                    }
                }
                if ui.button("Load").clicked() {
                    match self.engine.load_project(
                        std::path::Path::new(&self.save_path),
                        Some(std::path::Path::new(&self.asset_root)),
                    ) {
                        Ok(()) => {
                            self.engine.set_autosave_path(&self.save_path);
                            self.selected_scene = None;
                            self.selected_unit = self.engine.primary_unit();
                            self.asset_diagnostics = self.engine.asset_diagnostics();
                            self.rebind_declared_live_sources("loaded");
                        }
                        Err(e) => self.status = format!("load: {e}"),
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .selectable_label(self.settings_open, "Settings")
                        .clicked()
                    {
                        self.settings_open = !self.settings_open;
                    }
                    if ui.selectable_label(self.logs_open, "Logs").clicked() {
                        self.logs_open = !self.logs_open;
                    }
                    ui.label(format!("rev {}", self.engine.revision()));
                    ui.label(&self.status);
                });
            });
        });

        if self.settings_open {
            let mut close_settings = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("eiviz-settings"),
                egui::ViewportBuilder::default()
                    .with_title("eiviz Settings")
                    .with_inner_size([980.0, 780.0]),
                |ctx, _class| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui.heading("Media ingest");
                            ui.label("Add Image / Video / NDI / OMT / MixFeed from the Inputs window.");
                            ui.label(openh264_link_label(&self.openh264_path));
                            ui.horizontal(|ui| {
                                ui.label("asset root");
                                ui.text_edit_singleline(&mut self.asset_root);
                                if ui.button("Verify asset paths + SHA-256").clicked() {
                                    self.refresh_asset_diagnostics();
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("License-reviewed FDK AAC binary");
                                ui.text_edit_singleline(&mut self.fdk_aac_path);
                            });
                            ui.horizontal(|ui| {
                                ui.label("portable");
                                ui.text_edit_singleline(&mut self.portable_path);
                                if ui.button("Export .eiviz").clicked() {
                                    match self.engine.export_portable(
                                        std::path::Path::new(&self.portable_path),
                                        std::path::Path::new(&self.asset_root),
                                    ) {
                                        Ok(()) => self.status = "portable exported".into(),
                                        Err(error) => {
                                            self.status = format!("portable export: {error}")
                                        }
                                    }
                                }
                                if ui.button("Import .eiviz").clicked() {
                                    match self.engine.import_portable_into(
                                        std::path::Path::new(&self.portable_path),
                                        std::path::Path::new(&self.asset_root),
                                    ) {
                                        Ok(()) => {
                                            self.selected_scene = None;
                                            self.selected_unit = self.engine.primary_unit();
                                            self.asset_diagnostics = self.engine.asset_diagnostics();
                                            self.rebind_declared_live_sources("portable imported");
                                        }
                                        Err(error) => {
                                            self.status = format!("portable import: {error}")
                                        }
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("capability report");
                                ui.text_edit_singleline(&mut self.capability_report_path);
                                if ui.button("Export capabilities").clicked() {
                                    match self.engine.export_capability_report(
                                        std::path::Path::new(&self.capability_report_path),
                                        desktop_capabilities(),
                                    ) {
                                        Ok(()) => self.status = "capability report exported".into(),
                                        Err(error) => {
                                            self.status = format!("capability report: {error}")
                                        }
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("flight recorder");
                                ui.text_edit_singleline(&mut self.diagnostics_export_path);
                                if ui.button("Export diagnostics").clicked() {
                                    match self.engine.export_flight_recorder(std::path::Path::new(
                                        &self.diagnostics_export_path,
                                    )) {
                                        Ok(()) => self.status = "flight recorder exported".into(),
                                        Err(error) => {
                                            self.status = format!("flight recorder: {error}")
                                        }
                                    }
                                }
                                ui.label("crash report");
                                ui.text_edit_singleline(&mut self.crash_report_path);
                                if ui.button("Export crash snapshot").clicked() {
                                    match self.engine.export_crash_report(
                                        std::path::Path::new(&self.crash_report_path),
                                        "manual diagnostic snapshot",
                                        desktop_capabilities(),
                                    ) {
                                        Ok(()) => {
                                            self.status = "crash diagnostic snapshot exported".into()
                                        }
                                        Err(error) => {
                                            self.status = format!("crash report: {error}")
                                        }
                                    }
                                }
                            });
                            ui.separator();
                            ui.heading("Capabilities");
            ui.label(format!("compositor {:?}", project.compositor));
            ui.label(self.engine.compositor_detail());
            ui.separator();
            ui.heading("Video profile");
            let profiles = [
                ("1080p59.94 SDR 8-bit (baseline)", VideoFormat::hd_5994()),
                ("2160p59.94 SDR 8-bit", VideoFormat::uhd_5994_sdr()),
                (
                    "2160p59.94 HDR10 PQ 10-bit",
                    VideoFormat::uhd_5994_hdr10_pq(),
                ),
                (
                    "2160p59.94 HLG 10-bit",
                    VideoFormat::uhd_5994_hlg(),
                ),
                (
                    "1080i59.94 top-field-first",
                    VideoFormat::hd_interlaced_5994(eiviz_core::FieldOrder::TopFieldFirst),
                ),
                (
                    "1080i59.94 bottom-field-first",
                    VideoFormat::hd_interlaced_5994(eiviz_core::FieldOrder::BottomFieldFirst),
                ),
            ];
            let current_label = profiles
                .iter()
                .find_map(|(label, format)| (*format == project.video).then_some(*label))
                .unwrap_or("custom");
            let mut requested = None;
            egui::ComboBox::from_id_salt("video-profile")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    for (label, format) in profiles {
                        if ui.selectable_label(format == project.video, label).clicked() {
                            requested = Some(format);
                        }
                    }
                });
            if let Some(format) = requested {
                match self
                    .engine
                    .submit_payload(Command::SetVideoFormat { format })
                {
                    Ok(_) => self.status = "Video profile staged".into(),
                    Err(error) => {
                        self.status = format!("Video profile rejected (no fallback): {error}")
                    }
                }
            }
            ui.label(format!(
                "{}x{} {:?} {}-bit cadence={} field-order={:?}",
                project.video.width,
                project.video.height,
                project.video.color,
                project.video.bit_depth,
                project.video.frame_rate,
                project.video.field_order
            ));
            let mut conversion = project.video.color_conversion;
            let mut conversion_changed = false;
            let mut gpu_conversion = matches!(
                conversion,
                eiviz_core::ColorConversionPolicy::Gpu { .. }
            );
            if ui
                .checkbox(
                    &mut gpu_conversion,
                    "Explicit WGSL source color conversion",
                )
                .changed()
            {
                conversion_changed = true;
                conversion = if gpu_conversion {
                    eiviz_core::ColorConversionPolicy::Gpu {
                        tone_map: ToneMapPolicy::Disabled,
                    }
                } else {
                    eiviz_core::ColorConversionPolicy::Exact
                };
            }
            if let eiviz_core::ColorConversionPolicy::Gpu { tone_map } = &mut conversion {
                let mut hdr_to_sdr = matches!(tone_map, ToneMapPolicy::HdrToSdr { .. });
                if ui
                    .checkbox(&mut hdr_to_sdr, "Explicit HDR→SDR tone map")
                    .changed()
                {
                    conversion_changed = true;
                    *tone_map = if hdr_to_sdr {
                        ToneMapPolicy::HdrToSdr {
                            source_peak_nits: 1_000,
                            target_nits: 100,
                        }
                    } else {
                        ToneMapPolicy::Disabled
                    };
                }
            }
            if conversion_changed {
                let mut format = project.video.clone();
                format.color_conversion = conversion;
                match self
                    .engine
                    .submit_payload(Command::SetVideoFormat { format })
                {
                    Ok(_) => self.status = "Explicit color policy staged".into(),
                    Err(error) => self.status = format!("Color policy rejected: {error}"),
                }
            }
            let gpu_metrics = self.engine.metrics();
            ui.label(format!(
                "GPU pass: {} ns (frame {} ns, max {}); staging readback: {} ns (max {}, count {})",
                gpu_metrics.gpu_pass_nanos,
                gpu_metrics.gpu_frame_nanos,
                gpu_metrics.gpu_pass_max_nanos,
                gpu_metrics.gpu_readback_nanos,
                gpu_metrics.gpu_readback_max_nanos,
                gpu_metrics.gpu_readbacks,
            ));
            ui.label(format!(
                "GPU lifecycle: {}{}; pool={}/{} bytes, {}/{} resources (idle {}, allocations {}, reuse {}, evictions {}, misses {}, prewarms {}, reinjections {})",
                gpu_metrics.gpu_lifecycle_state,
                gpu_metrics
                    .gpu_lifecycle_detail
                    .as_ref()
                    .map_or_else(String::new, |detail| format!(" — {detail}")),
                gpu_metrics.gpu_pool_resident_bytes,
                gpu_metrics.gpu_pool_limit_bytes,
                gpu_metrics.gpu_pool_resident_resources,
                gpu_metrics.gpu_pool_limit_resources,
                gpu_metrics.gpu_pool_idle_resources,
                gpu_metrics.gpu_pool_allocations,
                gpu_metrics.gpu_pool_reuses,
                gpu_metrics.gpu_pool_evictions,
                gpu_metrics.gpu_pool_acquisition_misses,
                gpu_metrics.gpu_prewarm_generations,
                gpu_metrics.gpu_reinjections,
            ));
            if gpu_metrics.gpu_lifecycle_state == "restart-required" {
                ui.colored_label(
                    egui::Color32::RED,
                    "Wgpu remains selected and rendering is stopped. Restart to obtain a new eframe RenderState; CpuReference is not selected.",
                );
                if ui.button("Close for GPU restart").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            ui.label(format!(
                "Deadline slack: {} ns; misses={}; Program drop={} repeat={}",
                gpu_metrics.deadline_slack_nanos,
                gpu_metrics.deadline_misses,
                gpu_metrics.program_drops,
                gpu_metrics.program_repeats,
            ));
            ui.label(format!(
                "Auxiliary: {}; Preview drop={} decimated={} queue-high={}; Multiview drop={} decimated={} queue-high={}",
                gpu_metrics.auxiliary_load_shedding_state,
                gpu_metrics.dropped_preview,
                gpu_metrics.decimated_preview,
                gpu_metrics.preview_queue_high_water,
                gpu_metrics.dropped_multiview,
                gpu_metrics.decimated_multiview,
                gpu_metrics.multiview_queue_high_water,
            ));
            if let Some(diagnostic) = &gpu_metrics.auxiliary_admission_diagnostic {
                ui.colored_label(egui::Color32::RED, diagnostic);
            }
            ui.separator();
            ui.heading("Preview / Multiview admission");
            let mut shedding = project.auxiliary_load_shedding.clone();
            let mut enabled = matches!(
                shedding,
                AuxiliaryLoadSheddingPolicy::Thresholds(_)
            );
            let mut shedding_changed = ui
                .checkbox(&mut enabled, "Enable explicit auxiliary shedding")
                .changed();
            if shedding_changed {
                shedding = if enabled {
                    AuxiliaryLoadSheddingPolicy::broadcast_default()
                } else {
                    AuxiliaryLoadSheddingPolicy::Disabled
                };
            }
            if let AuxiliaryLoadSheddingPolicy::Thresholds(policy) = &mut shedding {
                ui.label("Deadline feedback (ns)");
                ui.horizontal(|ui| {
                    ui.label("overload ≤");
                    shedding_changed |= ui
                        .add(egui::DragValue::new(&mut policy.overload_slack_nanos))
                        .changed();
                    ui.label("recover ≥");
                    shedding_changed |= ui
                        .add(egui::DragValue::new(&mut policy.recover_slack_nanos))
                        .changed();
                });
                let mut gpu_feedback = policy.gpu_overload_nanos.is_some();
                if ui
                    .checkbox(&mut gpu_feedback, "Use GPU frame timing")
                    .changed()
                {
                    shedding_changed = true;
                    if gpu_feedback {
                        policy.gpu_overload_nanos = Some(6_000_000);
                        policy.gpu_recover_nanos = Some(4_000_000);
                    } else {
                        policy.gpu_overload_nanos = None;
                        policy.gpu_recover_nanos = None;
                    }
                }
                if let (Some(overload), Some(recover)) = (
                    &mut policy.gpu_overload_nanos,
                    &mut policy.gpu_recover_nanos,
                ) {
                    ui.horizontal(|ui| {
                        ui.label("GPU overload ≥");
                        shedding_changed |= ui.add(egui::DragValue::new(overload)).changed();
                        ui.label("recover ≤");
                        shedding_changed |= ui.add(egui::DragValue::new(recover)).changed();
                    });
                }
                ui.horizontal(|ui| {
                    ui.label("escalate frames");
                    shedding_changed |= ui
                        .add(egui::DragValue::new(&mut policy.escalation_frames).range(1..=10_000))
                        .changed();
                    ui.label("recover frames");
                    shedding_changed |= ui
                        .add(egui::DragValue::new(&mut policy.recovery_frames).range(1..=100_000))
                        .changed();
                });
                for (index, tier) in policy.tiers.iter_mut().enumerate() {
                    ui.label(format!("Tier {}", index + 1));
                    ui.horizontal(|ui| {
                        ui.label("PRV cadence /");
                        shedding_changed |= ui
                            .add(
                                egui::DragValue::new(&mut tier.preview_cadence_divisor)
                                    .range(1..=64),
                            )
                            .changed();
                        ui.label("resolution /");
                        shedding_changed |= ui
                            .add(
                                egui::DragValue::new(&mut tier.preview_resolution_divisor)
                                    .range(1..=64),
                            )
                            .changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("MV cadence /");
                        shedding_changed |= ui
                            .add(
                                egui::DragValue::new(&mut tier.multiview_cadence_divisor)
                                    .range(1..=64),
                            )
                            .changed();
                        ui.label("resolution /");
                        shedding_changed |= ui
                            .add(
                                egui::DragValue::new(&mut tier.multiview_resolution_divisor)
                                    .range(1..=64),
                            )
                            .changed();
                    });
                }
            }
            if shedding_changed {
                match self
                    .engine
                    .submit_payload(Command::SetAuxiliaryLoadShedding { policy: shedding })
                {
                    Ok(_) => self.status = "Auxiliary admission policy staged".into(),
                    Err(error) => self.status = format!("Auxiliary admission policy: {error}"),
                }
            }
            ui.label("Program profile/cadence and compositor backend are never changed by this policy.");
            if let Some(error) = &gpu_metrics.last_persistence_error {
                ui.colored_label(
                    egui::Color32::RED,
                    format!(
                        "Persistence errors={}: {error}",
                        gpu_metrics.persistence_errors
                    ),
                );
            }
            if let Some(loss) = &gpu_metrics.gpu_device_loss {
                ui.colored_label(
                    egui::Color32::RED,
                    format!(
                        "GPU device lost: {loss}; automatic recovery={}",
                        gpu_metrics.gpu_automatic_recovery
                    ),
                );
            }
            ui.separator();
            ui.heading("Clock / A/V timing");
            if gpu_metrics.timing_sources.is_empty() {
                ui.label("No attached source clocks");
            }
            for timing in &gpu_metrics.timing_sources {
                let summary = format!(
                    "{}: {} / {}; video={:?} ns audio={:?} ns A/V={:?} ns",
                    timing.input,
                    timing.policy,
                    timing.state,
                    timing.video_skew_nanos,
                    timing.audio_skew_nanos,
                    timing.av_drift_nanos,
                );
                if timing.state == "Locked" {
                    ui.label(summary);
                } else {
                    ui.colored_label(egui::Color32::RED, summary);
                }
                for mapper in &timing.mappers {
                    ui.label(format!(
                        "  {}→{} {} drift={:+} ppb offset={} residual={} obs={} duplicate={} bounded={} reset={} wrap={}",
                        mapper.source_domain,
                        mapper.target_domain,
                        mapper.state,
                        mapper.rate_ppb,
                        mapper.offset_ticks,
                        mapper.residual_ticks,
                        mapper.observations,
                        mapper.duplicates,
                        mapper.bounded_regressions,
                        mapper.discontinuities,
                        mapper.wraps,
                    ));
                }
            }
            ui.label("Clock deadlines use process monotonic only; UTC is not a timing domain.");
            ui.label(format!("missing-media {:?}", project.missing_media));
            if self.asset_diagnostics.is_empty() {
                ui.label("Asset integrity: no missing/hash-mismatched assets detected");
            }
            for diagnostic in &self.asset_diagnostics {
                ui.colored_label(
                    egui::Color32::RED,
                    format!(
                        "Asset {:?}: {} path={} expected-sha256={} actual-sha256={} policy={:?}; no alternate file is substituted",
                        diagnostic.kind,
                        diagnostic.original_name,
                        diagnostic.path,
                        diagnostic.expected_sha256,
                        diagnostic.actual_sha256.as_deref().unwrap_or("<unavailable>"),
                        diagnostic.policy,
                    ),
                );
            }
            ui.label(match self.control_ports {
                Some(ports) => format!(
                    "control HTTP :{} / TCP :{} / WS :{}; auth={}; rate={}/s; command queue={}",
                    ports.http,
                    ports.tcp,
                    ports.websocket,
                    if self.control_token_required {
                        "required"
                    } else {
                        "loopback-only"
                    },
                    self.control_rate,
                    self.control_queue_capacity
                ),
                None => "control disabled".into(),
            });
            let midi = eiviz_control::midi_capability();
            ui.label(format!(
                "MIDI: {} ({})",
                if midi.compiled {
                    "compiled"
                } else {
                    "unavailable"
                },
                midi.detail
            ));
            let omt = eiviz_io_omt::probe();
            ui.label(format!(
                "{}: {}",
                omt.id,
                if omt.available {
                    "ready"
                } else {
                    "unavailable"
                }
            ));
            for capability in self.engine.distribution_capabilities() {
                ui.label(format!(
                    "{}: {} ({})",
                    capability.id,
                    if capability.available {
                        "ready"
                    } else {
                        "unavailable"
                    },
                    capability.detail
                ));
            }
            #[cfg(feature = "decklink")]
            ui.label(format!(
                "{}: {} ({})",
                self.decklink_capability.id,
                if self.decklink_capability.available {
                    "ready"
                } else {
                    "unavailable"
                },
                self.decklink_capability.detail
            ));
            #[cfg(not(feature = "decklink"))]
            {
                let capability = eiviz_io_decklink::probe();
                ui.label(format!("{}: unavailable ({})", capability.id, capability.detail));
            }
            #[cfg(feature = "ndi")]
            ui.label(format!(
                "{}: {} ({})",
                self.ndi_capability.id,
                if self.ndi_capability.available {
                    "ready"
                } else {
                    "unavailable"
                },
                self.ndi_capability.detail
            ));
            #[cfg(not(feature = "ndi"))]
            ui.label("NDI: not compiled (enable the explicit `ndi` feature)");
            for cap in eiviz_io_audio::probe() {
                ui.label(format!(
                    "{}: {} ({})",
                    cap.id,
                    if cap.available {
                        "ready"
                    } else {
                        "unavailable"
                    },
                    cap.detail
                ));
            }
            ui.separator();
            ui.heading("Video output routing");
            let selected_source = output_feed_label(&project, self.output_source);
            egui::ComboBox::from_id_salt("video-output-source")
                .selected_text(selected_source)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            self.output_source == OutputVideoSource::Preview,
                            "Preview",
                        )
                        .clicked()
                    {
                        self.output_source = OutputVideoSource::Preview;
                    }
                    if ui
                        .selectable_label(
                            self.output_source == OutputVideoSource::Program,
                            "Program",
                        )
                        .clicked()
                    {
                        self.output_source = OutputVideoSource::Program;
                    }
                    for view in project
                        .multiviews
                        .values()
                        .filter(|view| view.owner == unit_id)
                    {
                        let source = OutputVideoSource::Multiview(view.id);
                        if ui
                            .selectable_label(
                                self.output_source == source,
                                format!("Multiview: {}", view.name),
                            )
                            .clicked()
                        {
                            self.output_source = source;
                        }
                    }
                });
            ui.label(
                "New NDI/OMT outputs can be started per Preview, Program, and each Multiview. Use the Outputs window to start several feeds at once.",
            );
            ui.separator();
            ui.heading("Distribution");
            ui.label(
                "Explicit baseline: hash/version-verified Cisco OpenH264 2.6.0 + raw FDK AAC-LC. FDK's upstream license grants no patent rights; use only a reviewed binary. No fallback.",
            );
            ui.label(openh264_link_label(&self.openh264_path));
            ui.label("License-reviewed FDK AAC dynamic binary");
            ui.text_edit_singleline(&mut self.fdk_aac_path);
            ui.label("RTMP H.264/AAC in FLV");
            ui.text_edit_singleline(&mut self.rtmp_url);
            if ui.button("Add stopped RTMP mapping").clicked() {
                self.configure_distribution("rtmp", unit_id);
            }
            ui.label("SRT H.264/AAC in MPEG-TS");
            ui.text_edit_singleline(&mut self.srt_url);
            if ui.button("Add stopped SRT mapping").clicked() {
                self.configure_distribution("srt", unit_id);
            }
            ui.label("Fragmented MP4 recording");
            ui.text_edit_singleline(&mut self.recording_path);
            if ui.button("Add stopped fMP4 mapping").clicked() {
                self.configure_distribution("mp4", unit_id);
            }
            let distribution_metrics = self.engine.metrics().distribution_outputs;
            let mut remove_distribution = None;
            for output in project
                .outputs
                .values()
                .filter(|output| output.distribution.is_some())
            {
                let diagnostic = distribution_metrics
                    .iter()
                    .find(|diagnostic| diagnostic.output_id == output.id.to_string());
                ui.push_id(output.id, |ui| {
                    ui.label(format!(
                        "{}: {} — {}; queue={}/{} high={} sent={} drop={} reconnect={}; AVC={} IDR={} AAC={} IDR-requests={}",
                        output.name,
                        diagnostic.map_or("unknown", |value| value.state.as_str()),
                        diagnostic.map_or("no diagnostics", |value| value.detail.as_str()),
                        diagnostic.map_or(0, |value| value.queue_depth),
                        output
                            .distribution
                            .as_ref()
                            .map_or(0, |profile| profile.queue_capacity),
                        diagnostic.map_or(0, |value| value.queue_high_water),
                        diagnostic.map_or(0, |value| value.sent),
                        diagnostic.map_or(0, |value| value.dropped),
                        diagnostic.map_or(0, |value| value.reconnects),
                        diagnostic.map_or(0, |value| value.video_frames),
                        diagnostic.map_or(0, |value| value.keyframes),
                        diagnostic.map_or(0, |value| value.audio_access_units),
                        diagnostic.map_or(0, |value| value.idr_requests),
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("Start").clicked() {
                            let any_running = project.outputs.values().any(|candidate| {
                                candidate.enabled && candidate.distribution.is_some()
                            });
                            if !any_running {
                                let fdk_path = (!self.fdk_aac_path.trim().is_empty()).then(|| {
                                    std::path::PathBuf::from(self.fdk_aac_path.trim())
                                });
                                if let Err(error) = self.engine.configure_distribution_binaries(
                                    std::path::PathBuf::from(self.openh264_path.trim()),
                                    fdk_path,
                                ) {
                                    self.status =
                                        format!("encoder binary configuration: {error}");
                                    return;
                                }
                            }
                            match self.engine.set_distribution_enabled(output.id, true) {
                                Ok(_) => self.status = format!("{} started", output.name),
                                Err(error) => {
                                    self.status = format!("{} not started: {error}", output.name)
                                }
                            }
                        }
                        if ui.button("Stop").clicked() {
                            match self.engine.set_distribution_enabled(output.id, false) {
                                Ok(_) => self.status = format!("{} stopped", output.name),
                                Err(error) => self.status = format!("stop {}: {error}", output.name),
                            }
                        }
                        if ui.button("Remove").clicked() {
                            remove_distribution = Some(output.id);
                        }
                    });
                });
            }
            if let Some(output) = remove_distribution {
                match self
                    .engine
                    .submit_payload(Command::RemoveOutput { id: output })
                {
                    Ok(_) => self.status = "distribution mapping removed".into(),
                    Err(error) => self.status = format!("remove distribution mapping: {error}"),
                }
            }
            ui.separator();
            ui.heading("Audio I/O");
            ui.label(format!(
                "Project: {} Hz / {} ch; policy {:?}",
                project.audio.sample_rate, project.audio.channels, project.audio.resampling
            ));
            ui.horizontal(|ui| {
                let policies = [
                    ("ExactRate", AudioResamplingPolicy::ExactRate),
                    (
                        "ASRC Broadcast",
                        AudioResamplingPolicy::Asrc {
                            profile: AsrcProfile::broadcast(),
                        },
                    ),
                    (
                        "ASRC Mastering",
                        AudioResamplingPolicy::Asrc {
                            profile: AsrcProfile::mastering(),
                        },
                    ),
                ];
                for (label, policy) in policies {
                    if ui
                        .selectable_label(project.audio.resampling == policy, label)
                        .clicked()
                    {
                        match self
                            .engine
                            .submit_payload(Command::SetAudioResampling { policy })
                        {
                            Ok(_) => self.status = format!("Audio resampling selected: {label}"),
                            Err(error) => {
                                self.status = format!("Audio resampling policy: {error}")
                            }
                        }
                    }
                }
            });
            ui.label("Rate conversion is disabled under ExactRate; a mismatch is a hard error.");
            let audio_metrics = self.engine.metrics();
            for diagnostic in &audio_metrics.audio_resamplers {
                ui.label(format!(
                    "ASRC {}: {}→{} Hz ratio={:.9} drift={:+.1} ppm buffer={}/{} under={} over={} reset={}",
                    diagnostic.endpoint,
                    diagnostic.input_rate,
                    diagnostic.output_rate,
                    diagnostic.ratio,
                    diagnostic.drift_ppm,
                    diagnostic.buffered_frames,
                    diagnostic.buffer_capacity_frames,
                    diagnostic.queue_underflows,
                    diagnostic.queue_overflows,
                    diagnostic.discontinuities,
                ));
            }
            #[cfg(feature = "audio-cpal")]
            {
                ui.label(
                    "Select one backend explicitly. Device IDs are persisted; no host/device fallback.",
                );
                for (index, backend) in self.audio_backends.iter().enumerate() {
                    if ui
                        .selectable_label(
                            self.audio_backend_selected == Some(index),
                            backend.id(),
                        )
                        .clicked()
                    {
                        self.audio_backend_selected = Some(index);
                        self.audio_devices.clear();
                        self.audio_input_selected = None;
                        self.audio_output_selected = None;
                    }
                }
                if ui.button("Refresh audio devices").clicked() {
                    self.refresh_audio_devices();
                }
                ui.label("Capture");
                for (index, device) in self.audio_devices.iter().enumerate() {
                    if device.supports_input
                        && ui
                            .selectable_label(
                                self.audio_input_selected == Some(index),
                                format!(
                                    "{} [{}]{}",
                                    device.display_name,
                                    device.persistent_id,
                                    if device.default_input { " (default)" } else { "" }
                                ),
                            )
                            .clicked()
                    {
                        self.audio_input_selected = Some(index);
                    }
                }
                if ui.button("Start audio capture").clicked() {
                    self.start_audio_input();
                }
                if let Some(device) = self
                    .audio_input_selected
                    .and_then(|index| self.audio_devices.get(index))
                {
                    for input in project.inputs.values() {
                        let InputSource::AudioDevice { binding } = &input.source else {
                            continue;
                        };
                        if let Some(current) = project.device_bindings.get(binding)
                            && ui
                                .button(format!(
                                    "Reassign {} [{}] → exact [{}]",
                                    input.name,
                                    current
                                        .last_seen_hardware_id
                                        .as_deref()
                                        .unwrap_or("<missing>"),
                                    device.persistent_id
                                ))
                                .clicked()
                        {
                            device_reassignment = Some((
                                *binding,
                                device.persistent_id.clone(),
                                device.display_name.clone(),
                            ));
                        }
                    }
                }
                let mut stop_input = None;
                for (input_id, source) in &self.audio_inputs {
                    let diagnostic = source.diagnostics();
                    ui.push_id(input_id, |ui| {
                        ui.label(format!(
                            "{}: {:?}; callbacks={} frames={} xruns={} over={} under={} sample={} capture={}ns{}",
                            diagnostic.name,
                            diagnostic.health,
                            diagnostic.callbacks,
                            diagnostic.device_frames,
                            diagnostic.xruns,
                            diagnostic.queue_overflows,
                            diagnostic.queue_underflows,
                            diagnostic.last_device_sample_index,
                            diagnostic.last_device_nanos,
                            diagnostic
                                .last_error
                                .as_deref()
                                .map_or(String::new(), |error| format!("; {error}")),
                        ));
                        if ui.button("Stop audio capture").clicked() {
                            stop_input = Some(*input_id);
                        }
                    });
                }
                if let Some(input_id) = stop_input {
                    let snapshot = self.engine.snapshot();
                    for route in snapshot
                        .audio_matrix
                        .routes
                        .iter()
                        .filter(|route| route.input == input_id)
                    {
                        let _ = self.engine.submit_payload(Command::ClearAudioRoute {
                            input: input_id,
                            bus: route.bus,
                        });
                    }
                    let _ = self
                        .engine
                        .submit_payload(Command::RemoveInput { id: input_id });
                    self.engine.detach_source(input_id);
                    self.audio_inputs.retain(|(id, _)| *id != input_id);
                    self.status = "Audio capture stopped".into();
                }
                ui.label("Master output");
                for (index, device) in self.audio_devices.iter().enumerate() {
                    if device.supports_output
                        && ui
                            .selectable_label(
                                self.audio_output_selected == Some(index),
                                format!(
                                    "{} [{}]{}",
                                    device.display_name,
                                    device.persistent_id,
                                    if device.default_output { " (default)" } else { "" }
                                ),
                            )
                            .clicked()
                    {
                        self.audio_output_selected = Some(index);
                    }
                }
                if ui.button("Start audio output").clicked() {
                    self.start_audio_output(unit_id);
                }
                if let Some(device) = self
                    .audio_output_selected
                    .and_then(|index| self.audio_devices.get(index))
                {
                    for output in project.outputs.values() {
                        let OutputKind::AudioDevice { binding } = &output.kind else {
                            continue;
                        };
                        if let Some(current) = project.device_bindings.get(binding)
                            && ui
                                .button(format!(
                                    "Reassign {} [{}] → exact [{}]",
                                    output.name,
                                    current
                                        .last_seen_hardware_id
                                        .as_deref()
                                        .unwrap_or("<missing>"),
                                    device.persistent_id
                                ))
                                .clicked()
                        {
                            device_reassignment = Some((
                                *binding,
                                device.persistent_id.clone(),
                                device.display_name.clone(),
                            ));
                        }
                    }
                }
                let mut stop_output = None;
                for (output_id, sink) in &self.audio_outputs {
                    let diagnostic = sink.diagnostics();
                    ui.push_id(output_id, |ui| {
                        ui.label(format!(
                            "{}: {:?}; callbacks={} frames={} xruns={} over={} under={} sample={} playback={}ns{}",
                            diagnostic.name,
                            diagnostic.health,
                            diagnostic.callbacks,
                            diagnostic.device_frames,
                            diagnostic.xruns,
                            diagnostic.queue_overflows,
                            diagnostic.queue_underflows,
                            diagnostic.last_device_sample_index,
                            diagnostic.last_device_nanos,
                            diagnostic
                                .last_error
                                .as_deref()
                                .map_or(String::new(), |error| format!("; {error}")),
                        ));
                        if ui.button("Stop audio output").clicked() {
                            stop_output = Some(*output_id);
                        }
                    });
                }
                if let Some(output_id) = stop_output {
                    let _ = self
                        .engine
                        .submit_payload(Command::RemoveOutput { id: output_id });
                    self.engine.detach_audio_output(output_id);
                    self.audio_outputs.retain(|(id, _)| *id != output_id);
                    self.status = "Audio output stopped".into();
                }
            }
            #[cfg(not(feature = "audio-cpal"))]
            ui.label(
                "Build with `--features audio-cpal`; use `audio-pipewire` or the separately licensed `audio-asio` profile when required.",
            );
            ui.separator();
            ui.heading("MIDI Control");
            #[cfg(feature = "midi")]
            {
                ui.label(
                    "Select one physical input by its backend ID. The mapping below emits a versioned TAKE CommandEnvelope; no default-device fallback is used.",
                );
                if ui.button("Refresh MIDI inputs").clicked() {
                    self.refresh_midi_inputs();
                }
                for (index, device) in self.midi_devices.iter().enumerate() {
                    if ui
                        .selectable_label(
                            self.midi_selected == Some(index),
                            format!("{} [{}]", device.name, device.id),
                        )
                        .clicked()
                    {
                        self.midi_selected = Some(index);
                    }
                }
                ui.horizontal(|ui| {
                    ui.label("channel (1-16)");
                    let mut display_channel = self.midi_channel + 1;
                    if ui
                        .add(egui::DragValue::new(&mut display_channel).range(1..=16))
                        .changed()
                    {
                        self.midi_channel = display_channel - 1;
                    }
                    ui.label("TAKE note");
                    ui.add(egui::DragValue::new(&mut self.midi_take_note).range(0..=127));
                });
                ui.horizontal(|ui| {
                    if ui.button("Start MIDI TAKE mapping").clicked() {
                        self.start_midi_take();
                    }
                    if ui.button("Stop MIDI").clicked() {
                        self.midi_handle = None;
                        self.status = "MIDI stopped".into();
                    }
                });
                if let Some(handle) = &self.midi_handle {
                    let status = handle.status();
                    ui.label(format!(
                        "{} [{}]: received={} matched={} queue-overflow={} submit-errors={}{}",
                        status.device_name,
                        status.device_id,
                        status.received_messages,
                        status.matched_messages,
                        status.queue_overflows,
                        status.submit_errors,
                        status
                            .last_error
                            .as_deref()
                            .map_or(String::new(), |error| format!("; {error}"))
                    ));
                }
            }
            #[cfg(not(feature = "midi"))]
            ui.label(
                "Build with `--features midi` to compile the platform midir backend. No no-op MIDI listener is present in portable builds.",
            );
            ui.separator();
            ui.heading("OMT");
            ui.label("Add OMT inputs from the Inputs window.");
            for (_id, source) in &self.omt_connections {
                let detail = source
                    .last_error()
                    .unwrap_or_else(|| "no adapter error".into());
                let control = eiviz_media::MediaSource::control_diagnostics(source.as_ref())
                    .unwrap_or_default();
                let input = eiviz_media::MediaSource::id(source.as_ref());
                let tally = self
                    .engine
                    .source_control_diagnostics()
                    .into_iter()
                    .find(|diagnostics| diagnostics.input == input)
                    .map(|diagnostics| diagnostics.tally)
                    .unwrap_or_default();
                ui.label(format!(
                    "{}: {:?}; tally PRV={} PGM={}; reconnects={} discontinuities={}; metadata={} dropped={}; {detail}",
                    source.address(),
                    source.health(),
                    tally.preview,
                    tally.program,
                    control.reconnects,
                    control.discontinuities,
                    control.metadata_received,
                    control.metadata_dropped,
                ));
            }
            for metadata in self
                .engine
                .source_metadata()
                .iter()
                .filter(|metadata| metadata.protocol == "omt")
                .rev()
                .take(5)
            {
                ui.label(format!(
                    "OMT metadata {:?}: {}",
                    metadata.categories, metadata.payload
                ));
            }
            ui.separator();
            ui.heading("DeckLink 1080p59.94");
            #[cfg(feature = "decklink")]
            {
                if ui.button("Refresh DeckLink devices").clicked() {
                    self.refresh_decklink();
                }
                ui.label("Capture");
                for (index, device) in self.decklink_devices.iter().enumerate() {
                    if !device.supports_capture {
                        continue;
                    }
                    if ui
                        .selectable_label(
                            self.decklink_capture_selected == Some(index),
                            format!("{} [{}]", device.display_name, device.persistent_id),
                        )
                        .clicked()
                    {
                        self.decklink_capture_selected = Some(index);
                    }
                }
                if ui.button("Start DeckLink Capture").clicked() {
                    self.connect_decklink_capture();
                }
                if let Some(device) = self
                    .decklink_capture_selected
                    .and_then(|index| self.decklink_devices.get(index))
                {
                    for input in project.inputs.values() {
                        let InputSource::DeckLink { binding } = &input.source else {
                            continue;
                        };
                        if let Some(current) = project.device_bindings.get(binding)
                            && ui
                                .button(format!(
                                    "Reassign {} [{}] → exact [{}]",
                                    input.name,
                                    current
                                        .last_seen_hardware_id
                                        .as_deref()
                                        .unwrap_or("<missing>"),
                                    device.persistent_id
                                ))
                                .clicked()
                        {
                            device_reassignment = Some((
                                *binding,
                                device.persistent_id.clone(),
                                device.display_name.clone(),
                            ));
                        }
                    }
                }
                for source in &self.decklink_sources {
                    let (video_drops, audio_drops) = source.dropped_frames();
                    let detail = source
                        .last_error()
                        .unwrap_or_else(|| "no adapter error".into());
                    ui.label(format!(
                        "{}: {:?}; queue drops video={video_drops} audio={audio_drops}; {detail}",
                        source.device().display_name,
                        source.health()
                    ));
                }
                ui.label("Scheduled program output");
                for (index, device) in self.decklink_devices.iter().enumerate() {
                    if !device.supports_playback {
                        continue;
                    }
                    if ui
                        .selectable_label(
                            self.decklink_playback_selected == Some(index),
                            format!("{} [{}]", device.display_name, device.persistent_id),
                        )
                        .clicked()
                    {
                        self.decklink_playback_selected = Some(index);
                    }
                }
                if ui.button("Start DeckLink Output").clicked() {
                    self.start_decklink_output(unit_id);
                }
                if let Some(device) = self
                    .decklink_playback_selected
                    .and_then(|index| self.decklink_devices.get(index))
                {
                    for output in project.outputs.values() {
                        let OutputKind::DeckLink { binding } = &output.kind else {
                            continue;
                        };
                        if let Some(current) = project.device_bindings.get(binding)
                            && ui
                                .button(format!(
                                    "Reassign {} [{}] → exact [{}]",
                                    output.name,
                                    current
                                        .last_seen_hardware_id
                                        .as_deref()
                                        .unwrap_or("<missing>"),
                                    device.persistent_id
                                ))
                                .clicked()
                        {
                            device_reassignment = Some((
                                *binding,
                                device.persistent_id.clone(),
                                device.display_name.clone(),
                            ));
                        }
                    }
                }
                let mut stop_output = None;
                for (output_id, sink) in &self.decklink_outputs {
                    let diagnostics = sink.diagnostics();
                    let reference = diagnostics
                        .reference_locked
                        .map_or("unknown", |locked| if locked { "locked" } else { "unlocked" });
                    let detail = sink
                        .last_error()
                        .unwrap_or_else(|| "no adapter error".into());
                    ui.push_id(output_id, |ui| {
                        ui.label(format!(
                            "{}: {:?}; ref={reference}; buffered v={} a={}; complete={} late={} dropped={} flushed={} queue-full={}; {detail}",
                            sink.device().display_name,
                            sink.health(),
                            diagnostics.buffered_video,
                            diagnostics.buffered_audio_frames,
                            diagnostics.completed_video,
                            diagnostics.late_video,
                            diagnostics.dropped_video,
                            diagnostics.flushed_video,
                            diagnostics.queue_rejections,
                        ));
                        if ui.button("Stop DeckLink Output").clicked() {
                            stop_output = Some(*output_id);
                        }
                    });
                }
                if let Some(output_id) = stop_output {
                    let _ = self
                        .engine
                        .submit_payload(Command::RemoveOutput { id: output_id });
                    self.engine.detach_output_sink(output_id);
                    self.decklink_outputs.retain(|(id, _)| *id != output_id);
                    self.status = "DeckLink output stopped".into();
                }
            }
            #[cfg(not(feature = "decklink"))]
            ui.label(
                "Build with `--features decklink` against an installed Desktop Video SDK 16. \
                 No simulator or backend fallback is available.",
            );
            ui.separator();
            ui.horizontal(|ui| {
                ui.heading("NDI");
                ui.hyperlink_to("ndi.video", "https://ndi.video/");
            });
            #[cfg(feature = "ndi")]
            {
                ui.label("Add NDI inputs from the Inputs window.");
                for (_id, source) in &self.ndi_connections {
                    let detail = source
                        .last_error()
                        .unwrap_or_else(|| "no adapter error".into());
                    let (video_drops, audio_drops) = source.dropped_frames();
                    let control = eiviz_media::MediaSource::control_diagnostics(source.as_ref())
                        .unwrap_or_default();
                    ui.label(format!(
                        "{}: {:?}, drops video={video_drops} audio={audio_drops}; reconnects={} discontinuities={}; metadata={} dropped={}; {}; {detail}",
                        source.source_name(),
                        source.health(),
                        control.reconnects,
                        control.discontinuities,
                        control.metadata_received,
                        control.metadata_dropped,
                        source.tally_support(),
                    ));
                }
                for metadata in self
                    .engine
                    .source_metadata()
                    .iter()
                    .filter(|metadata| metadata.protocol == "ndi")
                    .rev()
                    .take(5)
                {
                    ui.label(format!("NDI metadata: {}", metadata.payload));
                }
                ui.label("Program output");
                ui.text_edit_singleline(&mut self.ndi_output_name);
                ui.checkbox(
                    &mut self.ndi_output_nv12,
                    "NV12 BT.709 limited output (otherwise RGBA)",
                );
                if ui.button("Start NDI Output").clicked() {
                    self.start_ndi_output(unit_id, self.selected_output_source());
                }
                let mut stop_output = None;
                for (output_id, sink) in &self.ndi_outputs {
                    let detail = sink
                        .last_error()
                        .unwrap_or_else(|| "no adapter error".into());
                    ui.push_id(output_id, |ui| {
                        let mut enabled = project
                            .outputs
                            .get(output_id)
                            .is_some_and(|output| output.enabled);
                        if ui.checkbox(&mut enabled, "Enabled").changed() {
                            let _ = self.engine.submit_payload(Command::SetOutputEnabled {
                                id: *output_id,
                                enabled,
                            });
                        }
                        ui.label(format!(
                            "{}: {:?}, drops={}, receiver tally={:?} ({detail})",
                            eiviz_media::MediaSink::name(sink.as_ref()),
                            sink.health(),
                            sink.dropped_frames(),
                            sink.receiver_tally(),
                        ));
                        if let Some(output) = project.outputs.get(output_id)
                            && let Some(command) = output_color_format_combo(ui, output)
                        {
                            let _ = self.engine.submit_payload(command);
                        }
                        if ui.button("Stop NDI Output").clicked() {
                            stop_output = Some(*output_id);
                        }
                    });
                }
                if let Some(output_id) = stop_output {
                    let _ = self
                        .engine
                        .submit_payload(Command::RemoveOutput { id: output_id });
                    self.engine.detach_output_sink(output_id);
                    self.ndi_outputs.retain(|(id, _)| *id != output_id);
                    self.status = "NDI output stopped".into();
                }
            }
            #[cfg(not(feature = "ndi"))]
            ui.label("Build with `--features ndi` after installing the NDI 6 SDK/runtime.");
            ui.label("OMT Program output");
            ui.text_edit_singleline(&mut self.omt_output_name);
            ui.checkbox(&mut self.omt_output_uyvy, "UYVY 4:2:2 (otherwise BGRA)");
            if ui.button("Start OMT Output").clicked() {
                self.start_omt_output(unit_id, self.selected_output_source());
            }
            if let Some(u) = &unit {
                ui.separator();
                ui.heading("Mixing Unit");
                egui::ComboBox::from_id_salt("selected-mixing-unit")
                    .selected_text(&u.name)
                    .show_ui(ui, |ui| {
                        for candidate in project.mixing_units.values() {
                            ui.selectable_value(
                                &mut self.selected_unit,
                                candidate.id,
                                &candidate.name,
                            );
                        }
                    });
                if ui.button("Add Mixing Unit").clicked() {
                    self.add_mixing_unit();
                }
                ui.label(format!("PRV {:?}", u.preview.scene));
                ui.label(format!("PGM {:?}", u.program.scene));
                ui.heading("Audio Follow");
                ui.horizontal(|ui| {
                    for (label, policy) in [
                        ("Off", AudioFollowPolicy::Off),
                        ("Program", AudioFollowPolicy::Program),
                        ("Program+Preview", AudioFollowPolicy::ProgramAndPreview),
                    ] {
                        if ui
                            .selectable_label(u.audio_follow == policy, label)
                            .clicked()
                        {
                            self.submit(Command::SetAudioFollow {
                                unit: u.id,
                                policy,
                            });
                        }
                    }
                });
                ui.heading("Overlays");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.overlay_name_draft);
                    if ui.button("Add Overlay").clicked() {
                        self.add_overlay();
                    }
                });
                for overlay in &u.overlays {
                    ui.push_id(overlay.id, |ui| {
                        ui.horizontal(|ui| {
                            let mut on = overlay.enabled;
                            if ui.checkbox(&mut on, &overlay.name).changed() {
                                self.submit(Command::SetOverlayEnabled {
                                    unit: u.id,
                                    overlay: overlay.id,
                                    enabled: on,
                                });
                            }
                            let mut assigned = overlay.scene;
                            egui::ComboBox::from_id_salt(format!("overlay-scene-{}", overlay.id))
                                .selected_text(match assigned {
                                    Some(id) => project
                                        .scenes
                                        .get(&id)
                                        .map(|scene| scene.name.clone())
                                        .unwrap_or_else(|| id.to_string()),
                                    None => "(none)".into(),
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut assigned, None, "(none)");
                                    for scene in project.scenes.values() {
                                        ui.selectable_value(
                                            &mut assigned,
                                            Some(scene.id),
                                            &scene.name,
                                        );
                                    }
                                });
                            if assigned != overlay.scene {
                                self.submit(Command::SetOverlayScene {
                                    unit: u.id,
                                    overlay: overlay.id,
                                    scene: assigned,
                                });
                            }
                        });
                    });
                }
            }
                        });
                    });
                    if ctx.input(|i| i.viewport().close_requested()) {
                        close_settings = true;
                    }
                },
            );
            if close_settings {
                self.settings_open = false;
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_switcher_surface(ui, &project, unit_id);
        });
        self.show_aux_viewports(ctx, &project, unit.as_ref());
        #[cfg(any(feature = "decklink", feature = "audio-cpal"))]
        if let Some((binding, hardware_id, logical_name)) = device_reassignment {
            self.reassign_device_binding(binding, hardware_id, logical_name);
        }
        self.tick_layout_audit(ctx);
        ctx.request_repaint_after(std::time::Duration::from_millis(17));
    }
}

impl Drop for DesktopApp {
    fn drop(&mut self) {
        self.media_stop.store(true, Ordering::Release);
        self.control_stop.store(true, Ordering::Release);
        if let Some(thread) = self.media_thread.take() {
            let _ = thread.join();
        }
    }
}

fn bundled_openh264_file_name() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("openh264-2.6.0-win64.dll")
    } else if cfg!(all(windows, target_arch = "x86")) {
        Some("openh264-2.6.0-win32.dll")
    } else if cfg!(all(windows, target_arch = "aarch64")) {
        Some("openh264-2.6.0-win-arm64.dll")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("libopenh264-2.6.0-linux64.8.so")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("libopenh264-2.6.0-mac-x64.dylib")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("libopenh264-2.6.0-mac-arm64.dylib")
    } else {
        None
    }
}

fn resolve_openh264_binary() -> String {
    if let Ok(path) = std::env::var("EIVIZ_OPENH264_PATH") {
        let trimmed = path.trim();
        if !trimmed.is_empty() && std::path::Path::new(trimmed).is_file() {
            return trimmed.to_owned();
        }
    }
    let Some(file_name) = bundled_openh264_file_name() else {
        return String::new();
    };
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(file_name));
            candidates.push(dir.join("runtime").join("openh264").join(file_name));
        }
    }
    candidates.push(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("runtime")
            .join("openh264")
            .join(file_name),
    );
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn openh264_link_label(path: &str) -> String {
    if path.is_empty() {
        "OpenH264 2.6.0: not linked".into()
    } else {
        format!(
            "OpenH264 2.6.0 linked ({})",
            std::path::Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
        )
    }
}

fn default_user_dir(sub: &str) -> PathBuf {
    if let Ok(profile) = std::env::var("USERPROFILE") {
        let candidate = PathBuf::from(&profile).join(sub);
        if candidate.is_dir() {
            return candidate;
        }
        return PathBuf::from(profile);
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn default_media_dir() -> PathBuf {
    default_user_dir("Videos")
}

fn default_pictures_dir() -> PathBuf {
    default_user_dir("Pictures")
}

fn list_dir_entries(dir: &Path, exts: &[&str]) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (dirs, files);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
            continue;
        }
        let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };
        let ext = ext.to_ascii_lowercase();
        if exts.iter().any(|want| ext == *want) {
            files.push(path);
        }
    }
    dirs.sort();
    files.sort();
    (dirs, files)
}

fn output_feed_label(project: &Project, source: OutputVideoSource) -> String {
    match source {
        OutputVideoSource::Program => "PGM".into(),
        OutputVideoSource::Preview => "PRV".into(),
        OutputVideoSource::Multiview(id) => project
            .multiviews
            .get(&id)
            .map(|view| format!("MV {}", view.name))
            .unwrap_or_else(|| format!("MV {id}")),
    }
}

fn output_color_format_label(format: OutputColorFormat) -> &'static str {
    match format {
        OutputColorFormat::Rgba8 => "RGBA",
        OutputColorFormat::Bgra8 => "BGRA",
        OutputColorFormat::Nv12 => "NV12",
        OutputColorFormat::Uyvy => "UYVY",
    }
}

fn output_color_format_combo(ui: &mut egui::Ui, output: &Output) -> Option<Command> {
    let allowed = OutputColorFormat::allowed_for(&output.kind);
    if allowed.is_empty() {
        return None;
    }
    let current = output.effective_color_format();
    let mut selected = None;
    egui::ComboBox::from_id_salt(("output-color-format", output.id))
        .selected_text(
            current
                .map(output_color_format_label)
                .unwrap_or("—"),
        )
        .show_ui(ui, |ui| {
            for format in allowed {
                if ui
                    .selectable_label(current == Some(*format), output_color_format_label(*format))
                    .clicked()
                {
                    selected = Some(*format);
                }
            }
        });
    selected.filter(|format| current != Some(*format)).map(|format| {
        Command::SetOutputColorFormat {
            id: output.id,
            format: Some(format),
        }
    })
}

fn input_source_label(source: &InputSource) -> &'static str {
    match source {
        InputSource::ColorBars => "ColorBars",
        InputSource::SolidColor { .. } => "Color",
        InputSource::Image { .. } => "Image",
        InputSource::Video { .. } => "Video",
        InputSource::Ndi { .. } => "NDI",
        InputSource::Omt { .. } => "OMT",
        InputSource::DeckLink { .. } => "DeckLink",
        InputSource::AudioDevice { .. } => "Audio",
        InputSource::MixFeed { .. } => "MixFeed",
    }
}

fn input_source_detail(project: &Project, source: &InputSource) -> String {
    match source {
        InputSource::ColorBars => String::new(),
        InputSource::SolidColor { r, g, b, a } => format!("rgba({r},{g},{b},{a})"),
        InputSource::Image { asset } => project
            .assets
            .get(asset)
            .map(|asset| asset.original_name.clone())
            .unwrap_or_else(|| asset.to_string()),
        InputSource::Video { asset, .. } => project
            .assets
            .get(asset)
            .map(|asset| asset.original_name.clone())
            .unwrap_or_else(|| asset.to_string()),
        InputSource::Ndi { source_name } => source_name.clone(),
        InputSource::Omt { url } => url.clone(),
        InputSource::DeckLink { binding } | InputSource::AudioDevice { binding } => {
            binding.to_string()
        }
        InputSource::MixFeed { unit, tap } => {
            let name = project
                .mixing_units
                .get(unit)
                .map(|unit| unit.name.as_str())
                .unwrap_or("unknown");
            let tap = match tap {
                MixTap::Program => "PGM",
                MixTap::Preview => "PRV",
            };
            format!("{name} {tap}")
        }
    }
}

fn scene_id_for_input(project: &Project, input: InputId) -> Option<SceneId> {
    project.scenes.values().find_map(|scene| {
        scene
            .items
            .iter()
            .any(|item| item.input == input)
            .then_some(scene.id)
    })
}

fn draw_peak_meter(ui: &mut egui::Ui, name: &str, peak: f32, strip_h: f32) {
    let strip_w = 52.0;
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(strip_w, strip_h.max(80.0)), egui::Sense::hover());
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Center)),
        |ui| {
            ui.set_clip_rect(rect);
            ui.add(egui::Label::new(egui::RichText::new(name).small().strong()).truncate());
            let meter_h = (ui.available_height() - 4.0).max(48.0);
            let (bar, _) = ui.allocate_exact_size(egui::vec2(22.0, meter_h), egui::Sense::hover());
            ui.painter()
                .rect_filled(bar, 2.0, egui::Color32::from_gray(55));
            let level = peak.clamp(0.0, 1.0);
            let fill = egui::Rect::from_min_max(
                egui::pos2(bar.left(), bar.bottom() - bar.height() * level),
                bar.right_bottom(),
            );
            let color = if level > 0.95 {
                egui::Color32::from_rgb(200, 50, 50)
            } else if level > 0.8 {
                egui::Color32::from_rgb(200, 170, 40)
            } else {
                egui::Color32::from_rgb(50, 170, 80)
            };
            ui.painter().rect_filled(fill, 2.0, color);
        },
    );
}

fn show_frame(
    ui: &mut egui::Ui,
    frame: Option<eiviz_media::VideoFrame>,
    id: &str,
    fill: bool,
) -> Option<egui::Response> {
    let Some(frame) = frame else {
        ui.label("No frame yet");
        return None;
    };
    let display = if fill {
        ui.available_size()
    } else {
        fit_monitor_size(ui.available_size())
    };
    let blit_w = display.x.round().clamp(16.0, 1920.0) as u32;
    let blit_h = display.y.round().clamp(9.0, 1080.0) as u32;
    let mut img = vec![0u8; (blit_w * blit_h * 4) as usize];
    for y in 0..blit_h {
        for x in 0..blit_w {
            let sx = x * frame.width / blit_w;
            let sy = y * frame.height / blit_h;
            let px = frame.pixel(sx, sy);
            let i = ((y * blit_w + x) * 4) as usize;
            img[i..i + 4].copy_from_slice(&px);
        }
    }
    let color = egui::ColorImage::from_rgba_unmultiplied([blit_w as usize, blit_h as usize], &img);
    let tex = ui.ctx().load_texture(id, color, Default::default());
    Some(ui.image((tex.id(), display)))
}

fn show_frame_at(
    ui: &mut egui::Ui,
    frame: Option<eiviz_media::VideoFrame>,
    id: &str,
    size: egui::Vec2,
) -> Option<egui::Response> {
    let Some(frame) = frame else {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 2.0, egui::Color32::from_gray(24));
        return Some(response);
    };
    let blit_w = size.x.round().clamp(16.0, 1920.0) as u32;
    let blit_h = size.y.round().clamp(9.0, 1080.0) as u32;
    let mut img = vec![0u8; (blit_w * blit_h * 4) as usize];
    for y in 0..blit_h {
        for x in 0..blit_w {
            let sx = x * frame.width / blit_w;
            let sy = y * frame.height / blit_h;
            let px = frame.pixel(sx, sy);
            let i = ((y * blit_w + x) * 4) as usize;
            img[i..i + 4].copy_from_slice(&px);
        }
    }
    let color = egui::ColorImage::from_rgba_unmultiplied([blit_w as usize, blit_h as usize], &img);
    let tex = ui.ctx().load_texture(id, color, Default::default());
    Some(ui.image((tex.id(), size)))
}

fn write_layout_png(image: &egui::ColorImage, path: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
    for pixel in &image.pixels {
        rgba.extend_from_slice(&pixel.to_array());
    }
    image::save_buffer(
        path,
        &rgba,
        image.size[0] as u32,
        image.size[1] as u32,
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|error| error.to_string())
}

impl DesktopApp {
    #[allow(clippy::collapsible_if)]
    fn handle_preview_pointer(
        &mut self,
        ui: &egui::Ui,
        resp: &egui::Response,
        scene: &Scene,
        _unit: eiviz_core::MixingUnitId,
    ) {
        let Some(pos) = resp.hover_pos() else {
            return;
        };
        let rect = resp.rect;
        let nx = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let ny = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        if ui.input(|i| i.pointer.primary_pressed()) {
            if let Some(item) = scene
                .hit_test(nx, ny)
                .and_then(|id| scene.items.iter().find(|it| it.id == id))
            {
                self.drag_item = Some((scene.id, item.id, nx, ny, item.transform));
            }
        }
        if ui.input(|i| i.pointer.primary_down()) {
            if let Some((sid, iid, ox, oy, base)) = self.drag_item {
                let mut xf = base;
                xf.x = (base.x + (nx - ox)).clamp(-1.0, 1.0);
                xf.y = (base.y + (ny - oy)).clamp(-1.0, 1.0);
                let env = CommandEnvelope::new(
                    self.engine.client(),
                    Command::UpdateTransform {
                        scene: sid,
                        item: iid,
                        transform: xf,
                    },
                )
                .with_coalesce_key(format!("xf-{iid}"));
                let _ = self.engine.submit(env);
            }
        }
        if ui.input(|i| i.pointer.primary_released()) {
            self.drag_item = None;
        }
    }
}
