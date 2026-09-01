use std::num::NonZeroU64;

use crate::device::GpuDevice;

pub const UNIFORM_ALIGN: u64 = 256;
const UNIFORM_SLOTS: u32 = 512;

pub struct UniformPool {
    pub buffer: wgpu::Buffer,
    cursor: u32,
}

impl UniformPool {
    pub fn new(device: &GpuDevice) -> Self {
        let buffer = device.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("eiviz uniform pool"),
            size: UNIFORM_ALIGN * u64::from(UNIFORM_SLOTS),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { buffer, cursor: 0 }
    }

    pub fn reset(&mut self) {
        self.cursor = 0;
    }

    pub fn push<T: bytemuck::Pod>(&mut self, queue: &wgpu::Queue, value: &T) -> u32 {
        let slot = self.cursor.min(UNIFORM_SLOTS - 1);
        let offset = slot * UNIFORM_ALIGN as u32;
        queue.write_buffer(&self.buffer, u64::from(offset), bytemuck::bytes_of(value));
        if self.cursor < UNIFORM_SLOTS {
            self.cursor += 1;
        }
        offset
    }

    pub fn slot_binding(&self) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &self.buffer,
            offset: 0,
            size: Some(NonZeroU64::new(UNIFORM_ALIGN).expect("uniform align")),
        })
    }
}

pub fn uniform_dyn(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: NonZeroU64::new(UNIFORM_ALIGN),
        },
        count: None,
    }
}
