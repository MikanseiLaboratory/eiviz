//! Resizable BAR detection and D3D12 GPU-upload-heap frame uploads.
//!
//! wgpu's `queue.write_texture` stages through system memory. When the OS exposes
//! `D3D12_HEAP_TYPE_GPU_UPLOAD` (ReBAR on a discrete GPU, or UMA), CPU writes go
//! straight into VRAM. The default path then copies VRAM→VRAM into the compose
//! texture. Experimental direct-sample binds the upload-heap texture instead.

use crate::device::GpuDevice;

const LEGACY_BAR_BYTES: u64 = 256 * 1024 * 1024;
const STAGING_SLOTS: usize = 3;

#[derive(Clone, Copy, Debug)]
pub struct RebarSnapshot {
    pub available: bool,
    pub uma: bool,
    pub gpu_upload_heaps: bool,
    pub bar_bytes: u64,
    pub vram_bytes: u64,
    pub adapter: [u8; 128],
}

impl RebarSnapshot {
    pub fn unavailable(adapter: &str) -> Self {
        let mut name = [0u8; 128];
        copy_cstr(&mut name, adapter);
        Self {
            available: false,
            uma: false,
            gpu_upload_heaps: false,
            bar_bytes: 0,
            vram_bytes: 0,
            adapter: name,
        }
    }
}

pub fn probe(device: &GpuDevice) -> RebarSnapshot {
    probe_impl(device)
}

fn copy_cstr(dest: &mut [u8], text: &str) {
    let bytes = text.as_bytes();
    let n = bytes.len().min(dest.len().saturating_sub(1));
    dest[..n].copy_from_slice(&bytes[..n]);
}

#[cfg(target_os = "macos")]
fn probe_impl(device: &GpuDevice) -> RebarSnapshot {
    macos_probe(device)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn probe_impl(device: &GpuDevice) -> RebarSnapshot {
    RebarSnapshot::unavailable(&device.adapter.get_info().name)
}

#[cfg(target_os = "macos")]
fn macos_probe(device: &GpuDevice) -> RebarSnapshot {
    let info = device.adapter.get_info();
    let mut adapter = [0u8; 128];
    copy_cstr(&mut adapter, &info.name);
    let Some((uma, vram)) = (unsafe {
        device.device.as_hal::<wgpu::hal::api::Metal>().map(|hal| {
            use objc2_metal::MTLDevice;
            let raw = hal.raw_device();
            (raw.hasUnifiedMemory(), raw.recommendedMaxWorkingSetSize())
        })
    }) else {
        return RebarSnapshot::unavailable(&info.name);
    };
    // Intel iGPUs (Iris Plus, UHD, etc.) also report hasUnifiedMemory, but the
    // Shared-texture direct-sample path is for Apple Silicon only.
    let apple = cfg!(target_arch = "aarch64") && uma;
    RebarSnapshot {
        available: apple,
        uma: apple,
        gpu_upload_heaps: false,
        bar_bytes: if apple { vram.max(1) } else { 0 },
        vram_bytes: vram,
        adapter,
    }
}

#[cfg(windows)]
fn probe_impl(device: &GpuDevice) -> RebarSnapshot {
    windows_probe(device)
}

#[cfg(windows)]
fn windows_probe(device: &GpuDevice) -> RebarSnapshot {
    use windows::Win32::Graphics::Direct3D12::{
        D3D12_FEATURE_ARCHITECTURE1, D3D12_FEATURE_D3D12_OPTIONS16,
        D3D12_FEATURE_DATA_ARCHITECTURE1, D3D12_FEATURE_DATA_D3D12_OPTIONS16,
    };

    let info = device.adapter.get_info();
    let mut adapter = [0u8; 128];
    copy_cstr(&mut adapter, &info.name);

    let Some((gpu_upload, uma)) = (unsafe {
        device.device.as_hal::<wgpu::hal::api::Dx12>().map(|hal| {
            let raw = hal.raw_device();
            let mut opts = D3D12_FEATURE_DATA_D3D12_OPTIONS16::default();
            let gpu_upload = raw
                .CheckFeatureSupport(
                    D3D12_FEATURE_D3D12_OPTIONS16,
                    std::ptr::from_mut(&mut opts).cast(),
                    u32::try_from(std::mem::size_of::<D3D12_FEATURE_DATA_D3D12_OPTIONS16>()).unwrap_or(0),
                )
                .is_ok()
                && opts.GPUUploadHeapSupported.as_bool();
            let mut arch = D3D12_FEATURE_DATA_ARCHITECTURE1::default();
            let uma = raw
                .CheckFeatureSupport(
                    D3D12_FEATURE_ARCHITECTURE1,
                    std::ptr::from_mut(&mut arch).cast(),
                    u32::try_from(std::mem::size_of::<D3D12_FEATURE_DATA_ARCHITECTURE1>()).unwrap_or(0),
                )
                .is_ok()
                && arch.UMA.as_bool();
            (gpu_upload, uma)
        })
    }) else {
        let mut snap = RebarSnapshot::unavailable(&info.name);
        snap.bar_bytes = LEGACY_BAR_BYTES;
        return snap;
    };

    let vram_bytes = dxgi_vram(info.device);
    // Discrete GPU + GPU upload heaps ⇒ the full framebuffer is CPU-visible (ReBAR).
    let available = gpu_upload && !uma;
    let bar_bytes = if gpu_upload {
        vram_bytes.max(1)
    } else if uma {
        vram_bytes
    } else {
        LEGACY_BAR_BYTES
    };

    RebarSnapshot {
        available,
        uma,
        gpu_upload_heaps: gpu_upload,
        bar_bytes,
        vram_bytes,
        adapter,
    }
}

#[cfg(windows)]
fn dxgi_vram(device_id: u32) -> u64 {
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};

    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
        return 0;
    };
    let mut i = 0;
    loop {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else {
            break;
        };
        i += 1;
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        let _ = adapter;
        if desc.DeviceId == device_id {
            return desc.DedicatedVideoMemory as u64;
        }
    }
    0
}

