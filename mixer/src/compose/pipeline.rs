use std::borrow::Cow;

use crate::abi::{SRC_BLACK, SRC_BLUE};
use crate::device::GpuDevice;

pub(crate) fn stub_named_fn(src: &str, name: &str, stub: &str) -> String {
    let needle = format!("fn {name}");
    let Some(start) = src.find(&needle) else {
        return src.to_string();
    };
    let rest = &src[start..];
    let Some(brace) = rest.find('{') else {
        return src.to_string();
    };
    let mut depth = 0i32;
    let mut end = None;
    for (i, ch) in rest[brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + brace + i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(end) = end else {
        return src.to_string();
    };
    format!("{}{}{}", &src[..start], stub, &src[end..])
}

pub(crate) fn custom_mix_source(user_wgsl: &str) -> String {
    let body = stub_named_fn(
        user_wgsl,
        "user_compute",
        "fn user_compute(id: vec3<u32>, dim: vec2<u32>) {}",
    );
    format!(
        "{}\n{}\n@fragment\nfn fs_main(in: VsOut) -> @location(0) vec4<f32> {{\n    if params.mix <= 0.001 {{\n        return textureSample(pgm_tex, src_samp, in.uv);\n    }}\n    if params.mix >= 0.999 {{\n        return textureSample(pvw_tex, src_samp, in.uv);\n    }}\n    return user_transition(in.uv, params.mix);\n}}\n",
        CUSTOM_MIX_PREAMBLE, body
    )
}

pub(crate) fn custom_compute_source(user_wgsl: &str) -> String {
    let body = stub_named_fn(
        user_wgsl,
        "user_transition",
        "fn user_transition(uv: vec2<f32>, t: f32) -> vec4<f32> { return vec4<f32>(0.0); }",
    );
    format!(
        "{}\n{}\n@compute @workgroup_size(8, 8)\nfn cs_user(@builtin(global_invocation_id) id: vec3<u32>) {{\n    let dim = vec2<u32>(textureDimensions(aux_out));\n    if id.x >= dim.x || id.y >= dim.y {{ return; }}\n    user_compute(id, dim);\n}}\n",
        USER_COMPUTE_PREAMBLE, body
    )
}

const CUSTOM_MIX_PREAMBLE: &str = r#"
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
struct MixParams {
    mix: f32,
    kind: u32,
    direction: u32,
    softness: f32,
    dip: vec4<f32>,
    param: f32,
    time: f32,
    resolution: vec2<f32>,
}
@group(0) @binding(0) var pgm_tex: texture_2d<f32>;
@group(0) @binding(1) var pvw_tex: texture_2d<f32>;
@group(0) @binding(2) var src_samp: sampler;
@group(0) @binding(3) var<uniform> params: MixParams;
@group(0) @binding(4) var prev_tex: texture_2d<f32>;
@group(0) @binding(5) var src_samp_n: sampler;
@group(0) @binding(6) var flow_tex: texture_2d<f32>;
@group(0) @binding(7) var bloom_tex: texture_2d<f32>;
@group(0) @binding(8) var aux_tex: texture_2d<f32>;
@group(0) @binding(9) var aux2_tex: texture_2d<f32>;
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let pos = positions[index];
    var out: VsOut;
    out.clip = vec4<f32>(pos, 0.0, 1.0);
    out.uv = vec2<f32>(pos.x * 0.5 + 0.5, 1.0 - (pos.y * 0.5 + 0.5));
    return out;
}
"#;

const USER_COMPUTE_PREAMBLE: &str = r#"
struct MixParams {
    mix: f32,
    kind: u32,
    direction: u32,
    softness: f32,
    dip: vec4<f32>,
    param: f32,
    time: f32,
    resolution: vec2<f32>,
}
@group(0) @binding(0) var pgm_tex: texture_2d<f32>;
@group(0) @binding(1) var pvw_tex: texture_2d<f32>;
@group(0) @binding(2) var src_samp: sampler;
@group(0) @binding(3) var<uniform> params: MixParams;
@group(0) @binding(4) var prev_tex: texture_2d<f32>;
@group(0) @binding(5) var src_samp_n: sampler;
@group(0) @binding(6) var flow_tex: texture_2d<f32>;
@group(0) @binding(7) var bloom_tex: texture_2d<f32>;
@group(0) @binding(8) var aux_out: texture_storage_2d<rgba8unorm, write>;
fn user_store(p: vec2<i32>, c: vec4<f32>) {
    textureStore(aux_out, p, c);
}
"#;

