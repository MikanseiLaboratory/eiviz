use std::cell::Cell;
use std::path::{Path, PathBuf};
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
    #[cfg(not(any(windows, target_os = "macos")))]
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
        let instance = wgpu::Instance::new(instance_descriptor(backends));
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

fn instance_descriptor(backends: wgpu::Backends) -> wgpu::InstanceDescriptor {
    wgpu::InstanceDescriptor {
        backends,
        backend_options: wgpu::BackendOptions {
            dx12: wgpu::Dx12BackendOptions {
                shader_compiler: dx12_shader_compiler(),
                ..Default::default()
            },
            ..Default::default()
        },
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    }
}

/// wgpu's default `Dx12Compiler::Auto` LoadLibrary-searches `dxcompiler.dll` on PATH.
/// Other apps ship incompatible copies (CS Demo Manager is one); loading those can
/// crash with STATUS_ILLEGAL_INSTRUCTION. Only use a sidecar next to the host, else FXC.
fn dx12_shader_compiler() -> wgpu::Dx12Compiler {
    let compiler = dx12_shader_compiler_from(&native_sidecar_dirs());
    crate::diag::info(&format!("dx12 shader compiler: {compiler:?}"));
    compiler
}

fn native_sidecar_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf());
        }
    }
    dirs
}

fn dx12_shader_compiler_from(dirs: &[PathBuf]) -> wgpu::Dx12Compiler {
    if let Some(path) = bundled_dxc_path(dirs) {
        return wgpu::Dx12Compiler::DynamicDxc { dxc_path: path };
    }
    wgpu::Dx12Compiler::Fxc
}

fn bundled_dxc_path(dirs: &[PathBuf]) -> Option<String> {
    dirs.iter().find_map(|dir| {
        let path = dir.join("dxcompiler.dll");
        path.is_file()
            .then(|| path_for_loadlibrary(&path))
            .flatten()
    })
}

fn path_for_loadlibrary(path: &Path) -> Option<String> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !path.is_absolute() {
        return None;
    }
    Some(path.to_string_lossy().into_owned())
}

fn is_surface_local_error(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("surface is not configured")
        || text.contains("surface does not exist")
        || (text.contains("surface") && (text.contains("outdated") || text.contains("lost")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn dx12_compiler_uses_fxc_without_sidecar() {
        let dir = scratch_dir("dxc-test-empty");
        match dx12_shader_compiler_from(&[dir]) {
            wgpu::Dx12Compiler::Fxc => {}
            other => panic!("expected Fxc, got {other:?}"),
        }
    }

    #[test]
    fn dx12_compiler_uses_absolute_sidecar_not_path_basename() {
        let dir = scratch_dir("dxc-test-sidecar");
        let dll = dir.join("dxcompiler.dll");
        fs::write(&dll, []).expect("sidecar");
        match dx12_shader_compiler_from(&[dir]) {
            wgpu::Dx12Compiler::DynamicDxc { dxc_path } => {
                assert!(
                    Path::new(&dxc_path).is_absolute(),
                    "LoadLibrary basename would search PATH: {dxc_path}"
                );
                assert!(
                    dxc_path.replace('\\', "/").ends_with("/dxcompiler.dll"),
                    "{dxc_path}"
                );
            }
            other => panic!("expected DynamicDxc, got {other:?}"),
        }
    }
}
