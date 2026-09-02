use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::abi::{
    is_multiview, is_scene, mixing_unit_bus, mixing_unit_from_source, mixing_unit_multiview,
    mixing_unit_preview, mixing_unit_source, OverlayDesc, Rect, UnitState, GEN_BARS, GEN_SOLID, LABEL_BASE, MV_SLOT_MAX,
    OUTPUT_MULTIVIEW, OUTPUT_PREVIEW, OUTPUT_PROGRAM, SRC_BARS, SRC_BLACK, SRC_BLUE, SRC_COLOR,
    TRANSITION_BLOOM, TRANSITION_CUSTOM, TRANSITION_DATAMOSH, TRANSITION_FILM_BURN,
    TRANSITION_OPTICAL_FLOW, TRANSITION_STINGER,
};
use crate::device::GpuDevice;
use crate::pool::{uniform_dyn, UniformPool};
use crate::upload::{texture_bytes, CpuFormat, CpuFrameSnap};

const KEY_TALLY_RED: u64 = LABEL_BASE + 0xFF01;
const KEY_TALLY_GREEN: u64 = LABEL_BASE + 0xFF02;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ColorParams {
    color: [f32; 4],
    scroll: f32,
    flags: f32,
    scroll_y: f32,
    pad: f32,
}

#[derive(Clone, Copy, PartialEq)]
pub struct Generator {
    pub kind: u32,
    pub color: [f32; 4],
    pub scroll: bool,
    pub tone_hz: f32,
    pub tone_level_dbfs: f32,
}

impl Default for Generator {
    fn default() -> Self {
        Self {
            kind: GEN_SOLID,
            color: [0.0, 0.0, 0.0, 1.0],
            scroll: false,
            tone_hz: 0.0,
            tone_level_dbfs: -20.0,
        }
    }
}

fn generator_needs_rebake(spec: &Generator, last: Option<&Generator>, size_ok: bool) -> bool {
    spec.scroll || !size_ok || last != Some(spec)
}

const FULL_UV: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

fn crop_uv(crop: Rect) -> [f32; 4] {
    const MIN: f32 = 0.001;
    if crop.width <= 0.0 || crop.height <= 0.0 {
        FULL_UV
    } else {
        let x = crop.x.clamp(0.0, 1.0 - MIN);
        let y = crop.y.clamp(0.0, 1.0 - MIN);
        [
            x,
            y,
            crop.width.clamp(MIN, 1.0 - x),
            crop.height.clamp(MIN, 1.0 - y),
        ]
    }
}

