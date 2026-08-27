use std::sync::{Arc, Mutex};

use windows::core::{Interface, IUnknown};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Graphics::Direct3D::{D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11DeviceContext, ID3D11Multithread, ID3D11Resource, ID3D11Texture2D,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_TEXTURE2D_DESC,
};
use windows::Win32::Graphics::Direct3D11on12::{D3D11On12CreateDevice, ID3D11On12Device2};
use windows::Win32::Graphics::Direct3D12::{
    ID3D12CommandQueue, ID3D12Device, ID3D12Resource, D3D12_COMMAND_LIST_TYPE_DIRECT,
    D3D12_COMMAND_QUEUE_DESC,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_NV12,
    DXGI_FORMAT_R8G8B8A8_UNORM,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIResource1, DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE,
};
use windows::Win32::Media::MediaFoundation::{
    IMFDXGIBuffer, IMFDXGIDeviceManager, IMFSample, MFCreateDXGIDeviceManager,
};

use crate::convert::Nv12Converter;
use crate::device::GpuDevice;

pub struct DxgiVideo {
    context: Mutex<ID3D11DeviceContext>,
    on12: ID3D11On12Device2,
    queue_wgpu: ID3D12CommandQueue,
    device12: ID3D12Device,
    pub manager: IMFDXGIDeviceManager,
}

/// COM video objects: D3D11 multithread protection is on, and the DXGI device
/// manager is the MF-supported way to share the decoder device across threads.
unsafe impl Send for DxgiVideo {}
unsafe impl Sync for DxgiVideo {}

#[derive(Clone)]
pub struct GpuVideoContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub dxgi: Arc<DxgiVideo>,
    pub convert: Arc<Nv12Converter>,
}

impl GpuVideoContext {
    pub fn new(gpu: &GpuDevice) -> Result<Self, String> {
        let dxgi = DxgiVideo::new(&gpu.device)?;
        let convert = Nv12Converter::new(&gpu.device);
        Ok(Self {
            device: gpu.device.clone(),
            queue: gpu.queue.clone(),
            dxgi: Arc::new(dxgi),
            convert: Arc::new(convert),
        })
    }
}

impl DxgiVideo {
    pub fn new(device: &wgpu::Device) -> Result<Self, String> {
        let (device12, queue_wgpu) = unsafe {
            let hal = device
                .as_hal::<wgpu::hal::api::Dx12>()
                .ok_or("DX12 device is required")?;
            (hal.raw_device().clone(), hal.raw_queue().clone())
        };
        let queue_11on12: ID3D12CommandQueue = unsafe {
            device12
                .CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                    Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                    ..Default::default()
                })
                .map_err(|e| e.to_string())?
        };
        let flags = (D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT).0;
        let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
        let queue_unknown: IUnknown = queue_11on12.cast().map_err(|e| e.to_string())?;
        let queues = [Some(queue_unknown)];
        let mut d3d11 = None;
        let mut context = None;
        let mut chosen = D3D_FEATURE_LEVEL::default();
        unsafe {
            D3D11On12CreateDevice(
                &device12,
                flags,
                Some(&levels),
                Some(&queues),
                0,
                Some(&mut d3d11),
                Some(&mut context),
                Some(&mut chosen),
            )
            .map_err(|e| e.to_string())?;
        }
        let d3d11 = d3d11.ok_or("D3D11On12 device")?;
        let context = context.ok_or("D3D11On12 context")?;
        if let Ok(mt) = d3d11.cast::<ID3D11Multithread>() {
            unsafe {
                let _ = mt.SetMultithreadProtected(true);
            }
        }
        let on12: ID3D11On12Device2 = d3d11.cast().map_err(|e| e.to_string())?;
        let mut token = 0u32;
        let mut manager = None;
        unsafe {
            MFCreateDXGIDeviceManager(&mut token, &mut manager).map_err(|e| e.to_string())?;
        }
        let manager = manager.ok_or("DXGI device manager")?;
        unsafe {
            manager.ResetDevice(&d3d11, token).map_err(|e| e.to_string())?;
        }
        Ok(Self {
            context: Mutex::new(context),
            on12,
            queue_wgpu,
            device12,
            manager,
        })
    }

    pub fn import_sample(
        &self,
        gpu: &GpuVideoContext,
        sample: &IMFSample,
        pts: i64,
    ) -> Result<crate::upload::GpuVideoFrame, String> {
        unsafe {
            let buffer = sample.GetBufferByIndex(0).map_err(|e| e.to_string())?;
            let dxgi: IMFDXGIBuffer = buffer.cast().map_err(|_| "sample is not a DXGI buffer".to_string())?;
            let mut raw = std::ptr::null_mut();
            dxgi.GetResource(&ID3D11Texture2D::IID, &mut raw)
                .map_err(|e| e.to_string())?;
            let tex11 = ID3D11Texture2D::from_raw(raw);
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            tex11.GetDesc(&mut desc);
            {
                let ctx = self.context.lock().expect("d3d11 context");
                ctx.Flush();
            }
            let (resource12, wrapped) = self.unwrap_or_share(&tex11)?;
            let frame = match desc.Format {
                DXGI_FORMAT_NV12 => gpu.convert.convert_nv12(
                    &gpu.device,
                    &gpu.queue,
                    resource12,
                    desc.Width,
                    desc.Height,
                    pts,
                )?,
                DXGI_FORMAT_B8G8R8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB | DXGI_FORMAT_R8G8B8A8_UNORM => {
                    gpu.convert.copy_bgra(
                        &gpu.device,
                        &gpu.queue,
                        resource12,
                        desc.Width,
                        desc.Height,
                        pts,
                        desc.Format == DXGI_FORMAT_R8G8B8A8_UNORM,
                    )?
                }
                other => return Err(format!("unsupported DXGI format {}", other.0)),
            };
            if wrapped {
                let _ = self.on12.ReturnUnderlyingResource(
                    &tex11.cast::<ID3D11Resource>().map_err(|e| e.to_string())?,
                    0,
                    std::ptr::null(),
                    std::ptr::null(),
                );
            }
            Ok(frame)
        }
    }

    fn unwrap_or_share(&self, tex11: &ID3D11Texture2D) -> Result<(ID3D12Resource, bool), String> {
        unsafe {
            if let Ok(resource) = self
                .on12
                .UnwrapUnderlyingResource::<_, _, ID3D12Resource>(tex11, &self.queue_wgpu)
            {
                return Ok((resource, true));
            }
            let dxgi: IDXGIResource1 = tex11.cast().map_err(|e| e.to_string())?;
            let access = DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0;
            let handle = dxgi
                .CreateSharedHandle(None, access, windows::core::PCWSTR::null())
                .map_err(|e| e.to_string())?;
            let mut resource = None;
            let opened = self
                .device12
                .OpenSharedHandle::<ID3D12Resource>(handle, &mut resource);
            let _ = CloseHandle(handle);
            opened.map_err(|e| e.to_string())?;
            Ok((
                resource.ok_or_else(|| "OpenSharedHandle returned no resource".to_string())?,
                false,
            ))
        }
    }
}
