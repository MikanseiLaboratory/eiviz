//! GPU compositing for replay clip transitions.
//!
//! The CPU path blended the two decoded frames in memory (`combine` in the parent
//! module) and uploaded the result. This pass takes both frames as textures and writes
//! the combined picture, so nothing is blended on the CPU once the frames are up. It is
//! also the shape the decode needs to reach: when the decoder writes texture frames, the
//! transition runs on them without a copy at all.

use crate::replay::channel::{TransitionDirection, TransitionKind};

/// One transition draw. `outgoing` is the frame that is leaving, `incoming` the one
/// arriving; `mix` runs 0 to 1 across the transition.
pub struct TransitionPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
}

impl TransitionPass {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self::with_shader(
            device,
            format,
            include_str!("../../shaders/replay_transition.wgsl"),
            "replay-transition",
        )
    }

    /// The same transition over packed UYVY frames, the layout the replay decoder
    /// produces. One texel carries two pixels; both frames use the same layout, so
    /// blending texel against texel is the same picture as blending pixel against
    /// pixel.
    pub fn new_packed(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self::with_shader(
            device,
            format,
            include_str!("../../shaders/replay_transition_packed.wgsl"),
            "replay-transition-packed",
        )
    }

    fn with_shader(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        source: &str,
        label: &str,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("replay-transition-layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("replay-transition-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("replay-transition-pipeline"),
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
            label: Some("replay-transition-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("replay-transition-params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            sampler,
            uniforms,
            bind_group: None,
        }
    }


    /// Draws `outgoing` and `incoming` into `target` with the given transition.
    ///
    /// The bind group is rebuilt when the textures change, which is once per
    /// transition rather than once per frame: a dissolve holds the same two textures
    /// for its whole length.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        outgoing: &wgpu::TextureView,
        incoming: &wgpu::TextureView,
        target: &wgpu::TextureView,
        kind: TransitionKind,
        direction: TransitionDirection,
        mix: f32,
    ) {
        // Rebuilt each draw: a bind group is cheap and a pointer-keyed cache would be
        // wrong the moment a texture is recreated at the same address.
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("replay-transition-bind"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(outgoing),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(incoming),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.uniforms.as_entire_binding(),
                },
            ],
        }));
        let kind_code = match kind {
            TransitionKind::Cut => 0u32,
            TransitionKind::Dissolve => 1,
            TransitionKind::Dip => 2,
            TransitionKind::Wipe => 3,
            TransitionKind::Slide => 4,
            TransitionKind::Push => 5,
        };
        let direction_code = match direction {
            TransitionDirection::Left => 0u32,
            TransitionDirection::Right => 1,
            TransitionDirection::Up => 2,
            TransitionDirection::Down => 3,
        };
        let params = [kind_code, direction_code, mix.to_bits(), 0];
        queue.write_buffer(&self.uniforms, 0, as_bytes(&params));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("replay-transition"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        if let Some(group) = self.bind_group.as_ref() {
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// The uniform block is four 32-bit words; the shader reads the third as an f32, so the
/// bits travel as u32 to keep the buffer object-free.
fn as_bytes(words: &[u32; 4]) -> &[u8] {
    // Safety: a [u32; 4] is 16 bytes with no padding, and u8 has alignment 1.
    unsafe { std::slice::from_raw_parts(words.as_ptr() as *const u8, 16) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{BackendRequest, GpuDevice};

    const W: u32 = 8;
    const H: u32 = 8;
    /// Texture copies need rows padded to the device's copy alignment.
    const ROW: u32 = 256;

    fn solid(device: &GpuDevice, queue: &wgpu::Queue, colour: [u8; 4]) -> wgpu::Texture {
        let texture = device.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("replay-transition-test-source"),
            size: wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut pixels = vec![0u8; (ROW * H) as usize];
        for row in 0..H {
            for column in 0..W {
                let at = (row * ROW + column * 4) as usize;
                pixels[at..at + 4].copy_from_slice(&colour);
            }
        }
        queue.write_texture(
            texture.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ROW),
                rows_per_image: Some(H),
            },
            wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
        );
        texture
    }

    /// Runs one transition and returns the middle row's pixels.
    fn run(kind: TransitionKind, direction: TransitionDirection, mix: f32) -> Vec<[u8; 4]> {
        let device = match GpuDevice::with_backend(BackendRequest::Auto) {
            Ok(device) => device,
            Err(_) => return Vec::new(),
        };
        let outgoing = solid(&device, &device.queue, [255, 0, 0, 255]);
        let incoming = solid(&device, &device.queue, [0, 255, 0, 255]);
        let target = device.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("replay-transition-test-target"),
            size: wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut pass = TransitionPass::new(&device.device, wgpu::TextureFormat::Rgba8Unorm);
        let mut encoder = device
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("replay-transition-test"),
            });
        pass.run(
            &device.device,
            &device.queue,
            &mut encoder,
            &outgoing.create_view(&Default::default()),
            &incoming.create_view(&Default::default()),
            &target.create_view(&Default::default()),
            kind,
            direction,
            mix,
        );
        let readback = device.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("replay-transition-test-readback"),
            size: (ROW * H) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW),
                    rows_per_image: Some(H),
                },
            },
            wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
        );
        let submission = device.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        });
        let data = slice.get_mapped_range().expect("map transition readback");
        let middle = (ROW * (H / 2)) as usize;
        let row = data[middle..middle + (W * 4) as usize]
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect();
        drop(data);
        readback.unmap();
        row
    }

    /// Packed frames carry two pixels per texel: (Y0, U, Y1, V). Luma is what moves in
    /// a dissolve, so a packed blend is judged on the luma bytes.
    fn packed_solid(device: &GpuDevice, queue: &wgpu::Queue, luma: u8) -> wgpu::Texture {
        let texture = device.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("replay-transition-test-packed"),
            size: wgpu::Extent3d {
                width: W / 2,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut pixels = vec![0u8; (ROW * H) as usize];
        for row in 0..H {
            for column in 0..(W / 2) {
                let at = (row * ROW + column * 4) as usize;
                pixels[at..at + 4].copy_from_slice(&[luma, 128, luma, 128]);
            }
        }
        queue.write_texture(
            texture.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ROW),
                rows_per_image: Some(H),
            },
            wgpu::Extent3d {
                width: W / 2,
                height: H,
                depth_or_array_layers: 1,
            },
        );
        texture
    }

    /// Runs one packed transition and returns the middle row's luma samples.
    fn run_packed(kind: TransitionKind, mix: f32) -> Vec<u8> {
        let device = match GpuDevice::with_backend(BackendRequest::Auto) {
            Ok(device) => device,
            Err(_) => return Vec::new(),
        };
        let outgoing = packed_solid(&device, &device.queue, 16);
        let incoming = packed_solid(&device, &device.queue, 235);
        let target = device.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("replay-transition-test-packed-target"),
            size: wgpu::Extent3d {
                width: W / 2,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut pass = TransitionPass::new_packed(&device.device, wgpu::TextureFormat::Rgba8Unorm);
        let mut encoder = device
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("replay-transition-test-packed"),
            });
        pass.run(
            &device.device,
            &device.queue,
            &mut encoder,
            &outgoing.create_view(&Default::default()),
            &incoming.create_view(&Default::default()),
            &target.create_view(&Default::default()),
            kind,
            TransitionDirection::Left,
            mix,
        );
        let readback = device.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("replay-transition-test-packed-readback"),
            size: (ROW * H) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW),
                    rows_per_image: Some(H),
                },
            },
            wgpu::Extent3d {
                width: W / 2,
                height: H,
                depth_or_array_layers: 1,
            },
        );
        let submission = device.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        });
        let data = slice
            .get_mapped_range()
            .expect("map packed transition readback");
        let middle = (ROW * (H / 2)) as usize;
        let row = (0..(W / 2) as usize)
            .map(|i| data[middle + i * 4])
            .collect();
        drop(data);
        readback.unmap();
        row
    }

    #[test]
    fn a_packed_dissolve_mixes_luma() {
        let row = run_packed(TransitionKind::Dissolve, 0.5);
        if row.is_empty() {
            return;
        }
        let luma = row[row.len() / 2];
        assert!(
            (100..=150).contains(&luma),
            "luma should sit between the two frames: {luma}"
        );
    }

    #[test]
    fn a_packed_cut_stays_on_the_outgoing_frame() {
        let row = run_packed(TransitionKind::Cut, 0.5);
        if row.is_empty() {
            return;
        }
        assert!(
            row[row.len() / 2] < 40,
            "a cut at half shows the outgoing frame: {:?}",
            row
        );
    }

    #[test]
    fn a_packed_wipe_reaches_the_incoming_frame_on_the_far_side() {
        let row = run_packed(TransitionKind::Wipe, 0.5);
        if row.is_empty() {
            return;
        }
        assert!(row[0] < 40, "the left keeps the outgoing frame: {:?}", row);
        assert!(
            *row.last().unwrap() > 200,
            "the right shows the incoming frame: {:?}",
            row
        );
    }
    #[test]
    fn a_dissolve_mixes_both_frames() {
        let row = run(TransitionKind::Dissolve, TransitionDirection::Left, 0.5);
        if row.is_empty() {
            return;
        }
        let pixel = row[W as usize / 2];
        assert!(
            pixel[0] > 90 && pixel[0] < 165,
            "red should be half: {pixel:?}"
        );
        assert!(
            pixel[1] > 90 && pixel[1] < 165,
            "green should be half: {pixel:?}"
        );
    }

    #[test]
    fn a_cut_shows_only_the_outgoing_frame_until_it_finishes() {
        let row = run(TransitionKind::Cut, TransitionDirection::Left, 0.5);
        if row.is_empty() {
            return;
        }
        let pixel = row[W as usize / 2];
        assert!(
            pixel[0] > 200 && pixel[1] < 60,
            "a cut at half stays on the outgoing frame: {pixel:?}"
        );
    }

    #[test]
    fn a_wipe_puts_the_incoming_frame_on_the_far_side() {
        let row = run(TransitionKind::Wipe, TransitionDirection::Left, 0.5);
        if row.is_empty() {
            return;
        }
        let left = row[0];
        let right = row[W as usize - 1];
        assert!(
            left[0] > 200 && left[1] < 60,
            "left half keeps the outgoing frame: {left:?}"
        );
        assert!(
            right[1] > 200 && right[0] < 60,
            "right half shows the incoming frame: {right:?}"
        );
    }

    #[test]
    fn a_dip_through_black_darkens_both_sides() {
        let row = run(TransitionKind::Dip, TransitionDirection::Left, 0.5);
        if row.is_empty() {
            return;
        }
        let pixel = row[W as usize / 2];
        assert!(
            pixel[0] < 40 && pixel[1] < 40,
            "the middle of a dip is black: {pixel:?}"
        );
    }

    #[test]
    fn a_slide_moves_the_incoming_frame_in() {
        let row = run(TransitionKind::Slide, TransitionDirection::Left, 0.5);
        if row.is_empty() {
            return;
        }
        assert!(
            row[0][0] > 200,
            "the outgoing frame is still on the left: {:?}",
            row[0]
        );
        assert!(
            row[W as usize - 1][1] > 200,
            "the incoming frame covers the right: {:?}",
            row[W as usize - 1]
        );
    }

    #[test]
    fn a_push_moves_both_frames() {
        let row = run(TransitionKind::Push, TransitionDirection::Left, 0.5);
        if row.is_empty() {
            return;
        }
        assert!(
            row[0][0] > 200,
            "the outgoing frame is pushed left: {:?}",
            row[0]
        );
        assert!(
            row[W as usize - 1][1] > 200,
            "the incoming frame arrives from the right: {:?}",
            row[W as usize - 1]
        );
    }
}
