pub const OK: i32 = 0;
pub const ERR_ALREADY_CREATED: i32 = 1;
pub const ERR_NOT_CREATED: i32 = 2;
pub const ERR_INVALID_ARGUMENT: i32 = 3;
pub const ERR_DEVICE: i32 = 4;
pub const ERR_IO: i32 = 5;

pub const SRC_COLOR: u64 = 1;
pub const SRC_BARS: u64 = 2;
pub const SRC_BLACK: u64 = 3;
pub const SRC_BLUE: u64 = 4;

pub const INCOMING_PREVIEW: u64 = 0;
pub const INCOMING_PROGRAM: u64 = u64::MAX;

pub const FMT_UYVY: u32 = 0;
pub const FMT_BGRA: u32 = 1;
pub const FMT_UYVA: u32 = 2;
pub const FMT_RGBA: u32 = 3;

pub const TRANSITION_CUT: u32 = 0;
pub const TRANSITION_FADE: u32 = 1;
pub const TRANSITION_DIP: u32 = 2;
pub const TRANSITION_WIPE: u32 = 3;
pub const TRANSITION_SLIDE: u32 = 4;
pub const TRANSITION_PUSH: u32 = 5;
pub const TRANSITION_IRIS: u32 = 6;
pub const TRANSITION_BLINDS: u32 = 7;
pub const TRANSITION_ZOOM: u32 = 8;
pub const TRANSITION_ADDITIVE: u32 = 9;
pub const TRANSITION_CUBE: u32 = 10;
pub const TRANSITION_CROSS_ZOOM: u32 = 11;
pub const TRANSITION_FLY_ROTATE: u32 = 12;
pub const TRANSITION_BARN_DOOR: u32 = 13;
pub const TRANSITION_CLOCK: u32 = 14;
pub const TRANSITION_LOREZ: u32 = 15;
pub const TRANSITION_METAMIX: u32 = 16;
pub const TRANSITION_TILE: u32 = 17;
pub const TRANSITION_FLIP: u32 = 18;
pub const TRANSITION_GLITCH: u32 = 19;
pub const TRANSITION_SWIRL: u32 = 20;
pub const TRANSITION_LUMA_MORPH: u32 = 21;
pub const TRANSITION_PARTS: u32 = 22;
pub const TRANSITION_STATIC: u32 = 23;
pub const TRANSITION_SHIFT_RGB: u32 = 24;
pub const TRANSITION_DISPLACE: u32 = 25;
pub const TRANSITION_RIPPLE: u32 = 26;
pub const TRANSITION_GRID_DISSOLVE: u32 = 27;
pub const TRANSITION_CUBE_ZOOM: u32 = 28;
pub const TRANSITION_PAGE_CURL: u32 = 29;
pub const TRANSITION_KALEIDOSCOPE: u32 = 30;
pub const TRANSITION_POLAR: u32 = 31;
pub const TRANSITION_FILM_BURN: u32 = 32;
pub const TRANSITION_ZOOM_BLUR: u32 = 33;
pub const TRANSITION_MULTITASK: u32 = 34;
pub const TRANSITION_HEART: u32 = 35;
pub const TRANSITION_DIAMOND: u32 = 36;
pub const TRANSITION_STAR: u32 = 37;
pub const TRANSITION_ROLLER_DOOR: u32 = 38;
pub const TRANSITION_PIXEL_SORT: u32 = 39;
pub const TRANSITION_DATAMOSH: u32 = 40;
pub const TRANSITION_VISUAL_DISSOLVE: u32 = 41;
pub const TRANSITION_OPTICAL_FLOW: u32 = 42;
pub const TRANSITION_BLOOM: u32 = 43;
pub const TRANSITION_CUSTOM: u32 = 50;
pub const TRANSITION_STINGER: u32 = 100;

pub const TRANSITION_DIR_LEFT: u32 = 0;
pub const TRANSITION_DIR_RIGHT: u32 = 1;
pub const TRANSITION_DIR_UP: u32 = 2;
pub const TRANSITION_DIR_DOWN: u32 = 3;

