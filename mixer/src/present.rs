use std::collections::HashMap;
use std::time::Duration;

use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::abi::NativeSurface;
use crate::device::{self, GpuDevice};

/// wgpu surface created (and configured) before the render thread sees it.
/// On macOS this must happen on the AppKit main thread.
pub(crate) struct PreparedSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

#[cfg(target_os = "macos")]
#[derive(Clone)]
pub(crate) struct SurfaceGpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
}

pub struct Presenter {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    present: wgpu::RenderPipeline,
    uyvy: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    bind_key: u64,
    bind: Option<wgpu::BindGroup>,
    pending: Option<(u32, u32)>,
    ready: bool,
}

#[derive(Default)]
pub struct Presenters {
    by_key: HashMap<(u64, u32, NativeSurface), Presenter>,
    monitors: HashMap<u64, Presenter>,
    monitor_sources: HashMap<u64, u64>,
    monitor_intervals: HashMap<u64, u32>,
}

impl Presenters {
    pub fn attach(
        &mut self,
        device: &GpuDevice,
        unit_id: u64,
        kind: u32,
        surface: NativeSurface,
        width: u32,
        height: u32,
        prepared: Option<PreparedSurface>,
    ) -> Result<(), String> {
        let presenter = match prepared {
            Some(prepared) => presenter_from_prepared(device, prepared)?,
            None => create_presenter(device, surface, width, height)?,
        };
        self.by_key.insert((unit_id, kind, surface), presenter);
        Ok(())
    }

    pub fn resize(
        &mut self,
        device: &GpuDevice,
        unit_id: u64,
        kind: u32,
        surface: NativeSurface,
        width: u32,
        height: u32,
    ) {
        resize_presenter_for(
            device,
            self.by_key.get_mut(&(unit_id, kind, surface)),
            width,
            height,
        );
    }

    pub fn detach(&mut self, unit_id: u64, kind: u32, surface: NativeSurface) {
        self.by_key.remove(&(unit_id, kind, surface));
    }

    pub fn detach_unit(&mut self, unit_id: u64) {
        self.by_key.retain(|(id, _, _), _| *id != unit_id);
    }

    pub fn attach_monitor(
        &mut self,
        device: &GpuDevice,
        monitor_id: u64,
        source_id: u64,
        surface: NativeSurface,
        width: u32,
        height: u32,
        prepared: Option<PreparedSurface>,
    ) -> Result<(), String> {
        let presenter = match prepared {
            Some(prepared) => presenter_from_prepared(device, prepared)?,
            None => create_presenter(device, surface, width, height)?,
        };
        self.monitors.insert(monitor_id, presenter);
        self.monitor_sources.insert(monitor_id, source_id);
        self.monitor_intervals.entry(monitor_id).or_insert(1);
        Ok(())
    }

    pub fn resize_monitor(&mut self, device: &GpuDevice, monitor_id: u64, width: u32, height: u32) {
        resize_presenter_for(device, self.monitors.get_mut(&monitor_id), width, height);
    }

    pub fn detach_monitor(&mut self, monitor_id: u64) {
        self.monitors.remove(&monitor_id);
        self.monitor_sources.remove(&monitor_id);
        self.monitor_intervals.remove(&monitor_id);
    }

    pub fn set_monitor_source(&mut self, monitor_id: u64, source_id: u64) {
        if self.monitors.contains_key(&monitor_id) {
            self.monitor_sources.insert(monitor_id, source_id);
        }
    }

    pub fn set_monitor_interval(&mut self, monitor_id: u64, frames: u32) {
        self.monitor_intervals
            .insert(monitor_id, frames.clamp(1, 8));
    }

    fn interval_for(&self, monitor_id: u64) -> u32 {
        self.monitor_intervals
            .get(&monitor_id)
            .copied()
            .unwrap_or(1)
            .clamp(1, 8)
    }

    pub fn attached_monitor_sources(&self) -> Vec<u64> {
        self.monitor_sources.values().copied().collect()
    }

    pub fn attached_monitor_sources_due(&self, frame_i: u64) -> Vec<u64> {
        self.monitor_sources
            .iter()
            .filter(|(id, _)| frame_i % u64::from(self.interval_for(**id)) == 0)
            .map(|(_, source)| *source)
            .collect()
    }

