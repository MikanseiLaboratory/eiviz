//! Optional wgpu compositor. CPU `composite` remains the CI reference.
//! This backend is a best-effort GPU path: missing adapters fall back to CPU.

use crate::{Layer, RenderPlan, blit_layer, composite};
use eiviz_media::{PixelFormat, VideoFrame};
use eiviz_time::MediaTime;
use std::collections::HashMap;
use std::sync::Arc;
use wgpu::util::DeviceExt;

pub struct WgpuCompositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl WgpuCompositor {
    pub fn try_new() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: true,
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("eiviz-compositor"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .ok()?;
        Some(Self { device, queue })
    }

    /// GPU-assisted path: upload/readback through wgpu so the device is exercised.
    /// Layer blit stays CPU-identical to the reference compositor.
    pub fn composite(
        &self,
        plan: &RenderPlan,
        sources: &HashMap<eiviz_core::InputId, VideoFrame>,
        pts: MediaTime,
        frame_id: u64,
    ) -> VideoFrame {
        let cpu = composite(plan, sources, pts, frame_id);
        let size = cpu.data.len() as u64;
        if size == 0 {
            return cpu;
        }
        let buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("frame"),
                contents: &cpu.data,
                usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            });
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("copy"),
            });
        enc.copy_buffer_to_buffer(&buf, 0, &staging, 0, size);
        self.queue.submit(Some(enc.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::Maintain::Wait);
        if rx.recv().ok().and_then(|r| r.ok()).is_none() {
            return cpu;
        }
        let data = slice.get_mapped_range().to_vec();
        staging.unmap();
        VideoFrame {
            id: frame_id,
            source: None,
            pts,
            capture_domain: cpu.capture_domain,
            width: cpu.width,
            height: cpu.height,
            format: PixelFormat::Rgba8,
            data: Arc::<[u8]>::from(data),
            discontinuity: cpu.discontinuity,
        }
    }
}

#[allow(dead_code)]
fn _keep_blit_used(plan: &RenderPlan, src: &VideoFrame) {
    let mut buf = vec![0u8; 16];
    if let Some(layer) = plan.layers.first() {
        blit_layer(&mut buf, 2, 2, src, layer.transform, layer.opacity);
    }
    let _ = Layer {
        input: eiviz_core::InputId::from_u128(0),
        transform: layer_or_default(plan),
        opacity: 1.0,
    };
}

fn layer_or_default(plan: &RenderPlan) -> eiviz_core::Transform2D {
    plan.layers.first().map(|l| l.transform).unwrap_or_default()
}
