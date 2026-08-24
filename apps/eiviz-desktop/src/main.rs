use eiviz_command::{Command, CommandEnvelope};
#[cfg(any(feature = "decklink", feature = "ndi", feature = "audio-cpal"))]
use eiviz_core::{AudioRoute, RouteMode};
use eiviz_core::{
    AacEncoderProfile, CompositorBackend, DistributionProfile, H264EncoderProfile, Input, InputId,
    InputSource, Multiview, MultiviewId, MultiviewSource, MultiviewTile, Output, OutputId,
    OutputKind, Project, ReconnectProfile, Scene, SceneId, SceneItem, SceneItemId,
    Transform2D, TransitionStyle, TransportProfile,
};
#[cfg(any(feature = "decklink", feature = "audio-cpal"))]
use eiviz_core::{DeviceBinding, DeviceBindingId};
use eiviz_engine::Engine;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("eiviz"),
        ..Default::default()
    };
    eframe::run_native(
        "eiviz",
        native,
        Box::new(|_cc| Ok(Box::new(DesktopApp::new()?))),
    )
}

struct DesktopApp {
    engine: Arc<Engine>,
    status: String,
    selected_scene: Option<SceneId>,
    save_path: String,
    asset_root: String,
    image_path: String,
    video_path: String,
    openh264_path: String,
    portable_path: String,
    rtmp_url: String,
    srt_url: String,
    recording_path: String,
    omt_address: String,
    omt_output_name: String,
    omt_discovered: Vec<String>,
    omt_connections: Vec<Arc<eiviz_io_omt::OmtSource>>,
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
    ndi_connections: Vec<Arc<eiviz_io_ndi::NdiSource>>,
    #[cfg(feature = "ndi")]
    ndi_outputs: Vec<(OutputId, Arc<eiviz_io_ndi::NdiSink>)>,
    #[cfg(feature = "ndi")]
    ndi_output_name: String,
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
    http_port: Option<u16>,
    tcp_port: Option<u16>,
    drag_item: Option<(SceneId, SceneItemId, f32, f32, Transform2D)>,
}

