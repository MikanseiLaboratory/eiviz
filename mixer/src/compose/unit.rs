use crate::device::GpuDevice;
use crate::upload::texture_bytes;

use super::pipeline::make_texture;

pub struct UnitTargets {
    pub width: u32,
    pub height: u32,
    pub program: wgpu::Texture,
    pub preview: wgpu::Texture,
    pub mixed: wgpu::Texture,
    pub prev: wgpu::Texture,
    pub(crate) sort_a: wgpu::Texture,
    pub(crate) sort_b: wgpu::Texture,
    pub(crate) flow: wgpu::Texture,
    pub(crate) bloom_a: wgpu::Texture,
    pub(crate) bloom_b: wgpu::Texture,
    pub(crate) aux: wgpu::Texture,
    pub packed: Option<wgpu::Texture>,
    pub packed_prv: Option<wgpu::Texture>,
    pub(crate) program_view: wgpu::TextureView,
    pub(crate) preview_view: wgpu::TextureView,
    pub(crate) mixed_view: wgpu::TextureView,
    pub(crate) prev_view: wgpu::TextureView,
    pub(crate) sort_b_view: wgpu::TextureView,
    pub(crate) flow_view: wgpu::TextureView,
    pub(crate) bloom_a_view: wgpu::TextureView,
    pub(crate) bloom_b_view: wgpu::TextureView,
    pub(crate) aux_view: wgpu::TextureView,
    pub(crate) prev_seeded: bool,
    pub(crate) packed_view: Option<wgpu::TextureView>,
}

impl UnitTargets {
    pub(crate) fn new(device: &GpuDevice, width: u32, height: u32) -> Self {
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
            sort_b_view: sort_b.create_view(&Default::default()),
            flow_view: flow.create_view(&Default::default()),
            bloom_a_view: bloom_a.create_view(&Default::default()),
            bloom_b_view: bloom_b.create_view(&Default::default()),
            aux_view: aux.create_view(&Default::default()),
            prev_seeded: false,
            packed_view: None,
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
            packed_prv: None,
        }
    }

    pub(crate) fn vram_bytes(&self) -> u64 {
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
        if let Some(tex) = &self.packed_prv {
            total += texture_bytes(tex);
        }
        total
    }
}