/// Process GPU memory on this adapter (dedicated + shared), matching Task Manager.
#[cfg(windows)]
pub fn adapter_usage_bytes(device: &wgpu::Device) -> u64 {
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter3, IDXGIFactory1, DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
        DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL,
    };

    let Some(luid) = (unsafe {
        device
            .as_hal::<wgpu::hal::api::Dx12>()
            .map(|hal| hal.raw_device().GetAdapterLuid())
    }) else {
        return 0;
    };
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
        return 0;
    };
    let mut i = 0;
    loop {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else {
            break;
        };
        i += 1;
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        if desc.AdapterLuid.LowPart != luid.LowPart || desc.AdapterLuid.HighPart != luid.HighPart {
            continue;
        }
        let Ok(adapter3) = adapter.cast::<IDXGIAdapter3>() else {
            continue;
        };
        let mut local = Default::default();
        let mut shared = Default::default();
        let _ = unsafe {
            adapter3.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut local)
        };
        let _ = unsafe {
            adapter3.QueryVideoMemoryInfo(
                0,
                DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL,
                &mut shared,
            )
        };
        return local.CurrentUsage.saturating_add(shared.CurrentUsage);
    }
    0
}

#[cfg(not(windows))]
pub fn adapter_usage_bytes(_device: &wgpu::Device) -> u64 {
    0
}

#[cfg(windows)]
pub struct RebarUploader {
    d3d: windows::Win32::Graphics::Direct3D12::ID3D12Device,
    slots: Vec<StagingSlot>,
    next: usize,
    pending: Option<wgpu::CommandEncoder>,
}

#[cfg(windows)]
struct StagingSlot {
    resource: windows::Win32::Graphics::Direct3D12::ID3D12Resource,
    imported: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    row_pitch: u32,
}