pub const EASING_LINEAR: u32 = 0;
pub const EASING_IN: u32 = 1;
pub const EASING_OUT: u32 = 2;
pub const EASING_IN_OUT: u32 = 3;
pub const EASING_SMOOTHSTEP: u32 = 4;

pub const DURATION_FRAMES: u32 = 0;
pub const DURATION_MS: u32 = 1;

pub const OUTPUT_PROGRAM: u32 = 0;
pub const OUTPUT_PREVIEW: u32 = 1;
pub const OUTPUT_MULTIVIEW: u32 = 2;

pub const SCENE_BASE: u64 = 0x0001_0000;
pub const MULTIVIEW_BASE: u64 = 0x0002_0000;
pub const LABEL_BASE: u64 = 0x0003_0000;
pub const AUDIO_BUS_PEAK_BASE: u64 = 0x0004_0000;
pub const MU_SOURCE_FLAG: u64 = 0x8000_0000_0000_0000;

pub const OUT_OMT: u32 = 0;
pub const OUT_NDI: u32 = 1;
pub const OUT_DECKLINK: u32 = 2;

pub const SRC_KIND_SCENE: u32 = 0;
pub const SRC_KIND_MU_PREVIEW: u32 = 1;
pub const SRC_KIND_MU_PROGRAM: u32 = 2;
pub const SRC_KIND_MU_MULTIVIEW: u32 = 3;
pub const SRC_KIND_INPUT: u32 = 4;

pub const GEN_SOLID: u32 = 0;
pub const GEN_BARS: u32 = 1;

pub const SAVE_ALWAYS_LOW: u32 = 0;
pub const SAVE_NOT_ON_PROGRAM: u32 = 1;
pub const SAVE_NOT_ON_PREVIEW_OR_PROGRAM: u32 = 2;
pub const SAVE_ALWAYS_FULL: u32 = 3;
pub const SAVE_FLAG_MULTIVIEW: u32 = 1;

pub const MV_SLOT_MAX: usize = 16;
pub const MU_BUS_PREVIEW: u64 = 0x1000_0000_0000_0000;
pub const MU_BUS_MULTIVIEW: u64 = 0x2000_0000_0000_0000;
pub const MU_ID_MASK: u64 = 0x0FFF_FFFF_FFFF_FFFF;

pub const NATIVE_WIN32_HWND: u32 = 1;
pub const NATIVE_APPKIT_NSVIEW: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct NativeSurface {
    pub kind: u32,
    pub handle: isize,
}

