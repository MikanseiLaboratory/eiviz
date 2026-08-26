use std::collections::HashMap;

use wgpu::util::DeviceExt;

use crate::abi::{
    is_scene, mixing_unit_bus, mixing_unit_from_source, OverlayDesc, UnitState, GEN_BARS,
    MV_SLOT_MAX, OUTPUT_MULTIVIEW, OUTPUT_PREVIEW, OUTPUT_PROGRAM, SRC_BARS, SRC_BLACK, SRC_BLUE,
    SRC_COLOR, TRANSITION_DIP,
};
use crate::device::GpuDevice;
use crate::present::Presenters;
use crate::readback::ReadbackStore;
use crate::upload::{CpuFormat, UploadStore};

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
}

pub struct UnitTargets {
    pub width: u32,
    pub height: u32,
    pub program: wgpu::Texture,
    pub preview: wgpu::Texture,
    pub mixed: wgpu::Texture,
    pub packed: wgpu::Texture,
    pub multiview: wgpu::Texture,
    pub packed_mv: wgpu::Texture,
    pub packed_prv: wgpu::Texture,
    program_view: wgpu::TextureView,
    preview_view: wgpu::TextureView,
    mixed_view: wgpu::TextureView,
    packed_view: wgpu::TextureView,
    multiview_view: wgpu::TextureView,
}

struct SceneGpu {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    packed: wgpu::Texture,
    width: u32,
    height: u32,
    layers: Vec<OverlayDesc>,
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
    uyvy_bg_layout: wgpu::BindGroupLayout,
    mix_bg_layout: wgpu::BindGroupLayout,
    pack_bg_layout: wgpu::BindGroupLayout,
    sources: HashMap<u64, SourceGpu>,
    units: HashMap<u64, UnitTargets>,
    scenes: HashMap<u64, SceneGpu>,
    generators: HashMap<u64, Generator>,
    input_packed: HashMap<u64, wgpu::Texture>,
    scroll_phase: f32,
}

