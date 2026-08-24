//! Explicit wgpu 24 compositor. **No CPU fallback.**
//!
//! A hardware adapter is required. Every layer is sampled and alpha-composited
//! by a WGSL render pipeline. Readback is an explicit staging operation for the
//! current CPU-frame sink contract; it never calls the CPU reference compositor.

use crate::{Layer, RenderPlan};
use eiviz_core::{InputId, Transform2D};
use eiviz_media::{PixelFormat, VideoFrame};
use eiviz_time::MediaTime;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use wgpu::util::DeviceExt;

const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SHADER: &str = r#"
struct LayerUniform {
    rect: vec4<f32>,
    crop: vec4<f32>,
    rotation_opacity: vec4<f32>,
};

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> layer: LayerUniform;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var points = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    let local = points[vertex_index];
    let centered = (local - vec2<f32>(0.5, 0.5)) * layer.rect.zw;
    let angle = layer.rotation_opacity.x;
    let rotated = vec2<f32>(
        centered.x * cos(angle) - centered.y * sin(angle),
        centered.x * sin(angle) + centered.y * cos(angle),
    );
    let canvas = layer.rect.xy + layer.rect.zw * 0.5 + rotated;
    let ndc = vec2<f32>(canvas.x * 2.0 - 1.0, 1.0 - canvas.y * 2.0);
    let available = vec2<f32>(
        1.0 - layer.crop.x - layer.crop.z,
        1.0 - layer.crop.y - layer.crop.w,
    );

    var output: VertexOutput;
    output.position = vec4<f32>(ndc, 0.0, 1.0);
    output.uv = layer.crop.xy + local * available;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    var color = textureSample(source_texture, source_sampler, input.uv);
    color.a *= layer.rotation_opacity.y;
    return color;
}
"#;

#[derive(Debug, thiserror::Error)]
pub enum WgpuError {
    #[error("no hardware GPU adapter")]
    NoHardwareAdapter,
    #[error("wgpu request device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("wgpu validation: {0}")]
    Validation(String),
    #[error("invalid render plan: {0}")]
    InvalidPlan(String),
    #[error("missing source frame for input {0}")]
    MissingSource(InputId),
    #[error("unsupported source format for input {input}: {format:?}")]
    UnsupportedFormat { input: InputId, format: PixelFormat },
    #[error("wgpu map readback failed: {0}")]
    Map(String),
}