#[cfg(windows)]
impl RebarUploader {
    pub fn new(device: &GpuDevice) -> Option<Self> {
        let d3d = unsafe {
            device
                .device
                .as_hal::<wgpu::hal::api::Dx12>()
                .map(|hal| hal.raw_device().clone())
        }?;
        Some(Self {
            d3d,
            slots: Vec::new(),
            next: 0,
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
        let slot_i = self.ensure_slot(device, tex_width, height, format)?;
        let (resource, pitch, imported) = {
            let slot = &self.slots[slot_i];
            (slot.resource.clone(), slot.row_pitch, slot.imported.clone())
        };
        write_mapped(&resource, data, row_bytes, height, pitch)?;
        let encoder = self.pending.get_or_insert_with(|| {
            device
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("rebar upload"),
                })
        });
        encoder.copy_texture_to_texture(
            imported.as_image_copy(),
            dest.as_image_copy(),
            wgpu::Extent3d {
                width: tex_width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
        );
        self.next = (slot_i + 1) % STAGING_SLOTS;
        Ok(())
    }

    pub fn flush(&mut self, device: &GpuDevice) {
        if let Some(encoder) = self.pending.take() {
            device.submit(Some(encoder.finish()));
        }
    }

    fn ensure_slot(
        &mut self,
        device: &GpuDevice,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Result<usize, String> {
        let reuse = self.slots.get(self.next).is_some_and(|slot| {
            slot.width == width && slot.height == height && slot.format == format
        });
        if reuse {
            return Ok(self.next);
        }
        if self.slots.len() == STAGING_SLOTS {
            self.flush(device);
            self.slots.clear();
            self.next = 0;
        }
        while self.slots.len() < STAGING_SLOTS {
            self.slots
                .push(self.create_slot(device, width, height, format, false)?);
        }
        Ok(self.next)
    }

    fn create_slot(
        &self,
        device: &GpuDevice,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        sample: bool,
    ) -> Result<StagingSlot, String> {
        use windows::Win32::Graphics::Direct3D12::{
            ID3D12Resource, D3D12_HEAP_FLAG_NONE, D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_GPU_UPLOAD,
            D3D12_PLACED_SUBRESOURCE_FOOTPRINT, D3D12_RESOURCE_DESC, D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            D3D12_RESOURCE_STATE_COMMON, D3D12_TEXTURE_LAYOUT_UNKNOWN,
        };
        use windows::Win32::Graphics::Dxgi::Common::{
            DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
        };

        let dxgi = match format {
            wgpu::TextureFormat::Bgra8Unorm => DXGI_FORMAT_B8G8R8A8_UNORM,
            _ => DXGI_FORMAT_R8G8B8A8_UNORM,
        };
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Alignment: 0,
            Width: u64::from(width.max(1)),
            Height: height.max(1),
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: dxgi,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            Flags: Default::default(),
        };
        let heap = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_GPU_UPLOAD,
            ..Default::default()
        };
        let mut resource = None;
        unsafe {
            self.d3d
                .CreateCommittedResource::<ID3D12Resource>(
                    &heap,
                    D3D12_HEAP_FLAG_NONE,
                    &desc,
                    D3D12_RESOURCE_STATE_COMMON,
                    None,
                    &mut resource,
                )
                .map_err(|e| e.to_string())?;
        }
        let resource = resource.ok_or("GPU upload heap texture")?;
        let mut layout = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
        unsafe {
            self.d3d.GetCopyableFootprints(
                &desc,
                0,
                1,
                0,
                Some(std::ptr::from_mut(&mut layout)),
                None,
                None,
                None,
            );
        }
        let row_pitch = layout.Footprint.RowPitch.max(256);
        let extent = wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        };
        let hal = unsafe {
            wgpu::hal::dx12::Device::texture_from_raw(
                resource.clone(),
                format,
                wgpu::TextureDimension::D2,
                extent,
                1,
                1,
            )
        };
        let imported = unsafe {
            device.device.create_texture_from_hal::<wgpu::hal::api::Dx12>(
                hal,
                &wgpu::TextureDescriptor {
                    label: Some("eiviz rebar staging"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: if sample {
                        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC
                    } else {
                        wgpu::TextureUsages::COPY_SRC
                    },
                    view_formats: &[],
                },
                if sample {
                    wgpu::TextureUses::RESOURCE | wgpu::TextureUses::COPY_SRC
                } else {
                    wgpu::TextureUses::COPY_SRC
                },
            )
        };
        let view = imported.create_view(&Default::default());
        Ok(StagingSlot {
            resource,
            imported,
            view,
            width,
            height,
            format,
            row_pitch,
        })
    }
}

#[cfg(windows)]
pub struct RebarIngestRing {
    d3d: windows::Win32::Graphics::Direct3D12::ID3D12Device,
    device: wgpu::Device,
    queue: wgpu::Queue,
    slots: Vec<IngestSlot>,
    next: usize,
    dead: bool,
}