impl NativeSurface {
    pub fn parse(kind: u32, handle: isize) -> Result<Self, i32> {
        if handle == 0 {
            return Err(ERR_INVALID_ARGUMENT);
        }
        let supported = match kind {
            NATIVE_WIN32_HWND => cfg!(windows),
            NATIVE_APPKIT_NSVIEW => cfg!(target_os = "macos"),
            _ => false,
        };
        if !supported {
            return Err(ERR_INVALID_ARGUMENT);
        }
        Ok(Self { kind, handle })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MixerVideoInfo {
    pub playing: u32,
    pub is_file: u32,
    pub position_hns: i64,
    pub duration_hns: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VideoCaptureInfo {
    pub id: [u8; 512],
    pub name: [u8; 256],
}

impl Default for VideoCaptureInfo {
    fn default() -> Self {
        Self {
            id: [0; 512],
            name: [0; 256],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct VideoCaptureMode {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub format: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MixerRebarInfo {
    pub available: u32,
    pub active: u32,
    pub uma: u32,
    pub gpu_upload_heaps: u32,
    pub bar_bytes: u64,
    pub vram_bytes: u64,
    pub adapter: [u8; 128],
}

impl Default for MixerRebarInfo {
    fn default() -> Self {
        Self {
            available: 0,
            active: 0,
            uma: 0,
            gpu_upload_heaps: 0,
            bar_bytes: 0,
            vram_bytes: 0,
            adapter: [0; 128],
        }
    }
}

/// The ABI intentionally consists of fixed-width plain data only.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OverlayDesc {
    pub source_id: u64,
    pub rect: Rect,
    pub crop: Rect,
    pub opacity: f32,
    pub z: i32,
    pub audio_follow: u32,
    pub hidden: u32,
    pub label: *const std::ffi::c_char,
}

impl Default for OverlayDesc {
    fn default() -> Self {
        Self {
            source_id: 0,
            rect: Rect::default(),
            crop: Rect::default(),
            opacity: 0.0,
            z: 0,
            audio_follow: 0,
            hidden: 0,
            label: std::ptr::null(),
        }
    }
}

/// Host copies the UTF-8 string in `mixer_define_scene` and then nulls `label`.
/// Stored overlays never dereference this pointer.
unsafe impl Send for OverlayDesc {}
unsafe impl Sync for OverlayDesc {}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct UnitState {
    pub program_source: u64,
    pub preview_source: u64,
    pub mix: f32,
    pub transition_kind: u32,
    pub overlay_count: u32,
    pub mv_slot_count: u32,
    pub overlays: [OverlayDesc; 8],
    pub mv_slots: [u64; 16],
    pub transition_easing: u32,
    pub transition_direction: u32,
    pub keep_preview: u32,
    pub pad: u32,
    pub dip_r: f32,
    pub dip_g: f32,
    pub dip_b: f32,
    pub dip_a: f32,
    /// Report of the active mix incoming when it is not Preview. `0` means Preview.
    /// Auto/Cut arguments: `0` Preview, `u64::MAX` (`-1`) Program, otherwise a source id.
    pub incoming_source: u64,
    pub softness: f32,
    pub param: f32,
}

impl UnitState {
    pub fn mix_incoming(&self) -> u64 {
        if self.incoming_source != 0 {
            self.incoming_source
        } else {
            self.preview_source
        }
    }
}

pub type UnitSnap = (u64, u32, u32, u32, u32, UnitState, u64, Option<String>);

pub fn mixing_unit_source(unit_id: u64) -> u64 {
    MU_SOURCE_FLAG | (unit_id & MU_ID_MASK)
}

pub fn mixing_unit_preview(unit_id: u64) -> u64 {
    MU_SOURCE_FLAG | MU_BUS_PREVIEW | (unit_id & MU_ID_MASK)
}

pub fn mixing_unit_multiview(unit_id: u64) -> u64 {
    MU_SOURCE_FLAG | MU_BUS_MULTIVIEW | (unit_id & MU_ID_MASK)
}

pub fn mixing_unit_from_source(source_id: u64) -> Option<u64> {
    if source_id & MU_SOURCE_FLAG == 0 {
        None
    } else {
        Some(source_id & MU_ID_MASK)
    }
}

pub fn mixing_unit_bus(source_id: u64) -> u32 {
    if source_id & MU_BUS_MULTIVIEW != 0 {
        OUTPUT_MULTIVIEW
    } else if source_id & MU_BUS_PREVIEW != 0 {
        OUTPUT_PREVIEW
    } else {
        OUTPUT_PROGRAM
    }
}

pub fn is_scene(source_id: u64) -> bool {
    source_id >= SCENE_BASE && source_id < MU_SOURCE_FLAG
}

pub fn is_multiview(source_id: u64) -> bool {
    source_id >= MULTIVIEW_BASE && source_id < LABEL_BASE
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct AudioPeak {
    pub source_id: u64,
    pub left: f32,
    pub right: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MixerStats {
    pub render_ms: f32,
    pub frame_budget_ms: f32,
    pub ram_bytes: u64,
    pub vram_bytes: u64,
    pub compose_vram_bytes: u64,
    pub delay_vram_bytes: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SourceUsage {
    pub source_id: u64,
    pub width: u32,
    pub height: u32,
    pub ram_bytes: u64,
    pub vram_bytes: u64,
    pub gpu_pct: f32,
}
