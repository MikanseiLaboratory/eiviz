use windows::Win32::Graphics::Direct3D12::ID3D12Resource;

use crate::upload::GpuVideoFrame;

pub struct Nv12Converter {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

impl Nv12Converter {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nv12 convert"),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nv12 convert"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/nv12_to_rgba.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nv12 convert"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nv12 convert"),
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
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            layout,
            pipeline,
            sampler,
        }
    }

    pub fn convert_nv12(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resource: ID3D12Resource,
        width: u32,
        height: u32,
        pts: i64,
    ) -> Result<GpuVideoFrame, String> {
        let y_src = import_plane(
            device,
            resource.clone(),
            wgpu::TextureFormat::R8Unorm,
            width,
            height,
            0,
        )?;
        let uv_src = import_plane(
            device,
            resource,
            wgpu::TextureFormat::Rg8Unorm,
            width / 2,
            height / 2,
            1,
        )?;
        let y = owned_plane(device, wgpu::TextureFormat::R8Unorm, width, height);
        let uv = owned_plane(device, wgpu::TextureFormat::Rg8Unorm, width / 2, height / 2);
        let dest = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("eiviz gpu rgba"),
            size: wgpu::Extent3d {
                width: width.max(2),
                height: height.max(2),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let dest_view = dest.create_view(&Default::default());
        let y_view = y.create_view(&Default::default());
        let uv_view = uv.create_view(&Default::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nv12 convert bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&y_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&uv_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nv12 convert"),
        });
        encoder.copy_texture_to_texture(
            y_src.as_image_copy(),
            y.as_image_copy(),
            wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
        );
        encoder.copy_texture_to_texture(
            uv_src.as_image_copy(),
            uv.as_image_copy(),
            wgpu::Extent3d {
                width: (width / 2).max(1),
                height: (height / 2).max(1),
                depth_or_array_layers: 1,
            },
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nv12 convert"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &dest_view,
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
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..6, 0..1);
        }
        let index = {
            let _guard = crate::device::lock_gpu_queue();
            queue.submit(Some(encoder.finish()))
        };
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: None,
        });
        drop(y_src);
        drop(uv_src);
        drop(y);
        drop(uv);
        Ok(GpuVideoFrame {
            pts,
            width,
            height,
            texture: dest,
            view: dest_view,
        })
    }

    pub fn copy_bgra(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resource: ID3D12Resource,
        width: u32,
        height: u32,
        pts: i64,
        rgba: bool,
    ) -> Result<GpuVideoFrame, String> {
        let format = if rgba {
            wgpu::TextureFormat::Rgba8Unorm
        } else {
            wgpu::TextureFormat::Bgra8Unorm
        };
        let src = import_plane(device, resource, format, width, height, 0)?;
        let dest = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("eiviz gpu bgra"),
            size: wgpu::Extent3d {
                width: width.max(2),
                height: height.max(2),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bgra gpu copy"),
        });
        encoder.copy_texture_to_texture(
            src.as_image_copy(),
            dest.as_image_copy(),
            wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
        );
        let index = {
            let _guard = crate::device::lock_gpu_queue();
            queue.submit(Some(encoder.finish()))
        };
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: None,
        });
        drop(src);
        let view = dest.create_view(&Default::default());
        Ok(GpuVideoFrame {
            pts,
            width,
            height,
            texture: dest,
            view,
        })
    }
}

fn import_plane(
    device: &wgpu::Device,
    resource: ID3D12Resource,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    plane: u32,
) -> Result<wgpu::Texture, String> {
    let extent = wgpu::Extent3d {
        width: width.max(1),
        height: height.max(1),
        depth_or_array_layers: 1,
    };
    let mut hal = unsafe {
        wgpu::hal::dx12::Device::texture_from_raw(
            resource,
            format,
            wgpu::TextureDimension::D2,
            extent,
            1,
            1,
        )
    };
    if plane > 0 || format == wgpu::TextureFormat::R8Unorm || format == wgpu::TextureFormat::Rg8Unorm
    {
        hal = hal.with_plane_slice(plane);
    }
    Ok(unsafe {
        device.create_texture_from_hal::<wgpu::hal::api::Dx12>(
            hal,
            &wgpu::TextureDescriptor {
                label: Some("eiviz imported plane"),
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
    })
}

fn owned_plane(device: &wgpu::Device, format: wgpu::TextureFormat, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("eiviz nv12 plane"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}