#[cfg(windows)]
struct IngestSlot {
    resource: windows::Win32::Graphics::Direct3D12::ID3D12Resource,
    buffer: wgpu::Buffer,
    dest: wgpu::Texture,
    dest_view: wgpu::TextureView,
    row_pitch: u32,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

#[cfg(windows)]
impl RebarIngestRing {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        let d3d = unsafe {
            device
                .as_hal::<wgpu::hal::api::Dx12>()
                .map(|hal| hal.raw_device().clone())
        }?;
        Some(Self {
            d3d,
            device: device.clone(),
            queue: queue.clone(),
            slots: Vec::new(),
            next: 0,
            dead: false,
        })
    }

    pub fn is_live(&self) -> bool {
        !self.dead
    }

    pub fn vram_bytes(&self) -> u64 {
        self.slots
            .iter()
            .map(|slot| crate::upload::texture_bytes(&slot.dest) + slot.buffer.size())
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
            return Err("rebar ingest disabled".into());
        }
        match self.upload_inner(
            data, stride, row_bytes, width, height, packed, bgra, format, pts,
        ) {
            Ok(frame) => Ok(frame),
            Err(error) => {
                self.dead = true;
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
        let slot_i = self.ensure(tex_w, tex_h, format)?;
        let (resource, pitch, buffer, dest, dest_view) = {
            let slot = &self.slots[slot_i];
            (
                slot.resource.clone(),
                slot.row_pitch,
                slot.buffer.clone(),
                slot.dest.clone(),
                slot.dest_view.clone(),
            )
        };
        write_mapped_strided(&resource, data, stride, row_bytes, tex_h, pitch)?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("eiviz ndi rebar"),
            });
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: Some(tex_h),
                },
            },
            dest.as_image_copy(),
            wgpu::Extent3d {
                width: tex_w,
                height: tex_h,
                depth_or_array_layers: 1,
            },
        );
        {
            let _guard = crate::device::lock_gpu_queue();
            self.queue.submit(Some(encoder.finish()));
        }
        self.next = (slot_i + 1) % STAGING_SLOTS;
        Ok(crate::upload::GpuVideoFrame {
            pts,
            width,
            height,
            packed,
            bgra,
            texture: dest,
            view: dest_view,
        })
    }

    fn ensure(
        &mut self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Result<usize, String> {
        if self.slots.get(self.next).is_some_and(|slot| {
            slot.width == width && slot.height == height && slot.format == format
        }) {
            return Ok(self.next);
        }
        self.slots.clear();
        self.next = 0;
        while self.slots.len() < STAGING_SLOTS {
            self.slots
                .push(create_gpu_upload_buffer(&self.d3d, &self.device, width, height, format)?);
        }
        Ok(self.next)
    }
}

