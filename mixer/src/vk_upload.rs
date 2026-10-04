//! Vulkan host-visible (ReBAR / SAM) CPU→VRAM uploads.
//!
//! Built with ash + wgpu-hal only. No Win32 types, so the same module links on Linux.

use std::mem::ManuallyDrop;

use crate::device::GpuDevice;
use crate::rebar::RebarSnapshot;

const MAX_STAGING_SLOTS: usize = 6;

type StagingKey = (u32, u32, wgpu::TextureFormat);

struct HostMap {
    ptr: *mut u8,
    size: usize,
    coherent: bool,
}

unsafe impl Send for HostMap {}
unsafe impl Sync for HostMap {}

pub(crate) struct VkHandles {
    instance: ash::Instance,
    device: ash::Device,
    physical: ash::vk::PhysicalDevice,
}

fn vk_handles_from_wgpu(device: &wgpu::Device) -> Option<VkHandles> {
    unsafe {
        device
            .as_hal::<wgpu::hal::api::Vulkan>()
            .map(|hal| VkHandles {
                instance: hal.shared_instance().raw_instance().clone(),
                device: hal.raw_device().clone(),
                physical: hal.raw_physical_device(),
            })
    }
}

fn memory_properties(handles: &VkHandles) -> ash::vk::PhysicalDeviceMemoryProperties {
    unsafe {
        handles
            .instance
            .get_physical_device_memory_properties(handles.physical)
    }
}

fn find_memory_type(
    props: &ash::vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    required: ash::vk::MemoryPropertyFlags,
) -> Option<(u32, ash::vk::MemoryPropertyFlags)> {
    for (i, mem) in props.memory_types_as_slice().iter().enumerate() {
        if type_bits & (1 << i) == 0 {
            continue;
        }
        if mem.property_flags.contains(required) {
            return Some((i as u32, mem.property_flags));
        }
    }
    None
}

fn host_visible_device_local(
    props: &ash::vk::PhysicalDeviceMemoryProperties,
) -> Option<(u32, ash::vk::MemoryPropertyFlags)> {
    use ash::vk::MemoryPropertyFlags as Flags;
    let all = u32::MAX;
    find_memory_type(
        props,
        all,
        Flags::DEVICE_LOCAL | Flags::HOST_VISIBLE | Flags::HOST_COHERENT,
    )
    .or_else(|| find_memory_type(props, all, Flags::DEVICE_LOCAL | Flags::HOST_VISIBLE))
}

pub fn probe(device: &GpuDevice) -> RebarSnapshot {
    let info = device.adapter.get_info();
    let Some(handles) = vk_handles_from_wgpu(&device.device) else {
        return RebarSnapshot::unavailable(&info.name);
    };
    let props = memory_properties(&handles);
    let mut adapter = [0u8; 128];
    let bytes = info.name.as_bytes();
    let n = bytes.len().min(adapter.len().saturating_sub(1));
    adapter[..n].copy_from_slice(&bytes[..n]);

    let mut vram_bytes = 0u64;
    for heap in props.memory_heaps_as_slice() {
        if heap.flags.contains(ash::vk::MemoryHeapFlags::DEVICE_LOCAL) {
            vram_bytes = vram_bytes.saturating_add(heap.size);
        }
    }
    let available = host_visible_device_local(&props).is_some();
    let uma = matches!(
        info.device_type,
        wgpu::DeviceType::IntegratedGpu | wgpu::DeviceType::Cpu
    );
    RebarSnapshot {
        available,
        uma,
        gpu_upload_heaps: available,
        bar_bytes: if available { vram_bytes.max(1) } else { 0 },
        vram_bytes,
        adapter,
    }
}

/// Compose-side uploader: CPU writes into a host-visible VRAM buffer, then a buffer→texture copy
/// moves the pixels into the source texture. Buffers are pooled per frame size and only reused
/// once the GPU has finished the copy that reads them.
pub struct VulkanUploader {
    handles: VkHandles,
    pool: crate::staging::StagingPool<StagingKey, UploadSlot>,
    pending: Option<wgpu::CommandEncoder>,
}

