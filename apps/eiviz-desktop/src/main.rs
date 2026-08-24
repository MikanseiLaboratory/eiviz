use eiviz_command::{Command, CommandEnvelope};
#[cfg(feature = "ndi")]
use eiviz_core::{AudioRoute, RouteMode};
use eiviz_core::{
    CompositorBackend, Input, InputId, InputSource, Multiview, MultiviewId, MultiviewSource,
    MultiviewTile, Output, OutputId, OutputKind, Project, Scene, SceneId, SceneItem, SceneItemId,
    Transform2D, TransitionStyle,
};
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
    omt_address: String,
    omt_output_name: String,
    omt_discovered: Vec<String>,
    omt_connections: Vec<Arc<eiviz_io_omt::OmtSource>>,
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
            omt_address: std::env::var("EIVIZ_OMT_SOURCE").unwrap_or_default(),
            omt_output_name: "eiviz Program".into(),
            omt_discovered: Vec::new(),
            omt_connections: Vec::new(),
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
                eiviz_io_decklink::probe(),
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
                    "{}: {}",
                    cap.id,
                    if cap.available {
                        "ready"
                    } else {
                        "unavailable"
                    }
                ));
            }
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