#[cfg(windows)]
fn create_gpu_upload_buffer(
    d3d: &windows::Win32::Graphics::Direct3D12::ID3D12Device,
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> Result<IngestSlot, String> {
    use windows::Win32::Graphics::Direct3D12::{
        ID3D12Resource, D3D12_HEAP_FLAG_NONE, D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_GPU_UPLOAD,
        D3D12_PLACED_SUBRESOURCE_FOOTPRINT, D3D12_RESOURCE_DESC, D3D12_RESOURCE_DIMENSION_BUFFER,
        D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_STATE_GENERIC_READ,
        D3D12_TEXTURE_LAYOUT_ROW_MAJOR, D3D12_TEXTURE_LAYOUT_UNKNOWN,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
    };

    let dxgi = match format {
        wgpu::TextureFormat::Bgra8Unorm => DXGI_FORMAT_B8G8R8A8_UNORM,
        _ => DXGI_FORMAT_R8G8B8A8_UNORM,
    };
    let tex_desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Alignment: 0,
        Width: u64::from(width.max(1)),
        Height: height.max(1),
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: dxgi,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
        Flags: Default::default(),
    };
    let mut layout = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
    let mut total = 0u64;
    unsafe {
        d3d.GetCopyableFootprints(
            &tex_desc,
            0,
            1,
            0,
            Some(std::ptr::from_mut(&mut layout)),
            None,
            None,
            Some(std::ptr::from_mut(&mut total)),
        );
    }
    let row_pitch = layout.Footprint.RowPitch.max(256);
    let bytes = total.max(u64::from(row_pitch) * u64::from(height.max(1)));
    let buf_desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        Alignment: 0,
        Width: bytes.max(256),
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: Default::default(),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        Flags: Default::default(),
    };
    let heap = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_GPU_UPLOAD,
        ..Default::default()
    };
    let mut resource = None;
    unsafe {
        d3d.CreateCommittedResource::<ID3D12Resource>(
            &heap,
            D3D12_HEAP_FLAG_NONE,
            &buf_desc,
            D3D12_RESOURCE_STATE_GENERIC_READ,
            None,
            &mut resource,
        )
        .map_err(|e| format!("GPU upload buffer: {e}"))?;
    }
    let resource = resource.ok_or("GPU upload heap buffer")?;
    let hal = unsafe { wgpu::hal::dx12::Device::buffer_from_raw(resource.clone(), bytes.max(256)) };
    let buffer = unsafe {
        device.create_buffer_from_hal::<wgpu::hal::api::Dx12>(
            hal,
            &wgpu::BufferDescriptor {
                label: Some("eiviz ndi rebar staging"),
                size: bytes.max(256),
                usage: wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
        )
    };
    let dest = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("eiviz ndi rebar dest"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
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
    Ok(IngestSlot {
        resource,
        buffer,
        dest,
        dest_view,
        row_pitch,
        width,
        height,
        format,
    })
}

#[cfg(windows)]
fn write_mapped(
    resource: &windows::Win32::Graphics::Direct3D12::ID3D12Resource,
    data: &[u8],
    row_bytes: u32,
    height: u32,
    row_pitch: u32,
) -> Result<(), String> {
    write_mapped_strided(resource, data, row_bytes as usize, row_bytes as usize, height, row_pitch)
}

#[cfg(windows)]
fn write_mapped_strided(
    resource: &windows::Win32::Graphics::Direct3D12::ID3D12Resource,
    data: &[u8],
    stride: usize,
    row_bytes: usize,
    height: u32,
    row_pitch: u32,
) -> Result<(), String> {
    let pitch = row_pitch as usize;
    let row = row_bytes;
    unsafe {
        use windows::Win32::Graphics::Direct3D12::D3D12_RANGE;
        let mut ptr = std::ptr::null_mut();
        let no_read = D3D12_RANGE { Begin: 0, End: 0 };
        resource
            .Map(0, Some(&no_read), Some(std::ptr::from_mut(&mut ptr)))
            .map_err(|e| e.to_string())?;
        if ptr.is_null() {
            resource.Unmap(0, None);
            return Err("GPU upload heap Map returned null".into());
        }
        let dest_bytes = ptr.cast::<u8>();
        if stride == row && row == pitch {
            let n = row.saturating_mul(height as usize).min(data.len());
            if n > 0 {
                std::ptr::copy_nonoverlapping(data.as_ptr(), dest_bytes, n);
            }
        } else {
            for y in 0..height as usize {
                let src = y * stride;
                let dst = y * pitch;
                let n = row.min(data.len().saturating_sub(src)).min(pitch);
                if n > 0 {
                    std::ptr::copy_nonoverlapping(data.as_ptr().add(src), dest_bytes.add(dst), n);
                }
            }
        }
        let written = D3D12_RANGE {
            Begin: 0,
            End: pitch.saturating_mul(height as usize),
        };
        resource.Unmap(0, Some(&written));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub struct UmaUploader {
    mtl: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLDevice>>,
    rings: std::collections::HashMap<u64, UmaRing>,
}

#[cfg(target_os = "macos")]
struct UmaRing {
    slots: Vec<UmaSlot>,
    next: usize,
}

#[cfg(target_os = "macos")]
struct UmaSlot {
    raw: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLTexture>>,
    imported: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

#[cfg(target_os = "macos")]
impl UmaUploader {
    pub fn new(device: &GpuDevice) -> Option<Self> {
        let mtl = unsafe {
            device
                .device
                .as_hal::<wgpu::hal::api::Metal>()
                .map(|hal| hal.raw_device().clone())
        }?;
        Some(Self {
            mtl,
            rings: std::collections::HashMap::new(),
        })
    }

    pub fn retain(&mut self, needed: &std::collections::HashSet<u64>) {
        self.rings.retain(|id, _| needed.contains(id));
    }

    pub fn clear(&mut self) {
        self.rings.clear();
    }

    pub fn upload_direct(
        &mut self,
        device: &GpuDevice,
        source_id: u64,
        data: &[u8],
        row_bytes: u32,
        height: u32,
        tex_width: u32,
        format: wgpu::TextureFormat,
    ) -> Result<(wgpu::Texture, wgpu::TextureView), String> {
        let slot_i = self.ensure_slot(device, source_id, tex_width, height, format)?;
        let raw = self
            .rings
            .get(&source_id)
            .and_then(|ring| ring.slots.get(slot_i))
            .map(|slot| slot.raw.clone())
            .ok_or("uma ring")?;
        write_shared(&raw, data, row_bytes, height, tex_width)?;
        let (texture, view) = {
            let slot = self
                .rings
                .get(&source_id)
                .and_then(|ring| ring.slots.get(slot_i))
                .ok_or("uma ring")?;
            (slot.imported.clone(), slot.view.clone())
        };
        if let Some(ring) = self.rings.get_mut(&source_id) {
            ring.next = (slot_i + 1) % STAGING_SLOTS;
        }
        Ok((texture, view))
    }

    fn ensure_slot(
        &mut self,
        device: &GpuDevice,
        source_id: u64,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Result<usize, String> {
        let reuse = self.rings.get(&source_id).is_some_and(|ring| {
            ring.slots.get(ring.next).is_some_and(|slot| {
                slot.width == width && slot.height == height && slot.format == format
            })
        });
        if reuse {
            return Ok(self.rings[&source_id].next);
        }
        let mut ring = UmaRing {
            slots: Vec::new(),
            next: 0,
        };
        while ring.slots.len() < STAGING_SLOTS {
            ring.slots
                .push(self.create_slot(device, width, height, format)?);
        }
        self.rings.insert(source_id, ring);
        Ok(0)
    }

    fn create_slot(
        &self,
        device: &GpuDevice,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Result<UmaSlot, String> {
        use objc2_metal::{
            MTLDevice, MTLPixelFormat, MTLStorageMode, MTLTextureDescriptor, MTLTextureType,
            MTLTextureUsage,
        };

        let pixel = match format {
            wgpu::TextureFormat::Bgra8Unorm => MTLPixelFormat::BGRA8Unorm,
            _ => MTLPixelFormat::RGBA8Unorm,
        };
        let desc = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                pixel,
                width.max(1) as _,
                height.max(1) as _,
                false,
            )
        };
        desc.setStorageMode(MTLStorageMode::Shared);
        desc.setUsage(MTLTextureUsage::ShaderRead);
        let raw = self
            .mtl
            .newTextureWithDescriptor(&desc)
            .ok_or("unified-memory texture")?;
        let extent = wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        };
        let hal = unsafe {
            wgpu::hal::metal::Device::texture_from_raw(
                raw.clone(),
                format,
                MTLTextureType::Type2D,
                1,
                1,
                wgpu::hal::CopyExtent {
                    width: extent.width,
                    height: extent.height,
                    depth: 1,
                },
                None,
            )
        };
        let imported = unsafe {
            device.device.create_texture_from_hal::<wgpu::hal::api::Metal>(
                hal,
                &wgpu::TextureDescriptor {
                    label: Some("eiviz uma shared"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                },
                wgpu::TextureUses::RESOURCE | wgpu::TextureUses::COPY_SRC,
            )
        };
        let view = imported.create_view(&Default::default());
        Ok(UmaSlot {
            raw,
            imported,
            view,
            width,
            height,
            format,
        })
    }
}

#[cfg(target_os = "macos")]
fn write_shared(
    texture: &objc2::runtime::ProtocolObject<dyn objc2_metal::MTLTexture>,
    data: &[u8],
    row_bytes: u32,
    height: u32,
    tex_width: u32,
) -> Result<(), String> {
    use objc2_metal::{MTLOrigin, MTLRegion, MTLSize, MTLTexture};

    let Some(ptr) = std::ptr::NonNull::new(data.as_ptr() as *mut std::ffi::c_void) else {
        return Err("empty uma upload".into());
    };
    let region = MTLRegion {
        origin: MTLOrigin { x: 0, y: 0, z: 0 },
        size: MTLSize {
            width: tex_width.max(1) as _,
            height: height.max(1) as _,
            depth: 1,
        },
    };
    unsafe {
        texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, ptr, row_bytes as _);
    }
    Ok(())
}