    pub fn any_monitor_due(&self, frame_i: u64) -> bool {
        self.monitors
            .keys()
            .any(|id| frame_i % u64::from(self.interval_for(*id)) == 0)
    }

    pub fn present_unit_buses(
        &mut self,
        device: &GpuDevice,
        epoch: u64,
        view_for: impl Fn(u64, u32) -> Option<wgpu::TextureView>,
    ) -> Result<(), String> {
        let planned: Vec<_> = self
            .by_key
            .keys()
            .copied()
            .map(|(unit_id, kind, surface)| {
                let cache_key = unit_id ^ (u64::from(kind) << 48) ^ epoch.rotate_left(8);
                (unit_id, kind, surface, cache_key, view_for(unit_id, kind))
            })
            .collect();
        let mut encoder = device
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("eiviz present"),
            });
        let mut acquired = Vec::new();
        for (unit_id, kind, surface, cache_key, view) in planned {
            let Some(presenter) = self.by_key.get_mut(&(unit_id, kind, surface)) else {
                continue;
            };
            if let Some(item) = draw_presenter(
                device,
                presenter,
                cache_key,
                view.as_ref(),
                false,
                &mut encoder,
            ) {
                acquired.push(item);
            }
        }
        submit_presents(device, encoder, acquired)
    }

    pub fn present_monitors(
        &mut self,
        device: &GpuDevice,
        epoch: u64,
        frame_i: u64,
        source_view: impl Fn(u64) -> Option<(wgpu::TextureView, bool)>,
    ) -> Result<(), String> {
        let due: Vec<u64> = self
            .monitors
            .keys()
            .copied()
            .filter(|id| frame_i % u64::from(self.interval_for(*id)) == 0)
            .collect();
        let planned: Vec<_> = due
            .into_iter()
            .filter_map(|monitor_id| {
                let source_id = *self.monitor_sources.get(&monitor_id)?;
                let (view, packed) = match source_view(source_id) {
                    Some((view, packed)) => (Some(view), packed),
                    None => (None, false),
                };
                Some((monitor_id, source_id ^ epoch.rotate_left(8), view, packed))
            })
            .collect();
        let mut encoder = device
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("eiviz present"),
            });
        let mut acquired = Vec::new();
        for (monitor_id, cache_key, view, packed) in planned {
            let Some(presenter) = self.monitors.get_mut(&monitor_id) else {
                continue;
            };
            if let Some(item) = draw_presenter(
                device,
                presenter,
                cache_key,
                view.as_ref(),
                packed,
                &mut encoder,
            ) {
                acquired.push(item);
            }
        }
        submit_presents(device, encoder, acquired)
    }

    pub fn has_kind(&self, unit_id: u64, kind: u32) -> bool {
        self.by_key
            .keys()
            .any(|(id, k, _)| *id == unit_id && *k == kind)
    }

    pub fn reconfigure_pending(&mut self, device: &GpuDevice) {
        // Apply on the render thread. Surface *creation* already ran on the
        // AppKit main thread. dispatch_sync back to main from here deadlocks
        // when the host (or cargo test) is on main waiting for this thread
        // — attach reply, mixer_destroy join, or a sleeping integration test.
        reconfigure_pending_inner(device, self);
    }
}

fn reconfigure_pending_inner(device: &GpuDevice, presenters: &mut Presenters) {
    for presenter in presenters.by_key.values_mut() {
        apply_pending_size(device, presenter);
    }
    for presenter in presenters.monitors.values_mut() {
        apply_pending_size(device, presenter);
    }
}

fn raw_handles(surface: NativeSurface) -> Result<(RawDisplayHandle, RawWindowHandle), String> {
    match surface.kind {
        #[cfg(windows)]
        crate::abi::NATIVE_WIN32_HWND => {
            use std::num::NonZeroIsize;

            use raw_window_handle::{Win32WindowHandle, WindowsDisplayHandle};

            let hwnd = NonZeroIsize::new(surface.handle).ok_or("HWND cannot be null")?;
            Ok((
                RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
                RawWindowHandle::Win32(Win32WindowHandle::new(hwnd)),
            ))
        }
        #[cfg(target_os = "macos")]
        crate::abi::NATIVE_APPKIT_NSVIEW => {
            use std::ptr::NonNull;

            use raw_window_handle::{AppKitDisplayHandle, AppKitWindowHandle};

            let ns_view = NonNull::new(surface.handle as *mut core::ffi::c_void)
                .ok_or("NSView cannot be null")?;
            Ok((
                RawDisplayHandle::AppKit(AppKitDisplayHandle::new()),
                RawWindowHandle::AppKit(AppKitWindowHandle::new(ns_view)),
            ))
        }
        _ => Err("native surface is not supported on this OS".into()),
    }
}

