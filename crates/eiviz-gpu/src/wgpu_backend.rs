//! Explicit wgpu 24 backend. **No CPU fallback.**
//!
//! A hardware adapter is required. Software adapters (`force_fallback_adapter`)
//! and silent CPU substitution are forbidden. Until a GPU blit is implemented,
//! [`WgpuCompositor::composite`] returns [`WgpuError::NotImplemented`].

use crate::RenderPlan;
use eiviz_core::InputId;
use eiviz_media::VideoFrame;
use eiviz_time::MediaTime;
use std::collections::HashMap;
use wgpu::util::DeviceExt;

#[derive(Debug, thiserror::Error)]
pub enum WgpuError {
    #[error("no hardware GPU adapter")]
    NoHardwareAdapter,
    #[error("wgpu request device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("GPU compositor blit is not implemented; refusing CPU substitution")]
    NotImplemented,
}

pub struct WgpuCompositor {
    _instance: wgpu::Instance,
    _adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl WgpuCompositor {
    pub fn new() -> Result<Self, WgpuError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .ok_or(WgpuError::NoHardwareAdapter)?;
        let info = adapter.get_info();
        if matches!(info.device_type, wgpu::DeviceType::Cpu) {
            return Err(WgpuError::NoHardwareAdapter);
        }
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))?;
        Ok(Self {
            _instance: instance,
            _adapter: adapter,
            device,
            queue,
        })
    }

    /// Probe only: allocate a GPU buffer so the adapter is exercised.
    pub fn upload_probe(&self, frame: &VideoFrame) -> Result<u64, WgpuError> {
        let buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("eiviz-probe"),
                contents: &frame.data,
                usage: wgpu::BufferUsages::COPY_SRC,
            });
        self.queue.submit([]);
        Ok(buf.size())
    }

    /// GPU composition is not implemented. Must not call the CPU compositor.
    pub fn composite(
        &self,
        _plan: &RenderPlan,
        _sources: &HashMap<InputId, VideoFrame>,
        _pts: MediaTime,
        _frame_id: u64,
    ) -> Result<VideoFrame, WgpuError> {
        Err(WgpuError::NotImplemented)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_adapter_or_explicit_error() {
        match WgpuCompositor::new() {
            Ok(_) | Err(WgpuError::NoHardwareAdapter) | Err(WgpuError::Device(_)) => {}
            Err(other) => panic!("unexpected {other}"),
        }
    }
}
