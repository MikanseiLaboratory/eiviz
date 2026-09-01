use std::cell::Cell;
use std::sync::{Arc, Mutex, MutexGuard};

use thiserror::Error;

thread_local! {
    static SURFACE_CONFIGURE: Cell<bool> = const { Cell::new(false) };
    static SURFACE_CONFIGURE_FAILED: Cell<bool> = const { Cell::new(false) };
}

static GPU_QUEUE_LOCK: Mutex<()> = Mutex::new(());

/// Serializes `Queue::submit` against `Surface::configure`.
/// wgpu treats a submit during configure's wait-idle as a validation error.
pub fn lock_gpu_queue() -> MutexGuard<'static, ()> {
    GPU_QUEUE_LOCK.lock().expect("gpu queue lock")
}

pub fn with_surface_configure<R>(f: impl FnOnce() -> R) -> (R, bool) {
    SURFACE_CONFIGURE.set(true);
    SURFACE_CONFIGURE_FAILED.set(false);
    let result = f();
    SURFACE_CONFIGURE.set(false);
    (result, SURFACE_CONFIGURE_FAILED.get())
}

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
            if SURFACE_CONFIGURE.get() {
                SURFACE_CONFIGURE_FAILED.set(true);
                return;
            }
            crate::diag::mark_gpu_fault(&format!("wgpu: {error}"));
        }));

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    pub fn submit(
        &self,
        command_buffers: impl IntoIterator<Item = wgpu::CommandBuffer>,
    ) -> wgpu::SubmissionIndex {
        let _guard = lock_gpu_queue();
        self.queue.submit(command_buffers)
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
