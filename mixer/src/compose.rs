use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::abi::{
    is_multiview, is_scene, mixing_unit_bus, mixing_unit_from_source, mixing_unit_multiview,
    mixing_unit_preview, mixing_unit_source, OverlayDesc, UnitState, GEN_BARS, LABEL_BASE, MV_SLOT_MAX,
    OUTPUT_MULTIVIEW, OUTPUT_PREVIEW, SRC_BARS, SRC_BLACK, SRC_BLUE, SRC_COLOR, TRANSITION_DIP,
};
use crate::device::GpuDevice;
use crate::pool::{uniform_dyn, UniformPool};
use crate::upload::{CpuFormat, UploadStore};

const KEY_TALLY_RED: u64 = LABEL_BASE + 0xFF01;
const KEY_TALLY_GREEN: u64 = LABEL_BASE + 0xFF02;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ColorParams {
    color: [f32; 4],
    scroll: f32,
    flags: f32,
    pad: [f32; 2],
}

#[derive(Clone, Copy)]
pub struct Generator {
    pub kind: u32,
    pub color: [f32; 4],
    pub scroll: bool,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlitParams {
    dst: [f32; 4],
    opacity: f32,
    pad: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MixParams {
    mix: f32,
    kind: u32,
    pad: [f32; 2],
    dip: [f32; 4],
}

struct SourceGpu {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    packed: bool,
    bgra: bool,
    uploaded_pts: i64,
}

pub struct UnitTargets {
    pub width: u32,
    pub height: u32,
    pub program: wgpu::Texture,
    pub preview: wgpu::Texture,
    pub mixed: wgpu::Texture,
    pub packed: Option<wgpu::Texture>,
    pub packed_mv: Option<wgpu::Texture>,
    pub packed_prv: Option<wgpu::Texture>,
    pub multiview: Option<wgpu::Texture>,
    program_view: wgpu::TextureView,
    preview_view: wgpu::TextureView,
    mixed_view: wgpu::TextureView,
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
}

pub struct Composer {
    color: wgpu::RenderPipeline,
    bars: wgpu::RenderPipeline,
    blit: wgpu::RenderPipeline,
    uyvy: wgpu::RenderPipeline,
    mix: wgpu::RenderPipeline,
    pack: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    color_bg_layout: wgpu::BindGroupLayout,
    blit_bg_layout: wgpu::BindGroupLayout,
    mix_bg_layout: wgpu::BindGroupLayout,
    pack_bg_layout: wgpu::BindGroupLayout,
    sources: HashMap<u64, SourceGpu>,
    units: HashMap<u64, UnitTargets>,
    scenes: HashMap<u64, SceneGpu>,
    generators: HashMap<u64, Generator>,
    input_packed: HashMap<u64, wgpu::Texture>,
    scroll_phase: f32,
    tally_red: Option<(wgpu::Texture, wgpu::TextureView)>,
    tally_green: Option<(wgpu::Texture, wgpu::TextureView)>,
    pool: UniformPool,
    blit_groups: HashMap<u64, wgpu::BindGroup>,
    uyvy_groups: HashMap<u64, wgpu::BindGroup>,
    mix_groups: HashMap<u64, wgpu::BindGroup>,
    pack_groups: HashMap<u64, wgpu::BindGroup>,
    color_group: wgpu::BindGroup,
    gpu_epoch: u64,
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
            entries: &[sampled(0), sampled(1), sampler_entry(2), uniform_dyn(3)],
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

        let color = pipeline(device, "color", include_str!("../shaders/color.wgsl"), &color_bg_layout, wgpu::TextureFormat::Rgba8Unorm, false)?;
        let bars = pipeline(device, "bars", include_str!("../shaders/bars.wgsl"), &color_bg_layout, wgpu::TextureFormat::Rgba8Unorm, false)?;
        let blit = pipeline(device, "blit", include_str!("../shaders/blit.wgsl"), &blit_bg_layout, wgpu::TextureFormat::Rgba8Unorm, true)?;
        let uyvy = pipeline(device, "uyvy", include_str!("../shaders/uyvy_to_rgba.wgsl"), &blit_bg_layout, wgpu::TextureFormat::Rgba8Unorm, true)?;
        let mix = pipeline(device, "mix", include_str!("../shaders/mix.wgsl"), &mix_bg_layout, wgpu::TextureFormat::Rgba8Unorm, false)?;
        let pack = pipeline(device, "pack", include_str!("../shaders/rgba_to_uyvy.wgsl"), &pack_bg_layout, wgpu::TextureFormat::Rgba8Unorm, false)?;
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
            color_bg_layout,
            blit_bg_layout,
            mix_bg_layout,
            pack_bg_layout,
            sources: HashMap::new(),
            units: HashMap::new(),
            scenes: HashMap::new(),
            generators: HashMap::new(),
            input_packed: HashMap::new(),
            scroll_phase: 0.0,
            tally_red: None,
            tally_green: None,
            pool,
            blit_groups: HashMap::new(),
            uyvy_groups: HashMap::new(),
            mix_groups: HashMap::new(),
            pack_groups: HashMap::new(),
            color_group,
            gpu_epoch: 1,
        })
    }

    pub fn begin_frame(&mut self) {
        self.pool.reset();
    }

    pub fn gpu_epoch(&self) -> u64 {
        self.gpu_epoch
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

    pub fn upload_sources(&mut self, device: &GpuDevice, uploads: &UploadStore, needed: &HashSet<u64>) {
        for id in needed {
            let Some(ring) = uploads.get(*id) else { continue };
            if !ring.has_frame {
                continue;
            }
            let packed = matches!(ring.format, CpuFormat::Uyvy | CpuFormat::Uyva);
            let bgra = ring.format == CpuFormat::Bgra;
            if let Some(frame) = ring.gpu.as_ref() {
                let needs_new = self.sources.get(id).is_none_or(|gpu| {
                    gpu.width != frame.width
                        || gpu.height != frame.height
                        || gpu.packed
                        || gpu.uploaded_pts != frame.pts
                });
                if needs_new {
                    self.blit_groups.remove(id);
                    self.uyvy_groups.remove(id);
                    self.gpu_epoch = self.gpu_epoch.wrapping_add(1);
                    self.sources.insert(
                        *id,
                        SourceGpu {
                            texture: frame.texture.clone(),
                            view: frame.view.clone(),
                            width: frame.width,
                            height: frame.height,
                            packed: false,
                            bgra: false,
                            uploaded_pts: frame.pts,
                        },
                    );
                }
                continue;
            }
            if ring.format == CpuFormat::GpuRgba {
                continue;
            }
            let tex_w = if packed { ring.width / 2 } else { ring.width };
            let needs_new = self.sources.get(id).is_none_or(|gpu| {
                gpu.width != tex_w
                    || gpu.height != ring.height
                    || gpu.packed != packed
                    || gpu.bgra != bgra
            });
            if !needs_new
                && self
                    .sources
                    .get(id)
                    .is_some_and(|gpu| gpu.uploaded_pts == ring.last_pts)
            {
                continue;
            }
            if needs_new {
                self.blit_groups.remove(id);
                self.uyvy_groups.remove(id);
                self.gpu_epoch = self.gpu_epoch.wrapping_add(1);
                let format = if packed {
                    wgpu::TextureFormat::Rgba8Unorm
                } else if bgra {
                    wgpu::TextureFormat::Bgra8Unorm
                } else {
                    wgpu::TextureFormat::Rgba8Unorm
                };
                let texture = make_texture_format(
                    device,
                    tex_w,
                    ring.height,
                    wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format,
                );
                let view = texture.create_view(&Default::default());
                self.sources.insert(
                    *id,
                    SourceGpu {
                        texture,
                        view,
                        width: tex_w,
                        height: ring.height,
                        packed,
                        bgra,
                        uploaded_pts: i64::MIN,
                    },
                );
            }
            let gpu = self.sources.get(id).expect("source inserted");
            write_aligned_texture(
                device,
                &gpu.texture,
                ring.latest_rgba_or_packed(),
                if packed { ring.width * 2 } else { ring.width * 4 },
                ring.height,
                tex_w,
            );
            if let Some(gpu) = self.sources.get_mut(id) {
                gpu.uploaded_pts = ring.last_pts;
            }
        }
    }

    pub fn sync_scenes(
        &mut self,
        device: &GpuDevice,
        specs: &[(u64, u32, u32, Arc<[OverlayDesc]>)],
    ) {
        let keep: std::collections::HashSet<u64> = specs.iter().map(|spec| spec.0).collect();
        self.scenes.retain(|id, _| keep.contains(id));
        for (id, width, height, layers) in specs {
            if let Some(existing) = self.scenes.get_mut(id) {
                if existing.width == *width && existing.height == *height {
                    if !Arc::ptr_eq(&existing.layers, layers) {
                        existing.layers = Arc::clone(layers);
                    }
                    continue;
                }
            }
            self.define_scene(device, *id, *width, *height, Arc::clone(layers));
        }
    }

    pub fn define_scene(
        &mut self,
        device: &GpuDevice,
        scene_id: u64,
        width: u32,
        height: u32,
        layers: Arc<[OverlayDesc]>,
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
        tallies: &HashMap<u64, (u64, u64)>,
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
        tallies: &HashMap<u64, (u64, u64)>,
    ) -> Result<(), String> {
        let (width, height, mut layers, view) = {
            let scene = self.scenes.get(&scene_id).ok_or("scene missing")?;
            (
                scene.width,
                scene.height,
                scene.layers.iter().copied().collect::<Vec<_>>(),
                scene.view.clone(),
            )
        };
        layers.sort_by_key(|layer| layer.z);
        {
            let mut pass = begin_clear(encoder, &view);
            if layers.is_empty() {
                self.blit_builtin_pass(device, &mut pass, SRC_BLACK, [0.0, 0.0, 1.0, 1.0], 1.0);
            } else {
                for layer in &layers {
                    self.draw_source_pass(
                        device,
                        &mut pass,
                        layer.source_id,
                        [layer.rect.x, layer.rect.y, layer.rect.width, layer.rect.height],
                        layer.opacity,
                    )?;
                }
            }
            if is_multiview(scene_id)
                && let Some(&(preview_source, program_source)) = tallies.get(&scene_id)
            {
                self.draw_mv_tally_pass(
                    device,
                    &mut pass,
                    &layers,
                    preview_source,
                    program_source,
                    width,
                    height,
                );
            }
        }
        Ok(())
    }

    fn draw_mv_tally_pass(
        &mut self,
        device: &GpuDevice,
        pass: &mut wgpu::RenderPass,
        layers: &[OverlayDesc],
        preview_source: u64,
        program_source: u64,
        width: u32,
        height: u32,
    ) {
        self.ensure_tally_swatches(device);
        let tx = 3.0 / width.max(1) as f32;
        let ty = 3.0 / height.max(1) as f32;
        let video: Vec<_> = layers.iter().filter(|layer| layer.z < 100).copied().collect();
        for (index, layer) in video.iter().enumerate().take(10) {
            let mut use_red = index == 1;
            let mut use_green = index == 0;
            if index >= 2 {
                if layer.source_id != 0 && layer.source_id == program_source {
                    use_red = true;
                    use_green = false;
                } else if layer.source_id != 0 && layer.source_id == preview_source {
                    use_green = true;
                }
            }
            if !use_red && !use_green {
                continue;
            }
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
            self.tally_red = Some(solid_swatch(device, [220, 32, 32, 255]));
        }
        if self.tally_green.is_none() {
            self.tally_green = Some(solid_swatch(device, [32, 200, 64, 255]));
        }
    }

    pub fn render_unit(
        &mut self,
        device: &GpuDevice,
        unit_id: u64,
        state: &UnitState,
        encoder: &mut wgpu::CommandEncoder,
        compose_mv: bool,
        pack_pgm: bool,
    ) -> Result<(), String> {
        let (width, height) = {
            let unit = self.units.get(&unit_id).ok_or("unit targets missing")?;
            (unit.width, unit.height)
        };
        self.draw_bus(device, encoder, unit_id, state.program_source, true, width, height)?;
        self.draw_bus(device, encoder, unit_id, state.preview_source, false, width, height)?;
        self.draw_mix(device, encoder, unit_id, state)?;
        self.draw_overlays_on_program(device, encoder, unit_id, state)?;
        if compose_mv {
            self.ensure_multiview(device, unit_id);
            self.draw_multiview(device, encoder, unit_id, state)?;
        }
        if pack_pgm {
            self.ensure_packed(device, unit_id);
            self.draw_pack(device, encoder, unit_id)?;
        }
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
        self.draw_source_pass(device, &mut pass, source_id, [0.0, 0.0, 1.0, 1.0], 1.0)?;
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
                self.draw_source_pass(
                    device,
                    &mut pass,
                    overlay.source_id,
                    [
                        overlay.rect.x,
                        overlay.rect.y,
                        overlay.rect.width,
                        overlay.rect.height,
                    ],
                    overlay.opacity,
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
                    self.draw_source_pass(device, &mut pass, slot, [x, y, w, h], 1.0)?;
                }
            }
        }
        Ok(())
    }

    fn draw_mix(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        unit_id: u64,
        state: &UnitState,
    ) -> Result<(), String> {
        self.ensure_mix_group(device, unit_id);
        let offset = self.pool.push(
            &device.queue,
            &MixParams {
                mix: state.mix,
                kind: state.transition_kind,
                pad: [0.0; 2],
                dip: if state.transition_kind == TRANSITION_DIP {
                    [0.0, 0.0, 0.0, 1.0]
                } else {
                    [0.0; 4]
                },
            },
        );
        let dest = self.units.get(&unit_id).ok_or("unit missing")?.mixed_view.clone();
        let group = self
            .mix_groups
            .get(&unit_id)
            .ok_or("mix bind group missing")?;
        let mut pass = begin(encoder, &dest);
        pass.set_pipeline(&self.mix);
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
    ) -> Result<(), String> {
        if is_scene(source_id) {
            if let Some(view) = self.scenes.get(&source_id).map(|scene| scene.view.clone()) {
                self.blit_pass(device, pass, source_id, &view, dst, opacity, false);
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
                self.blit_pass(device, pass, source_id, &view, dst, opacity, false);
                return Ok(());
            }
        }
        if self.generators.contains_key(&source_id) {
            self.blit_builtin_pass(device, pass, source_id, dst, opacity);
            return Ok(());
        }
        match source_id {
            SRC_COLOR | SRC_BLACK | SRC_BLUE | SRC_BARS => {
                self.blit_builtin_pass(device, pass, source_id, dst, opacity);
                Ok(())
            }
            id => {
                let copied = self
                    .sources
                    .get(&id)
                    .map(|gpu| (gpu.view.clone(), gpu.packed));
                if let Some((view, packed)) = copied {
                    self.blit_pass(device, pass, id, &view, dst, opacity, packed);
                    Ok(())
                } else {
                    self.blit_builtin_pass(device, pass, SRC_BLACK, dst, opacity);
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
        self.blit_pass(device, pass, key, &view, dst, opacity, false);
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
        let offset = self.pool.push(
            &device.queue,
            &BlitParams {
                dst,
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
                pad: [0.0; 2],
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
            let texture = make_texture(
                device,
                128,
                72,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            let view = texture.create_view(&Default::default());
            let mut encoder = device.device.create_command_encoder(&Default::default());
            match id {
                SRC_BARS => self.fill_target(device, &mut encoder, &view, [0.0; 4], true, false),
                _ => self.fill_target(device, &mut encoder, &view, color_for(id), false, false),
            }
            device.queue.submit(Some(encoder.finish()));
            self.sources.insert(
                id,
                SourceGpu {
                    texture,
                    view,
                    width: 128,
                    height: 72,
                    packed: false,
                    bgra: false,
                    uploaded_pts: i64::MIN,
                },
            );
        }
    }

    pub fn bake_generators(&mut self, device: &GpuDevice, encoder: &mut wgpu::CommandEncoder) {
        let gens: Vec<(u64, Generator)> = self.generators.iter().map(|(id, spec)| (*id, *spec)).collect();
        for (id, spec) in gens {
            if !spec.scroll && self.sources.contains_key(&id) {
                continue;
            }
            if !self.sources.contains_key(&id) {
                let texture = make_texture(
                    device,
                    128,
                    72,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                );
                let view = texture.create_view(&Default::default());
                self.sources.insert(
                    id,
                    SourceGpu {
                        texture,
                        view,
                        width: 128,
                        height: 72,
                        packed: false,
                        bgra: false,
                        uploaded_pts: i64::MIN,
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
        }
    }

    pub fn sync_generators(&mut self, generators: &[(u64, Generator)], phase: f32) {
        self.scroll_phase = phase;
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

    pub fn source_is_packed(&self, source_id: u64) -> bool {
        self.sources.get(&source_id).is_some_and(|gpu| gpu.packed)
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
            | wgpu::TextureUsages::COPY_SRC;
        let program = make_texture(device, width, height, usage);
        let preview = make_texture(device, width, height, usage);
        let mixed = make_texture(device, width, height, usage);
        Self {
            width,
            height,
            program_view: program.create_view(&Default::default()),
            preview_view: preview.create_view(&Default::default()),
            mixed_view: mixed.create_view(&Default::default()),
            packed_view: None,
            multiview_view: None,
            program,
            preview,
            mixed,
            packed: None,
            packed_mv: None,
            packed_prv: None,
            multiview: None,
        }
    }
}

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
) {
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