fn crop_blit(rect: [f32; 4], crop: Rect) -> ([f32; 4], [f32; 4]) {
    let uv = crop_uv(crop);
    (
        [
            rect[0] + rect[2] * uv[0],
            rect[1] + rect[3] * uv[1],
            rect[2] * uv[2],
            rect[3] * uv[3],
        ],
        uv,
    )
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlitParams {
    dst: [f32; 4],
    src: [f32; 4],
    opacity: f32,
    pad: [f32; 3],
}

enum Fx2Kind {
    Flow,
    Bloom,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MixParams {
    mix: f32,
    kind: u32,
    direction: u32,
    softness: f32,
    dip: [f32; 4],
    param: f32,
    time: f32,
    resolution: [f32; 2],
}

struct SourceGpu {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    packed: bool,
    bgra: bool,
    uploaded_pts: i64,
    direct: bool,
    owned: bool,
}

pub struct UnitTargets {
    pub width: u32,
    pub height: u32,
    pub program: wgpu::Texture,
    pub preview: wgpu::Texture,
    pub mixed: wgpu::Texture,
    pub prev: wgpu::Texture,
    sort_a: wgpu::Texture,
    sort_b: wgpu::Texture,
    flow: wgpu::Texture,
    bloom_a: wgpu::Texture,
    bloom_b: wgpu::Texture,
    aux: wgpu::Texture,
    pub packed: Option<wgpu::Texture>,
    pub packed_mv: Option<wgpu::Texture>,
    pub packed_prv: Option<wgpu::Texture>,
    pub multiview: Option<wgpu::Texture>,
    program_view: wgpu::TextureView,
    preview_view: wgpu::TextureView,
    mixed_view: wgpu::TextureView,
    prev_view: wgpu::TextureView,
    sort_a_view: wgpu::TextureView,
    sort_b_view: wgpu::TextureView,
    flow_view: wgpu::TextureView,
    bloom_a_view: wgpu::TextureView,
    bloom_b_view: wgpu::TextureView,
    aux_view: wgpu::TextureView,
    prev_seeded: bool,
    packed_view: Option<wgpu::TextureView>,
    multiview_view: Option<wgpu::TextureView>,
}

struct SceneGpu {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    packed: Option<wgpu::Texture>,
    packed_view: Option<wgpu::TextureView>,
    width: u32,
    height: u32,
    layers: Arc<[OverlayDesc]>,
    labels: Arc<[String]>,
    label_size: f32,
    label_percent: bool,
    label_top: bool,
}

pub struct Composer {
    color: wgpu::RenderPipeline,
    bars: wgpu::RenderPipeline,
    blit: wgpu::RenderPipeline,
    uyvy: wgpu::RenderPipeline,
    mix: wgpu::RenderPipeline,
    pack: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    nearest: wgpu::Sampler,
    mix_time: f32,
    color_bg_layout: wgpu::BindGroupLayout,
    blit_bg_layout: wgpu::BindGroupLayout,
    mix_bg_layout: wgpu::BindGroupLayout,
    pack_bg_layout: wgpu::BindGroupLayout,
    sources: HashMap<u64, SourceGpu>,
    units: HashMap<u64, UnitTargets>,
    scenes: HashMap<u64, SceneGpu>,
    generators: HashMap<u64, Generator>,
    generator_bake: HashMap<u64, Generator>,
    input_packed: HashMap<u64, wgpu::Texture>,
    scroll_phase: f32,
    scroll_phase_y: f32,
    tally_red: Option<(wgpu::Texture, wgpu::TextureView)>,
    tally_green: Option<(wgpu::Texture, wgpu::TextureView)>,
    preview_rgb: [u8; 3],
    program_rgb: [u8; 3],
    inactive_rgb: [u8; 3],
    label_cache: HashMap<LabelTexKey, (wgpu::Texture, wgpu::TextureView)>,
    label_size: f32,
    label_percent: bool,
    label_top: bool,
    pool: UniformPool,
    blit_groups: HashMap<u64, wgpu::BindGroup>,
    uyvy_groups: HashMap<u64, wgpu::BindGroup>,
    mix_groups: HashMap<u64, wgpu::BindGroup>,
    custom_mix: HashMap<u64, wgpu::RenderPipeline>,
    custom_mix_src: HashMap<u64, String>,
    custom_compute: HashMap<u64, wgpu::ComputePipeline>,
    sort_cs: wgpu::ComputePipeline,
    flow_cs: wgpu::ComputePipeline,
    bloom_cs: wgpu::ComputePipeline,
    bloom_blur_cs: wgpu::ComputePipeline,
    fx1_layout: wgpu::BindGroupLayout,
    fx2_layout: wgpu::BindGroupLayout,
    user_cs_layout: wgpu::BindGroupLayout,
    pack_groups: HashMap<u64, wgpu::BindGroup>,
    color_group: wgpu::BindGroup,
    gpu_epoch: u64,
    #[cfg(windows)]
    rebar: Option<crate::rebar::RebarUploader>,
    #[cfg(target_os = "macos")]
    uma: Option<crate::rebar::UmaUploader>,
}

impl Composer {
    pub fn new(device: &GpuDevice) -> Result<Self, String> {
        let color_bg_layout = device.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("color"),
            entries: &[uniform_dyn(0)],
        });
        let blit_bg_layout = device.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit"),
            entries: &[sampled(0), sampler_entry(1), uniform_dyn(2)],
        });
        let mix_bg_layout = device.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mix"),
            entries: &[
                sampled(0),
                sampled(1),
                sampler_entry(2),
                uniform_dyn(3),
                sampled(4),
                sampler_entry(5),
                sampled(6),
                sampled(7),
                sampled(8),
                sampled(9),
            ],
        });
        let fx1_layout = device.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx1"),
            entries: &[sampled_compute(0), storage_write(1), uniform_dyn(2)],
        });
        let fx2_layout = device.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx2"),
            entries: &[
                sampled_compute(0),
                sampled_compute(1),
                storage_write(2),
                uniform_dyn(3),
            ],
        });
        let user_cs_layout = device.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("user-cs"),
            entries: &[
                sampled_compute(0),
                sampled_compute(1),
                sampler_compute(2),
                uniform_dyn(3),
                sampled_compute(4),
                sampler_compute(5),
                sampled_compute(6),
                sampled_compute(7),
                storage_write(8),
            ],
        });
        let pack_bg_layout = device.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pack"),
            entries: &[sampled(0), sampler_entry(1)],
        });

        let sampler = device.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let nearest = device.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let color = pipeline(device, "color", include_str!("../shaders/color.wgsl"), &color_bg_layout, wgpu::TextureFormat::Rgba8Unorm, false)?;
        let bars = pipeline(device, "bars", include_str!("../shaders/bars.wgsl"), &color_bg_layout, wgpu::TextureFormat::Rgba8Unorm, false)?;
        let blit = pipeline(device, "blit", include_str!("../shaders/blit.wgsl"), &blit_bg_layout, wgpu::TextureFormat::Rgba8Unorm, true)?;
        let uyvy = pipeline(device, "uyvy", include_str!("../shaders/uyvy_to_rgba.wgsl"), &blit_bg_layout, wgpu::TextureFormat::Rgba8Unorm, true)?;
        let mix = pipeline(device, "mix", include_str!("../shaders/mix.wgsl"), &mix_bg_layout, wgpu::TextureFormat::Rgba8Unorm, false)?;
        let pack = pipeline(device, "pack", include_str!("../shaders/rgba_to_uyvy.wgsl"), &pack_bg_layout, wgpu::TextureFormat::Rgba8Unorm, false)?;
        let sort_cs = compute_pipeline(device, "sort", include_str!("../shaders/sort.wgsl"), &fx1_layout, "cs_main")?;
        let flow_cs = compute_pipeline(device, "flow", include_str!("../shaders/flow.wgsl"), &fx2_layout, "cs_main")?;
        let bloom_cs = compute_pipeline(device, "bloom", include_str!("../shaders/bloom.wgsl"), &fx2_layout, "cs_main")?;
        let bloom_blur_cs = compute_pipeline(device, "bloom-blur", include_str!("../shaders/bloom_blur.wgsl"), &fx1_layout, "cs_main")?;
        let pool = UniformPool::new(device);
        let color_group = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("color pool"),
            layout: &color_bg_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: pool.slot_binding(),
            }],
        });
        Ok(Self {
            color,
            bars,
            blit,
            uyvy,
            mix,
            pack,
            sampler,
            nearest,
            mix_time: 0.0,
            color_bg_layout,
            blit_bg_layout,
            mix_bg_layout,
            pack_bg_layout,
            sources: HashMap::new(),
            units: HashMap::new(),
            scenes: HashMap::new(),
            generators: HashMap::new(),
            generator_bake: HashMap::new(),
            input_packed: HashMap::new(),
            scroll_phase: 0.0,
            scroll_phase_y: 0.0,
            tally_red: None,
            tally_green: None,
            preview_rgb: [0, 255, 0],
            program_rgb: [255, 0, 0],
            inactive_rgb: [64, 64, 64],
            label_cache: HashMap::new(),
            label_size: 18.0,
            label_percent: false,
            label_top: false,
            pool,
            blit_groups: HashMap::new(),
            uyvy_groups: HashMap::new(),
            mix_groups: HashMap::new(),
            custom_mix: HashMap::new(),
            custom_mix_src: HashMap::new(),
            custom_compute: HashMap::new(),
            sort_cs,
            flow_cs,
            bloom_cs,
            bloom_blur_cs,
            fx1_layout,
            fx2_layout,
            user_cs_layout,
            pack_groups: HashMap::new(),
            color_group,
            gpu_epoch: 1,
            #[cfg(windows)]
            rebar: crate::rebar::RebarUploader::new(device),
            #[cfg(target_os = "macos")]
            uma: crate::rebar::UmaUploader::new(device),
        })
    }

    pub fn begin_frame(&mut self) {
        self.pool.reset();
        self.mix_time = self.mix_time + 1.0 / 60.0;
    }

    pub fn gpu_epoch(&self) -> u64 {
        self.gpu_epoch
    }

    pub fn unit(&self, unit_id: u64) -> Option<&UnitTargets> {
        self.units.get(&unit_id)
    }

    pub fn ensure_unit(&mut self, device: &GpuDevice, unit_id: u64, width: u32, height: u32) {
        if self.units.get(&unit_id).is_some_and(|unit| unit.width == width && unit.height == height) {
            return;
        }
        self.mix_groups.remove(&unit_id);
        self.pack_groups.remove(&unit_id);
        self.pack_groups.remove(&mixing_unit_preview(unit_id));
        self.pack_groups.remove(&mixing_unit_multiview(unit_id));
        self.blit_groups.remove(&mixing_unit_source(unit_id));
        self.blit_groups.remove(&mixing_unit_preview(unit_id));
        self.gpu_epoch = self.gpu_epoch.wrapping_add(1);
        self.units.insert(unit_id, UnitTargets::new(device, width, height));
    }

    fn ensure_packed(&mut self, device: &GpuDevice, unit_id: u64) {
        {
            let Some(unit) = self.units.get_mut(&unit_id) else {
                return;
            };
            if unit.packed.is_some() {
                return;
            }
            let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC;
            let packed = make_texture(device, unit.width / 2, unit.height, usage);
            unit.packed_view = Some(packed.create_view(&Default::default()));
            unit.packed = Some(packed);
        }
        self.pack_groups.remove(&unit_id);
    }

    fn ensure_multiview(&mut self, device: &GpuDevice, unit_id: u64) {
        {
            let Some(unit) = self.units.get_mut(&unit_id) else {
                return;
            };
            if unit.multiview.is_some() {
                return;
            }
            let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC;
            let texture = make_texture(device, unit.width, unit.height, usage);
            unit.multiview_view = Some(texture.create_view(&Default::default()));
            unit.multiview = Some(texture);
        }
        self.gpu_epoch = self.gpu_epoch.wrapping_add(1);
    }

    fn ensure_packed_bus(&mut self, device: &GpuDevice, unit_id: u64, preview: bool) {
        let Some(unit) = self.units.get_mut(&unit_id) else {
            return;
        };
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC;
        if preview {
            if unit.packed_prv.is_none() {
                unit.packed_prv = Some(make_texture(device, unit.width / 2, unit.height, usage));
            }
        } else if unit.packed_mv.is_none() {
            unit.packed_mv = Some(make_texture(device, unit.width / 2, unit.height, usage));
        }
    }

    pub fn upload_sources(
        &mut self,
        device: &GpuDevice,
        snaps: &[CpuFrameSnap],
        use_rebar: bool,
        direct_sample: bool,
    ) {
        #[cfg(not(windows))]
        let _ = use_rebar;
        let needed: HashSet<u64> = snaps.iter().map(|snap| snap.id).collect();
        #[cfg(windows)]
        let _ = needed;
        #[cfg(target_os = "macos")]
        if let Some(uma) = self.uma.as_mut() {
            if direct_sample {
                uma.retain(&needed);
            } else {
                uma.clear();
            }
        }
        for snap in snaps {
            let id = snap.id;
            if !snap.has_frame {
                continue;
            }
            let packed = matches!(snap.format, CpuFormat::Uyvy | CpuFormat::Uyva);
            let bgra = snap.format == CpuFormat::Bgra;
            if let Some(frame) = snap.gpu.as_ref() {
                let tex_w = frame.texture.size().width;
                let tex_h = frame.texture.size().height;
                let needs_new = self.sources.get(&id).is_none_or(|gpu| {
                    gpu.width != tex_w
                        || gpu.height != tex_h
                        || gpu.packed != frame.packed
                        || gpu.bgra != frame.bgra
                        || gpu.uploaded_pts != frame.pts
                });
                if needs_new {
                    self.blit_groups.remove(&id);
                    self.uyvy_groups.remove(&id);
                    self.gpu_epoch = self.gpu_epoch.wrapping_add(1);
                    self.sources.insert(
                        id,
                        SourceGpu {
                            texture: frame.texture.clone(),
                            view: frame.view.clone(),
                            width: tex_w,
                            height: tex_h,
                            packed: frame.packed,
                            bgra: frame.bgra,
                            uploaded_pts: frame.pts,
                            direct: false,
                            owned: false,
                        },
                    );
                }
                continue;
            }
            if snap.format == CpuFormat::GpuRgba {
                continue;
            }
            let tex_w = if packed { snap.width / 2 } else { snap.width };
            let needs_new = self.sources.get(&id).is_none_or(|gpu| {
                gpu.width != tex_w
                    || gpu.height != snap.height
                    || gpu.packed != packed
                    || gpu.bgra != bgra
                    || gpu.direct != direct_sample
            });
            if !needs_new
                && self
                    .sources
                    .get(&id)
                    .is_some_and(|gpu| gpu.uploaded_pts == snap.last_pts)
            {
                continue;
            }
            let format = if packed {
                wgpu::TextureFormat::Rgba8Unorm
            } else if bgra {
                wgpu::TextureFormat::Bgra8Unorm
            } else {
                wgpu::TextureFormat::Rgba8Unorm
            };
            let pixels = snap.pixels.as_slice();
            #[cfg(target_os = "macos")]
            if direct_sample {
                if let Some(uploader) = self.uma.as_mut() {
                    if let Ok((texture, view)) = uploader.upload_direct(
                        device,
                        id,
                        pixels,
                        if packed { snap.width * 2 } else { snap.width * 4 },
                        snap.height,
                        tex_w,
                        format,
                    ) {
                        self.blit_groups.remove(&id);
                        self.uyvy_groups.remove(&id);
                        self.gpu_epoch = self.gpu_epoch.wrapping_add(1);
                        self.sources.insert(
                            id,
                            SourceGpu {
                                texture,
                                view,
                                width: tex_w,
                                height: snap.height,
                                packed,
                                bgra,
                                uploaded_pts: snap.last_pts,
                                direct: true,
                                owned: true,
                            },
                        );
                        continue;
                    }
                }
            }
            let recreate = needs_new || self.sources.get(&id).is_some_and(|gpu| gpu.direct);
            if recreate {
                self.blit_groups.remove(&id);
                self.uyvy_groups.remove(&id);
                self.gpu_epoch = self.gpu_epoch.wrapping_add(1);
                let texture = make_texture_format(
                    device,
                    tex_w,
                    snap.height,
                    wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::COPY_SRC,
                    format,
                );
                let view = texture.create_view(&Default::default());
                self.sources.insert(
                    id,
                    SourceGpu {
                        texture,
                        view,
                        width: tex_w,
                        height: snap.height,
                        packed,
                        bgra,
                        uploaded_pts: i64::MIN,
                        direct: false,
                        owned: true,
                    },
                );
            }
            let texture = self.sources.get(&id).expect("source inserted").texture.clone();
            write_aligned_texture(
                device,
                &texture,
                pixels,
                if packed { snap.width * 2 } else { snap.width * 4 },
                snap.height,
                tex_w,
                format,
                #[cfg(windows)]
                self.rebar.as_mut().filter(|_| use_rebar),
            );
            if let Some(gpu) = self.sources.get_mut(&id) {
                gpu.uploaded_pts = snap.last_pts;
            }
        }
        #[cfg(windows)]
        if let Some(rebar) = self.rebar.as_mut() {
            rebar.flush(device);
        }
    }

    pub fn set_bus_colors(&mut self, preview: [u8; 3], program: [u8; 3], inactive: [u8; 3]) {
        if self.preview_rgb == preview && self.program_rgb == program && self.inactive_rgb == inactive
        {
            return;
        }
        self.preview_rgb = preview;
        self.program_rgb = program;
        self.inactive_rgb = inactive;
        self.tally_red = None;
        self.tally_green = None;
        self.clear_labels();
    }

    pub fn set_mv_label(&mut self, size: f32, percent: bool, top: bool) {
        let size = crate::labels::clamp_size(size);
        let same = (self.label_size - size).abs() < f32::EPSILON && self.label_percent == percent;
        if same && self.label_top == top {
            return;
        }
        self.label_size = size;
        self.label_percent = percent;
        self.label_top = top;
        if !same {
            self.clear_labels();
        }
    }

    fn clear_labels(&mut self) {
        for key in self.label_cache.keys() {
            self.blit_groups.remove(&label_cache_key(key));
        }
        self.label_cache.clear();
    }

    pub fn sync_scenes(
        &mut self,
        device: &GpuDevice,
        specs: &[(u64, u32, u32, Arc<[OverlayDesc]>, crate::MvLabelStyle)],
        labels: &HashMap<u64, Arc<[String]>>,
    ) {
        let keep: std::collections::HashSet<u64> = specs.iter().map(|spec| spec.0).collect();
        self.scenes.retain(|id, _| keep.contains(id));
        for (id, width, height, layers, style) in specs {
            let scene_labels = labels
                .get(id)
                .cloned()
                .unwrap_or_else(|| Arc::from([]));
            if let Some(existing) = self.scenes.get_mut(id) {
                existing.label_size = style.size;
                existing.label_percent = style.percent;
                existing.label_top = style.top;
                if existing.width == *width && existing.height == *height {
                    if !Arc::ptr_eq(&existing.layers, layers) {
                        existing.layers = Arc::clone(layers);
                    }
                    if !Arc::ptr_eq(&existing.labels, &scene_labels) {
                        existing.labels = scene_labels;
                    }
                    continue;
                }
            }
            self.define_scene(
                device,
                *id,
                *width,
                *height,
                Arc::clone(layers),
                scene_labels,
                *style,
            );
        }
    }

    pub fn define_scene(
        &mut self,
        device: &GpuDevice,
        scene_id: u64,
        width: u32,
        height: u32,
        layers: Arc<[OverlayDesc]>,
        labels: Arc<[String]>,
        style: crate::MvLabelStyle,
    ) {
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC;
        let texture = make_texture(device, width, height, usage);
        let view = texture.create_view(&Default::default());
        self.blit_groups.remove(&scene_id);
        self.pack_groups.remove(&scene_id);
        self.gpu_epoch = self.gpu_epoch.wrapping_add(1);
        self.scenes.insert(
            scene_id,
            SceneGpu {
                texture,
                view,
                packed: None,
                packed_view: None,
                width,
                height,
                layers,
                labels,
                label_size: style.size,
                label_percent: style.percent,
                label_top: style.top,
            },
        );
    }

    pub fn destroy_scene(&mut self, scene_id: u64) {
        self.scenes.remove(&scene_id);
        self.blit_groups.remove(&scene_id);
        self.pack_groups.remove(&scene_id);
    }

    pub fn render_scenes(
        &mut self,
        device: &GpuDevice,
        used: &HashSet<u64>,
        encoder: &mut wgpu::CommandEncoder,
        tallies: &[(u64, u64)],
    ) -> Result<(), String> {
        let mut order = Vec::new();
        let mut visited = HashSet::new();
        for id in used {
            self.visit_scene(*id, used, &mut visited, &mut order);
        }
        self.bake_generators(device, encoder);
        for id in order {
            self.draw_scene(device, encoder, id, tallies)?;
        }
        Ok(())
    }

    fn visit_scene(
        &self,
        id: u64,
        used: &HashSet<u64>,
        visited: &mut HashSet<u64>,
        order: &mut Vec<u64>,
    ) {
        if !used.contains(&id) || !self.scenes.contains_key(&id) || !visited.insert(id) {
            return;
        }
        if let Some(scene) = self.scenes.get(&id) {
            for layer in scene.layers.iter() {
                if is_scene(layer.source_id) {
                    self.visit_scene(layer.source_id, used, visited, order);
                }
            }
        }
        order.push(id);
    }

    fn draw_scene(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        scene_id: u64,
        tallies: &[(u64, u64)],
    ) -> Result<(), String> {
        let (width, height, mut layers, labels, view, label_size, label_percent, label_top) = {
            let scene = self.scenes.get(&scene_id).ok_or("scene missing")?;
            (
                scene.width,
                scene.height,
                scene.layers.iter().copied().collect::<Vec<_>>(),
                scene.labels.clone(),
                scene.view.clone(),
                scene.label_size,
                scene.label_percent,
                scene.label_top,
            )
        };
        layers.sort_by_key(|layer| layer.z);
        {
            let mut pass = begin_clear(encoder, &view);
            if layers.is_empty() {
                self.blit_builtin_pass(device, &mut pass, SRC_BLACK, [0.0, 0.0, 1.0, 1.0], 1.0);
            } else {
                for layer in &layers {
                    if layer.hidden != 0 {
                        continue;
                    }
                    let (dest, uv) = crop_blit(
                        [layer.rect.x, layer.rect.y, layer.rect.width, layer.rect.height],
                        layer.crop,
                    );
                    self.draw_source_pass(
                        device,
                        &mut pass,
                        layer.source_id,
                        dest,
                        layer.opacity,
                        uv,
                    )?;
                }
            }
            if is_multiview(scene_id) {
                self.draw_mv_label_pass(
                    device,
                    &mut pass,
                    &layers,
                    &labels,
                    width,
                    height,
                    label_size,
                    label_percent,
                    label_top,
                );
                self.draw_mv_tally_pass(device, &mut pass, &layers, tallies, width, height);
            }
        }
        Ok(())
    }

    fn draw_mv_label_pass(
        &mut self,
        device: &GpuDevice,
        pass: &mut wgpu::RenderPass,
        layers: &[OverlayDesc],
        labels: &[String],
        canvas_w: u32,
        canvas_h: u32,
        label_size: f32,
        label_percent: bool,
        label_top: bool,
    ) {
        let video: Vec<_> = layers.iter().filter(|layer| layer.z < 100).copied().collect();
        for (index, layer) in video.iter().enumerate() {
            let rgb = mv_label_rgb(layer.source_id, self.preview_rgb, self.program_rgb, self.inactive_rgb);
            let text = labels.get(index).map(String::as_str).unwrap_or("");
            let tile_h = layer.rect.height * canvas_h.max(1) as f32;
            let dest_w = (layer.rect.width * canvas_w.max(1) as f32).round().max(1.0) as u32;
            let font_px = crate::labels::font_px(label_size, label_percent, tile_h);
            let dest_h = crate::labels::band_height(font_px);
            let Some(view) = self.ensure_label_texture(device, text, rgb, font_px, dest_w, dest_h) else {
                continue;
            };
            let key = LabelTexKey::new(text, rgb, font_px, dest_w);
            let nh = dest_h as f32 / canvas_h.max(1) as f32;
            let y = if label_top {
                layer.rect.y
            } else {
                layer.rect.y + layer.rect.height - nh
            };
            let dst = [layer.rect.x, y, layer.rect.width, nh];
            self.blit_pass(device, pass, label_cache_key(&key), &view, dst, 1.0, false);
        }
    }

    fn ensure_label_texture(
        &mut self,
        device: &GpuDevice,
        text: &str,
        rgb: [u8; 3],
        font_px: f32,
        dest_w: u32,
        dest_h: u32,
    ) -> Option<wgpu::TextureView> {
        let key = LabelTexKey::new(text, rgb, font_px, dest_w);
        if let Some((_, view)) = self.label_cache.get(&key) {
            return Some(view.clone());
        }
        let raster = crate::labels::raster(text, rgb, font_px, dest_w, dest_h);
        let texture = make_texture(
            device,
            raster.width,
            raster.height,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        write_aligned_texture(
            device,
            &texture,
            &raster.pixels,
            raster.width * 4,
            raster.height,
            raster.width,
            wgpu::TextureFormat::Rgba8Unorm,
            #[cfg(windows)]
            self.rebar.as_mut(),
        );
        let view = texture.create_view(&Default::default());
        self.blit_groups.remove(&label_cache_key(&key));
        self.label_cache.insert(key, (texture, view.clone()));
        Some(view)
    }

    fn draw_mv_tally_pass(
        &mut self,
        device: &GpuDevice,
        pass: &mut wgpu::RenderPass,
        layers: &[OverlayDesc],
        tallies: &[(u64, u64)],
        width: u32,
        height: u32,
    ) {
        self.ensure_tally_swatches(device);
        let tx = 3.0 / width.max(1) as f32;
        let ty = 3.0 / height.max(1) as f32;
        let video: Vec<_> = layers.iter().filter(|layer| layer.z < 100).copied().collect();
        for layer in video {
            let Some(use_red) = mv_tally_program(layer.source_id, tallies) else {
                continue;
            };
            let (key, swatch) = if use_red {
                (KEY_TALLY_RED, self.tally_red.as_ref().map(|item| item.1.clone()))
            } else {
                (KEY_TALLY_GREEN, self.tally_green.as_ref().map(|item| item.1.clone()))
            };
            let Some(swatch) = swatch else {
                continue;
            };
            let x = layer.rect.x;
            let y = layer.rect.y;
            let w = layer.rect.width;
            let h = layer.rect.height;
            self.blit_pass(device, pass, key, &swatch, [x, y, w, ty], 1.0, false);
            self.blit_pass(device, pass, key, &swatch, [x, y + h - ty, w, ty], 1.0, false);
            self.blit_pass(device, pass, key, &swatch, [x, y, tx, h], 1.0, false);
            self.blit_pass(device, pass, key, &swatch, [x + w - tx, y, tx, h], 1.0, false);
        }
    }

    fn ensure_tally_swatches(&mut self, device: &GpuDevice) {
        if self.tally_red.is_some() && self.tally_green.is_some() {
            return;
        }
        if self.tally_red.is_none() {
            self.tally_red = Some(solid_swatch(
                device,
                [self.program_rgb[0], self.program_rgb[1], self.program_rgb[2], 255],
            ));
        }
        if self.tally_green.is_none() {
            self.tally_green = Some(solid_swatch(
                device,
                [self.preview_rgb[0], self.preview_rgb[1], self.preview_rgb[2], 255],
            ));
        }
    }

    pub fn set_custom_mix(
        &mut self,
        device: &GpuDevice,
        unit_id: u64,
        user_wgsl: &str,
    ) -> Result<(), String> {
        let trimmed = user_wgsl.trim();
        if trimmed.is_empty() {
            self.custom_mix.remove(&unit_id);
            self.custom_mix_src.remove(&unit_id);
            self.custom_compute.remove(&unit_id);
            return Ok(());
        }
        if self.custom_mix_src.get(&unit_id).map(String::as_str) == Some(trimmed) {
            return Ok(());
        }
        let source = custom_mix_source(user_wgsl);
        let pipeline = pipeline(
            device,
            "mix-custom",
            &source,
            &self.mix_bg_layout,
            wgpu::TextureFormat::Rgba8Unorm,
            false,
        )?;
        if trimmed.contains("fn user_compute") {
            let cs = custom_compute_source(user_wgsl);
            self.custom_compute.insert(
                unit_id,
                compute_pipeline(device, "mix-custom-cs", &cs, &self.user_cs_layout, "cs_user")?,
            );
        } else {
            self.custom_compute.remove(&unit_id);
        }
        self.custom_mix_src.insert(unit_id, trimmed.to_string());
        self.custom_mix.insert(unit_id, pipeline);
        Ok(())
    }

    pub fn validate_custom_wgsl(user_wgsl: &str) -> Result<(), String> {
        let trimmed = user_wgsl.trim();
        if trimmed.is_empty() {
            return Err("WGSL is empty".into());
        }
        if !trimmed.contains("fn user_transition") {
            return Err("Define fn user_transition(uv: vec2<f32>, t: f32) -> vec4<f32>".into());
        }
        let source = custom_mix_source(user_wgsl);
        naga::front::wgsl::parse_str(&source).map_err(|error| error.to_string())?;
        if trimmed.contains("fn user_compute") {
            naga::front::wgsl::parse_str(&custom_compute_source(user_wgsl))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub fn render_unit(
        &mut self,
        device: &GpuDevice,
        unit_id: u64,
        state: &UnitState,
        mix_preview: u64,
        encoder: &mut wgpu::CommandEncoder,
        compose_mv: bool,
        pack_pgm: bool,
    ) -> Result<(), String> {
        let (width, height) = {
            let unit = self.units.get(&unit_id).ok_or("unit targets missing")?;
            (unit.width, unit.height)
        };
        self.draw_bus(device, encoder, unit_id, state.program_source, true, width, height)?;
        let mix_source = if mix_preview == 0 {
            state.preview_source
        } else {
            mix_preview
        };
        if mix_source == state.program_source {
            let unit = self.units.get(&unit_id).ok_or("unit targets missing")?;
            encoder.copy_texture_to_texture(
                unit.program.as_image_copy(),
                unit.preview.as_image_copy(),
                wgpu::Extent3d {
                    width: unit.width.max(1),
                    height: unit.height.max(1),
                    depth_or_array_layers: 1,
                },
            );
        } else {
            self.draw_bus(device, encoder, unit_id, mix_source, false, width, height)?;
        }
        self.seed_mix_history(encoder, unit_id);
        if state.mix == 0.0 {
            let unit = self.units.get(&unit_id).ok_or("unit targets missing")?;
            encoder.copy_texture_to_texture(
                unit.program.as_image_copy(),
                unit.mixed.as_image_copy(),
                wgpu::Extent3d {
                    width: unit.width.max(1),
                    height: unit.height.max(1),
                    depth_or_array_layers: 1,
                },
            );
        } else {
            self.draw_mix(device, encoder, unit_id, state)?;
        }
        if state.preview_source != mix_source {
            self.draw_bus(device, encoder, unit_id, state.preview_source, false, width, height)?;
        }
        self.draw_overlays_on_program(device, encoder, unit_id, state)?;
        if compose_mv {
            self.ensure_multiview(device, unit_id);
            self.draw_multiview(device, encoder, unit_id, state)?;
        }
        if pack_pgm {
            self.ensure_packed(device, unit_id);
            self.draw_pack(device, encoder, unit_id)?;
        }
        self.store_mix_history(encoder, unit_id);
        Ok(())
    }

    fn draw_bus(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        unit_id: u64,
        source_id: u64,
        program: bool,
        _width: u32,
        _height: u32,
    ) -> Result<(), String> {
        let target_view = if program {
            self.units[&unit_id].program_view.clone()
        } else {
            self.units[&unit_id].preview_view.clone()
        };
        let mut pass = begin_clear(encoder, &target_view);
        self.draw_source_pass(device, &mut pass, source_id, [0.0, 0.0, 1.0, 1.0], 1.0, FULL_UV)?;
        drop(pass);
        Ok(())
    }

    fn draw_overlays_on_program(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        unit_id: u64,
        state: &UnitState,
    ) -> Result<(), String> {
        if state.overlay_count == 0 {
            return Ok(());
        }
        let dest = {
            let unit = self.units.get(&unit_id).ok_or("unit missing")?;
            unit.mixed_view.clone()
        };
        let mut overlays: Vec<OverlayDesc> = state.overlays[..state.overlay_count as usize].to_vec();
        overlays.sort_by_key(|overlay| overlay.z);
        {
            let mut pass = begin(encoder, &dest);
            for overlay in &overlays {
                if overlay.hidden != 0 {
                    continue;
                }
                let (dest, uv) = crop_blit(
                    [
                        overlay.rect.x,
                        overlay.rect.y,
                        overlay.rect.width,
                        overlay.rect.height,
                    ],
                    overlay.crop,
                );
                self.draw_source_pass(
                    device,
                    &mut pass,
                    overlay.source_id,
                    dest,
                    overlay.opacity,
                    uv,
                )?;
            }
        }
        Ok(())
    }

    fn draw_multiview(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        unit_id: u64,
        state: &UnitState,
    ) -> Result<(), String> {
        let Some(dest) = self.units.get(&unit_id).and_then(|unit| unit.multiview_view.clone()) else {
            return Ok(());
        };
        let preview = self.units[&unit_id].preview_view.clone();
        let mixed = self.units[&unit_id].mixed_view.clone();
        {
            let mut pass = begin_clear(encoder, &dest);
            self.blit_pass(
                device,
                &mut pass,
                mixing_unit_preview(unit_id),
                &preview,
                [0.0, 0.0, 0.5, 0.5],
                1.0,
                false,
            );
            self.blit_pass(
                device,
                &mut pass,
                mixing_unit_source(unit_id),
                &mixed,
                [0.5, 0.0, 0.5, 0.5],
                1.0,
                false,
            );
            let count = (state.mv_slot_count as usize).min(MV_SLOT_MAX).max(1);
            let (cols, rows) = tile_grid(count as u32);
            for index in 0..count {
                let col = (index as u32 % cols) as f32;
                let row = (index as u32 / cols) as f32;
                let x = col / cols as f32;
                let y = 0.5 + row / rows as f32 * 0.5;
                let w = 1.0 / cols as f32;
                let h = 0.5 / rows as f32;
                let slot = state.mv_slots[index];
                if slot == 0 {
                    self.blit_builtin_pass(device, &mut pass, SRC_BLACK, [x, y, w, h], 1.0);
                } else {
                    self.draw_source_pass(device, &mut pass, slot, [x, y, w, h], 1.0, FULL_UV)?;
                }
            }
        }
        Ok(())
    }

    fn seed_mix_history(&mut self, encoder: &mut wgpu::CommandEncoder, unit_id: u64) {
        let Some(unit) = self.units.get_mut(&unit_id) else {
            return;
        };
        if unit.prev_seeded {
            return;
        }
        encoder.copy_texture_to_texture(
            unit.program.as_image_copy(),
            unit.prev.as_image_copy(),
            wgpu::Extent3d {
                width: unit.width.max(1),
                height: unit.height.max(1),
                depth_or_array_layers: 1,
            },
        );
        unit.prev_seeded = true;
    }

    fn store_mix_history(&mut self, encoder: &mut wgpu::CommandEncoder, unit_id: u64) {
        let Some(unit) = self.units.get(&unit_id) else {
            return;
        };
        encoder.copy_texture_to_texture(
            unit.mixed.as_image_copy(),
            unit.prev.as_image_copy(),
            wgpu::Extent3d {
                width: unit.width.max(1),
                height: unit.height.max(1),
                depth_or_array_layers: 1,
            },
        );
    }

    fn mix_params(state: &UnitState, mix_time: f32, width: f32, height: f32, direction: Option<u32>) -> MixParams {
        MixParams {
            mix: state.mix,
            kind: if state.transition_kind == TRANSITION_STINGER {
                crate::abi::TRANSITION_FADE
            } else {
                state.transition_kind
            },
            direction: direction.unwrap_or(state.transition_direction),
            softness: state.softness,
            dip: [
                state.dip_r,
                state.dip_g,
                state.dip_b,
                if state.dip_a <= 0.0 { 1.0 } else { state.dip_a },
            ],
            param: state.param,
            time: mix_time,
            resolution: [width, height],
        }
    }

    fn prepare_mix_fx(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        unit_id: u64,
        state: &UnitState,
        width: f32,
        height: f32,
    ) -> Result<(), String> {
        let kind = state.transition_kind;
        let custom = kind == TRANSITION_CUSTOM && self.custom_mix.contains_key(&unit_id);
        let need_sort = false;
        let need_flow = matches!(kind, TRANSITION_DATAMOSH | TRANSITION_OPTICAL_FLOW) || custom;
        let need_bloom = matches!(kind, TRANSITION_BLOOM | TRANSITION_FILM_BURN) || custom;
        let need_user = custom && self.custom_compute.contains_key(&unit_id);
        if !need_sort && !need_flow && !need_bloom && !need_user {
            return Ok(());
        }
        let half_w = (width * 0.5).max(1.0);
        let half_h = (height * 0.5).max(1.0);
        if need_sort {
            let offset = self.pool.push(
                &device.queue,
                &Self::mix_params(state, self.mix_time, width, height, None),
            );
            let lines = if state.transition_direction <= 1 {
                height as u32
            } else {
                width as u32
            };
            let gx = lines.div_ceil(64);
            self.dispatch_fx1(device, encoder, true, unit_id, offset, gx, 1);
            self.dispatch_fx1(device, encoder, false, unit_id, offset, gx, 1);
        }
        if need_flow {
            let offset = self.pool.push(
                &device.queue,
                &Self::mix_params(state, self.mix_time, half_w, half_h, None),
            );
            self.dispatch_fx2(device, encoder, Fx2Kind::Flow, unit_id, offset, half_w, half_h);
        }
        if need_bloom {
            let extract = self.pool.push(
                &device.queue,
                &Self::mix_params(state, self.mix_time, half_w, half_h, None),
            );
            self.dispatch_fx2(device, encoder, Fx2Kind::Bloom, unit_id, extract, half_w, half_h);
            let blur_h = self.pool.push(
                &device.queue,
                &Self::mix_params(state, self.mix_time, half_w, half_h, Some(0)),
            );
            self.dispatch_bloom_blur(device, encoder, unit_id, true, blur_h, half_w, half_h);
            let blur_v = self.pool.push(
                &device.queue,
                &Self::mix_params(state, self.mix_time, half_w, half_h, Some(1)),
            );
            self.dispatch_bloom_blur(device, encoder, unit_id, false, blur_v, half_w, half_h);
        }
        if need_user {
            let offset = self.pool.push(
                &device.queue,
                &Self::mix_params(state, self.mix_time, width, height, None),
            );
            self.dispatch_user_compute(device, encoder, unit_id, offset, width, height);
        }
        Ok(())
    }

    fn dispatch_fx1(
        &self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        program: bool,
        unit_id: u64,
        offset: u32,
        gx: u32,
        gy: u32,
    ) {
        let Some(unit) = self.units.get(&unit_id) else {
            return;
        };
        let (src, dst) = if program {
            (&unit.program_view, &unit.aux_view)
        } else {
            (&unit.preview_view, &unit.sort_b_view)
        };
        let group = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx1"),
            layout: &self.fx1_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(dst),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.pool.slot_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fx1"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.sort_cs);
        pass.set_bind_group(0, &group, &[offset]);
        pass.dispatch_workgroups(gx.max(1), gy.max(1), 1);
    }

    fn dispatch_fx2(
        &self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        kind: Fx2Kind,
        unit_id: u64,
        offset: u32,
        width: f32,
        height: f32,
    ) {
        let Some(unit) = self.units.get(&unit_id) else {
            return;
        };
        let dst = match kind {
            Fx2Kind::Flow => &unit.flow_view,
            Fx2Kind::Bloom => &unit.bloom_a_view,
        };
        let group = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx2"),
            layout: &self.fx2_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&unit.program_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&unit.preview_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(dst),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.pool.slot_binding(),
                },
            ],
        });
        let gx = (width as u32).div_ceil(8).max(1);
        let gy = (height as u32).div_ceil(8).max(1);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fx2"),
            timestamp_writes: None,
        });
        pass.set_pipeline(match kind {
            Fx2Kind::Flow => &self.flow_cs,
            Fx2Kind::Bloom => &self.bloom_cs,
        });
        pass.set_bind_group(0, &group, &[offset]);
        pass.dispatch_workgroups(gx, gy, 1);
    }

    fn dispatch_bloom_blur(
        &self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        unit_id: u64,
        horizontal: bool,
        offset: u32,
        width: f32,
        height: f32,
    ) {
        let Some(unit) = self.units.get(&unit_id) else {
            return;
        };
        let (src, dst) = if horizontal {
            (&unit.bloom_a_view, &unit.bloom_b_view)
        } else {
            (&unit.bloom_b_view, &unit.bloom_a_view)
        };
        let group = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-blur"),
            layout: &self.fx1_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(dst),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.pool.slot_binding(),
                },
            ],
        });
        let gx = (width as u32).div_ceil(8).max(1);
        let gy = (height as u32).div_ceil(8).max(1);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("bloom-blur"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.bloom_blur_cs);
        pass.set_bind_group(0, &group, &[offset]);
        pass.dispatch_workgroups(gx, gy, 1);
    }

    fn dispatch_user_compute(
        &self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        unit_id: u64,
        offset: u32,
        width: f32,
        height: f32,
    ) {
        let Some(pipeline) = self.custom_compute.get(&unit_id) else {
            return;
        };
        let Some(unit) = self.units.get(&unit_id) else {
            return;
        };
        let group = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("user-cs"),
            layout: &self.user_cs_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&unit.program_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&unit.preview_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.pool.slot_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&unit.prev_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.nearest),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&unit.flow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&unit.bloom_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&unit.aux_view),
                },
            ],
        });
        let gx = (width as u32).div_ceil(8).max(1);
        let gy = (height as u32).div_ceil(8).max(1);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("user-cs"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[offset]);
        pass.dispatch_workgroups(gx, gy, 1);
    }

    fn draw_mix(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        unit_id: u64,
        state: &UnitState,
    ) -> Result<(), String> {
        self.ensure_mix_group(device, unit_id);
        let (width, height) = self
            .units
            .get(&unit_id)
            .map(|unit| (unit.width as f32, unit.height as f32))
            .unwrap_or((1920.0, 1080.0));
        self.prepare_mix_fx(device, encoder, unit_id, state, width, height)?;
        let offset = self.pool.push(
            &device.queue,
            &MixParams {
                mix: state.mix,
                kind: if state.transition_kind == TRANSITION_STINGER {
                    crate::abi::TRANSITION_FADE
                } else {
                    state.transition_kind
                },
                direction: state.transition_direction,
                softness: state.softness,
                dip: [
                    state.dip_r,
                    state.dip_g,
                    state.dip_b,
                    if state.dip_a <= 0.0 { 1.0 } else { state.dip_a },
                ],
                param: state.param,
                time: self.mix_time,
                resolution: [width, height],
            },
        );
        let dest = self.units.get(&unit_id).ok_or("unit missing")?.mixed_view.clone();
        let group = self
            .mix_groups
            .get(&unit_id)
            .ok_or("mix bind group missing")?;
        let mut pass = begin(encoder, &dest);
        let use_custom = state.transition_kind == TRANSITION_CUSTOM && self.custom_mix.contains_key(&unit_id);
        if use_custom {
            pass.set_pipeline(self.custom_mix.get(&unit_id).unwrap());
        } else {
            pass.set_pipeline(&self.mix);
        }
        pass.set_bind_group(0, group, &[offset]);
        pass.draw(0..3, 0..1);
        Ok(())
    }

    fn draw_pack(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        unit_id: u64,
    ) -> Result<(), String> {
        self.ensure_pack_group(device, unit_id);
        let dest = self
            .units
            .get(&unit_id)
            .and_then(|unit| unit.packed_view.clone())
            .ok_or("packed missing")?;
        let group = self
            .pack_groups
            .get(&unit_id)
            .ok_or("pack bind group missing")?;
        let mut pass = begin_clear(encoder, &dest);
        pass.set_pipeline(&self.pack);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }

    fn draw_source_pass(
        &mut self,
        device: &GpuDevice,
        pass: &mut wgpu::RenderPass,
        source_id: u64,
        dst: [f32; 4],
        opacity: f32,
        crop: [f32; 4],
    ) -> Result<(), String> {
        if is_scene(source_id) {
            if let Some(view) = self.scenes.get(&source_id).map(|scene| scene.view.clone()) {
                self.blit_uv(device, pass, source_id, &view, dst, crop, opacity, false);
                return Ok(());
            }
        }
        if let Some(other) = mixing_unit_from_source(source_id) {
            let view = self.units.get(&other).map(|unit| match mixing_unit_bus(source_id) {
                OUTPUT_PREVIEW => unit.preview_view.clone(),
                OUTPUT_MULTIVIEW => unit
                    .multiview_view
                    .clone()
                    .unwrap_or_else(|| unit.mixed_view.clone()),
                _ => unit.mixed_view.clone(),
            });
            if let Some(view) = view {
                self.blit_uv(device, pass, source_id, &view, dst, crop, opacity, false);
                return Ok(());
            }
        }
        if self.generators.contains_key(&source_id) {
            self.blit_builtin_uv(device, pass, source_id, dst, crop, opacity);
            return Ok(());
        }
        match source_id {
            SRC_COLOR | SRC_BLACK | SRC_BLUE | SRC_BARS => {
                self.blit_builtin_uv(device, pass, source_id, dst, crop, opacity);
                Ok(())
            }
            id => {
                let copied = self
                    .sources
                    .get(&id)
                    .map(|gpu| (gpu.view.clone(), gpu.packed));
                if let Some((view, packed)) = copied {
                    self.blit_uv(device, pass, id, &view, dst, crop, opacity, packed);
                    Ok(())
                } else {
                    self.blit_builtin_uv(device, pass, SRC_BLACK, dst, crop, opacity);
                    Ok(())
                }
            }
        }
    }

    fn blit_builtin_pass(
        &mut self,
        device: &GpuDevice,
        pass: &mut wgpu::RenderPass,
        source_id: u64,
        dst: [f32; 4],
        opacity: f32,
    ) {
        let key = if self.sources.contains_key(&source_id) {
            source_id
        } else if self.generators.contains_key(&source_id) {
            if self.generators[&source_id].kind == GEN_BARS {
                SRC_BARS
            } else {
                SRC_COLOR
            }
        } else {
            SRC_BLACK
        };
        let Some(view) = self.sources.get(&key).map(|gpu| gpu.view.clone()) else {
            return;
        };
        self.blit_uv(device, pass, key, &view, dst, FULL_UV, opacity, false);
    }

    fn blit_builtin_uv(
        &mut self,
        device: &GpuDevice,
        pass: &mut wgpu::RenderPass,
        source_id: u64,
        dst: [f32; 4],
        crop: [f32; 4],
        opacity: f32,
    ) {
        let key = if self.sources.contains_key(&source_id) {
            source_id
        } else if self.generators.contains_key(&source_id) {
            if self.generators[&source_id].kind == GEN_BARS {
                SRC_BARS
            } else {
                SRC_COLOR
            }
        } else {
            SRC_BLACK
        };
        let Some(view) = self.sources.get(&key).map(|gpu| gpu.view.clone()) else {
            return;
        };
        self.blit_uv(device, pass, key, &view, dst, crop, opacity, false);
    }

    fn blit_pass(
        &mut self,
        device: &GpuDevice,
        pass: &mut wgpu::RenderPass,
        key: u64,
        src: &wgpu::TextureView,
        dst: [f32; 4],
        opacity: f32,
        uyvy: bool,
    ) {
        self.blit_uv(device, pass, key, src, dst, FULL_UV, opacity, uyvy);
    }

    fn blit_uv(
        &mut self,
        device: &GpuDevice,
        pass: &mut wgpu::RenderPass,
        key: u64,
        src: &wgpu::TextureView,
        dst: [f32; 4],
        uv: [f32; 4],
        opacity: f32,
        uyvy: bool,
    ) {
        let offset = self.pool.push(
            &device.queue,
            &BlitParams {
                dst,
                src: uv,
                opacity,
                pad: [0.0; 3],
            },
        );
        self.ensure_tex_group(device, key, src, uyvy);
        pass.set_pipeline(if uyvy { &self.uyvy } else { &self.blit });
        let group = if uyvy {
            &self.uyvy_groups[&key]
        } else {
            &self.blit_groups[&key]
        };
        pass.set_bind_group(0, group, &[offset]);
        pass.draw(0..6, 0..1);
    }

    fn ensure_tex_group(
        &mut self,
        device: &GpuDevice,
        key: u64,
        src: &wgpu::TextureView,
        uyvy: bool,
    ) {
        let groups = if uyvy {
            &mut self.uyvy_groups
        } else {
            &mut self.blit_groups
        };
        if groups.contains_key(&key) {
            return;
        }
        let group = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit cached"),
            layout: &self.blit_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.pool.slot_binding(),
                },
            ],
        });
        groups.insert(key, group);
    }

    fn ensure_mix_group(&mut self, device: &GpuDevice, unit_id: u64) {
        if self.mix_groups.contains_key(&unit_id) {
            return;
        }
        let Some(unit) = self.units.get(&unit_id) else {
            return;
        };
        let group = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mix cached"),
            layout: &self.mix_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&unit.program_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&unit.preview_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.pool.slot_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&unit.prev_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.nearest),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&unit.flow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&unit.bloom_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&unit.aux_view),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&unit.sort_b_view),
                },
            ],
        });
        self.mix_groups.insert(unit_id, group);
    }

    fn ensure_pack_group(&mut self, device: &GpuDevice, unit_id: u64) {
        if self.pack_groups.contains_key(&unit_id) {
            return;
        }
        let Some(unit) = self.units.get(&unit_id) else {
            return;
        };
        let group = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pack cached"),
            layout: &self.pack_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&unit.mixed_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.pack_groups.insert(unit_id, group);
    }

    fn fill_target(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        dest: &wgpu::TextureView,
        color: [f32; 4],
        bars: bool,
        scroll: bool,
    ) {
        let offset = self.pool.push(
            &device.queue,
            &ColorParams {
                color,
                scroll: if scroll { self.scroll_phase } else { 0.0 },
                flags: if scroll { 1.0 } else { 0.0 },
                scroll_y: if scroll { self.scroll_phase_y } else { 0.0 },
                pad: 0.0,
            },
        );
        let mut pass = begin(encoder, dest);
        pass.set_pipeline(if bars { &self.bars } else { &self.color });
        pass.set_bind_group(0, &self.color_group, &[offset]);
        pass.draw(0..3, 0..1);
    }


    pub fn ensure_builtins(&mut self, device: &GpuDevice) {
        for id in [SRC_COLOR, SRC_BARS, SRC_BLACK, SRC_BLUE] {
            if self.sources.contains_key(&id) {
                continue;
            }
            let (width, height) = if id == SRC_BARS { (1920, 1080) } else { (128, 72) };
            let texture = make_texture(
                device,
                width,
                height,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            let view = texture.create_view(&Default::default());
            let mut encoder = device.device.create_command_encoder(&Default::default());
            match id {
                SRC_BARS => self.fill_target(device, &mut encoder, &view, [0.0; 4], true, false),
                _ => self.fill_target(device, &mut encoder, &view, color_for(id), false, false),
            }
            device.submit(Some(encoder.finish()));
            self.sources.insert(
                id,
                SourceGpu {
                    texture,
                    view,
                    width,
                    height,
                    packed: false,
                    bgra: false,
                    uploaded_pts: i64::MIN,
                    direct: false,
                    owned: true,
                },
            );
        }
    }

    pub fn bake_generators(&mut self, device: &GpuDevice, encoder: &mut wgpu::CommandEncoder) {
        let gens: Vec<(u64, Generator)> = self.generators.iter().map(|(id, spec)| (*id, *spec)).collect();
        for (id, spec) in gens {
            let (width, height) = if spec.kind == GEN_BARS { (1920, 1080) } else { (128, 72) };
            let size_ok = self
                .sources
                .get(&id)
                .is_some_and(|source| source.width == width && source.height == height);
            if !generator_needs_rebake(&spec, self.generator_bake.get(&id), size_ok) {
                continue;
            }
            if !size_ok {
                let texture = make_texture(
                    device,
                    width,
                    height,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                );
                let view = texture.create_view(&Default::default());
                self.sources.insert(
                    id,
                    SourceGpu {
                        texture,
                        view,
                        width,
                        height,
                        packed: false,
                        bgra: false,
                        uploaded_pts: i64::MIN,
                        direct: false,
                        owned: true,
                    },
                );
                self.blit_groups.remove(&id);
            }
            let view = self.sources[&id].view.clone();
            self.fill_target(
                device,
                encoder,
                &view,
                spec.color,
                spec.kind == GEN_BARS,
                spec.scroll,
            );
            self.generator_bake.insert(id, spec);
        }
    }

    pub fn sync_generators(&mut self, generators: &[(u64, Generator)], phase: f32, phase_y: f32) {
        self.scroll_phase = phase;
        self.scroll_phase_y = phase_y;
        self.generators.clear();
        for (id, spec) in generators {
            self.generators.insert(*id, *spec);
        }
    }

    pub fn unit_view(&self, unit_id: u64, kind: u32) -> Option<wgpu::TextureView> {
        let unit = self.units.get(&unit_id)?;
        Some(match kind {
            OUTPUT_PREVIEW => unit.preview_view.clone(),
            OUTPUT_MULTIVIEW => unit.multiview_view.clone().unwrap_or_else(|| unit.mixed_view.clone()),
            _ => unit.mixed_view.clone(),
        })
    }

    pub fn view_for_source(&self, source_id: u64) -> Option<wgpu::TextureView> {
        if let Some(scene) = self.scenes.get(&source_id) {
            return Some(scene.view.clone());
        }
        if let Some(gpu) = self.sources.get(&source_id) {
            return Some(gpu.view.clone());
        }
        mixing_unit_from_source(source_id).and_then(|unit_id| {
            self.units.get(&unit_id).map(|unit| match mixing_unit_bus(source_id) {
                OUTPUT_PREVIEW => unit.preview_view.clone(),
                OUTPUT_MULTIVIEW => unit
                    .multiview_view
                    .clone()
                    .unwrap_or_else(|| unit.mixed_view.clone()),
                _ => unit.mixed_view.clone(),
            })
        })
    }

    pub fn source_texture(&self, source_id: u64) -> Option<&wgpu::Texture> {
        self.sources.get(&source_id).map(|gpu| &gpu.texture)
    }

    pub fn scene_texture(&self, scene_id: u64) -> Option<&wgpu::Texture> {
        self.scenes.get(&scene_id).map(|scene| &scene.texture)
    }

    pub fn source_is_packed(&self, source_id: u64) -> bool {
        self.sources.get(&source_id).is_some_and(|gpu| gpu.packed)
    }

    pub fn vram_bytes(&self) -> u64 {
        let mut total = self.units.values().map(UnitTargets::vram_bytes).sum::<u64>();
        for source in self.sources.values() {
            if source.owned {
                total += texture_bytes(&source.texture);
            }
        }
        for scene in self.scenes.values() {
            total += texture_bytes(&scene.texture);
            if let Some(packed) = &scene.packed {
                total += texture_bytes(packed);
            }
        }
        for texture in self.input_packed.values() {
            total += texture_bytes(texture);
        }
        for (texture, _) in self.label_cache.values() {
            total += texture_bytes(texture);
        }
        if let Some((texture, _)) = &self.tally_red {
            total += texture_bytes(texture);
        }
        if let Some((texture, _)) = &self.tally_green {
            total += texture_bytes(texture);
        }
        total
    }

    pub fn packed_texture(&self, unit_id: u64, kind: u32) -> Option<&wgpu::Texture> {
        let unit = self.units.get(&unit_id)?;
        match kind {
            OUTPUT_PREVIEW => unit.packed_prv.as_ref(),
            OUTPUT_MULTIVIEW => unit.packed_mv.as_ref(),
            _ => unit.packed.as_ref(),
        }
    }

    pub fn pack_aux(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        unit_id: u64,
        preview: bool,
        multiview: bool,
    ) {
        if preview {
            self.ensure_packed_bus(device, unit_id, true);
        }
        if multiview {
            self.ensure_multiview(device, unit_id);
            self.ensure_packed_bus(device, unit_id, false);
        }
        if preview {
            let (src, dest) = {
                let Some(unit) = self.units.get(&unit_id) else {
                    return;
                };
                match unit.packed_prv.as_ref() {
                    Some(packed) => (unit.preview_view.clone(), packed.create_view(&Default::default())),
                    None => return,
                }
            };
            self.pack_to(device, encoder, mixing_unit_preview(unit_id), &src, &dest);
        }
        if multiview {
            let (src, dest) = {
                let Some(unit) = self.units.get(&unit_id) else {
                    return;
                };
                match (unit.multiview_view.as_ref(), unit.packed_mv.as_ref()) {
                    (Some(mv_view), Some(packed)) => {
                        (mv_view.clone(), packed.create_view(&Default::default()))
                    }
                    _ => return,
                }
            };
            self.pack_to(device, encoder, mixing_unit_multiview(unit_id), &src, &dest);
        }
    }

    pub fn pack_source(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        source_id: u64,
        width: u32,
        height: u32,
    ) -> Option<&wgpu::Texture> {
        let view = self.view_for_source(source_id)?;
        let packed_w = (width / 2).max(1);
        let packed_h = height.max(1);
        let reuse = self.input_packed.get(&source_id).is_some_and(|tex| {
            tex.size().width == packed_w && tex.size().height == packed_h
        });
        if !reuse {
            let packed = make_texture(
                device,
                packed_w,
                packed_h,
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            self.input_packed.insert(source_id, packed);
            self.pack_groups.remove(&(0x4000_0000_0000_0000 | source_id));
        }
        let dest = self
            .input_packed
            .get(&source_id)?
            .create_view(&Default::default());
        self.pack_to(
            device,
            encoder,
            0x4000_0000_0000_0000 | source_id,
            &view,
            &dest,
        );
        self.input_packed.get(&source_id)
    }

    pub fn pack_scene(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        scene_id: u64,
    ) -> Option<&wgpu::Texture> {
        self.ensure_scene_packed(device, scene_id);
        let (src, dest) = {
            let scene = self.scenes.get(&scene_id)?;
            (scene.view.clone(), scene.packed_view.clone()?)
        };
        self.pack_to(device, encoder, scene_id, &src, &dest);
        self.scenes.get(&scene_id).and_then(|scene| scene.packed.as_ref())
    }

    fn ensure_scene_packed(&mut self, device: &GpuDevice, scene_id: u64) {
        let Some(scene) = self.scenes.get_mut(&scene_id) else {
            return;
        };
        if scene.packed.is_some() {
            return;
        }
        let packed = make_texture(
            device,
            scene.width / 2,
            scene.height,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        scene.packed_view = Some(packed.create_view(&Default::default()));
        scene.packed = Some(packed);
    }

    fn pack_to(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        key: u64,
        src: &wgpu::TextureView,
        dest: &wgpu::TextureView,
    ) {
        self.ensure_pack_src(device, key, src);
        let Some(group) = self.pack_groups.get(&key) else {
            return;
        };
        let mut pass = begin_clear(encoder, dest);
        pass.set_pipeline(&self.pack);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn ensure_pack_src(&mut self, device: &GpuDevice, key: u64, src: &wgpu::TextureView) {
        if self.pack_groups.contains_key(&key) {
            return;
        }
        let group = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pack cached"),
            layout: &self.pack_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.pack_groups.insert(key, group);
    }
}

impl UnitTargets {
    fn new(device: &GpuDevice, width: u32, height: u32) -> Self {
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST;
        let program = make_texture(device, width, height, usage);
        let preview = make_texture(device, width, height, usage);
        let mixed = make_texture(device, width, height, usage);
        let prev = make_texture(device, width, height, usage);
        let fx = wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_DST;
        let sort_a = make_texture(device, width, height, fx);
        let sort_b = make_texture(device, width, height, fx);
        let aux = make_texture(device, width, height, fx);
        let half_w = (width / 2).max(1);
        let half_h = (height / 2).max(1);
        let flow = make_texture(device, half_w, half_h, fx);
        let bloom_a = make_texture(device, half_w, half_h, fx);
        let bloom_b = make_texture(device, half_w, half_h, fx);
        Self {
            width,
            height,
            program_view: program.create_view(&Default::default()),
            preview_view: preview.create_view(&Default::default()),
            mixed_view: mixed.create_view(&Default::default()),
            prev_view: prev.create_view(&Default::default()),
            sort_a_view: sort_a.create_view(&Default::default()),
            sort_b_view: sort_b.create_view(&Default::default()),
            flow_view: flow.create_view(&Default::default()),
            bloom_a_view: bloom_a.create_view(&Default::default()),
            bloom_b_view: bloom_b.create_view(&Default::default()),
            aux_view: aux.create_view(&Default::default()),
            prev_seeded: false,
            packed_view: None,
            multiview_view: None,
            program,
            preview,
            mixed,
            prev,
            sort_a,
            sort_b,
            flow,
            bloom_a,
            bloom_b,
            aux,
            packed: None,
            packed_mv: None,
            packed_prv: None,
            multiview: None,
        }
    }

    fn vram_bytes(&self) -> u64 {
        let mut total = texture_bytes(&self.program)
            + texture_bytes(&self.preview)
            + texture_bytes(&self.mixed)
            + texture_bytes(&self.prev)
            + texture_bytes(&self.sort_a)
            + texture_bytes(&self.sort_b)
            + texture_bytes(&self.flow)
            + texture_bytes(&self.bloom_a)
            + texture_bytes(&self.bloom_b)
            + texture_bytes(&self.aux);
        if let Some(tex) = &self.packed {
            total += texture_bytes(tex);
        }
        if let Some(tex) = &self.packed_mv {
            total += texture_bytes(tex);
        }
        if let Some(tex) = &self.packed_prv {
            total += texture_bytes(tex);
        }
        if let Some(tex) = &self.multiview {
            total += texture_bytes(tex);
        }
        total
    }
}

fn stub_named_fn(src: &str, name: &str, stub: &str) -> String {
    let needle = format!("fn {name}");
    let Some(start) = src.find(&needle) else {
        return src.to_string();
    };
    let rest = &src[start..];
    let Some(brace) = rest.find('{') else {
        return src.to_string();
    };
    let mut depth = 0i32;
    let mut end = None;
    for (i, ch) in rest[brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + brace + i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(end) = end else {
        return src.to_string();
    };
    format!("{}{}{}", &src[..start], stub, &src[end..])
}

fn custom_mix_source(user_wgsl: &str) -> String {
    let body = stub_named_fn(
        user_wgsl,
        "user_compute",
        "fn user_compute(id: vec3<u32>, dim: vec2<u32>) {}",
    );
    format!(
        "{}\n{}\n@fragment\nfn fs_main(in: VsOut) -> @location(0) vec4<f32> {{\n    if params.mix <= 0.001 {{\n        return textureSample(pgm_tex, src_samp, in.uv);\n    }}\n    if params.mix >= 0.999 {{\n        return textureSample(pvw_tex, src_samp, in.uv);\n    }}\n    return user_transition(in.uv, params.mix);\n}}\n",
        CUSTOM_MIX_PREAMBLE, body
    )
}

fn custom_compute_source(user_wgsl: &str) -> String {
    let body = stub_named_fn(
        user_wgsl,
        "user_transition",
        "fn user_transition(uv: vec2<f32>, t: f32) -> vec4<f32> { return vec4<f32>(0.0); }",
    );
    format!(
        "{}\n{}\n@compute @workgroup_size(8, 8)\nfn cs_user(@builtin(global_invocation_id) id: vec3<u32>) {{\n    let dim = vec2<u32>(textureDimensions(aux_out));\n    if id.x >= dim.x || id.y >= dim.y {{ return; }}\n    user_compute(id, dim);\n}}\n",
        USER_COMPUTE_PREAMBLE, body
    )
}

const CUSTOM_MIX_PREAMBLE: &str = r#"
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
struct MixParams {
    mix: f32,
    kind: u32,
    direction: u32,
    softness: f32,
    dip: vec4<f32>,
    param: f32,
    time: f32,
    resolution: vec2<f32>,
}
@group(0) @binding(0) var pgm_tex: texture_2d<f32>;
@group(0) @binding(1) var pvw_tex: texture_2d<f32>;
@group(0) @binding(2) var src_samp: sampler;
@group(0) @binding(3) var<uniform> params: MixParams;
@group(0) @binding(4) var prev_tex: texture_2d<f32>;
@group(0) @binding(5) var src_samp_n: sampler;
@group(0) @binding(6) var flow_tex: texture_2d<f32>;
@group(0) @binding(7) var bloom_tex: texture_2d<f32>;
@group(0) @binding(8) var aux_tex: texture_2d<f32>;
@group(0) @binding(9) var aux2_tex: texture_2d<f32>;
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let pos = positions[index];
    var out: VsOut;
    out.clip = vec4<f32>(pos, 0.0, 1.0);
    out.uv = vec2<f32>(pos.x * 0.5 + 0.5, 1.0 - (pos.y * 0.5 + 0.5));
    return out;
}
"#;

const USER_COMPUTE_PREAMBLE: &str = r#"
struct MixParams {
    mix: f32,
    kind: u32,
    direction: u32,
    softness: f32,
    dip: vec4<f32>,
    param: f32,
    time: f32,
    resolution: vec2<f32>,
}
@group(0) @binding(0) var pgm_tex: texture_2d<f32>;
@group(0) @binding(1) var pvw_tex: texture_2d<f32>;
@group(0) @binding(2) var src_samp: sampler;
@group(0) @binding(3) var<uniform> params: MixParams;
@group(0) @binding(4) var prev_tex: texture_2d<f32>;
@group(0) @binding(5) var src_samp_n: sampler;
@group(0) @binding(6) var flow_tex: texture_2d<f32>;
@group(0) @binding(7) var bloom_tex: texture_2d<f32>;
@group(0) @binding(8) var aux_out: texture_storage_2d<rgba8unorm, write>;
fn user_store(p: vec2<i32>, c: vec4<f32>) {
    textureStore(aux_out, p, c);
}
"#;

fn tile_grid(count: u32) -> (u32, u32) {
    match count {
        0 | 1 => (1, 1),
        2 => (2, 1),
        3 | 4 => (2, 2),
        5 | 6 => (3, 2),
        7 | 8 => (4, 2),
        9..=12 => (4, 3),
        _ => (4, 4),
    }
}

fn color_for(id: u64) -> [f32; 4] {
    match id {
        SRC_BLUE => [0.0, 0.0, 1.0, 1.0],
        SRC_BLACK => [0.0, 0.0, 0.0, 1.0],
        _ => [1.0, 0.0, 0.0, 1.0],
    }
}

fn make_texture(device: &GpuDevice, width: u32, height: u32, usage: wgpu::TextureUsages) -> wgpu::Texture {
    make_texture_format(device, width, height, usage, wgpu::TextureFormat::Rgba8Unorm)
}

fn make_texture_format(
    device: &GpuDevice,
    width: u32,
    height: u32,
    usage: wgpu::TextureUsages,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("eiviz target"),
        size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn pipeline(
    device: &GpuDevice,
    label: &str,
    source: &str,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    blend: bool,
) -> Result<wgpu::RenderPipeline, String> {
    let shader = device.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline_layout = device.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    Ok(device.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: blend.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    }))
}

fn compute_pipeline(
    device: &GpuDevice,
    label: &str,
    source: &str,
    layout: &wgpu::BindGroupLayout,
    entry: &str,
) -> Result<wgpu::ComputePipeline, String> {
    let shader = device.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline_layout = device.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    Ok(device.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    }))
}

