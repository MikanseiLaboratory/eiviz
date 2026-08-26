use std::{collections::HashMap, num::NonZeroIsize};

use raw_window_handle::{RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle};
use wgpu::util::DeviceExt;

use crate::device::GpuDevice;

pub struct Presenter {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    blit: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

#[derive(Default)]
pub struct Presenters {
    by_key: HashMap<(u64, u32, isize), Presenter>,
    monitors: HashMap<u64, Presenter>,
    monitor_sources: HashMap<u64, u64>,
}

impl Presenters {
    pub fn attach(
        &mut self,
        device: &GpuDevice,
        unit_id: u64,
        kind: u32,
        hwnd: isize,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let presenter = create_presenter(device, hwnd, width, height)?;
        self.by_key.insert((unit_id, kind, hwnd), presenter);
        Ok(())
    }

    pub fn resize(
        &mut self,
        device: &GpuDevice,
        unit_id: u64,
        kind: u32,
        hwnd: isize,
        width: u32,
        height: u32,
    ) {
        resize_presenter(device, self.by_key.get_mut(&(unit_id, kind, hwnd)), width, height);
    }

    pub fn detach(&mut self, unit_id: u64, kind: u32, hwnd: isize) {
        self.by_key.remove(&(unit_id, kind, hwnd));
    }

    pub fn detach_unit(&mut self, unit_id: u64) {
        self.by_key.retain(|(id, _, _), _| *id != unit_id);
    }

    pub fn attach_monitor(
        &mut self,
        device: &GpuDevice,
        monitor_id: u64,
        source_id: u64,
        hwnd: isize,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let presenter = create_presenter(device, hwnd, width, height)?;
        self.monitors.insert(monitor_id, presenter);
        self.monitor_sources.insert(monitor_id, source_id);
        Ok(())
    }

    pub fn resize_monitor(&mut self, device: &GpuDevice, monitor_id: u64, width: u32, height: u32) {
        resize_presenter(device, self.monitors.get_mut(&monitor_id), width, height);
    }

    pub fn detach_monitor(&mut self, monitor_id: u64) {
        self.monitors.remove(&monitor_id);
        self.monitor_sources.remove(&monitor_id);
    }

    pub fn set_monitor_source(&mut self, monitor_id: u64, source_id: u64) {
        if self.monitors.contains_key(&monitor_id) {
            self.monitor_sources.insert(monitor_id, source_id);
        }
    }

    pub fn present_texture(
        &self,
        device: &GpuDevice,
        unit_id: u64,
        kind: u32,
        tiles: &[(wgpu::TextureView, [f32; 4])],
    ) -> Result<(), String> {
        for ((id, k, _), presenter) in &self.by_key {
            if *id == unit_id && *k == kind {
                present_tiles(device, presenter, tiles)?;
            }
        }
        Ok(())
    }

    pub fn present_monitors(
        &self,
        device: &GpuDevice,
        source_view: impl Fn(u64) -> Option<wgpu::TextureView>,
    ) -> Result<(), String> {
        for (monitor_id, presenter) in &self.monitors {
            let Some(source_id) = self.monitor_sources.get(monitor_id) else {
                continue;
            };
            let Some(view) = source_view(*source_id) else {
                present_tiles(device, presenter, &[])?;
                continue;
            };
            present_tiles(device, presenter, &[(view, [0.0, 0.0, 1.0, 1.0])])?;
        }
        Ok(())
    }

    pub fn keys_for_unit(&self, unit_id: u64) -> Vec<u32> {
        let mut kinds = Vec::new();
        for (id, kind, _) in self.by_key.keys() {
            if *id == unit_id && !kinds.contains(kind) {
                kinds.push(*kind);
            }
        }
        kinds
    }
}

fn create_presenter(
    device: &GpuDevice,
    hwnd: isize,
    width: u32,
    height: u32,
) -> Result<Presenter, String> {
    let hwnd = NonZeroIsize::new(hwnd).ok_or("HWND cannot be null")?;
    let raw_window_handle = RawWindowHandle::Win32(Win32WindowHandle::new(hwnd));
    // SAFETY: C# owns the HwndHost and detaches the surface before destroying the HWND.
    let surface = unsafe {
        device
            .instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(RawDisplayHandle::Windows(WindowsDisplayHandle::new())),
                raw_window_handle,
            })
            .map_err(|error| error.to_string())?
    };
    let mut config = surface
        .get_default_config(&device.adapter, width.max(2), height.max(2))
        .ok_or("DX12 surface exposes no compatible configuration")?;
    config.present_mode = wgpu::PresentMode::AutoVsync;
    surface.configure(&device.device, &config);
    let (layout, blit, sampler) = present_blit(device, config.format)?;
    Ok(Presenter {
        surface,
        config,
        blit,
        layout,
        sampler,
    })
}

fn resize_presenter(device: &GpuDevice, presenter: Option<&mut Presenter>, width: u32, height: u32) {
    let Some(presenter) = presenter else {
        return;
    };
    let width = width.max(2);
    let height = height.max(2);
    if presenter.config.width == width && presenter.config.height == height {
        return;
    }
    presenter.config.width = width;
    presenter.config.height = height;
    presenter
        .surface
        .configure(&device.device, &presenter.config);
}

fn present_tiles(
    device: &GpuDevice,
    presenter: &Presenter,
    tiles: &[(wgpu::TextureView, [f32; 4])],
) -> Result<(), String> {
    let texture = match presenter.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(texture)
        | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
        result => return Err(format!("could not acquire surface: {result:?}")),
    };
    let dest = texture.texture.create_view(&Default::default());
    let mut encoder = device
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("eiviz present"),
        });
    let binds: Vec<_> = tiles
        .iter()
        .map(|(src, dst)| {
            let params = BlitParams {
                dst: *dst,
                opacity: 1.0,
                pad: [0.0; 3],
            };
            let buffer = device
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("present blit"),
                    contents: bytemuck::bytes_of(&params),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            (
                buffer,
                src.clone(),
            )
        })
        .collect();
    let groups: Vec<_> = binds
        .iter()
        .map(|(buffer, src)| {
            device.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("present bg"),
                layout: &presenter.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&presenter.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: buffer.as_entire_binding(),
                    },
                ],
            })
        })
        .collect();
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("present"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &dest,
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
        pass.set_pipeline(&presenter.blit);
        for group in &groups {
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..6, 0..1);
        }
    }
    device.queue.submit(Some(encoder.finish()));
    device.queue.present(texture);
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlitParams {
    dst: [f32; 4],
    opacity: f32,
    pad: [f32; 3],
}

fn present_blit(
    device: &GpuDevice,
    format: wgpu::TextureFormat,
) -> Result<(wgpu::BindGroupLayout, wgpu::RenderPipeline, wgpu::Sampler), String> {
    let layout = device.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("present blit"),
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
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let shader = device.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("present blit"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/blit.wgsl").into()),
    });
    let pipeline_layout = device.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("present blit"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let blit = device.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("present blit"),
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
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let sampler = device.device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    Ok((layout, blit, sampler))
}