impl Composer {
    pub fn new(device: &GpuDevice) -> Result<Self, String> {
        let color_bg_layout = device.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("color"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let blit_bg_layout = device.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit"),
            entries: &[
                sampled(0),
                sampler_entry(1),
                uniform(2),
            ],
        });
        let uyvy_bg_layout = device.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uyvy"),
            entries: &[sampled(0), sampler_entry(1)],
        });
        let mix_bg_layout = device.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mix"),
            entries: &[sampled(0), sampled(1), sampler_entry(2), uniform(3)],
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

        Ok(Self {
            color: pipeline(device, "color", include_str!("../shaders/color.wgsl"), &color_bg_layout, wgpu::TextureFormat::Rgba8Unorm, false)?,
            bars: pipeline(device, "bars", include_str!("../shaders/bars.wgsl"), &color_bg_layout, wgpu::TextureFormat::Rgba8Unorm, false)?,
            blit: pipeline(device, "blit", include_str!("../shaders/blit.wgsl"), &blit_bg_layout, wgpu::TextureFormat::Rgba8Unorm, true)?,
            uyvy: pipeline(device, "uyvy", include_str!("../shaders/uyvy_to_rgba.wgsl"), &uyvy_bg_layout, wgpu::TextureFormat::Rgba8Unorm, false)?,
            mix: pipeline(device, "mix", include_str!("../shaders/mix.wgsl"), &mix_bg_layout, wgpu::TextureFormat::Rgba8Unorm, false)?,
            pack: pipeline(device, "pack", include_str!("../shaders/rgba_to_uyvy.wgsl"), &pack_bg_layout, wgpu::TextureFormat::Rgba8Unorm, false)?,
            sampler,
            color_bg_layout,
            blit_bg_layout,
            uyvy_bg_layout,
            mix_bg_layout,
            pack_bg_layout,
            sources: HashMap::new(),
            units: HashMap::new(),
            scenes: HashMap::new(),
            generators: HashMap::new(),
            input_packed: HashMap::new(),
            scroll_phase: 0.0,
        })
    }

    pub fn ensure_unit(&mut self, device: &GpuDevice, unit_id: u64, width: u32, height: u32) {
        if self.units.get(&unit_id).is_some_and(|unit| unit.width == width && unit.height == height) {
            return;
        }
        self.units.insert(unit_id, UnitTargets::new(device, width, height));
    }

    pub fn upload_sources(&mut self, device: &GpuDevice, uploads: &UploadStore) {
        for id in uploads.ids() {
            let Some(ring) = uploads.get(id) else { continue };
            if !ring.has_frame {
                continue;
            }
            let packed = matches!(ring.format, CpuFormat::Uyvy | CpuFormat::Uyva);
            let tex_w = if packed { ring.width / 2 } else { ring.width };
            let needs_new = self.sources.get(&id).is_none_or(|gpu| {
                gpu.width != tex_w || gpu.height != ring.height || gpu.packed != packed
            });
            if needs_new {
                let texture = make_texture(device, tex_w, ring.height, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT);
                let view = texture.create_view(&Default::default());
                self.sources.insert(id, SourceGpu { texture, view, width: tex_w, height: ring.height, packed });
            }
            let gpu = self.sources.get(&id).expect("source inserted");
            device.queue.write_texture(
                gpu.texture.as_image_copy(),
                ring.latest_rgba_or_packed(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(if packed { ring.width * 2 } else { ring.width * 4 }),
                    rows_per_image: Some(ring.height),
                },
                wgpu::Extent3d { width: tex_w, height: ring.height, depth_or_array_layers: 1 },
            );
        }
    }

    pub fn sync_scenes(
        &mut self,
        device: &GpuDevice,
        specs: &[(u64, u32, u32, Vec<OverlayDesc>)],
    ) {
        let keep: std::collections::HashSet<u64> = specs.iter().map(|spec| spec.0).collect();
        self.scenes.retain(|id, _| keep.contains(id));
        for (id, width, height, layers) in specs {
            if let Some(existing) = self.scenes.get_mut(id) {
                if existing.width == *width && existing.height == *height {
                    existing.layers = layers.clone();
                    continue;
                }
            }
            self.define_scene(device, *id, *width, *height, layers.clone());
        }
    }

    pub fn define_scene(
        &mut self,
        device: &GpuDevice,
        scene_id: u64,
        width: u32,
        height: u32,
        layers: Vec<OverlayDesc>,
    ) {
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC;
        let texture = make_texture(device, width, height, usage);
        let packed = make_texture(device, width / 2, height, usage);
        let view = texture.create_view(&Default::default());
        self.scenes.insert(
            scene_id,
            SceneGpu {
                texture,
                view,
                packed,
                width,
                height,
                layers,
            },
        );
    }

    pub fn destroy_scene(&mut self, scene_id: u64) {
        self.scenes.remove(&scene_id);
    }

    pub fn render_scenes(&mut self, device: &GpuDevice) -> Result<(), String> {
        let ids: Vec<u64> = self.scenes.keys().copied().collect();
        for id in ids {
            self.draw_scene(device, id)?;
        }
        Ok(())
    }

    fn draw_scene(&mut self, device: &GpuDevice, scene_id: u64) -> Result<(), String> {
        let (width, height, layers, view) = {
            let scene = self.scenes.get(&scene_id).ok_or("scene missing")?;
            (
                scene.width,
                scene.height,
                scene.layers.clone(),
                scene.view.clone(),
            )
        };
        let mut encoder = device.device.create_command_encoder(&Default::default());
        {
            let _pass = begin_clear(&mut encoder, &view);
        }
        let mut layers = layers;
        layers.sort_by_key(|layer| layer.z);
        if layers.is_empty() {
            self.fill_color(
                device,
                &mut encoder,
                &view,
                [0.0, 0.0, 0.0, 1.0],
                [0.0, 0.0, 1.0, 1.0],
                1.0,
                false,
            );
        } else {
            for layer in layers {
                self.draw_source(
                    device,
                    &mut encoder,
                    &view,
                    layer.source_id,
                    [layer.rect.x, layer.rect.y, layer.rect.width, layer.rect.height],
                    layer.opacity,
                    width,
                    height,
                )?;
            }
        }
        device.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    pub fn render_unit(
        &mut self,
        device: &GpuDevice,
        unit_id: u64,
        state: &UnitState,
        presenters: &Presenters,
        readbacks: &mut ReadbackStore,
        uploads: &UploadStore,
    ) -> Result<(), String> {
        let (width, height) = {
            let unit = self.units.get(&unit_id).ok_or("unit targets missing")?;
            (unit.width, unit.height)
        };
        self.draw_bus(device, unit_id, state.program_source, true, width, height)?;
        self.draw_bus(device, unit_id, state.preview_source, false, width, height)?;
        self.draw_mix(device, unit_id, state)?;
        self.draw_overlays_on_program(device, unit_id, state)?;
        self.draw_multiview(device, unit_id, state)?;
        self.draw_pack(device, unit_id)?;

        let mixed_view = self.units[&unit_id].mixed.create_view(&Default::default());
        let preview_view = self.units[&unit_id].preview.create_view(&Default::default());
        let mv_view = self.units[&unit_id].multiview.create_view(&Default::default());

        for kind in presenters.keys_for_unit(unit_id) {
            match kind {
                OUTPUT_PROGRAM => presenters.present_texture(
                    device,
                    unit_id,
                    kind,
                    &[(mixed_view.clone(), [0.0, 0.0, 1.0, 1.0])],
                )?,
                OUTPUT_PREVIEW => presenters.present_texture(
                    device,
                    unit_id,
                    kind,
                    &[(preview_view.clone(), [0.0, 0.0, 1.0, 1.0])],
                )?,
                OUTPUT_MULTIVIEW => presenters.present_texture(
                    device,
                    unit_id,
                    kind,
                    &[(mv_view.clone(), [0.0, 0.0, 1.0, 1.0])],
                )?,
                _ => {}
            }
        }

        let packed = self.units[&unit_id].packed.clone();
        let readback = readbacks.ensure(device, unit_id, width, height);
        let mut encoder = device.device.create_command_encoder(&Default::default());
        readback.copy_from(&mut encoder, &packed);
        device.queue.submit(Some(encoder.finish()));
        readback.advance(device);
        let _ = uploads;
        Ok(())
    }

    fn draw_bus(
        &mut self,
        device: &GpuDevice,
        unit_id: u64,
        source_id: u64,
        program: bool,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let target_view = if program {
            self.units[&unit_id].program_view.clone()
        } else {
            self.units[&unit_id].preview_view.clone()
        };
        let mut encoder = device.device.create_command_encoder(&Default::default());
        self.draw_source(
            device,
            &mut encoder,
            &target_view,
            source_id,
            [0.0, 0.0, 1.0, 1.0],
            1.0,
            width,
            height,
        )?;
        device.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    fn draw_overlays_on_program(
        &mut self,
        device: &GpuDevice,
        unit_id: u64,
        state: &UnitState,
    ) -> Result<(), String> {
        if state.overlay_count == 0 {
            return Ok(());
        }
        let (width, height, dest) = {
            let unit = self.units.get(&unit_id).ok_or("unit missing")?;
            (unit.width, unit.height, unit.mixed_view.clone())
        };
        let mut overlays: Vec<OverlayDesc> = state.overlays[..state.overlay_count as usize].to_vec();
        overlays.sort_by_key(|overlay| overlay.z);
        let mut encoder = device.device.create_command_encoder(&Default::default());
        for overlay in overlays {
            self.draw_source(
                device,
                &mut encoder,
                &dest,
                overlay.source_id,
                [
                    overlay.rect.x,
                    overlay.rect.y,
                    overlay.rect.width,
                    overlay.rect.height,
                ],
                overlay.opacity,
                width,
                height,
            )?;
        }
        device.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    fn draw_multiview(
        &mut self,
        device: &GpuDevice,
        unit_id: u64,
        state: &UnitState,
    ) -> Result<(), String> {
        let (width, height, dest, preview, mixed) = {
            let unit = self.units.get(&unit_id).ok_or("unit missing")?;
            (
                unit.width,
                unit.height,
                unit.multiview_view.clone(),
                unit.preview_view.clone(),
                unit.mixed_view.clone(),
            )
        };
        let mut encoder = device.device.create_command_encoder(&Default::default());
        {
            let _pass = begin_clear(&mut encoder, &dest);
        }
        self.fill_color(device, &mut encoder, &dest, [0.91, 0.47, 0.13, 1.0], [0.0, 0.0, 0.5, 0.045], 1.0, false);
        self.fill_color(device, &mut encoder, &dest, [0.18, 0.49, 0.20, 1.0], [0.5, 0.0, 0.5, 0.045], 1.0, false);
        self.blit_to(device, &mut encoder, &preview, &dest, [0.0, 0.045, 0.5, 0.455], 1.0);
        self.blit_to(device, &mut encoder, &mixed, &dest, [0.5, 0.045, 0.5, 0.455], 1.0);
        let count = (state.mv_slot_count as usize).min(MV_SLOT_MAX).max(1);
        let (cols, rows) = tile_grid(count as u32);
        for index in 0..count {
            let col = (index as u32 % cols) as f32;
            let row = (index as u32 / cols) as f32;
            let x = col / cols as f32;
            let y = 0.5 + row / rows as f32 * 0.5;
            let w = 1.0 / cols as f32;
            let h = 0.5 / rows as f32;
            self.fill_color(
                device,
                &mut encoder,
                &dest,
                [0.12, 0.32, 0.62, 1.0],
                [x, y, w, h * 0.16],
                1.0,
                false,
            );
            let slot = state.mv_slots[index];
            if slot == 0 {
                self.fill_color(
                    device,
                    &mut encoder,
                    &dest,
                    [0.0, 0.0, 0.0, 1.0],
                    [x, y + h * 0.16, w, h * 0.84],
                    1.0,
                    false,
                );
            } else {
                self.draw_source(
                    device,
                    &mut encoder,
                    &dest,
                    slot,
                    [x, y + h * 0.16, w, h * 0.84],
                    1.0,
                    width,
                    height,
                )?;
            }
        }
        device.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    fn draw_mix(&self, device: &GpuDevice, unit_id: u64, state: &UnitState) -> Result<(), String> {
        let unit = self.units.get(&unit_id).ok_or("unit missing")?;
        let params = MixParams {
            mix: state.mix,
            kind: state.transition_kind,
            pad: [0.0; 2],
            dip: if state.transition_kind == TRANSITION_DIP {
                [0.0, 0.0, 0.0, 1.0]
            } else {
                [0.0; 4]
            },
        };
        let buffer = uniform_buf(device, &params);
        let bind = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mix bg"),
            layout: &self.mix_bg_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&unit.program_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&unit.preview_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: buffer.as_entire_binding() },
            ],
        });
        let mut encoder = device.device.create_command_encoder(&Default::default());
        {
            let mut pass = begin(&mut encoder, &unit.mixed_view);
            pass.set_pipeline(&self.mix);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        device.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    fn draw_pack(&self, device: &GpuDevice, unit_id: u64) -> Result<(), String> {
        let unit = self.units.get(&unit_id).ok_or("unit missing")?;
        let bind = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pack bg"),
            layout: &self.pack_bg_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&unit.mixed_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let mut encoder = device.device.create_command_encoder(&Default::default());
        {
            let mut pass = begin(&mut encoder, &unit.packed_view);
            pass.set_pipeline(&self.pack);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        device.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    fn draw_source(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        dest: &wgpu::TextureView,
        source_id: u64,
        dst: [f32; 4],
        opacity: f32,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        if is_scene(source_id) {
            if let Some(scene) = self.scenes.get(&source_id) {
                let view = scene.view.clone();
                self.blit_to(device, encoder, &view, dest, dst, opacity);
                return Ok(());
            }
        }
        if let Some(other) = mixing_unit_from_source(source_id) {
            if let Some(unit) = self.units.get(&other) {
                let view = match mixing_unit_bus(source_id) {
                    OUTPUT_PREVIEW => unit.preview_view.clone(),
                    OUTPUT_MULTIVIEW => unit.multiview_view.clone(),
                    _ => unit.mixed_view.clone(),
                };
                self.blit_to(device, encoder, &view, dest, dst, opacity);
                return Ok(());
            }
        }
        if let Some(spec) = self.generators.get(&source_id).copied() {
            if spec.kind == GEN_BARS {
                self.fill_bars(device, encoder, dest, dst, opacity, spec.scroll);
            } else {
                self.fill_color(device, encoder, dest, spec.color, dst, opacity, spec.scroll);
            }
            return Ok(());
        }
        match source_id {
            SRC_COLOR | SRC_BLACK | SRC_BLUE => {
                self.fill_color(device, encoder, dest, color_for(source_id), dst, opacity, false);
                Ok(())
            }
            SRC_BARS => {
                self.fill_bars(device, encoder, dest, dst, opacity, false);
                Ok(())
            }
            id => {
                if let Some(gpu) = self.sources.get(&id) {
                    if gpu.packed {
                        let decoded = make_texture(
                            device,
                            width,
                            height,
                            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                        );
                        let decoded_view = decoded.create_view(&Default::default());
                        let bind = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("uyvy bg"),
                            layout: &self.uyvy_bg_layout,
                            entries: &[
                                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&gpu.view) },
                                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                            ],
                        });
                        {
                            let mut pass = begin(encoder, &decoded_view);
                            pass.set_pipeline(&self.uyvy);
                            pass.set_bind_group(0, &bind, &[]);
                            pass.draw(0..3, 0..1);
                        }
                        self.blit_to(device, encoder, &decoded_view, dest, dst, opacity);
                    } else {
                        let view = gpu.view.clone();
                        self.blit_to(device, encoder, &view, dest, dst, opacity);
                    }
                    Ok(())
                } else {
                    self.fill_color(device, encoder, dest, [0.0, 0.0, 0.0, 1.0], dst, opacity, false);
                    Ok(())
                }
            }
        }
    }

    fn blit_to(
        &self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        src: &wgpu::TextureView,
        dest: &wgpu::TextureView,
        dst: [f32; 4],
        opacity: f32,
    ) {
        let buffer = uniform_buf(device, &BlitParams { dst, opacity, pad: [0.0; 3] });
        let bind = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit bg"),
            layout: &self.blit_bg_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: buffer.as_entire_binding() },
            ],
        });
        let mut pass = begin(encoder, dest);
        pass.set_pipeline(&self.blit);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..6, 0..1);
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
                SRC_BARS => self.fill_bars(device, &mut encoder, &view, [0.0, 0.0, 1.0, 1.0], 1.0, false),
                _ => self.fill_color(
                    device,
                    &mut encoder,
                    &view,
                    color_for(id),
                    [0.0, 0.0, 1.0, 1.0],
                    1.0,
                    false,
                ),
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
                },
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
                OUTPUT_MULTIVIEW => unit.multiview_view.clone(),
                _ => unit.mixed_view.clone(),
            })
        })
    }

    pub fn packed_texture(&self, unit_id: u64, kind: u32) -> Option<&wgpu::Texture> {
        let unit = self.units.get(&unit_id)?;
        Some(match kind {
            OUTPUT_PREVIEW => &unit.packed_prv,
            OUTPUT_MULTIVIEW => &unit.packed_mv,
            _ => &unit.packed,
        })
    }

    pub fn pack_aux(&self, device: &GpuDevice, unit_id: u64) {
        let Some(unit) = self.units.get(&unit_id) else {
            return;
        };
        pack_view(self, device, &unit.preview_view, &unit.packed_prv.create_view(&Default::default()));
        pack_view(self, device, &unit.multiview_view, &unit.packed_mv.create_view(&Default::default()));
    }

    pub fn pack_source(&mut self, device: &GpuDevice, source_id: u64, width: u32, height: u32) -> Option<&wgpu::Texture> {
        let view = self.view_for_source(source_id)?;
        let packed = make_texture(
            device,
            (width / 2).max(1),
            height.max(1),
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        pack_view(self, device, &view, &packed.create_view(&Default::default()));
        self.input_packed.insert(source_id, packed);
        self.input_packed.get(&source_id)
    }

    pub fn pack_scene(&self, device: &GpuDevice, scene_id: u64) -> Option<&wgpu::Texture> {
        let scene = self.scenes.get(&scene_id)?;
        pack_view(self, device, &scene.view, &scene.packed.create_view(&Default::default()));
        Some(&scene.packed)
    }

    fn fill_color(
        &self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        dest: &wgpu::TextureView,
        color: [f32; 4],
        dst: [f32; 4],
        opacity: f32,
        scroll: bool,
    ) {
        let buffer = uniform_buf(
            device,
            &ColorParams {
                color,
                scroll: if scroll { self.scroll_phase } else { 0.0 },
                flags: if scroll { 1.0 } else { 0.0 },
                pad: [0.0; 2],
            },
        );
        let bind = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("color bg"),
            layout: &self.color_bg_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        });
        if dst == [0.0, 0.0, 1.0, 1.0] && opacity >= 1.0 {
            let mut pass = begin(encoder, dest);
            pass.set_pipeline(&self.color);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
            return;
        }
        let temp = make_texture(device, 8, 8, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let view = temp.create_view(&Default::default());
        {
            let mut pass = begin(encoder, &view);
            pass.set_pipeline(&self.color);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.blit_to(device, encoder, &view, dest, dst, opacity);
    }

    fn fill_bars(
        &self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        dest: &wgpu::TextureView,
        dst: [f32; 4],
        opacity: f32,
        scroll: bool,
    ) {
        let buffer = uniform_buf(
            device,
            &ColorParams {
                color: [0.0; 4],
                scroll: if scroll { self.scroll_phase } else { 0.0 },
                flags: if scroll { 1.0 } else { 0.0 },
                pad: [0.0; 2],
            },
        );
        let bind = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bars bg"),
            layout: &self.color_bg_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        });
        if dst == [0.0, 0.0, 1.0, 1.0] && opacity >= 1.0 {
            let mut pass = begin(encoder, dest);
            pass.set_pipeline(&self.bars);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
            return;
        }
        let temp = make_texture(device, 64, 36, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let view = temp.create_view(&Default::default());
        {
            let mut pass = begin(encoder, &view);
            pass.set_pipeline(&self.bars);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.blit_to(device, encoder, &view, dest, dst, opacity);
    }

    fn draw_builtin_to(
        &self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        dest: &wgpu::TextureView,
        source_id: u64,
        dst: [f32; 4],
    ) {
        match source_id {
            SRC_BARS => self.fill_bars(device, encoder, dest, dst, 1.0, false),
            id => self.fill_color(device, encoder, dest, color_for(id), dst, 1.0, false),
        }
    }
}