fn begin_clear<'a>(encoder: &'a mut wgpu::CommandEncoder, view: &'a wgpu::TextureView) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("eiviz clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    })
}

fn write_aligned_texture(
    device: &GpuDevice,
    texture: &wgpu::Texture,
    data: &[u8],
    row_bytes: u32,
    height: u32,
    tex_width: u32,
    format: wgpu::TextureFormat,
    #[cfg(windows)] rebar: Option<&mut crate::rebar::RebarUploader>,
) {
    #[cfg(windows)]
    if let Some(uploader) = rebar {
        if uploader
            .upload(device, texture, data, row_bytes, height, tex_width, format)
            .is_ok()
        {
            return;
        }
    }
    #[cfg(not(windows))]
    let _ = format;
    let aligned = row_bytes.div_ceil(256) * 256;
    let (bytes, pitch) = if aligned == row_bytes {
        (Cow::Borrowed(data), row_bytes)
    } else {
        let mut padded = vec![0u8; aligned as usize * height as usize];
        let row = row_bytes as usize;
        for y in 0..height as usize {
            let src = y * row;
            let dst = y * aligned as usize;
            if src + row <= data.len() {
                padded[dst..dst + row].copy_from_slice(&data[src..src + row]);
            }
        }
        (Cow::Owned(padded), aligned)
    };
    device.queue.write_texture(
        texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(pitch),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width: tex_width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

fn solid_swatch(device: &GpuDevice, rgba: [u8; 4]) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = make_texture(
        device,
        8,
        8,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let pixels = vec![rgba; 64];
    let bytes: Vec<u8> = pixels.into_iter().flatten().collect();
    let mut padded = vec![0u8; 256 * 8];
    for y in 0..8 {
        let src = y * 32;
        let dst = y * 256;
        padded[dst..dst + 32].copy_from_slice(&bytes[src..src + 32]);
    }
    device.queue.write_texture(
        texture.as_image_copy(),
        &padded,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(256),
            rows_per_image: Some(8),
        },
        wgpu::Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn begin<'a>(encoder: &'a mut wgpu::CommandEncoder, view: &'a wgpu::TextureView) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("eiviz pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    })
}

fn sampled_compute(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_write(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba8Unorm,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn sampler_compute(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn sampled(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct LabelTexKey {
    text: String,
    rgb: [u8; 3],
    font_q: u16,
    dest_w: u32,
}

impl LabelTexKey {
    fn new(text: &str, rgb: [u8; 3], font_px: f32, dest_w: u32) -> Self {
        Self {
            text: text.to_string(),
            rgb,
            font_q: (font_px * 2.0).round() as u16,
            dest_w,
        }
    }
}

fn label_cache_key(key: &LabelTexKey) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    LABEL_BASE.wrapping_add(0xF000) ^ hasher.finish()
}

fn mv_label_rgb(source_id: u64, preview: [u8; 3], program: [u8; 3], inactive: [u8; 3]) -> [u8; 3] {
    if mixing_unit_from_source(source_id).is_none() {
        return inactive;
    }
    match mixing_unit_bus(source_id) {
        OUTPUT_PREVIEW => preview,
        OUTPUT_PROGRAM => program,
        _ => inactive,
    }
}

#[cfg(test)]
mod tests {
    use super::{crop_blit, FULL_UV};
    use crate::abi::Rect;

    #[test]
    fn full_crop_keeps_dest_and_uv() {
        let (dest, uv) = crop_blit(
            [0.1, 0.2, 0.5, 0.4],
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
        );
        assert_eq!(dest, [0.1, 0.2, 0.5, 0.4]);
        assert_eq!(uv, FULL_UV);
    }

    #[test]
    fn down_inset_shortens_dest_from_bottom() {
        let (dest, uv) = crop_blit(
            [0.0, 0.0, 1.0, 1.0],
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 0.75,
            },
        );
        assert_eq!(dest, [0.0, 0.0, 1.0, 0.75]);
        assert_eq!(uv, [0.0, 0.0, 1.0, 0.75]);
    }

    #[test]
    fn left_inset_moves_dest_right() {
        let (dest, uv) = crop_blit(
            [0.0, 0.0, 1.0, 1.0],
            Rect {
                x: 0.2,
                y: 0.0,
                width: 0.8,
                height: 1.0,
            },
        );
        assert_eq!(dest, [0.2, 0.0, 0.8, 1.0]);
        assert_eq!(uv[0], 0.2);
        assert_eq!(uv[2], 0.8);
    }

    #[test]
    fn static_generator_skips_identical_rebake() {
        let spec = super::Generator {
            kind: crate::abi::GEN_SOLID,
            color: [1.0, 0.0, 0.0, 1.0],
            ..super::Generator::default()
        };
        assert!(!super::generator_needs_rebake(&spec, Some(&spec), true));
    }

    #[test]
    fn static_generator_rebakes_on_color_change() {
        let prev = super::Generator {
            kind: crate::abi::GEN_SOLID,
            color: [1.0, 0.0, 0.0, 1.0],
            ..super::Generator::default()
        };
        let next = super::Generator {
            color: [0.0, 0.0, 1.0, 1.0],
            ..prev
        };
        assert!(super::generator_needs_rebake(&next, Some(&prev), true));
    }

    #[test]
    fn scrolling_generator_always_rebakes() {
        let spec = super::Generator {
            kind: crate::abi::GEN_BARS,
            scroll: true,
            ..super::Generator::default()
        };
        assert!(super::generator_needs_rebake(&spec, Some(&spec), true));
    }
}

fn mv_tally_program(source_id: u64, tallies: &[(u64, u64)]) -> Option<bool> {
    if mixing_unit_from_source(source_id).is_some() {
        return match mixing_unit_bus(source_id) {
            OUTPUT_PREVIEW => Some(false),
            OUTPUT_PROGRAM => Some(true),
            _ => None,
        };
    }
    if source_id != 0 && tallies.iter().any(|(_, program)| *program == source_id) {
        return Some(true);
    }
    if source_id != 0 && tallies.iter().any(|(preview, _)| *preview == source_id) {
        return Some(false);
    }
    None
}