fn create_presenter(
    device: &GpuDevice,
    surface: NativeSurface,
    width: u32,
    height: u32,
) -> Result<Presenter, String> {
    let prepared = prepare_surface(
        &device.instance,
        &device.adapter,
        &device.device,
        surface,
        width,
        height,
    )?;
    presenter_from_prepared(device, prepared)
}

pub(crate) fn prepare_surface(
    instance: &wgpu::Instance,
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    native: NativeSurface,
    width: u32,
    height: u32,
) -> Result<PreparedSurface, String> {
    let (raw_display_handle, raw_window_handle) = raw_handles(native)?;
    // SAFETY: the host owns the HWND / NSView and detaches the surface before destroying it.
    let surface = unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(raw_display_handle),
                raw_window_handle,
            })
            .map_err(|error| error.to_string())?
    };
    let mut config = surface
        .get_default_config(adapter, width.max(2), height.max(2))
        .ok_or("surface exposes no compatible configuration")?;
    let caps = surface.get_capabilities(adapter);
    config.format = pick_surface_format(&caps.formats);
    config.alpha_mode = pick_alpha_mode(&caps.alpha_modes);
    config.present_mode = pick_present_mode(&caps.present_modes);
    configure_surface(device, &surface, &config)?;
    Ok(PreparedSurface { surface, config })
}

fn presenter_from_prepared(
    device: &GpuDevice,
    prepared: PreparedSurface,
) -> Result<Presenter, String> {
    let (layout, present, uyvy, sampler) = present_pipelines(device, prepared.config.format)?;
    let params = device.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("present params"),
        size: 256,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    Ok(Presenter {
        surface: prepared.surface,
        config: prepared.config,
        present,
        uyvy,
        layout,
        sampler,
        params,
        bind_key: 0,
        bind: None,
        pending: None,
        ready: true,
    })
}

fn resize_presenter_for(
    _device: &GpuDevice,
    presenter: Option<&mut Presenter>,
    width: u32,
    height: u32,
) {
    let Some(presenter) = presenter else {
        return;
    };
    presenter.pending = Some((width.max(2), height.max(2)));
}

fn apply_pending_size(device: &GpuDevice, presenter: &mut Presenter) {
    let Some((width, height)) = presenter.pending else {
        return;
    };
    if presenter.ready && presenter.config.width == width && presenter.config.height == height {
        presenter.pending = None;
        return;
    }
    let mut config = presenter.config.clone();
    config.width = width;
    config.height = height;
    if configure_surface(&device.device, &presenter.surface, &config).is_ok() {
        presenter.config = config;
        presenter.bind = None;
        presenter.bind_key = 0;
        presenter.pending = None;
        presenter.ready = true;
    }
}

fn wait_gpu_idle(device: &wgpu::Device) {
    for _ in 0..8 {
        match device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_millis(50)),
        }) {
            Ok(wgpu::PollStatus::QueueEmpty) => return,
            Ok(_) => continue,
            Err(_) => return,
        }
    }
}

fn configure_surface(
    device: &wgpu::Device,
    surface: &wgpu::Surface,
    config: &wgpu::SurfaceConfiguration,
) -> Result<(), String> {
    for _ in 0..8 {
        {
            let _guard = device::lock_gpu_queue();
            wait_gpu_idle(device);
            let (_, failed) = device::with_surface_configure(|| {
                surface.configure(device, config);
            });
            if !failed {
                return Ok(());
            }
        }
        std::thread::yield_now();
    }
    Err("surface configure failed".into())
}

