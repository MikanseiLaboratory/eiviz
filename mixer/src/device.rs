use std::sync::Arc;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DeviceError {
    #[error("no GPU adapter is available")]
    NoAdapter,
    #[error("this platform has no supported GPU backend")]
    UnsupportedPlatform,
    #[error("failed to create the GPU device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),
}

/// Single GPU device shared by all mixing units. Backend is fixed per OS.
pub struct GpuDevice {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GpuDevice {
    pub fn new() -> Result<Self, DeviceError> {
        let backends = Self::backends()?;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .map_err(|_| DeviceError::NoAdapter)?;

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
        device.on_uncaptured_error(Arc::new(|error| {
            eprintln!("eiviz wgpu: {error}");
        }));

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    fn backends() -> Result<wgpu::Backends, DeviceError> {
        #[cfg(windows)]
        {
            Ok(wgpu::Backends::DX12)
        }
        #[cfg(target_os = "macos")]
        {
            Ok(wgpu::Backends::METAL)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            Err(DeviceError::UnsupportedPlatform)
        }
    }
}
