//! Resizable BAR detection and D3D12 GPU-upload-heap frame uploads.
//!
//! wgpu's `queue.write_texture` stages through system memory. When the OS exposes
//! `D3D12_HEAP_TYPE_GPU_UPLOAD` (ReBAR on a discrete GPU, or UMA), CPU writes go
//! straight into VRAM and we copy VRAM→VRAM into the compose texture.

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

#[cfg(not(windows))]
fn probe_impl(device: &GpuDevice) -> RebarSnapshot {
    RebarSnapshot::unavailable(&device.adapter.get_info().name)
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

#[cfg(windows)]
pub struct RebarUploader {
    d3d: windows::Win32::Graphics::Direct3D12::ID3D12Device,
    slots: Vec<StagingSlot>,
    next: usize,
}

#[cfg(windows)]
struct StagingSlot {
    resource: windows::Win32::Graphics::Direct3D12::ID3D12Resource,
    imported: wgpu::Texture,
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
        let slot = &self.slots[slot_i];
        let pitch = slot.row_pitch as usize;
        let row = row_bytes as usize;
        unsafe {
            let mut ptr = std::ptr::null_mut();
            slot.resource
                .Map(0, None, Some(std::ptr::from_mut(&mut ptr)))
                .map_err(|e| e.to_string())?;
            if ptr.is_null() {
                slot.resource.Unmap(0, None);
                return Err("GPU upload heap Map returned null".into());
            }
            let dest_bytes = ptr.cast::<u8>();
            for y in 0..height as usize {
                let src = y * row;
                let dst = y * pitch;
                let n = row.min(data.len().saturating_sub(src)).min(pitch);
                if n > 0 {
                    std::ptr::copy_nonoverlapping(data.as_ptr().add(src), dest_bytes.add(dst), n);
                }
            }
            slot.resource.Unmap(0, None);
        }
        let mut encoder = device.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rebar upload"),
        });
        encoder.copy_texture_to_texture(
            slot.imported.as_image_copy(),
            dest.as_image_copy(),
            wgpu::Extent3d {
                width: tex_width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
        );
        device.submit(Some(encoder.finish()));
        self.next = (slot_i + 1) % STAGING_SLOTS;
        Ok(())
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
            self.slots.clear();
            self.next = 0;
        }
        while self.slots.len() < STAGING_SLOTS {
            self.slots
                .push(self.create_slot(device, width, height, format)?);
        }
        Ok(self.next)
    }

    fn create_slot(
        &self,
        device: &GpuDevice,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
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
                    usage: wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                },
                wgpu::TextureUses::COPY_SRC,
            )
        };
        Ok(StagingSlot {
            resource,
            imported,
            width,
            height,
            format,
            row_pitch,
        })
    }
}
