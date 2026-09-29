use std::num::NonZeroU64;

use crate::device::GpuDevice;

pub const UNIFORM_ALIGN: u64 = 256;
const UNIFORM_SLOTS: u32 = 512;
const UNIFORM_SLOTS_MAX: u32 = 1 << 16;

pub struct UniformPool {
    pub buffer: wgpu::Buffer,
    cursor: u32,
    capacity: u32,
}

impl UniformPool {
    pub fn new(device: &GpuDevice) -> Self {
        Self {
            buffer: create_buffer(device, UNIFORM_SLOTS),
            cursor: 0,
            capacity: UNIFORM_SLOTS,
        }
    }

    pub fn reset(&mut self) {
        self.cursor = 0;
    }

    pub fn is_full(&self) -> bool {
        self.cursor >= self.capacity
    }

    pub fn can_grow(&self) -> bool {
        self.capacity < UNIFORM_SLOTS_MAX
    }

    /// Doubles the capacity. Every bind group that references the old buffer is stale afterwards
    /// and must be rebuilt by the caller.
    pub fn grow(&mut self, device: &GpuDevice) {
        self.capacity = (self.capacity * 2).min(UNIFORM_SLOTS_MAX);
        self.buffer = create_buffer(device, self.capacity);
    }

    pub fn push<T: bytemuck::Pod>(&mut self, queue: &wgpu::Queue, value: &T) -> u32 {
        let slot = self.cursor.min(self.capacity - 1);
        let offset = slot * UNIFORM_ALIGN as u32;
        queue.write_buffer(&self.buffer, u64::from(offset), bytemuck::bytes_of(value));
        if self.cursor < self.capacity {
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

fn create_buffer(device: &GpuDevice, slots: u32) -> wgpu::Buffer {
    device.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("eiviz uniform pool"),
        size: UNIFORM_ALIGN * u64::from(slots),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub fn uniform_dyn(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT
            | wgpu::ShaderStages::VERTEX
            | wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: NonZeroU64::new(UNIFORM_ALIGN),
        },
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_grows_instead_of_reusing_the_last_slot() {
        let device = GpuDevice::with_backend(crate::device::BackendRequest::Auto).expect("gpu");
        let mut pool = UniformPool::new(&device);
        let mut offsets = std::collections::HashSet::new();
        for _ in 0..UNIFORM_SLOTS {
            offsets.insert(pool.push(&device.queue, &[0.0f32; 4]));
        }
        assert!(pool.is_full());
        assert!(pool.can_grow());
        pool.grow(&device);
        assert!(!pool.is_full());
        let next = pool.push(&device.queue, &[1.0f32; 4]);
        assert_eq!(offsets.len(), UNIFORM_SLOTS as usize);
        assert!(!offsets.contains(&next));
        pool.reset();
        assert_eq!(pool.push(&device.queue, &[0.0f32; 4]), 0);
    }
}