pub(crate) fn color_for(id: u64) -> [f32; 4] {
    match id {
        SRC_BLUE => [0.0, 0.0, 1.0, 1.0],
        SRC_BLACK => [0.0, 0.0, 0.0, 1.0],
        _ => [1.0, 0.0, 0.0, 1.0],
    }
}

pub(crate) fn copy_texture(
    encoder: &mut wgpu::CommandEncoder,
    src: &wgpu::Texture,
    dst: &wgpu::Texture,
) {
    let size = src.size();
    if size != dst.size() {
        return;
    }
    encoder.copy_texture_to_texture(
        src.as_image_copy(),
        dst.as_image_copy(),
        wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
    );
}

pub(crate) fn make_texture(
    device: &GpuDevice,
    width: u32,
    height: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    make_texture_format(
        device,
        width,
        height,
        usage,
        wgpu::TextureFormat::Rgba8Unorm,
    )
}

pub(crate) fn make_texture_format(
    device: &GpuDevice,
    width: u32,
    height: u32,
    usage: wgpu::TextureUsages,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("eiviz target"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

pub(crate) fn pipeline(
    device: &GpuDevice,
    label: &str,
    source: &str,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    blend: bool,
) -> Result<wgpu::RenderPipeline, String> {
    let shader = device
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    let pipeline_layout = device
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
    Ok(device
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: blend.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        }))
}

pub(crate) fn compute_pipeline(
    device: &GpuDevice,
    label: &str,
    source: &str,
    layout: &wgpu::BindGroupLayout,
    entry: &str,
) -> Result<wgpu::ComputePipeline, String> {
    let shader = device
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    let pipeline_layout = device
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
    Ok(device
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        }))
}

pub(crate) fn begin_clear<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &'a wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("eiviz clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    })
}

pub(crate) fn write_aligned_texture(
    device: &GpuDevice,
    texture: &wgpu::Texture,
    data: &[u8],
    row_bytes: u32,
    height: u32,
    tex_width: u32,
    format: wgpu::TextureFormat,
    uploader: Option<&mut crate::rebar::FrameUploader>,
) {
    if let Some(uploader) = uploader {
        if uploader
            .upload(device, texture, data, row_bytes, height, tex_width, format)
            .is_ok()
        {
            return;
        }
    }
    let aligned = row_bytes.div_ceil(256) * 256;
    let (bytes, pitch) = if aligned == row_bytes {
        (Cow::Borrowed(data), row_bytes)
    } else {
        let mut padded = vec![0u8; aligned as usize * height as usize];
        let row = row_bytes as usize;
        for y in 0..height as usize {
            let src = y * row;
            let dst = y * aligned as usize;
            if src + row <= data.len() {
                padded[dst..dst + row].copy_from_slice(&data[src..src + row]);
            }
        }
        (Cow::Owned(padded), aligned)
    };
    device.queue.write_texture(
        texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(pitch),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width: tex_width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

pub(crate) fn solid_swatch(
    device: &GpuDevice,
    rgba: [u8; 4],
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = make_texture(
        device,
        8,
        8,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let pixels = vec![rgba; 64];
    let bytes: Vec<u8> = pixels.into_iter().flatten().collect();
    let mut padded = vec![0u8; 256 * 8];
    for y in 0..8 {
        let src = y * 32;
        let dst = y * 256;
        padded[dst..dst + 32].copy_from_slice(&bytes[src..src + 32]);
    }
    device.queue.write_texture(
        texture.as_image_copy(),
        &padded,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(256),
            rows_per_image: Some(8),
        },
        wgpu::Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&Default::default());
    (texture, view)
}

pub(crate) fn begin<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &'a wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("eiviz pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    })
}

pub(crate) fn sampled_compute(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

pub(crate) fn storage_write(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba8Unorm,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

pub(crate) fn sampler_compute(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

pub(crate) fn sampled(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

pub(crate) fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}