pub struct WgpuCompositor {
    _instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
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
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("eiviz-wgpu-compositor"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))?;
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("eiviz-layer-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("eiviz-layer-shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("eiviz-compositor-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("eiviz-compositor-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OUTPUT_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("eiviz-nearest-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let _ = device.poll(wgpu::Maintain::Wait);
        if let Some(error) = pollster::block_on(device.pop_error_scope()) {
            return Err(WgpuError::Validation(error.to_string()));
        }
        Ok(Self {
            _instance: instance,
            adapter,
            device,
            queue,
            bind_group_layout,
            pipeline,
            sampler,
        })
    }

    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.adapter.get_info()
    }

    /// Composite all layers on the selected hardware GPU, then explicitly read
    /// the result back for the current `VideoFrame` sink contract.
    pub fn composite(
        &self,
        plan: &RenderPlan,
        sources: &HashMap<InputId, VideoFrame>,
        pts: MediaTime,
        frame_id: u64,
    ) -> Result<VideoFrame, WgpuError> {
        validate_plan(plan)?;
        let max_dimension = self.device.limits().max_texture_dimension_2d;
        if plan.width > max_dimension || plan.height > max_dimension {
            return Err(WgpuError::InvalidPlan(format!(
                "output {}x{} exceeds GPU max dimension {max_dimension}",
                plan.width, plan.height
            )));
        }
        self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let layers = match self.prepare_layers(plan, sources) {
            Ok(layers) => layers,
            Err(error) => {
                let _ = self.device.poll(wgpu::Maintain::Wait);
                let _ = pollster::block_on(self.device.pop_error_scope());
                return Err(error);
            }
        };
        let output = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("eiviz-compositor-output"),
            size: wgpu::Extent3d {
                width: plan.width,
                height: plan.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OUTPUT_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let output_view = output.create_view(&wgpu::TextureViewDescriptor::default());
        let unpadded_bytes_per_row = plan.width * 4;
        let padded_bytes_per_row = unpadded_bytes_per_row
            .div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback_size = u64::from(padded_bytes_per_row) * u64::from(plan.height);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("eiviz-compositor-readback"),
            size: readback_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("eiviz-compositor-encoder"),
            });
        {
            let color_attachment = Some(wgpu::RenderPassColorAttachment {
                view: &output_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("eiviz-compositor-pass"),
                color_attachments: &[color_attachment],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            for layer in &layers {
                pass.set_bind_group(0, &layer.bind_group, &[]);
                pass.draw(0..6, 0..1);
            }
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &output,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(plan.height),
                },
            },
            wgpu::Extent3d {
                width: plan.width,
                height: plan.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        let _ = self.device.poll(wgpu::Maintain::Wait);
        let map_result = receiver
            .recv()
            .map_err(|error| WgpuError::Map(error.to_string()))?
            .map_err(|error| WgpuError::Map(error.to_string()));
        let validation_error = pollster::block_on(self.device.pop_error_scope());
        map_result?;
        if let Some(error) = validation_error {
            return Err(WgpuError::Validation(error.to_string()));
        }
        let mapped = slice.get_mapped_range();
        let mut rgba = Vec::with_capacity(unpadded_bytes_per_row as usize * plan.height as usize);
        for row in mapped.chunks_exact(padded_bytes_per_row as usize) {
            rgba.extend_from_slice(&row[..unpadded_bytes_per_row as usize]);
        }
        drop(mapped);
        readback.unmap();
        Ok(VideoFrame {
            id: frame_id,
            source: None,
            pts,
            capture_domain: eiviz_time::ClockDomain::Virtual,
            width: plan.width,
            height: plan.height,
            format: PixelFormat::Rgba8,
            data: Arc::from(rgba),
            discontinuity: false,
        })
    }

    /// GPU transition. Both frames are uploaded and blended by the same WGSL
    /// pipeline; the CPU reference mixer is never called.
    pub fn mix(
        &self,
        a: &VideoFrame,
        b: &VideoFrame,
        factor: f32,
        pts: MediaTime,
        frame_id: u64,
    ) -> Result<VideoFrame, WgpuError> {
        let a_id = InputId::from_u128(1);
        let b_id = InputId::from_u128(2);
        let sources = HashMap::from([(a_id, a.clone()), (b_id, b.clone())]);
        let plan = RenderPlan {
            width: a.width,
            height: a.height,
            layers: vec![
                Layer {
                    input: a_id,
                    transform: Transform2D::fullscreen(),
                    opacity: 1.0,
                },
                Layer {
                    input: b_id,
                    transform: Transform2D::fullscreen(),
                    opacity: factor.clamp(0.0, 1.0),
                },
            ],
        };
        self.composite(&plan, &sources, pts, frame_id)
    }

    fn prepare_layers(
        &self,
        plan: &RenderPlan,
        sources: &HashMap<InputId, VideoFrame>,
    ) -> Result<Vec<PreparedLayer>, WgpuError> {
        let mut prepared = Vec::with_capacity(plan.layers.len());
        for layer in &plan.layers {
            let source = sources
                .get(&layer.input)
                .ok_or(WgpuError::MissingSource(layer.input))?;
            if source.format != PixelFormat::Rgba8 {
                return Err(WgpuError::UnsupportedFormat {
                    input: layer.input,
                    format: source.format,
                });
            }
            validate_source(source, layer.input)?;
            let max_dimension = self.device.limits().max_texture_dimension_2d;
            if source.width > max_dimension || source.height > max_dimension {
                return Err(WgpuError::InvalidPlan(format!(
                    "source {} {}x{} exceeds GPU max dimension {max_dimension}",
                    layer.input, source.width, source.height
                )));
            }
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("eiviz-layer-source"),
                size: wgpu::Extent3d {
                    width: source.width,
                    height: source.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: OUTPUT_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &source.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(source.width * 4),
                    rows_per_image: Some(source.height),
                },
                wgpu::Extent3d {
                    width: source.width,
                    height: source.height,
                    depth_or_array_layers: 1,
                },
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let uniform_data = layer_uniform_bytes(plan, layer)?;
            let uniform = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("eiviz-layer-uniform"),
                    contents: &uniform_data,
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("eiviz-layer-bind-group"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: uniform.as_entire_binding(),
                    },
                ],
            });
            prepared.push(PreparedLayer { bind_group });
        }
        Ok(prepared)
    }
}

