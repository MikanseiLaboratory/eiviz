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

pub const FMT_UYVY: u32 = 0;
pub const FMT_BGRA: u32 = 1;
pub const FMT_UYVA: u32 = 2;
pub const FMT_RGBA: u32 = 3;

pub const TRANSITION_CUT: u32 = 0;
pub const TRANSITION_FADE: u32 = 1;
pub const TRANSITION_DIP: u32 = 2;

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
#[derive(Clone, Copy, Debug, Default)]
pub struct OverlayDesc {
    pub source_id: u64,
    pub rect: Rect,
    pub opacity: f32,
    pub z: i32,
    pub audio_follow: u32,
    pub pad: u32,
}

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
}

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
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SourceUsage {
    pub source_id: u64,
    pub width: u32,
    pub height: u32,
    pub ram_bytes: u64,
    pub vram_bytes: u64,
}
