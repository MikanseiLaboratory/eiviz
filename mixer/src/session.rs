//! Canonical session JSON shared by every host.
//! Shape matches `host/SessionStore.cs` (camelCase, string enums, version 1).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    #[serde(default = "version_one")]
    pub version: i32,
    #[serde(default)]
    pub settings: SessionSettings,
    #[serde(default)]
    pub inputs: Vec<InputDto>,
    #[serde(default)]
    pub scenes: Vec<SceneDto>,
    #[serde(default)]
    pub units: Vec<UnitDto>,
    #[serde(default)]
    pub outputs: Vec<OutputDto>,
    #[serde(default)]
    pub multiviews: Vec<MultiviewDto>,
    #[serde(default)]
    pub buses: Vec<BusDto>,
    #[serde(default)]
    pub next_input_id: u64,
    #[serde(default)]
    pub next_scene_id: u64,
    #[serde(default)]
    pub next_unit_id: u64,
    #[serde(default)]
    pub next_output_id: u64,
    #[serde(default)]
    pub next_multiview_id: u64,
    #[serde(default)]
    pub next_bus_id: u64,
    #[serde(default)]
    pub selected_unit_id: u64,
    #[serde(default)]
    pub headphone_copy_master: bool,
}

fn version_one() -> i32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSettings {
    #[serde(default = "fps_num")]
    pub master_fps_num: u32,
    #[serde(default = "fps_den")]
    pub master_fps_den: u32,
    #[serde(default = "width_1080")]
    pub default_width: u32,
    #[serde(default = "height_1080")]
    pub default_height: u32,
    #[serde(default = "theme_charcoal")]
    pub theme: String,
    #[serde(default = "one")]
    pub default_multiview_unit_id: u64,
    #[serde(default = "three")]
    pub frame_buffer_frames: u32,
    #[serde(default = "three")]
    pub default_present_interval: u32,
    #[serde(default)]
    pub internal_color_format: InternalColorFormat,
    #[serde(default = "default_true", deserialize_with = "de_bool_null_true")]
    pub rebar_optimization: bool,
    #[serde(default)]
    pub rebar_direct_sample: bool,
    #[serde(default = "default_true", deserialize_with = "de_bool_null_true")]
    pub ndi_gpu_upload: bool,
    #[serde(default = "preview_color", deserialize_with = "de_preview_color")]
    pub preview_color: RgbColor,
    #[serde(default = "program_color", deserialize_with = "de_program_color")]
    pub program_color: RgbColor,
    #[serde(default = "inactive_color", deserialize_with = "de_inactive_color")]
    pub inactive_color: RgbColor,
    pub last_session_path: Option<String>,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            master_fps_num: fps_num(),
            master_fps_den: fps_den(),
            default_width: width_1080(),
            default_height: height_1080(),
            theme: theme_charcoal(),
            default_multiview_unit_id: 1,
            frame_buffer_frames: 3,
            default_present_interval: 3,
            internal_color_format: InternalColorFormat::Uyvy,
            rebar_optimization: true,
            rebar_direct_sample: false,
            ndi_gpu_upload: true,
            preview_color: preview_color(),
            program_color: program_color(),
            inactive_color: inactive_color(),
            last_session_path: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct RgbColor {
    #[serde(default)]
    pub r: u8,
    #[serde(default)]
    pub g: u8,
    #[serde(default)]
    pub b: u8,
}

fn preview_color() -> RgbColor {
    RgbColor { r: 0, g: 255, b: 0 }
}

fn program_color() -> RgbColor {
    RgbColor { r: 255, g: 0, b: 0 }
}

fn de_preview_color<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<RgbColor, D::Error> {
    Ok(Option::<RgbColor>::deserialize(deserializer)?.unwrap_or_else(preview_color))
}

fn de_program_color<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<RgbColor, D::Error> {
    Ok(Option::<RgbColor>::deserialize(deserializer)?.unwrap_or_else(program_color))
}

fn inactive_color() -> RgbColor {
    RgbColor { r: 64, g: 64, b: 64 }
}

fn de_inactive_color<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<RgbColor, D::Error> {
    Ok(Option::<RgbColor>::deserialize(deserializer)?.unwrap_or_else(inactive_color))
}

fn fps_num() -> u32 {
    60_000
}
fn fps_den() -> u32 {
    1_001
}
fn width_1080() -> u32 {
    1920
}
fn height_1080() -> u32 {
    1080
}
fn theme_charcoal() -> String {
    "Charcoal".into()
}
fn one() -> u64 {
    1
}
fn default_true() -> bool {
    true
}

fn de_bool_null_true<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    Ok(Option::<bool>::deserialize(deserializer)?.unwrap_or(true))
}
fn three() -> u32 {
    3
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum InternalColorFormat {
    #[default]
    Uyvy,
    Bgra,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum InputKind {
    Color,
    Bars,
    Black,
    Still,
    Video,
    Omt,
    Ndi,
    Uvc,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum BandwidthSave {
    AlwaysLow,
    NotOnProgram,
    #[default]
    NotOnPreviewOrProgram,
    AlwaysFull,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum OmtQuality {
    #[default]
    Default,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum NdiBandwidth {
    #[default]
    Highest,
    Lowest,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputTransport {
    #[default]
    Omt,
    Ndi,
    DeckLink,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputSourceKind {
    Scene,
    MuPreview,
    #[default]
    MuProgram,
    Multiview,
    Input,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum MvSlotKind {
    #[default]
    None,
    Input,
    Scene,
    MuPreview,
    MuProgram,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum MultiviewTemplate {
    #[default]
    PreviewProgram8,
    PreviewProgram4,
    PreviewProgram12,
    PreviewProgram16,
    Grid2x2,
    Grid3x3,
    Grid4x4,
}

impl MultiviewTemplate {
    pub fn tile_count(self) -> usize {
        match self {
            Self::PreviewProgram8 => 8,
            Self::PreviewProgram4 => 4,
            Self::PreviewProgram12 => 12,
            Self::PreviewProgram16 => 16,
            Self::Grid2x2 => 4,
            Self::Grid3x3 => 9,
            Self::Grid4x4 => 16,
        }
    }

    pub fn has_bus_panes(self) -> bool {
        matches!(
            self,
            Self::PreviewProgram8 | Self::PreviewProgram4 | Self::PreviewProgram12 | Self::PreviewProgram16
        )
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum AudioBusRole {
    #[default]
    Master,
    Headphone,
    Aux,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum AudioDeviceKind {
    #[default]
    None,
    Wasapi,
    Asio,
    CoreAudio,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum AudioLinkMode {
    #[default]
    Follow,
    Independent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputDto {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    pub kind: InputKind,
    pub path_or_address: Option<String>,
    #[serde(default)]
    pub color_r: f32,
    #[serde(default)]
    pub color_g: f32,
    #[serde(default)]
    pub color_b: f32,
    #[serde(default)]
    pub scroll: bool,
    #[serde(default)]
    pub tone_hz: f32,
    #[serde(default = "tone_level")]
    pub tone_level_dbfs: f32,
    #[serde(default = "one_u32")]
    pub bus_mask: u32,
    #[serde(default = "one_f32")]
    pub gain: f32,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub use_gpu: bool,
    #[serde(default = "one_u32")]
    pub frame_buffer_frames: u32,
    #[serde(default)]
    pub bandwidth_save: BandwidthSave,
    #[serde(default)]
    pub keep_full_on_multiview: bool,
    #[serde(default)]
    pub omt_quality: OmtQuality,
    #[serde(default)]
    pub ndi_bandwidth: NdiBandwidth,
    #[serde(default = "default_true")]
    pub video_loop: bool,
    #[serde(default)]
    pub video_play_when: VideoPlayWhen,
    #[serde(default)]
    pub video_restart_when: VideoTriggerWhen,
    #[serde(default)]
    pub video_pause_when: VideoTriggerWhen,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum VideoPlayWhen {
    #[default]
    Never,
    OnActive,
    OnPreview,
    Always,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum VideoTriggerWhen {
    #[default]
    Never,
    OnActive,
    OnDeactivated,
    OnPreview,
}

fn one_u32() -> u32 {
    1
}
fn one_f32() -> f32 {
    1.0
}
fn tone_level() -> f32 {
    -20.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneLayer {
    pub input_id: u64,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default = "one_f32")]
    pub width: f32,
    #[serde(default = "one_f32")]
    pub height: f32,
    #[serde(default = "one_f32")]
    pub opacity: f32,
    #[serde(default)]
    pub z: i32,
    #[serde(default = "true_bool")]
    pub audio_follow: bool,
}

fn true_bool() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneDto {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub layers: Vec<SceneLayer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionPreset {
    #[serde(default = "fade")]
    pub kind: u32,
    #[serde(default = "thirty")]
    pub duration_frames: u32,
    #[serde(default = "true_bool")]
    pub swap: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

fn fade() -> u32 {
    1
}
fn thirty() -> u32 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySlot {
    #[serde(default)]
    pub scene_gpu_id: u64,
    #[serde(default = "overlay_x")]
    pub x: f32,
    #[serde(default = "overlay_y")]
    pub y: f32,
    #[serde(default = "overlay_w")]
    pub width: f32,
    #[serde(default = "overlay_h")]
    pub height: f32,
    #[serde(default = "one_f32")]
    pub opacity: f32,
    #[serde(default)]
    pub z: i32,
    #[serde(default = "true_bool")]
    pub enabled: bool,
}

fn overlay_x() -> f32 {
    0.62
}
fn overlay_y() -> f32 {
    0.08
}
fn overlay_w() -> f32 {
    0.32
}
fn overlay_h() -> f32 {
    0.32
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MvSlot {
    #[serde(default)]
    pub kind: MvSlotKind,
    #[serde(default)]
    pub source_id: u64,
    #[serde(default = "default_true")]
    pub label_follow: bool,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitDto {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default = "width_1080")]
    pub width: u32,
    #[serde(default = "height_1080")]
    pub height: u32,
    #[serde(default = "fps_num")]
    pub fps_num: u32,
    #[serde(default = "fps_den")]
    pub fps_den: u32,
    #[serde(default)]
    pub transitions: Vec<TransitionPreset>,
    #[serde(default)]
    pub overlays: Vec<OverlaySlot>,
    #[serde(default)]
    pub multiview_tiles: Vec<MvSlot>,
    #[serde(default = "one")]
    pub audio_bus_id: u64,
    #[serde(default)]
    pub audio_link: AudioLinkMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputDto {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub transport: OutputTransport,
    #[serde(default)]
    pub source_kind: OutputSourceKind,
    #[serde(default)]
    pub source_id: u64,
    #[serde(default = "one")]
    pub unit_id: u64,
    #[serde(default)]
    pub use_gpu: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiviewDto {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub preview_unit_id: u64,
    #[serde(default)]
    pub program_unit_id: u64,
    #[serde(default)]
    pub present_interval: u32,
    #[serde(default)]
    pub tiles: Vec<MvSlot>,
    #[serde(default)]
    pub template: MultiviewTemplate,
    #[serde(default = "default_true")]
    pub preview_label_follow: bool,
    #[serde(default)]
    pub preview_label: String,
    #[serde(default = "default_true")]
    pub program_label_follow: bool,
    #[serde(default)]
    pub program_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BusDto {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: AudioBusRole,
    #[serde(default)]
    pub device_kind: AudioDeviceKind,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub map_left: i32,
    #[serde(default = "one_i32")]
    pub map_right: i32,
    #[serde(default)]
    pub exclusive: bool,
    #[serde(default)]
    pub bit: u32,
    #[serde(default = "one_f32")]
    pub gain: f32,
    #[serde(default)]
    pub mute: bool,
}

fn one_i32() -> i32 {
    1
}

impl Document {
    pub fn canonicalize(self) -> Self {
        let mut doc = self;
        doc.version = 1;
        doc.settings.frame_buffer_frames = doc.settings.frame_buffer_frames.clamp(1, 8);
        if doc.settings.frame_buffer_frames == 0 {
            doc.settings.frame_buffer_frames = 3;
        }
        doc.settings.default_present_interval = doc.settings.default_present_interval.clamp(1, 8);
        if doc.settings.master_fps_num == 0 {
            doc.settings.master_fps_num = fps_num();
        }
        if doc.settings.master_fps_den == 0 {
            doc.settings.master_fps_den = fps_den();
        }
        if doc.settings.default_width == 0 {
            doc.settings.default_width = width_1080();
        }
        if doc.settings.default_height == 0 {
            doc.settings.default_height = height_1080();
        }
        if doc.settings.theme.is_empty() {
            doc.settings.theme = theme_charcoal();
        }
        for input in &mut doc.inputs {
            if input.bus_mask == 0 {
                input.bus_mask = 1;
            }
            if input.frame_buffer_frames == 0 {
                input.frame_buffer_frames = 1;
            }
            input.frame_buffer_frames = input.frame_buffer_frames.clamp(1, 8);
            if input.gain < 0.0 {
                input.gain = 1.0;
            }
        }
        for unit in &mut doc.units {
            if unit.width == 0 {
                unit.width = width_1080();
            }
            if unit.height == 0 {
                unit.height = height_1080();
            }
            if unit.fps_num == 0 {
                unit.fps_num = fps_num();
            }
            if unit.fps_den == 0 {
                unit.fps_den = fps_den();
            }
            if unit.audio_bus_id == 0 {
                unit.audio_bus_id = 1;
            }
            if unit.transitions.is_empty() {
                unit.transitions = vec![
                    TransitionPreset {
                        kind: 0,
                        duration_frames: 1,
                        swap: true,
                        label: Some("Cut".into()),
                    },
                    TransitionPreset {
                        kind: 1,
                        duration_frames: 30,
                        swap: true,
                        label: Some("Fade".into()),
                    },
                ];
            } else {
                for preset in &mut unit.transitions {
                    preset.label = Some(match preset.kind {
                        0 => "Cut".into(),
                        2 => "Dip".into(),
                        _ => "Fade".into(),
                    });
                    if preset.duration_frames == 0 {
                        preset.duration_frames = 1;
                    }
                }
            }
            while unit.multiview_tiles.len() < 8 {
                unit.multiview_tiles.push(MvSlot {
                    kind: MvSlotKind::None,
                    source_id: 0,
                    label_follow: true,
                    label: String::new(),
                });
            }
            unit.multiview_tiles.truncate(8);
        }
        for layout in &mut doc.multiviews {
            if layout.present_interval > 0 {
                layout.present_interval = layout.present_interval.clamp(1, 8);
            }
            let want = layout.template.tile_count();
            while layout.tiles.len() < want {
                layout.tiles.push(MvSlot {
                    kind: MvSlotKind::None,
                    source_id: 0,
                    label_follow: true,
                    label: String::new(),
                });
            }
            layout.tiles.truncate(want);
            for tile in &mut layout.tiles {
                if matches!(tile.kind, MvSlotKind::MuPreview | MvSlotKind::MuProgram) {
                    tile.kind = MvSlotKind::None;
                    tile.source_id = 0;
                }
            }
        }
        for bus in &mut doc.buses {
            if bus.gain < 0.0 {
                bus.gain = 1.0;
            }
        }
        if doc.next_input_id == 0 {
            doc.next_input_id = doc.inputs.iter().map(|item| item.id).max().unwrap_or(9) + 1;
        }
        if doc.next_scene_id == 0 {
            doc.next_scene_id = doc.scenes.iter().map(|item| item.id).max().unwrap_or(0) + 1;
        }
        if doc.next_unit_id == 0 {
            doc.next_unit_id = doc.units.iter().map(|item| item.id).max().unwrap_or(0) + 1;
        }
        if doc.next_output_id == 0 {
            doc.next_output_id = doc.outputs.iter().map(|item| item.id).max().unwrap_or(99) + 1;
        }
        if doc.next_multiview_id == 0 {
            doc.next_multiview_id = doc.multiviews.iter().map(|item| item.id).max().unwrap_or(0) + 1;
        }
        if doc.next_bus_id == 0 {
            doc.next_bus_id = doc.buses.iter().map(|item| item.id).max().unwrap_or(2) + 1;
        }
        if doc.selected_unit_id == 0 {
            doc.selected_unit_id = doc.units.first().map(|unit| unit.id).unwrap_or(1);
        }
        doc
    }
}

pub fn parse(bytes: &[u8]) -> Result<Document, String> {
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let doc: Document = serde_json::from_str(text).map_err(|error| error.to_string())?;
    Ok(doc.canonicalize())
}

pub fn to_vec(doc: &Document) -> Result<Vec<u8>, String> {
    let text = serde_json::to_string_pretty(doc).map_err(|error| error.to_string())?;
    Ok(text.into_bytes())
}

pub fn canonicalize_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    to_vec(&parse(bytes)?)
}

pub fn load_file(path: &str) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    canonicalize_bytes(&bytes)
}

pub fn save_file(path: &str, bytes: &[u8]) -> Result<(), String> {
    let canonical = canonicalize_bytes(bytes)?;
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
    }
    std::fs::write(path, canonical).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_shaped_json_is_stable() {
        let src = r#"{
  "version": 1,
  "settings": { "masterFpsNum": 60000, "masterFpsDen": 1001 },
  "inputs": [{ "id": 2, "name": "SMPTE Bars", "kind": "Bars" }],
  "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
  "units": [{ "id": 1, "name": "Mixing Unit 1", "audioBusId": 1, "audioLink": "Follow" }],
  "outputs": [{ "id": 100, "name": "eiviz-pgm", "transport": "Omt", "sourceKind": "MuProgram", "useGpu": true }],
  "buses": [
    { "id": 1, "name": "Master", "role": "Master", "deviceKind": "Wasapi", "mapRight": 1 },
    { "id": 2, "name": "Headphone", "role": "Headphone", "deviceKind": "None", "mapRight": 1, "bit": 1 }
  ],
  "nextInputId": 10,
  "nextSceneId": 2,
  "nextUnitId": 2,
  "nextOutputId": 101,
  "selectedUnitId": 1
}"#;
        let a = canonicalize_bytes(src.as_bytes()).expect("parse");
        let b = canonicalize_bytes(&a).expect("again");
        assert_eq!(a, b);
        let doc = parse(src.as_bytes()).unwrap();
        assert_eq!(doc.inputs[0].kind, InputKind::Bars);
        assert_eq!(doc.buses[0].device_kind, AudioDeviceKind::Wasapi);
        assert_eq!(doc.units[0].transitions.len(), 2);
        assert_eq!(doc.units[0].multiview_tiles.len(), 8);
        assert!(doc.settings.rebar_optimization);
        assert!(!doc.settings.rebar_direct_sample);
        assert!(doc.settings.ndi_gpu_upload);
        assert_eq!(doc.settings.preview_color, RgbColor { r: 0, g: 255, b: 0 });
        assert_eq!(doc.settings.program_color, RgbColor { r: 255, g: 0, b: 0 });
        assert_eq!(doc.settings.inactive_color, RgbColor { r: 64, g: 64, b: 64 });
    }

    #[test]
    fn preview_program_colors_roundtrip() {
        let src = r#"{
  "version": 1,
  "settings": {
    "previewColor": { "r": 10, "g": 20, "b": 30 },
    "programColor": { "r": 40, "g": 50, "b": 60 }
  }
}"#;
        let doc = parse(src.as_bytes()).unwrap();
        assert_eq!(doc.settings.preview_color, RgbColor { r: 10, g: 20, b: 30 });
        assert_eq!(doc.settings.program_color, RgbColor { r: 40, g: 50, b: 60 });
        let text = String::from_utf8(to_vec(&doc).unwrap()).unwrap();
        let again = parse(text.as_bytes()).unwrap();
        assert_eq!(again.settings.preview_color, RgbColor { r: 10, g: 20, b: 30 });
        assert_eq!(again.settings.program_color, RgbColor { r: 40, g: 50, b: 60 });
        assert_eq!(again.settings.inactive_color, RgbColor { r: 64, g: 64, b: 64 });
    }

    #[test]
    fn inactive_color_and_mv_labels_roundtrip() {
        let src = r#"{
  "version": 1,
  "settings": {
    "inactiveColor": { "r": 8, "g": 9, "b": 10 }
  },
  "multiviews": [{
    "id": 1,
    "name": "MV",
    "previewLabelFollow": false,
    "previewLabel": "PRV custom",
    "programLabelFollow": true,
    "tiles": [{ "kind": "Scene", "sourceId": 1, "labelFollow": false, "label": "Cam 1" }]
  }]
}"#;
        let doc = parse(src.as_bytes()).unwrap();
        assert_eq!(doc.settings.inactive_color, RgbColor { r: 8, g: 9, b: 10 });
        assert!(!doc.multiviews[0].preview_label_follow);
        assert_eq!(doc.multiviews[0].preview_label, "PRV custom");
        assert!(doc.multiviews[0].program_label_follow);
        assert!(!doc.multiviews[0].tiles[0].label_follow);
        assert_eq!(doc.multiviews[0].tiles[0].label, "Cam 1");
        assert_eq!(doc.multiviews[0].template, MultiviewTemplate::PreviewProgram8);
    }

    #[test]
    fn multiview_template_canonicalize_tile_count() {
        let src = r#"{
  "version": 1,
  "multiviews": [{ "id": 1, "name": "MV", "template": "Grid4x4", "tiles": [] }]
}"#;
        let text = String::from_utf8(canonicalize_bytes(src.as_bytes()).unwrap()).unwrap();
        let doc = parse(text.as_bytes()).unwrap();
        assert_eq!(doc.multiviews[0].template, MultiviewTemplate::Grid4x4);
        assert!(!doc.multiviews[0].template.has_bus_panes());
        assert_eq!(doc.multiviews[0].tiles.len(), 16);
    }

    #[test]
    fn core_audio_enum_roundtrips() {
        let src = r#"{ "version": 1, "buses": [{ "id": 1, "name": "Master", "role": "Master", "deviceKind": "CoreAudio" }] }"#;
        let doc = parse(src.as_bytes()).unwrap();
        assert_eq!(doc.buses[0].device_kind, AudioDeviceKind::CoreAudio);
        let text = String::from_utf8(to_vec(&doc).unwrap()).unwrap();
        assert!(text.contains("\"CoreAudio\""));
    }
}