struct UploadSlot {
    map: HostMap,
    memory: ash::vk::DeviceMemory,
    vk: ash::Device,
    imported: ManuallyDrop<wgpu::Buffer>,
    row_pitch: u32,
}

impl Drop for UploadSlot {
    fn drop(&mut self) {
        unsafe {
            if !self.map.ptr.is_null() {
                self.vk.unmap_memory(self.memory);
                self.map.ptr = std::ptr::null_mut();
            }
            ManuallyDrop::drop(&mut self.imported);
        }
    }
}

impl VulkanUploader {
    pub fn new(device: &GpuDevice) -> Option<Self> {
        let handles = vk_handles_from_wgpu(&device.device)?;
        host_visible_device_local(&memory_properties(&handles))?;
        Some(Self {
            handles,
            pool: crate::staging::StagingPool::new(MAX_STAGING_SLOTS),
            pending: None,
        })
    }

    pub fn upload(
        &mut self,
        device: &GpuDevice,
        dest: &wgpu::Texture,
        data: &[u8],
        row_bytes: u32,
        height: u32,
        tex_width: u32,
        format: wgpu::TextureFormat,
    ) -> Result<(), String> {
        let key = (tex_width.max(1), height.max(1), format);
        let Self {
            handles,
            pool,
            pending,
        } = self;
        let index = pool.acquire(
            &device.device,
            key,
            || create_upload_slot(handles, &device.device, key.0, key.1),
            || {
                pending
                    .take()
                    .map(|encoder| device.submit(Some(encoder.finish())))
            },
        )?;
        let slot = pool.slot(key, index).ok_or("staging slot missing")?;
        write_host(
            &slot.map,
            &slot.vk,
            slot.memory,
            data,
            row_bytes as usize,
            row_bytes as usize,
            key.1,
            slot.row_pitch,
        )?;
        let encoder = pending.get_or_insert_with(|| {
            device
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("eiviz vulkan upload"),
                })
        });
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &slot.imported,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(slot.row_pitch),
                    rows_per_image: Some(key.1),
                },
            },
            dest.as_image_copy(),
            wgpu::Extent3d {
                width: key.0,
                height: key.1,
                depth_or_array_layers: 1,
            },
        );
        pool.mark_pending(key, index);
        Ok(())
    }

    pub fn flush(&mut self, device: &GpuDevice) {
        if let Some(encoder) = self.pending.take() {
            let index = device.submit(Some(encoder.finish()));
            self.pool.arm_pending(&index);
        }
        self.pool.end_frame();
    }
}

#[cfg(windows)]
pub struct VulkanIngestRing {
    handles: VkHandles,
    device: wgpu::Device,
    queue: wgpu::Queue,
    slots: Vec<IngestSlot>,
    dead: bool,
}

#[cfg(windows)]
struct IngestSlot {
    staging: UploadSlot,
    fence: crate::staging::Fence,
    lease: crate::upload::FrameLease,
    dest: wgpu::Texture,
    dest_view: wgpu::TextureView,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

#[cfg(windows)]
impl VulkanIngestRing {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        let handles = vk_handles_from_wgpu(device)?;
        host_visible_device_local(&memory_properties(&handles))?;
        Some(Self {
            handles,
            device: device.clone(),
            queue: queue.clone(),
            slots: Vec::new(),
            dead: false,
        })
    }

    pub fn is_live(&self) -> bool {
        !self.dead
    }

    pub fn vram_bytes(&self) -> u64 {
        self.slots
            .iter()
            .map(|slot| crate::upload::texture_bytes(&slot.dest) + slot.staging.imported.size())
            .sum()
    }

    pub fn upload(
        &mut self,
        data: &[u8],
        stride: usize,
        row_bytes: usize,
        width: u32,
        height: u32,
        packed: bool,
        bgra: bool,
        format: wgpu::TextureFormat,
        pts: i64,
    ) -> Result<crate::upload::GpuVideoFrame, String> {
        if self.dead {
            return Err("vulkan ingest disabled".into());
        }
        match self.upload_inner(
            data, stride, row_bytes, width, height, packed, bgra, format, pts,
        ) {
            Ok(frame) => Ok(frame),
            Err(error) => {
                if !crate::staging::is_exhausted(&error) {
                    self.dead = true;
                }
                Err(error)
            }
        }
    }

