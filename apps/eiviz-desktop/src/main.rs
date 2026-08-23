use eiviz_command::Command;
use eiviz_core::{
    Input, InputId, InputSource, Scene, SceneId, SceneItem, SceneItemId, Transform2D,
    TransitionStyle,
};
use eiviz_engine::Engine;
use std::sync::Arc;

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
        Box::new(|_cc| Ok(Box::new(DesktopApp::new()))),
    )
}

struct DesktopApp {
    engine: Arc<Engine>,
    status: String,
    selected_scene: Option<SceneId>,
}

impl DesktopApp {
    fn new() -> Self {
        let engine = Engine::new("Untitled").shared();
        bootstrap(&engine);
        Self {
            engine,
            status: "ready".into(),
            selected_scene: None,
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
}

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let _ = self.engine.tick();
        let project = self.engine.snapshot();
        let unit_id = self.engine.primary_unit();
        let unit = project.mixing_units.get(&unit_id).cloned();

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("eiviz");
                if ui.button("TAKE").clicked() {
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
        });

        egui::SidePanel::left("inputs").show(ctx, |ui| {
            ui.heading("Inputs");
            for input in project.inputs.values() {
                ui.label(format!("{} [{}]", input.name, input.tags.join(",")));
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
            for cap in [
                eiviz_io_ndi::probe(),
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
            if let Some(u) = &unit {
                ui.separator();
                ui.label(format!("PRV {:?}", u.preview.scene));
                ui.label(format!("PGM {:?}", u.program.scene));
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Program / Preview");
            if let Some(frame) = self.engine.last_program(unit_id) {
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
                let tex = ui.ctx().load_texture("pgm", color, Default::default());
                ui.image(&tex);
            } else {
                ui.label("No program frame yet");
            }
            ui.separator();
            ui.label("Mouse: select a scene on the left, then TAKE. All edits go through CommandEnvelope.");
        });
        ctx.request_repaint();
    }
}