fn pick_present_mode(modes: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    const PREFERRED: [wgpu::PresentMode; 4] = [
        wgpu::PresentMode::Mailbox,
        wgpu::PresentMode::Immediate,
        wgpu::PresentMode::AutoNoVsync,
        wgpu::PresentMode::FifoRelaxed,
    ];
    PREFERRED
        .into_iter()
        .find(|mode| modes.contains(mode))
        .unwrap_or(wgpu::PresentMode::Fifo)
}

fn pick_surface_format(formats: &[wgpu::TextureFormat]) -> wgpu::TextureFormat {
    const PREFERRED: [wgpu::TextureFormat; 2] = [
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
    ];
    PREFERRED
        .into_iter()
        .find(|format| formats.contains(format))
        .or_else(|| formats.first().copied())
        .unwrap_or(wgpu::TextureFormat::Bgra8Unorm)
}

fn pick_alpha_mode(modes: &[wgpu::CompositeAlphaMode]) -> wgpu::CompositeAlphaMode {
    const PREFERRED: [wgpu::CompositeAlphaMode; 2] = [
        wgpu::CompositeAlphaMode::Opaque,
        wgpu::CompositeAlphaMode::Auto,
    ];
    PREFERRED
        .into_iter()
        .find(|mode| modes.contains(mode))
        .unwrap_or(wgpu::CompositeAlphaMode::Auto)
}

fn draw_presenter(
    device: &GpuDevice,
    presenter: &mut Presenter,
    cache_key: u64,
    src: Option<&wgpu::TextureView>,
    packed: bool,
    encoder: &mut wgpu::CommandEncoder,
) -> Option<(wgpu::SurfaceTexture, wgpu::TextureView)> {
    if !presenter.ready {
        return None;
    }
    let texture = match presenter.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(texture)
        | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
        wgpu::CurrentSurfaceTexture::Outdated
        | wgpu::CurrentSurfaceTexture::Lost
        | wgpu::CurrentSurfaceTexture::Validation => {
            presenter.ready = false;
            presenter.pending = Some((presenter.config.width, presenter.config.height));
            return None;
        }
        _ => return None,
    };
    let dest = texture.texture.create_view(&Default::default());
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
        if let Some(src) = src {
            if presenter.bind_key != cache_key || presenter.bind.is_none() {
                let params = BlitParams {
                    dst: [0.0, 0.0, 1.0, 1.0],
                    opacity: 1.0,
                    pad: [0.0; 3],
                };
                device
                    .queue
                    .write_buffer(&presenter.params, 0, bytemuck::bytes_of(&params));
                presenter.bind =
                    Some(device.device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                                resource: presenter.params.as_entire_binding(),
                            },
                        ],
                    }));
                presenter.bind_key = cache_key;
            }
            pass.set_pipeline(if packed {
                &presenter.uyvy
            } else {
                &presenter.present
            });
            pass.set_bind_group(0, presenter.bind.as_ref().expect("present bind"), &[]);
            pass.draw(0..6, 0..1);
        }
    }
    Some((texture, dest))
}

fn submit_presents(
    device: &GpuDevice,
    encoder: wgpu::CommandEncoder,
    acquired: Vec<(wgpu::SurfaceTexture, wgpu::TextureView)>,
) -> Result<(), String> {
    if acquired.is_empty() {
        return Ok(());
    }
    device.submit(Some(encoder.finish()));
    for (texture, dest) in acquired {
        drop(dest);
        device.queue.present(texture);
    }
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlitParams {
    dst: [f32; 4],
    opacity: f32,
    pad: [f32; 3],
}

fn present_pipelines(
    device: &GpuDevice,
    format: wgpu::TextureFormat,
) -> Result<
    (
        wgpu::BindGroupLayout,
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
        wgpu::Sampler,
    ),
    String,
> {
    let layout = device
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present"),
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
    let pipeline_layout = device
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
    let present = make_present_pipeline(
        device,
        &pipeline_layout,
        format,
        include_str!("../shaders/present.wgsl"),
        "present",
    );
    let uyvy = make_present_pipeline(
        device,
        &pipeline_layout,
        format,
        include_str!("../shaders/uyvy_to_rgba.wgsl"),
        "present uyvy",
    );
    let sampler = device.device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    Ok((layout, present, uyvy, sampler))
}

fn make_present_pipeline(
    device: &GpuDevice,
    pipeline_layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    source: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    let shader = device
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    device
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(pipeline_layout),
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
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
}
