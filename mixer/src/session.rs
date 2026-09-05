//! Canonical session JSON shared by every host.
//! Shape matches `host/SessionStore.cs` (camelCase, string enums, version 2).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    #[serde(default = "version_two")]
    pub version: i32,
    #[serde(default)]
    pub scene_presets: Vec<SceneLayoutPreset>,
    #[serde(default)]
    pub input_tags: Vec<String>,
    #[serde(default)]
    pub scene_tags: Vec<String>,
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

fn version_two() -> i32 {
    2
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
    pub flip_swapchain_limit: u32,
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
    #[serde(default = "mv_label_size")]
    pub multiview_label_size: f32,
    #[serde(default)]
    pub multiview_label_unit: MvLabelUnit,
    #[serde(default)]
    pub multiview_label_anchor: MvLabelAnchor,
    pub last_session_path: Option<String>,
    #[serde(default = "default_true", deserialize_with = "de_bool_null_true")]
    pub vmix_api_enabled: bool,
    #[serde(default = "api_port")]
    pub vmix_api_port: u32,
    #[serde(default)]
    pub vmix_api_user: String,
    #[serde(default)]
    pub vmix_api_password: String,
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
            flip_swapchain_limit: 0,
            internal_color_format: InternalColorFormat::Uyvy,
            rebar_optimization: true,
            rebar_direct_sample: false,
            ndi_gpu_upload: true,
            preview_color: preview_color(),
            program_color: program_color(),
            inactive_color: inactive_color(),
            multiview_label_size: mv_label_size(),
            multiview_label_unit: MvLabelUnit::Px,
            multiview_label_anchor: MvLabelAnchor::Bottom,
            last_session_path: None,
            vmix_api_enabled: true,
            vmix_api_port: api_port(),
            vmix_api_user: String::new(),
            vmix_api_password: String::new(),
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

fn de_preview_color<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<RgbColor, D::Error> {
    Ok(Option::<RgbColor>::deserialize(deserializer)?.unwrap_or_else(preview_color))
}

fn de_program_color<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<RgbColor, D::Error> {
    Ok(Option::<RgbColor>::deserialize(deserializer)?.unwrap_or_else(program_color))
}

fn inactive_color() -> RgbColor {
    RgbColor {
        r: 64,
        g: 64,
        b: 64,
    }
}

fn de_inactive_color<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<RgbColor, D::Error> {
    Ok(Option::<RgbColor>::deserialize(deserializer)?.unwrap_or_else(inactive_color))
}

fn mv_label_size() -> f32 {
    18.0
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum MvLabelUnit {
    #[default]
    Px,
    Percent,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum MvLabelAnchor {
    Top,
    #[default]
    Bottom,
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

fn api_port() -> u32 {
    8088
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
    Mix,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum MixSource {
    MuPreview,
    #[default]
    MuProgram,
    SessionMultiview,
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
    PreviewProgram8Bottom,
    PreviewProgram8Left,
    PreviewProgram8Right,
    PreviewProgram2,
    Quad4TopLeft,
    Quad4TopRight,
    Quad4BottomLeft,
    Quad4BottomRight,
    Large5TopLeft,
    Large5TopRight,
    Large5BottomLeft,
    Large5BottomRight,
    Grid2x2,
    Grid3x3,
    Grid4x4,
}

impl MultiviewTemplate {
    pub fn tile_count(self) -> usize {
        match self {
            Self::PreviewProgram2 | Self::Grid2x2 => 4,
            Self::Quad4TopLeft
            | Self::Quad4TopRight
            | Self::Quad4BottomLeft
            | Self::Quad4BottomRight => 7,
            Self::Large5TopLeft
            | Self::Large5TopRight
            | Self::Large5BottomLeft
            | Self::Large5BottomRight => 6,
            Self::Grid3x3 => 9,
            Self::Grid4x4 => 16,
            _ => 10,
        }
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
    pub guid: String,
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
    #[serde(default)]
    pub capture_width: u32,
    #[serde(default)]
    pub capture_height: u32,
    #[serde(default)]
    pub capture_fps_num: u32,
    #[serde(default)]
    pub capture_fps_den: u32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub mix_source: MixSource,
    #[serde(default)]
    pub mix_target_id: u64,
    #[serde(default)]
    pub mix_audio_bus_id: u64,
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
    #[serde(default)]
    pub locked: bool,
    #[serde(default = "true_bool")]
    pub size_linked: bool,
    #[serde(default)]
    pub crop_x: f32,
    #[serde(default)]
    pub crop_y: f32,
    #[serde(default = "one_f32")]
    pub crop_width: f32,
    #[serde(default = "one_f32")]
    pub crop_height: f32,
    #[serde(default)]
    pub hidden: bool,
}

fn true_bool() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneDto {
    pub id: u64,
    #[serde(default)]
    pub guid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub layers: Vec<SceneLayer>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub preview_collapsed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneLayoutPreset {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub layers: Vec<SceneLayerGeom>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneLayerGeom {
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
    #[serde(default)]
    pub crop_x: f32,
    #[serde(default)]
    pub crop_y: f32,
    #[serde(default = "one_f32")]
    pub crop_width: f32,
    #[serde(default = "one_f32")]
    pub crop_height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionPreset {
    #[serde(default = "fade")]
    pub kind: u32,
    #[serde(default = "thirty")]
    pub duration_value: u32,
    #[serde(default)]
    pub duration_unit: u32,
    #[serde(default = "true_bool")]
    pub swap: bool,
    #[serde(default = "true_bool")]
    pub keep_preview: bool,
    #[serde(default)]
    pub easing: u32,
    #[serde(default)]
    pub direction: u32,
    #[serde(default)]
    pub dip_r: f32,
    #[serde(default)]
    pub dip_g: f32,
    #[serde(default)]
    pub dip_b: f32,
    #[serde(default = "one_f32")]
    pub dip_a: f32,
    #[serde(default = "softness_default")]
    pub softness: f32,
    #[serde(default)]
    pub param: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_wgsl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

fn fade() -> u32 {
    1
}
fn thirty() -> u32 {
    30
}
fn softness_default() -> f32 {
    0.02
}

pub(crate) fn transition_label(kind: u32) -> &'static str {
    match kind {
        0 => "Cut",
        2 => "Dip",
        3 => "Wipe",
        4 => "Slide",
        5 => "Push",
        6 => "Iris",
        7 => "Blinds",
        8 => "Zoom",
        9 => "Additive",
        10 => "Cube",
        11 => "CrossZoom",
        12 => "FlyRotate",
        13 => "BarnDoor",
        14 => "Clock",
        15 => "LoRez",
        16 => "MetaMix",
        17 => "Tile",
        18 => "Flip",
        19 => "Glitch",
        20 => "Swirl",
        21 => "LumaMorph",
        22 => "Parts",
        23 => "Static",
        24 => "Shift RGB",
        25 => "Displace",
        26 => "Ripple",
        27 => "GridDissolve",
        28 => "CubeZoom",
        29 => "PageCurl",
        30 => "Kaleidoscope",
        31 => "Polar",
        32 => "FilmBurn",
        33 => "ZoomBlur",
        34 => "MultiTask",
        35 => "Heart",
        36 => "Diamond",
        37 => "Star",
        38 => "RollerDoor",
        39 => "PixelSort",
        40 => "Datamosh",
        41 => "VisualDissolve",
        42 => "OpticalFlow",
        43 => "Bloom",
        50 => "Custom",
        100 => "Stinger",
        _ => "Fade",
    }
}

impl Default for TransitionPreset {
    fn default() -> Self {
        Self {
            kind: 1,
            duration_value: 30,
            duration_unit: 0,
            swap: true,
            keep_preview: true,
            easing: 0,
            direction: 0,
            dip_r: 0.0,
            dip_g: 0.0,
            dip_b: 0.0,
            dip_a: 1.0,
            softness: 0.02,
            param: 0.0,
            custom_wgsl: None,
            label: None,
        }
    }
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
    #[serde(default = "fade")]
    pub transition_kind: u32,
    #[serde(default = "fifteen")]
    pub duration_value: u32,
    #[serde(default)]
    pub duration_unit: u32,
    #[serde(default = "true_bool")]
    pub audio_follow: bool,
    #[serde(default)]
    pub source_kind: u32,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub hidden: bool,
}

fn fifteen() -> u32 {
    15
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
    #[serde(default = "one")]
    pub audio_bus_id: u64,
    #[serde(default)]
    pub audio_link: AudioLinkMode,
    #[serde(default)]
    pub switcher_scene_filter: SwitcherSceneFilter,
    #[serde(default)]
    pub switcher_scene_ids: Vec<u64>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum SwitcherSceneFilter {
    #[default]
    All,
    Include,
    Exclude,
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
    #[serde(default = "one")]
    pub audio_bus_id: u64,
    #[serde(default = "default_true")]
    pub skip_encode_when_no_receivers: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_anchor: Option<MvLabelAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_unit: Option<MvLabelUnit>,
    #[serde(default = "default_true")]
    pub always_on_top: bool,
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
        doc.version = 2;
        doc.settings.frame_buffer_frames = doc.settings.frame_buffer_frames.clamp(1, 8);
        if doc.settings.frame_buffer_frames == 0 {
            doc.settings.frame_buffer_frames = 3;
        }
        doc.settings.default_present_interval = doc.settings.default_present_interval.clamp(1, 8);
        doc.settings.flip_swapchain_limit = match doc.settings.flip_swapchain_limit {
            0 | 4 | 6 | 8 | 10 | 12 | 16 => doc.settings.flip_swapchain_limit,
            _ => 0,
        };
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
        if doc.settings.vmix_api_port == 0 || doc.settings.vmix_api_port > 65535 {
            doc.settings.vmix_api_port = api_port();
        }
        doc.settings.multiview_label_size =
            crate::labels::clamp_size(doc.settings.multiview_label_size);
        for input in &mut doc.inputs {
            if input.kind == InputKind::Mix {
                input.bus_mask = 0;
            } else if input.bus_mask == 0 {
                input.bus_mask = 1;
            }
            if input.frame_buffer_frames == 0 {
                input.frame_buffer_frames = 1;
            }
            input.frame_buffer_frames = input.frame_buffer_frames.clamp(1, 8);
            if input.gain < 0.0 {
                input.gain = 1.0;
            }
            if input.kind != InputKind::Mix {
                input.mix_source = MixSource::MuProgram;
                input.mix_target_id = 0;
                input.mix_audio_bus_id = 0;
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
                        duration_value: 1,
                        label: Some("Cut".into()),
                        ..TransitionPreset::default()
                    },
                    TransitionPreset {
                        kind: 1,
                        duration_value: 30,
                        label: Some("Fade".into()),
                        ..TransitionPreset::default()
                    },
                ];
            } else {
                for preset in &mut unit.transitions {
                    preset.label = Some(transition_label(preset.kind).into());
                    if preset.duration_value == 0 {
                        preset.duration_value = 1;
                    }
                }
            }
        }
        for output in &mut doc.outputs {
            if output.source_kind == OutputSourceKind::Multiview {
                output.audio_bus_id = 0;
            }
        }
        for layout in &mut doc.multiviews {
            if layout.present_interval > 0 {
                layout.present_interval = layout.present_interval.clamp(1, 8);
            }
            if let Some(size) = layout.label_size {
                layout.label_size = Some(crate::labels::clamp_size(size));
            }
            absorb_fixed_bus_panes(layout);
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
            doc.next_multiview_id =
                doc.multiviews.iter().map(|item| item.id).max().unwrap_or(0) + 1;
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

fn absorb_fixed_bus_panes(layout: &mut MultiviewDto) {
    let prv = layout.preview_unit_id.max(1);
    let pgm = layout.program_unit_id.max(1);
    let buses = [
        MvSlot {
            kind: MvSlotKind::MuPreview,
            source_id: prv,
            label_follow: layout.preview_label_follow,
            label: layout.preview_label.clone(),
        },
        MvSlot {
            kind: MvSlotKind::MuProgram,
            source_id: pgm,
            label_follow: layout.program_label_follow,
            label: layout.program_label.clone(),
        },
    ];
    match layout.template {
        MultiviewTemplate::PreviewProgram2 if layout.tiles.len() == 2 => {
            layout.tiles.splice(0..0, buses);
        }
        MultiviewTemplate::PreviewProgram8 | MultiviewTemplate::PreviewProgram8Left
            if layout.tiles.len() == 8 =>
        {
            layout.tiles.splice(0..0, buses);
        }
        MultiviewTemplate::PreviewProgram8Bottom | MultiviewTemplate::PreviewProgram8Right
            if layout.tiles.len() == 8 =>
        {
            layout.tiles.extend(buses);
        }
        _ => {}
    }
}

pub fn canonicalize_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    to_vec(&parse(bytes)?)
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
  "version": 2,
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
        assert_eq!(doc.outputs[0].audio_bus_id, 1);
        assert!(doc.outputs[0].skip_encode_when_no_receivers);
        assert_eq!(doc.buses[0].device_kind, AudioDeviceKind::Wasapi);
        assert_eq!(doc.units[0].transitions.len(), 2);
        assert!(doc.settings.rebar_optimization);
        assert!(!doc.settings.rebar_direct_sample);
        assert!(doc.settings.ndi_gpu_upload);
        assert_eq!(doc.settings.preview_color, RgbColor { r: 0, g: 255, b: 0 });
        assert_eq!(doc.settings.program_color, RgbColor { r: 255, g: 0, b: 0 });
        assert_eq!(
            doc.settings.inactive_color,
            RgbColor {
                r: 64,
                g: 64,
                b: 64
            }
        );
        assert!(doc.input_tags.is_empty());
        assert!(doc.scene_tags.is_empty());
        assert!(doc.inputs[0].tags.is_empty());
        assert!(doc.scenes[0].tags.is_empty());
        assert!(!doc.scenes[0].preview_collapsed);
        assert_eq!(doc.units[0].switcher_scene_filter, SwitcherSceneFilter::All);
        assert!(doc.units[0].switcher_scene_ids.is_empty());
    }

    #[test]
    fn explicit_output_audio_none_is_kept() {
        let src = r#"{
  "version": 2,
  "outputs": [{ "id": 100, "name": "eiviz-pgm", "transport": "Omt", "audioBusId": 0 }]
}"#;
        let doc = parse(src.as_bytes()).unwrap();
        assert_eq!(doc.outputs[0].audio_bus_id, 0);
    }

    #[test]
    fn skip_encode_when_no_receivers_false_roundtrip() {
        let src = r#"{
  "version": 2,
  "outputs": [{ "id": 100, "name": "eiviz-pgm", "transport": "Omt", "skipEncodeWhenNoReceivers": false }]
}"#;
        let doc = parse(src.as_bytes()).unwrap();
        assert!(!doc.outputs[0].skip_encode_when_no_receivers);
        let bytes = canonicalize_bytes(src.as_bytes()).expect("canonicalize");
        let again = parse(&bytes).unwrap();
        assert!(!again.outputs[0].skip_encode_when_no_receivers);
    }

    #[test]
    fn multiview_output_audio_is_forced_silent() {
        let src = r#"{
  "version": 2,
  "outputs": [{ "id": 100, "name": "eiviz-mv", "transport": "Omt", "sourceKind": "Multiview", "audioBusId": 1 }]
}"#;
        let doc = parse(src.as_bytes()).unwrap();
        assert_eq!(doc.outputs[0].audio_bus_id, 0);
    }

    #[test]
    fn tags_and_preview_collapsed_roundtrip() {
        let src = r#"{
  "version": 2,
  "inputTags": ["Cameras", "VTR"],
  "sceneTags": ["Open"],
  "inputs": [{ "id": 10, "name": "Cam 1", "kind": "Ndi", "tags": ["Cameras"] }],
  "scenes": [{ "id": 1, "name": "Scene 1", "tags": ["Open"], "previewCollapsed": true }]
}"#;
        let doc = parse(src.as_bytes()).unwrap();
        assert_eq!(doc.input_tags, vec!["Cameras", "VTR"]);
        assert_eq!(doc.scene_tags, vec!["Open"]);
        assert_eq!(doc.inputs[0].tags, vec!["Cameras"]);
        assert_eq!(doc.scenes[0].tags, vec!["Open"]);
        assert!(doc.scenes[0].preview_collapsed);
        let text = String::from_utf8(to_vec(&doc).unwrap()).unwrap();
        let again = parse(text.as_bytes()).unwrap();
        assert_eq!(again.input_tags, vec!["Cameras", "VTR"]);
        assert_eq!(again.scene_tags, vec!["Open"]);
        assert_eq!(again.inputs[0].tags, vec!["Cameras"]);
        assert_eq!(again.scenes[0].tags, vec!["Open"]);
        assert!(again.scenes[0].preview_collapsed);
    }

    #[test]
    fn mix_input_roundtrip_and_legacy_defaults() {
        let legacy = r#"{
  "version": 2,
  "inputs": [{ "id": 2, "name": "SMPTE Bars", "kind": "Bars" }]
}"#;
        let doc = parse(legacy.as_bytes()).unwrap();
        assert_eq!(doc.inputs[0].mix_source, MixSource::MuProgram);
        assert_eq!(doc.inputs[0].mix_target_id, 0);
        assert_eq!(doc.inputs[0].mix_audio_bus_id, 0);

        let src = r#"{
  "version": 2,
  "inputs": [{
    "id": 20,
    "name": "MU1 PGM",
    "kind": "Mix",
    "mixSource": "MuPreview",
    "mixTargetId": 1,
    "mixAudioBusId": 2,
    "frameBufferFrames": 4
  }]
}"#;
        let mix = parse(src.as_bytes()).unwrap();
        assert_eq!(mix.inputs[0].kind, InputKind::Mix);
        assert_eq!(mix.inputs[0].mix_source, MixSource::MuPreview);
        assert_eq!(mix.inputs[0].mix_target_id, 1);
        assert_eq!(mix.inputs[0].mix_audio_bus_id, 2);
        assert_eq!(mix.inputs[0].frame_buffer_frames, 4);
        assert_eq!(mix.inputs[0].bus_mask, 0);
        let text = String::from_utf8(to_vec(&mix).unwrap()).unwrap();
        let again = parse(text.as_bytes()).unwrap();
        assert_eq!(again.inputs[0].kind, InputKind::Mix);
        assert_eq!(again.inputs[0].mix_source, MixSource::MuPreview);
        assert_eq!(again.inputs[0].mix_target_id, 1);
        assert_eq!(again.inputs[0].mix_audio_bus_id, 2);
        assert_eq!(again.inputs[0].frame_buffer_frames, 4);
        assert_eq!(again.inputs[0].bus_mask, 0);
    }

    #[test]
    fn switcher_scene_filter_defaults_and_roundtrip() {
        let missing = r#"{
  "version": 2,
  "units": [{ "id": 1, "name": "Mixing Unit 1" }]
}"#;
        let doc = parse(missing.as_bytes()).unwrap();
        assert_eq!(doc.units[0].switcher_scene_filter, SwitcherSceneFilter::All);
        assert!(doc.units[0].switcher_scene_ids.is_empty());

        let src = r#"{
  "version": 2,
  "units": [{
    "id": 1,
    "name": "Mixing Unit 1",
    "switcherSceneFilter": "Exclude",
    "switcherSceneIds": [3, 5]
  }]
}"#;
        let filtered = parse(src.as_bytes()).unwrap();
        assert_eq!(
            filtered.units[0].switcher_scene_filter,
            SwitcherSceneFilter::Exclude
        );
        assert_eq!(filtered.units[0].switcher_scene_ids, vec![3, 5]);
        let text = String::from_utf8(to_vec(&filtered).unwrap()).unwrap();
        let again = parse(text.as_bytes()).unwrap();
        assert_eq!(
            again.units[0].switcher_scene_filter,
            SwitcherSceneFilter::Exclude
        );
        assert_eq!(again.units[0].switcher_scene_ids, vec![3, 5]);
    }

    #[test]
    fn preview_program_colors_roundtrip() {
        let src = r#"{
  "version": 2,
  "settings": {
    "previewColor": { "r": 10, "g": 20, "b": 30 },
    "programColor": { "r": 40, "g": 50, "b": 60 }
  }
}"#;
        let doc = parse(src.as_bytes()).unwrap();
        assert_eq!(
            doc.settings.preview_color,
            RgbColor {
                r: 10,
                g: 20,
                b: 30
            }
        );
        assert_eq!(
            doc.settings.program_color,
            RgbColor {
                r: 40,
                g: 50,
                b: 60
            }
        );
        let text = String::from_utf8(to_vec(&doc).unwrap()).unwrap();
        let again = parse(text.as_bytes()).unwrap();
        assert_eq!(
            again.settings.preview_color,
            RgbColor {
                r: 10,
                g: 20,
                b: 30
            }
        );
        assert_eq!(
            again.settings.program_color,
            RgbColor {
                r: 40,
                g: 50,
                b: 60
            }
        );
        assert_eq!(
            again.settings.inactive_color,
            RgbColor {
                r: 64,
                g: 64,
                b: 64
            }
        );
        assert_eq!(again.settings.multiview_label_size, 18.0);
        assert_eq!(again.settings.multiview_label_unit, MvLabelUnit::Px);
        assert_eq!(again.settings.multiview_label_anchor, MvLabelAnchor::Bottom);
    }

    #[test]
    fn mv_label_size_roundtrip() {
        let src = r#"{
  "version": 2,
  "settings": { "multiviewLabelSize": 4, "multiviewLabelUnit": "Percent", "multiviewLabelAnchor": "Top" }
}"#;
        let doc = parse(src.as_bytes()).unwrap();
        assert_eq!(doc.settings.multiview_label_size, 4.0);
        assert_eq!(doc.settings.multiview_label_unit, MvLabelUnit::Percent);
        assert_eq!(doc.settings.multiview_label_anchor, MvLabelAnchor::Top);
    }

    #[test]
    fn inactive_color_and_mv_labels_roundtrip() {
        let src = r#"{
  "version": 2,
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
        assert_eq!(
            doc.multiviews[0].template,
            MultiviewTemplate::PreviewProgram8
        );
        assert_eq!(doc.multiviews[0].label_anchor, None);
        assert_eq!(doc.multiviews[0].label_size, None);
        assert_eq!(doc.multiviews[0].label_unit, None);
        assert!(doc.multiviews[0].always_on_top);
    }

    #[test]
    fn mv_per_layout_fields_roundtrip() {
        let src = r#"{
  "version": 2,
  "multiviews": [{
    "id": 1,
    "name": "MV",
    "labelAnchor": "Top",
    "labelSize": 6,
    "labelUnit": "Percent",
    "alwaysOnTop": false
  }]
}"#;
        let doc = parse(src.as_bytes()).unwrap();
        assert_eq!(doc.multiviews[0].label_anchor, Some(MvLabelAnchor::Top));
        assert_eq!(doc.multiviews[0].label_size, Some(6.0));
        assert_eq!(doc.multiviews[0].label_unit, Some(MvLabelUnit::Percent));
        assert!(!doc.multiviews[0].always_on_top);
        let text = String::from_utf8(to_vec(&doc).unwrap()).unwrap();
        let again = parse(text.as_bytes()).unwrap();
        assert_eq!(again.multiviews[0].label_anchor, Some(MvLabelAnchor::Top));
        assert_eq!(again.multiviews[0].label_size, Some(6.0));
        assert_eq!(again.multiviews[0].label_unit, Some(MvLabelUnit::Percent));
        assert!(!again.multiviews[0].always_on_top);
    }

    #[test]
    fn multiview_template_canonicalize_tile_count() {
        let src = r#"{
  "version": 2,
  "multiviews": [{ "id": 1, "name": "MV", "template": "Grid4x4", "tiles": [] }]
}"#;
        let text = String::from_utf8(canonicalize_bytes(src.as_bytes()).unwrap()).unwrap();
        let doc = parse(text.as_bytes()).unwrap();
        assert_eq!(doc.multiviews[0].template, MultiviewTemplate::Grid4x4);
        assert_eq!(doc.multiviews[0].tiles.len(), 16);
    }

    #[test]
    fn multiview_aspect_templates_tile_counts() {
        assert_eq!(MultiviewTemplate::PreviewProgram8Left.tile_count(), 10);
        assert_eq!(MultiviewTemplate::PreviewProgram2.tile_count(), 4);
        assert_eq!(MultiviewTemplate::Quad4TopLeft.tile_count(), 7);
        assert_eq!(MultiviewTemplate::Large5TopLeft.tile_count(), 6);
    }

    #[test]
    fn multiview_absorbs_fixed_bus_panes() {
        let src = r#"{
  "version": 2,
  "multiviews": [{
    "id": 1,
    "name": "MV",
    "template": "PreviewProgram8",
    "previewUnitId": 2,
    "programUnitId": 3,
    "tiles": [
      { "kind": "Input", "sourceId": 2 },
      { "kind": "Input", "sourceId": 3 },
      { "kind": "None" },
      { "kind": "None" },
      { "kind": "None" },
      { "kind": "None" },
      { "kind": "None" },
      { "kind": "None" }
    ]
  }]
}"#;
        let doc = parse(src.as_bytes()).unwrap();
        assert_eq!(doc.multiviews[0].tiles.len(), 10);
        assert_eq!(doc.multiviews[0].tiles[0].kind, MvSlotKind::MuPreview);
        assert_eq!(doc.multiviews[0].tiles[0].source_id, 2);
        assert_eq!(doc.multiviews[0].tiles[1].kind, MvSlotKind::MuProgram);
        assert_eq!(doc.multiviews[0].tiles[1].source_id, 3);
        assert_eq!(doc.multiviews[0].tiles[2].kind, MvSlotKind::Input);
        assert_eq!(doc.multiviews[0].tiles[2].source_id, 2);
    }

    #[test]
    fn core_audio_enum_roundtrips() {
        let src = r#"{ "version": 2, "buses": [{ "id": 1, "name": "Master", "role": "Master", "deviceKind": "CoreAudio" }] }"#;
        let doc = parse(src.as_bytes()).unwrap();
        assert_eq!(doc.buses[0].device_kind, AudioDeviceKind::CoreAudio);
        let text = String::from_utf8(to_vec(&doc).unwrap()).unwrap();
        assert!(text.contains("\"CoreAudio\""));
    }

    #[test]
    fn version_two_transition_roundtrips() {
        let src = r#"{
  "version": 2,
  "units": [{
    "id": 1,
    "name": "MU",
    "transitions": [{
      "kind": 2,
      "durationValue": 12,
      "durationUnit": 1,
      "swap": true,
      "keepPreview": true,
      "easing": 3,
      "direction": 1,
      "dipR": 0.1,
      "dipG": 0.2,
      "dipB": 0.3,
      "dipA": 1.0
    }]
  }]
}"#;
        let doc = parse(src.as_bytes()).unwrap().canonicalize();
        assert_eq!(doc.version, 2);
        assert_eq!(doc.units[0].transitions[0].kind, 2);
        assert_eq!(doc.units[0].transitions[0].duration_value, 12);
        assert_eq!(doc.units[0].transitions[0].duration_unit, 1);
        assert!(doc.units[0].transitions[0].keep_preview);
        assert_eq!(doc.units[0].transitions[0].easing, 3);
        assert!((doc.units[0].transitions[0].dip_g - 0.2).abs() < f32::EPSILON);
    }
}