impl UnitTargets {
    fn new(device: &GpuDevice, width: u32, height: u32) -> Self {
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC;
        let program = make_texture(device, width, height, usage);
        let preview = make_texture(device, width, height, usage);
        let mixed = make_texture(device, width, height, usage);
        let packed = make_texture(device, width / 2, height, usage);
        let packed_mv = make_texture(device, width / 2, height, usage);
        let packed_prv = make_texture(device, width / 2, height, usage);
        let multiview = make_texture(device, width, height, usage);
        Self {
            width,
            height,
            program_view: program.create_view(&Default::default()),
            preview_view: preview.create_view(&Default::default()),
            mixed_view: mixed.create_view(&Default::default()),
            packed_view: packed.create_view(&Default::default()),
            multiview_view: multiview.create_view(&Default::default()),
            program,
            preview,
            mixed,
            packed,
            packed_mv,
            packed_prv,
            multiview,
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
    device.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("eiviz target"),
        size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage,
        view_formats: &[],
    })
}

fn uniform_buf<T: bytemuck::Pod>(device: &GpuDevice, value: &T) -> wgpu::Buffer {
    device.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("uniform"),
        contents: bytemuck::bytes_of(value),
        usage: wgpu::BufferUsages::UNIFORM,
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

fn pack_view(composer: &Composer, device: &GpuDevice, src: &wgpu::TextureView, dest: &wgpu::TextureView) {
    let bind = device.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pack aux"),
        layout: &composer.pack_bg_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&composer.sampler) },
        ],
    });
    let mut encoder = device.device.create_command_encoder(&Default::default());
    {
        let mut pass = begin(&mut encoder, dest);
        pass.set_pipeline(&composer.pack);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
    device.queue.submit(Some(encoder.finish()));
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

fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
