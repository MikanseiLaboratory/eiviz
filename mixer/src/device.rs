use std::cell::Cell;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use thiserror::Error;

thread_local! {
    static SURFACE_CONFIGURE: Cell<bool> = const { Cell::new(false) };
    static SURFACE_CONFIGURE_FAILED: Cell<bool> = const { Cell::new(false) };
}

static GPU_QUEUE_LOCK: OnceLock<Arc<Mutex<()>>> = OnceLock::new();

fn gpu_queue_lock_arc() -> &'static Arc<Mutex<()>> {
    GPU_QUEUE_LOCK.get_or_init(|| Arc::new(Mutex::new(())))
}

/// Same mutex as [`lock_gpu_queue`], for OMT decode/encode on other threads.
pub fn gpu_queue_lock_handle() -> Arc<Mutex<()>> {
    Arc::clone(gpu_queue_lock_arc())
}

/// Serializes `Queue::submit` against `Surface::configure`.
/// wgpu treats a submit during configure's wait-idle as a validation error.
pub fn lock_gpu_queue() -> MutexGuard<'static, ()> {
    gpu_queue_lock_arc().lock().expect("gpu queue lock")
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
            let text = format!("{error}");
            if is_surface_local_error(&text) {
                crate::diag::error(&format!("wgpu surface: {text}"));
                return;
            }
            crate::diag::mark_fatal(format!("GPU device error: {text}"));
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

fn is_surface_local_error(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("surface is not configured")
        || text.contains("surface does not exist")
        || (text.contains("surface") && (text.contains("outdated") || text.contains("lost")))
}