    fn upload_inner(
        &mut self,
        data: &[u8],
        stride: usize,
        row_bytes: usize,
        width: u32,
        height: u32,
        packed: bool,
        bgra: bool,
        format: wgpu::TextureFormat,
        pts: i64,
    ) -> Result<crate::upload::GpuVideoFrame, String> {
        let tex_w = if packed {
            (width / 2).max(1)
        } else {
            width.max(1)
        };
        let tex_h = height.max(1);
        let slot_i = self.acquire(tex_w, tex_h, format)?;
        {
            let slot = &self.slots[slot_i].staging;
            write_host(
                &slot.map,
                &slot.vk,
                slot.memory,
                data,
                stride,
                row_bytes,
                tex_h,
                slot.row_pitch,
            )?;
        }
        let slot = &self.slots[slot_i];
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("eiviz ndi vulkan"),
            });
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &slot.staging.imported,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(slot.staging.row_pitch),
                    rows_per_image: Some(tex_h),
                },
            },
            slot.dest.as_image_copy(),
            wgpu::Extent3d {
                width: tex_w,
                height: tex_h,
                depth_or_array_layers: 1,
            },
        );
        let index = {
            let _guard = crate::device::lock_gpu_queue();
            self.queue.submit(Some(encoder.finish()))
        };
        let frame = crate::upload::GpuVideoFrame {
            pts,
            width,
            height,
            packed,
            bgra,
            texture: slot.dest.clone(),
            view: slot.dest_view.clone(),
            lease: Some(std::sync::Arc::clone(&slot.lease)),
        };
        self.slots[slot_i].fence.arm(index);
        Ok(frame)
    }

    /// A slot is free once its texture is no longer referenced by any frame and the GPU has
    /// finished the copy that reads its staging buffer.
    fn acquire(
        &mut self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Result<usize, String> {
        if self
            .slots
            .iter()
            .any(|slot| slot.width != width || slot.height != height || slot.format != format)
        {
            self.slots.clear();
        }
        let device = &self.device;
        if let Some(index) = self.slots.iter().position(|slot| {
            std::sync::Arc::strong_count(&slot.lease) == 1 && slot.fence.is_idle(device)
        }) {
            return Ok(index);
        }
        if self.slots.len() < MAX_STAGING_SLOTS {
            let staging = create_upload_slot(&self.handles, &self.device, width, height)?;
            let dest = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("eiviz ndi vulkan dest"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let dest_view = dest.create_view(&Default::default());
            self.slots.push(IngestSlot {
                staging,
                fence: crate::staging::Fence::default(),
                lease: std::sync::Arc::new(()),
                dest,
                dest_view,
                width,
                height,
                format,
            });
            return Ok(self.slots.len() - 1);
        }
        Err(format!(
            "{}: all {} slots are still in use",
            crate::staging::EXHAUSTED,
            self.slots.len()
        ))
    }
}

fn create_upload_slot(
    handles: &VkHandles,
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> Result<UploadSlot, String> {
    let created = create_mapped_buffer(handles, device, width, height)?;
    Ok(UploadSlot {
        map: created.map,
        memory: created.memory,
        vk: handles.device.clone(),
        imported: created.imported,
        row_pitch: created.row_pitch,
    })
}

struct MappedBuffer {
    map: HostMap,
    memory: ash::vk::DeviceMemory,
    imported: ManuallyDrop<wgpu::Buffer>,
    row_pitch: u32,
}

fn create_mapped_buffer(
    handles: &VkHandles,
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> Result<MappedBuffer, String> {
    use ash::vk;

    let width = width.max(1);
    let height = height.max(1);
    let row_pitch = width
        .saturating_mul(4)
        .div_ceil(256)
        .saturating_mul(256)
        .max(256);
    let bytes = u64::from(row_pitch) * u64::from(height);
    let info = vk::BufferCreateInfo::default()
        .size(bytes.max(1))
        .usage(vk::BufferUsageFlags::TRANSFER_SRC)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let buffer = unsafe {
        handles
            .device
            .create_buffer(&info, None)
            .map_err(|error| format!("vkCreateBuffer: {error}"))?
    };
    let req = unsafe { handles.device.get_buffer_memory_requirements(buffer) };
    let props = memory_properties(handles);
    let Some((type_index, flags)) = find_memory_type(
        &props,
        req.memory_type_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL
            | vk::MemoryPropertyFlags::HOST_VISIBLE
            | vk::MemoryPropertyFlags::HOST_COHERENT,
    )
    .or_else(|| {
        find_memory_type(
            &props,
            req.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL | vk::MemoryPropertyFlags::HOST_VISIBLE,
        )
    }) else {
        unsafe { handles.device.destroy_buffer(buffer, None) };
        return Err("no DEVICE_LOCAL|HOST_VISIBLE memory type".into());
    };
    let alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(req.size)
        .memory_type_index(type_index);
    let memory = unsafe {
        match handles.device.allocate_memory(&alloc, None) {
            Ok(memory) => memory,
            Err(error) => {
                handles.device.destroy_buffer(buffer, None);
                return Err(format!("vkAllocateMemory: {error}"));
            }
        }
    };
    if let Err(error) = unsafe { handles.device.bind_buffer_memory(buffer, memory, 0) } {
        unsafe {
            handles.device.free_memory(memory, None);
            handles.device.destroy_buffer(buffer, None);
        }
        return Err(format!("vkBindBufferMemory: {error}"));
    }
    let ptr = unsafe {
        match handles
            .device
            .map_memory(memory, 0, req.size, vk::MemoryMapFlags::empty())
        {
            Ok(ptr) => ptr,
            Err(error) => {
                handles.device.free_memory(memory, None);
                handles.device.destroy_buffer(buffer, None);
                return Err(format!("vkMapMemory: {error}"));
            }
        }
    };
    let hal = unsafe { wgpu::hal::vulkan::Buffer::from_raw_managed(buffer, memory, 0, req.size) };
    let imported = unsafe {
        device.create_buffer_from_hal::<wgpu::hal::api::Vulkan>(
            hal,
            &wgpu::BufferDescriptor {
                label: Some("eiviz vulkan host-visible"),
                size: req.size,
                usage: wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
        )
    };
    Ok(MappedBuffer {
        map: HostMap {
            ptr: ptr.cast(),
            size: req.size as usize,
            coherent: flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT),
        },
        memory,
        imported: ManuallyDrop::new(imported),
        row_pitch,
    })
}

fn write_host(
    map: &HostMap,
    device: &ash::Device,
    memory: ash::vk::DeviceMemory,
    data: &[u8],
    stride: usize,
    row_bytes: usize,
    height: u32,
    row_pitch: u32,
) -> Result<(), String> {
    if map.ptr.is_null() {
        return Err("host-visible mapping is null".into());
    }
    let pitch = row_pitch as usize;
    let needed = pitch.saturating_mul(height.max(1) as usize);
    if needed > map.size {
        return Err("host-visible slot too small".into());
    }
    let copy = row_bytes.min(pitch);
    for y in 0..height as usize {
        let src = y * stride;
        let dst = y * pitch;
        if src + copy > data.len() {
            return Err(format!(
                "frame data too short: row {y} needs {} bytes, buffer has {}",
                src + copy,
                data.len()
            ));
        }
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr().add(src), map.ptr.add(dst), copy);
        }
    }
    if !map.coherent {
        let range = ash::vk::MappedMemoryRange::default()
            .memory(memory)
            .offset(0)
            .size(ash::vk::WHOLE_SIZE);
        unsafe {
            device
                .flush_mapped_memory_ranges(std::slice::from_ref(&range))
                .map_err(|error| format!("vkFlushMappedMemoryRanges: {error}"))?;
        }
    }
    Ok(())
}