impl DesktopApp {
    fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
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
        let engine = Engine::from_project(project)?.shared();
        bootstrap(&engine);
        let control_stop = Arc::new(AtomicBool::new(false));
        let control_enabled = std::env::var("EIVIZ_CONTROL").as_deref() != Ok("off");
        let (http_port, tcp_port) = if control_enabled {
            let max_requests_per_sec = std::env::var("EIVIZ_CONTROL_RATE")
                .unwrap_or_else(|_| "60".into())
                .parse::<u32>()
                .map_err(|error| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("invalid EIVIZ_CONTROL_RATE: {error}"),
                    )
                })?;
            let http_port = eiviz_control::spawn_http(
                engine.clone(),
                eiviz_control::ControlConfig {
                    bind: std::env::var("EIVIZ_HTTP_BIND")
                        .unwrap_or_else(|_| "127.0.0.1:8090".into()),
                    require_token: std::env::var("EIVIZ_CONTROL_TOKEN")
                        .ok()
                        .filter(|token| !token.is_empty()),
                    max_requests_per_sec,
                },
                control_stop.clone(),
            )?;
            let tcp_result = eiviz_control::spawn_tcp(
                engine.clone(),
                &std::env::var("EIVIZ_TCP_BIND").unwrap_or_else(|_| "127.0.0.1:8091".into()),
                control_stop.clone(),
            );
            match tcp_result {
                Ok(tcp_port) => (Some(http_port), Some(tcp_port)),
                Err(error) => {
                    control_stop.store(true, Ordering::Release);
                    return Err(error.into());
                }
            }
        } else {
            (None, None)
        };
        let mut app = Self {
            engine,
            status: "ready".into(),
            selected_scene: None,
            save_path: "project.json".into(),
            asset_root: "eiviz-assets".into(),
            image_path: String::new(),
            video_path: String::new(),
            openh264_path: std::env::var("EIVIZ_OPENH264_PATH").unwrap_or_default(),
            portable_path: "project.eiviz".into(),
            rtmp_url: "rtmp://127.0.0.1:1935/live/eiviz".into(),
            srt_url: "srt://127.0.0.1:9000".into(),
            recording_path: "recording.mp4".into(),
            omt_address: std::env::var("EIVIZ_OMT_SOURCE").unwrap_or_default(),
            omt_output_name: "eiviz Program".into(),
            omt_discovered: Vec::new(),
            omt_connections: Vec::new(),
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
            ndi_output_name: "eiviz Program".into(),
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
            http_port,
            tcp_port,
            drag_item: None,
        };
        if !app.omt_address.is_empty() {
            app.connect_omt();
        }
        Ok(app)
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

    #[cfg(feature = "decklink")]
    fn connect_decklink_capture(&mut self) {
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
        let unit = self.engine.primary_unit();
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
        self.engine.attach_source(source.clone());
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
            kind: OutputKind::DeckLink {
                binding: binding.id,
            },
            enabled: true,
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
        self.engine.attach_source(source.clone());
        self.omt_connections.push(source);
        let unit = self.engine.primary_unit();
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
        let unit = self.engine.primary_unit();
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
        let input = match self.engine.ingest_video(
            std::path::Path::new(self.video_path.trim()),
            std::path::Path::new(self.asset_root.trim()),
            std::path::Path::new(self.openh264_path.trim()),
            Default::default(),
        ) {
            Ok(input) => input,
            Err(error) => {
                self.status = format!("video ingest: {error}");
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
        if let Err(error) = self.engine.submit_payload(Command::AddScene {
            scene: scene.clone(),
        }) {
            self.status = format!("video scene: {error}");
            return;
        }
        let unit = self.engine.primary_unit();
        if let Err(error) = self.engine.submit_payload(Command::SetPreview {
            unit,
            scene: Some(scene.id),
        }) {
            self.status = format!("video preview: {error}");
            return;
        }
        self.selected_scene = Some(scene.id);
        self.status = format!("video ready: {}", scene.name);
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
        let unit = self.engine.primary_unit();
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
        self.engine.attach_source(source.clone());
        self.ndi_connections.push(source);
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
    fn start_ndi_output(&mut self, owner: eiviz_core::MixingUnitId) {
        let name = self.ndi_output_name.trim().to_owned();
        let project = self.engine.snapshot();
        let sink = match eiviz_io_ndi::NdiSink::create(
            &name,
            project.video.frame_rate,
            eiviz_io_ndi::NdiConfig::default(),
        ) {
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
            kind: OutputKind::Ndi { name: name.clone() },
            enabled: true,
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
        self.status = format!("NDI output started: {name}");
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
        let source = match eiviz_io_audio::CpalInput::open(input.id, &binding, backend, config) {
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
        self.engine.attach_source(source.clone());
        self.audio_inputs.push((input.id, source));
        self.status = format!("Audio input started: {}", device.display_name);
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
        let sink = match eiviz_io_audio::CpalOutput::open(&name, &binding, backend, config) {
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
            kind: OutputKind::AudioDevice {
                binding: binding.id,
            },
            enabled: true,
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
        self.audio_outputs.push((output.id, sink));
        self.status = format!("Audio output started: {}", device.display_name);
    }

    fn configure_distribution(&mut self, transport_name: &str, owner: eiviz_core::MixingUnitId) {
        let (name, kind, transport) = match transport_name {
            "rtmp" => (
                "RTMP Program".to_owned(),
                OutputKind::Rtmp {
                    url: self.rtmp_url.trim().to_owned(),
                },
                TransportProfile::RtmpPublish {
                    chunk_size: 4096,
                    connect_timeout_ms: 5_000,
                },
            ),
            "srt" => (
                "SRT Program".to_owned(),
                OutputKind::Srt {
                    url: self.srt_url.trim().to_owned(),
                },
                TransportProfile::SrtCallerMpegTs {
                    latency_ms: 120,
                    stream_id: None,
                },
            ),
            "mp4" => (
                "Fragmented MP4 Program".to_owned(),
                OutputKind::Mp4 {
                    path: self.recording_path.trim().to_owned(),
                },
                TransportProfile::FragmentedMp4 {
                    segment_duration_ms: 2_000,
                    recover_incomplete_tail: true,
                },
            ),
            _ => {
                self.status = format!("unknown distribution transport {transport_name}");
                return;
            }
        };
        let output = Output {
            id: OutputId::new(),
            name,
            owner,
            kind,
            enabled: false,
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
        };
        match self.engine.configure_distribution_output(output) {
            Ok(_) => {
                self.status = format!(
                    "{transport_name} mapping saved stopped; start will hard-fail until explicit H.264/AAC encoders are available"
                )
            }
            Err(error) => self.status = format!("{transport_name} mapping: {error}"),
        }
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
        let _ = self.engine.tick();
        let project = self.engine.snapshot();
        let unit_id = self.engine.primary_unit();
        let unit = project.mixing_units.get(&unit_id).cloned();

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("eiviz");
                if ui.button("TAKE").clicked() || ui.input(|i| i.key_pressed(egui::Key::Space)) {
                    if let Some(u) = &unit {
                        let _ = self.engine.submit_payload(Command::Take {
                            unit: u.id,
                            swap: false,
                            style: TransitionStyle::Cut,
                            duration_frames: 0,
                        });
                        self.status = format!("TAKE rev {}", self.engine.revision());
                    }
                }
                if ui.button("Tick").clicked() {
                    let _ = self.engine.tick();
                }
                ui.label(format!("rev {}", self.engine.revision()));
                ui.label(&self.status);
            });
            ui.horizontal(|ui| {
                ui.label("path");
                ui.text_edit_singleline(&mut self.save_path);
                if ui.button("Save").clicked() {
                    match self.engine.save(std::path::Path::new(&self.save_path)) {
                        Ok(()) => self.status = "saved".into(),
                        Err(e) => self.status = format!("save: {e}"),
                    }
                }
                if ui.button("Load").clicked() {
                    match self.engine.load_project(
                        std::path::Path::new(&self.save_path),
                        Some(std::path::Path::new(&self.asset_root)),
                    ) {
                        Ok(()) => {
                            self.selected_scene = None;
                            self.status = "loaded".into();
                        }
                        Err(e) => self.status = format!("load: {e}"),
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("asset root");
                ui.text_edit_singleline(&mut self.asset_root);
                ui.label("image");
                ui.text_edit_singleline(&mut self.image_path);
                if ui.button("Add Image").clicked() {
                    self.add_image();
                }
            });
            ui.horizontal(|ui| {
                ui.label("H.264 MP4");
                ui.text_edit_singleline(&mut self.video_path);
                ui.label("Cisco OpenH264 2.6.0 binary");
                ui.text_edit_singleline(&mut self.openh264_path);
                if ui.button("Add Video").clicked() {
                    self.add_video();
                }
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
                        Err(error) => self.status = format!("portable export: {error}"),
                    }
                }
                if ui.button("Import .eiviz").clicked() {
                    match self.engine.import_portable_into(
                        std::path::Path::new(&self.portable_path),
                        std::path::Path::new(&self.asset_root),
                    ) {
                        Ok(()) => {
                            self.selected_scene = None;
                            self.status = "portable imported".into();
                        }
                        Err(error) => self.status = format!("portable import: {error}"),
                    }
                }
            });
        });

        egui::SidePanel::left("inputs").show(ctx, |ui| {
            ui.heading("Inputs");
            for input in project.inputs.values() {
                ui.label(format!("{} [{}]", input.name, input.tags.join(",")));
                if let InputSource::Video { playback, .. } = &input.source {
                    let mut updated = playback.clone();
                    let mut changed = false;
                    ui.horizontal(|ui| {
                        if ui
                            .button(if playback.playing { "Pause" } else { "Play" })
                            .clicked()
                        {
                            updated.playing = !playback.playing;
                            changed = true;
                        }
                        changed |= ui.checkbox(&mut updated.loop_playback, "Loop").changed();
                        ui.label("seek us");
                        changed |= ui
                            .add(egui::DragValue::new(&mut updated.position_us).speed(1_000.0))
                            .changed();
                    });
                    if changed {
                        match self.engine.set_video_playback(input.id, updated) {
                            Ok(_) => self.status = format!("video playback: {}", input.name),
                            Err(error) => self.status = format!("video playback: {error}"),
                        }
                    }
                }
            }
            ui.separator();
            ui.heading("Scenes");
            for scene in project.scenes.values() {
                let selected = Some(scene.id) == self.selected_scene;
                if ui.selectable_label(selected, &scene.name).clicked() {
                    self.selected_scene = Some(scene.id);
                    if let Some(u) = &unit {
                        let _ = self.engine.submit_payload(Command::SetPreview {
                            unit: u.id,
                            scene: Some(scene.id),
                        });
                    }
                }
            }
        });

        egui::SidePanel::right("caps").show(ctx, |ui| {
            ui.heading("Capabilities");
            ui.label(format!("compositor {:?}", project.compositor));
            ui.label(self.engine.compositor_detail());
            ui.label(format!("missing-media {:?}", project.missing_media));
            ui.label(match (self.http_port, self.tcp_port) {
                (Some(http), Some(tcp)) => {
                    format!("control localhost HTTP :{http} / TCP :{tcp}")
                }
                _ => "control disabled".into(),
            });
            for cap in [
                eiviz_io_omt::probe(),
                eiviz_codec_gpu_video::probe(),
            ] {
                ui.label(format!(
                    "{}: {}",
                    cap.id,
                    if cap.available {
                        "ready"
                    } else {
                        "unavailable"
                    }
                ));
            }
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
            ui.heading("Distribution");
            ui.label(
                "Explicit baseline: H.264 Annex-B + raw AAC-LC. Mappings are created stopped; unavailable encoders hard-fail on Start.",
            );
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
                        "{}: {} — {}",
                        output.name,
                        diagnostic.map_or("unknown", |value| value.state.as_str()),
                        diagnostic.map_or("no diagnostics", |value| value.detail.as_str())
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("Start").clicked() {
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
            ui.heading("OMT Capture");
            ui.text_edit_singleline(&mut self.omt_address);
            ui.horizontal(|ui| {
                if ui.button("Discover").clicked() {
                    match eiviz_io_omt::discover_sources() {
                        Ok(sources) => {
                            self.omt_discovered = sources;
                            self.status = format!("OMT: {} source(s)", self.omt_discovered.len());
                        }
                        Err(error) => self.status = format!("OMT discovery: {error}"),
                    }
                }
                if ui.button("Connect").clicked() {
                    self.connect_omt();
                }
            });
            for address in &self.omt_discovered {
                if ui
                    .selectable_label(self.omt_address == *address, address)
                    .clicked()
                {
                    self.omt_address.clone_from(address);
                }
            }
            for source in &self.omt_connections {
                let detail = source
                    .last_error()
                    .unwrap_or_else(|| "no adapter error".into());
                ui.label(format!(
                    "{}: {:?} ({detail})",
                    source.address(),
                    source.health()
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
                ui.horizontal(|ui| {
                    if ui.button("Discover NDI").clicked() {
                        match eiviz_io_ndi::discover_sources(std::time::Duration::from_secs(2)) {
                            Ok(sources) => {
                                self.ndi_discovered = sources;
                                self.ndi_selected = None;
                                self.status =
                                    format!("NDI: {} source(s)", self.ndi_discovered.len());
                            }
                            Err(error) => self.status = format!("NDI discovery: {error}"),
                        }
                    }
                    if ui.button("Connect NDI").clicked() {
                        self.connect_ndi();
                    }
                });
                for (index, source) in self.ndi_discovered.iter().enumerate() {
                    if ui
                        .selectable_label(self.ndi_selected == Some(index), source.label())
                        .clicked()
                    {
                        self.ndi_selected = Some(index);
                    }
                }
                for source in &self.ndi_connections {
                    let detail = source
                        .last_error()
                        .unwrap_or_else(|| "no adapter error".into());
                    let (video_drops, audio_drops) = source.dropped_frames();
                    ui.label(format!(
                        "{}: {:?}, drops video={video_drops} audio={audio_drops} ({detail})",
                        source.source_name(),
                        source.health()
                    ));
                }
                ui.label("Program output");
                ui.text_edit_singleline(&mut self.ndi_output_name);
                if ui.button("Start NDI Output").clicked() {
                    self.start_ndi_output(unit_id);
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
                            "{}: {:?}, drops={} ({detail})",
                            eiviz_media::MediaSink::name(sink.as_ref()),
                            sink.health(),
                            sink.dropped_frames()
                        ));
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
            ui.label("Program output");
            ui.text_edit_singleline(&mut self.omt_output_name);
            if ui.button("Start OMT Output").clicked() {
                let output_name = self.omt_output_name.trim().to_owned();
                match eiviz_io_omt::OmtSink::create(&output_name, project.video.frame_rate) {
                    Ok(sink) => {
                        let output = Output {
                            id: OutputId::new(),
                            name: output_name.clone(),
                            owner: unit_id,
                            kind: OutputKind::Omt {
                                url: output_name.clone(),
                            },
                            enabled: true,
                            distribution: None,
                        };
                        match self.engine.submit_payload(Command::AddOutput {
                            output: output.clone(),
                        }) {
                            Ok(_) => {
                                match self.engine.attach_output_sink(output.id, Arc::new(sink)) {
                                    Ok(()) => {
                                        self.status = format!("OMT output started: {output_name}");
                                    }
                                    Err(error) => {
                                        self.status = format!("OMT output route: {error}");
                                    }
                                }
                            }
                            Err(error) => {
                                self.status = format!("OMT output project: {error}");
                            }
                        }
                    }
                    Err(error) => self.status = format!("OMT output: {error}"),
                }
            }
            if let Some(u) = &unit {
                ui.separator();
                ui.label(format!("PRV {:?}", u.preview.scene));
                ui.label(format!("PGM {:?}", u.program.scene));
                ui.heading("Overlays");
                for overlay in &u.overlays {
                    let mut on = overlay.enabled;
                    if ui.checkbox(&mut on, &overlay.name).changed() {
                        let _ = self.engine.submit_payload(Command::SetOverlayEnabled {
                            unit: u.id,
                            overlay: overlay.id,
                            enabled: on,
                        });
                    }
                }
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Program / Preview");
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label("Program");
                    show_frame(ui, self.engine.last_program(unit_id), "pgm");
                });
                ui.vertical(|ui| {
                    ui.label("Preview");
                    let preview_resp = show_frame(ui, self.engine.last_preview(unit_id), "prv");
                    if let (Some(resp), Some(_u), Some(scene_id)) =
                        (preview_resp, unit.as_ref(), u_preview_scene(&unit))
                    {
                        if let Some(scene) = project.scenes.get(&scene_id) {
                            self.handle_preview_pointer(ui, &resp, scene, unit_id);
                        }
                    }
                });
            });
            ui.separator();
            ui.label("Mouse: drag on Preview to move a SceneItem (UpdateTransform). Space = TAKE.");
            for multiview in project.multiviews.values() {
                ui.separator();
                ui.heading(format!("Multiview: {}", multiview.name));
                let texture_id = format!("multiview-{}", multiview.id);
                show_frame(ui, self.engine.last_multiview(multiview.id), &texture_id);
            }
        });
        ctx.request_repaint();
    }
}

impl Drop for DesktopApp {
    fn drop(&mut self) {
        self.control_stop.store(true, Ordering::Release);
    }
}

fn u_preview_scene(unit: &Option<eiviz_core::MixingUnit>) -> Option<SceneId> {
    unit.as_ref().and_then(|u| u.preview.scene)
}

fn show_frame(
    ui: &mut egui::Ui,
    frame: Option<eiviz_media::VideoFrame>,
    id: &str,
) -> Option<egui::Response> {
    let Some(frame) = frame else {
        ui.label("No frame yet");
        return None;
    };
    let w = 480u32;
    let h = 270u32;
    let mut img = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let sx = x * frame.width / w;
            let sy = y * frame.height / h;
            let px = frame.pixel(sx, sy);
            let i = ((y * w + x) * 4) as usize;
            img[i..i + 4].copy_from_slice(&px);
        }
    }
    let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &img);
    let tex = ui.ctx().load_texture(id, color, Default::default());
    Some(ui.image(&tex))
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