struct PreparedLayer {
    bind_group: wgpu::BindGroup,
}

fn validate_plan(plan: &RenderPlan) -> Result<(), WgpuError> {
    if plan.width == 0 || plan.height == 0 {
        return Err(WgpuError::InvalidPlan("zero-sized output".into()));
    }
    if plan.width > u32::MAX / 4 {
        return Err(WgpuError::InvalidPlan(
            "output row byte count overflows u32".into(),
        ));
    }
    let bytes = u64::from(plan.width)
        .checked_mul(u64::from(plan.height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| WgpuError::InvalidPlan("output size overflow".into()))?;
    if bytes > usize::MAX as u64 {
        return Err(WgpuError::InvalidPlan(
            "output exceeds addressable memory".into(),
        ));
    }
    Ok(())
}

fn validate_source(source: &VideoFrame, input: InputId) -> Result<(), WgpuError> {
    if source.width == 0 || source.height == 0 {
        return Err(WgpuError::InvalidPlan(format!(
            "source {input} is zero-sized"
        )));
    }
    if source.width > u32::MAX / 4 {
        return Err(WgpuError::InvalidPlan(format!(
            "source {input} row byte count overflows u32"
        )));
    }
    let required = u64::from(source.width)
        .checked_mul(u64::from(source.height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| WgpuError::InvalidPlan(format!("source {input} size overflow")))?;
    if source.data.len() as u64 != required {
        return Err(WgpuError::InvalidPlan(format!(
            "source {input} data length {}, expected {required}",
            source.data.len()
        )));
    }
    Ok(())
}

fn layer_uniform_bytes(plan: &RenderPlan, layer: &Layer) -> Result<Vec<u8>, WgpuError> {
    let transform = layer.transform;
    let (x, y, width, height) = if transform.pixel_space {
        (
            transform.x / plan.width as f32,
            transform.y / plan.height as f32,
            transform.width / plan.width as f32,
            transform.height / plan.height as f32,
        )
    } else {
        (transform.x, transform.y, transform.width, transform.height)
    };
    let crop = transform.crop;
    let crop_values = [
        crop.left.clamp(0.0, 1.0),
        crop.top.clamp(0.0, 1.0),
        crop.right.clamp(0.0, 1.0),
        crop.bottom.clamp(0.0, 1.0),
    ];
    if crop_values[0] + crop_values[2] >= 1.0 || crop_values[1] + crop_values[3] >= 1.0 {
        return Err(WgpuError::InvalidPlan(
            "crop removes the entire source".into(),
        ));
    }
    let values = [
        x,
        y,
        width,
        height,
        crop_values[0],
        crop_values[1],
        crop_values[2],
        crop_values[3],
        transform.rotation_deg.to_radians(),
        layer.opacity.clamp(0.0, 1.0),
        0.0,
        0.0,
    ];
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wgsl_parses_and_validates_without_an_adapter() {
        let module = naga::front::wgsl::parse_str(SHADER).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn hardware_adapter_or_explicit_error() {
        match WgpuCompositor::new() {
            Ok(_) | Err(WgpuError::NoHardwareAdapter) | Err(WgpuError::Device(_)) => {}
            Err(other) => panic!("unexpected {other}"),
        }
    }

    #[test]
    fn uniform_converts_pixel_space_and_crop() {
        let plan = RenderPlan {
            width: 1920,
            height: 1080,
            layers: vec![],
        };
        let layer = Layer {
            input: InputId::from_u128(1),
            transform: Transform2D {
                x: 192.0,
                y: 108.0,
                width: 960.0,
                height: 540.0,
                crop: eiviz_core::Crop {
                    left: 0.1,
                    top: 0.2,
                    right: 0.3,
                    bottom: 0.1,
                },
                pixel_space: true,
                ..Transform2D::default()
            },
            opacity: 0.5,
        };
        let bytes = layer_uniform_bytes(&plan, &layer).unwrap();
        let values = bytes
            .chunks_exact(4)
            .map(|bytes| f32::from_ne_bytes(bytes.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(&values[..4], &[0.1, 0.1, 0.5, 0.5]);
        assert_eq!(&values[4..8], &[0.1, 0.2, 0.3, 0.1]);
        assert_eq!(values[9], 0.5);
    }
}
