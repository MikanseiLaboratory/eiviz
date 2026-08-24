//! Demand-driven compositor.
//!
//! [`composite`] is the **explicit** [`eiviz_core::CompositorBackend::CpuReference`]
//! implementation (CI / golden frames). Optional `wgpu-backend` is a separate
//! backend. There is no implicit switch from GPU to CPU.

use eiviz_core::{
    ColorConversionPolicy, ColorMetadata, FieldKind, FieldOrder, MixingUnit, Project, Transform2D,
};
use eiviz_media::{PixelFormat, VideoFrame};
use eiviz_time::MediaTime;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    #[error("missing source frame for input {0}")]
    MissingSource(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, GpuError>;

#[derive(Clone, Debug)]
pub struct RenderPlan {
    pub width: u32,
    pub height: u32,
    pub output_format: PixelFormat,
    pub color: ColorMetadata,
    pub field_order: Option<FieldOrder>,
    pub color_conversion: ColorConversionPolicy,
    /// Conservative resident bytes for output, source uploads, and one
    /// readback/staging surface. Admission compares this before activation.
    pub vram_bytes: u64,
    pub layers: Vec<Layer>,
}

impl RenderPlan {
    pub fn estimate_vram_bytes(
        width: u32,
        height: u32,
        output_format: PixelFormat,
        layer_count: usize,
    ) -> u64 {
        let surface = output_format.frame_bytes(width, height).unwrap_or(u64::MAX);
        surface.saturating_mul(layer_count as u64 + 2)
    }

    pub fn field_at(&self, boundary_index: u64) -> FieldKind {
        match self.field_order {
            None => FieldKind::Progressive,
            Some(FieldOrder::TopFieldFirst) if boundary_index.is_multiple_of(2) => FieldKind::Top,
            Some(FieldOrder::TopFieldFirst) => FieldKind::Bottom,
            Some(FieldOrder::BottomFieldFirst) if boundary_index.is_multiple_of(2) => {
                FieldKind::Bottom
            }
            Some(FieldOrder::BottomFieldFirst) => FieldKind::Top,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub input: eiviz_core::InputId,
    pub transform: Transform2D,
    pub opacity: f32,
}

pub fn plan_program(project: &Project, unit: &MixingUnit) -> RenderPlan {
    plan_bus(project, unit.program.scene, unit, true)
}

pub fn plan_preview(project: &Project, unit: &MixingUnit) -> RenderPlan {
    plan_bus(project, unit.preview.scene, unit, false)
}

fn plan_bus(
    project: &Project,
    scene_id: Option<eiviz_core::SceneId>,
    unit: &MixingUnit,
    include_overlays: bool,
) -> RenderPlan {
    let mut layers = Vec::new();
    if let Some(id) = scene_id
        && let Some(scene) = project.scenes.get(&id)
    {
        for item in scene.sorted_items() {
            if item.transform.visible() {
                layers.push(Layer {
                    input: item.input,
                    transform: item.transform,
                    opacity: item.transform.opacity,
                });
            }
        }
    }
    if include_overlays {
        let mut overlays = unit.overlays.clone();
        overlays.sort_by_key(|o| o.z_order);
        for overlay in overlays {
            if !overlay.enabled {
                continue;
            }
            if let Some(id) = overlay.scene
                && let Some(scene) = project.scenes.get(&id)
            {
                for item in scene.sorted_items() {
                    if item.transform.visible() {
                        layers.push(Layer {
                            input: item.input,
                            transform: item.transform,
                            opacity: item.transform.opacity,
                        });
                    }
                }
            }
        }
    }
    let output_format = if project.video.bit_depth > 8 {
        PixelFormat::Rgba16Float
    } else {
        PixelFormat::Rgba8
    };
    let vram_bytes = RenderPlan::estimate_vram_bytes(
        project.video.width,
        project.video.height,
        output_format,
        layers.len(),
    );
    RenderPlan {
        width: project.video.width,
        height: project.video.height,
        output_format,
        color: project.video.color_metadata(),
        field_order: project.video.field_order,
        color_conversion: project.video.color_conversion,
        vram_bytes,
        layers,
    }
}

pub fn composite(
    plan: &RenderPlan,
    sources: &HashMap<eiviz_core::InputId, VideoFrame>,
    pts: MediaTime,
    frame_id: u64,
) -> VideoFrame {
    assert_eq!(
        plan.output_format,
        PixelFormat::Rgba8,
        "CpuReference accepts only the explicit 8-bit baseline"
    );
    let mut buf = vec![0u8; plan.width as usize * plan.height as usize * 4];
    for px in buf.chunks_exact_mut(4) {
        px.copy_from_slice(&[0, 0, 0, 255]);
    }
    for layer in &plan.layers {
        let Some(src) = sources.get(&layer.input) else {
            continue;
        };
        blit_layer(
            &mut buf,
            plan.width,
            plan.height,
            src,
            layer.transform,
            layer.opacity,
        );
    }
    VideoFrame {
        id: frame_id,
        source: None,
        pts,
        capture_domain: eiviz_time::ClockDomain::Virtual,
        clock_observation: None,
        width: plan.width,
        height: plan.height,
        format: PixelFormat::Rgba8,
        color: plan.color,
        field: plan.field_at(frame_id),
        data: Arc::<[u8]>::from(buf),
        discontinuity: false,
    }
}

pub fn mix_frames(
    a: &VideoFrame,
    b: &VideoFrame,
    t: f32,
    pts: MediaTime,
    frame_id: u64,
) -> VideoFrame {
    let t = t.clamp(0.0, 1.0);
    let n = a.data.len().min(b.data.len());
    let mut out = vec![0u8; n];
    for (dst, (av, bv)) in out.iter_mut().zip(a.data.iter().zip(b.data.iter())) {
        let mixed = (*av as f32) * (1.0 - t) + (*bv as f32) * t;
        *dst = mixed.round() as u8;
    }
    VideoFrame {
        id: frame_id,
        source: None,
        pts,
        capture_domain: eiviz_time::ClockDomain::Virtual,
        clock_observation: None,
        width: a.width,
        height: a.height,
        format: PixelFormat::Rgba8,
        color: a.color,
        field: a.field,
        data: out.into(),
        discontinuity: false,
    }
}

fn blit_layer(dst: &mut [u8], dw: u32, dh: u32, src: &VideoFrame, xf: Transform2D, opacity: f32) {
    let x0 = (xf.x * dw as f32).round() as i32;
    let y0 = (xf.y * dh as f32).round() as i32;
    let x1 = ((xf.x + xf.width) * dw as f32).round() as i32;
    let y1 = ((xf.y + xf.height) * dh as f32).round() as i32;
    let x0c = x0.max(0) as u32;
    let y0c = y0.max(0) as u32;
    let x1c = x1.clamp(0, dw as i32) as u32;
    let y1c = y1.clamp(0, dh as i32) as u32;
    if x1c <= x0c || y1c <= y0c {
        return;
    }
    let tw = (x1c - x0c).max(1);
    let th = (y1c - y0c).max(1);
    for y in y0c..y1c {
        for x in x0c..x1c {
            let u = (x - x0c) as f32 / tw as f32;
            let v = (y - y0c) as f32 / th as f32;
            let sx = ((u * src.width as f32) as u32).min(src.width.saturating_sub(1));
            let sy = ((v * src.height as f32) as u32).min(src.height.saturating_sub(1));
            let [sr, sg, sb, sa] = src.pixel(sx, sy);
            let a = (sa as f32 / 255.0) * opacity.clamp(0.0, 1.0);
            let di = ((y * dw + x) * 4) as usize;
            let dr = dst[di] as f32;
            let dg = dst[di + 1] as f32;
            let db = dst[di + 2] as f32;
            dst[di] = (sr as f32 * a + dr * (1.0 - a)).round() as u8;
            dst[di + 1] = (sg as f32 * a + dg * (1.0 - a)).round() as u8;
            dst[di + 2] = (sb as f32 * a + db * (1.0 - a)).round() as u8;
            dst[di + 3] = 255;
        }
    }
}

pub fn color_bars(id: u64, pts: MediaTime, width: u32, height: u32) -> VideoFrame {
    let colors: [[u8; 4]; 8] = [
        [192, 192, 192, 255],
        [192, 192, 0, 255],
        [0, 192, 192, 255],
        [0, 192, 0, 255],
        [192, 0, 192, 255],
        [192, 0, 0, 255],
        [0, 0, 192, 255],
        [0, 0, 0, 255],
    ];
    let mut data = vec![0u8; width as usize * height as usize * 4];
    let stripe = (width / 8).max(1);
    for y in 0..height {
        for x in 0..width {
            let c = colors[(x / stripe).min(7) as usize];
            let i = ((y * width + x) * 4) as usize;
            data[i..i + 4].copy_from_slice(&c);
        }
    }
    VideoFrame {
        id,
        source: None,
        pts,
        capture_domain: eiviz_time::ClockDomain::Virtual,
        clock_observation: None,
        width,
        height,
        format: PixelFormat::Rgba8,
        color: eiviz_core::ColorSpace::Bt709Sdr.metadata(),
        field: FieldKind::Progressive,
        data: data.into(),
        discontinuity: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_core::{Input, InputSource, Project, Scene, SceneItem, Transform2D};
    use eiviz_time::MediaTime;

    #[test]
    fn take_target_covers_program_canvas() {
        let mut p = Project::new("gpu");
        let input = Input {
            id: eiviz_core::InputId::new(),
            name: "red".into(),
            tags: vec![],
            groups: vec![],
            source: InputSource::SolidColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
        };
        let scene = Scene {
            id: eiviz_core::SceneId::new(),
            name: "red".into(),
            items: vec![SceneItem {
                id: eiviz_core::SceneItemId::new(),
                input: input.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        p.inputs.insert(input.id, input.clone());
        p.scenes.insert(scene.id, scene.clone());
        let mut unit = p.mixing_units.values().next().unwrap().clone();
        unit.program.scene = Some(scene.id);
        let plan = plan_program(&p, &unit);
        let mut srcs = HashMap::new();
        srcs.insert(
            input.id,
            VideoFrame::rgba_solid(0, MediaTime::ZERO, 16, 16, [255, 0, 0, 255]),
        );
        let frame = composite(&plan, &srcs, MediaTime::ZERO, 1);
        assert_eq!(frame.pixel(0, 0), [255, 0, 0, 255]);
        assert_eq!(frame.pixel(frame.width - 1, frame.height - 1)[0], 255);
    }

    #[test]
    fn extended_render_plan_carries_format_field_and_vram_contract() {
        let mut project = Project::new("extended plan");
        project.compositor = eiviz_core::CompositorBackend::Wgpu;
        project.video = eiviz_core::VideoFormat::uhd_5994_hdr10_pq();
        let unit = project.mixing_units.values().next().unwrap().clone();
        let plan = plan_program(&project, &unit);
        assert_eq!(plan.output_format, PixelFormat::Rgba16Float);
        assert_eq!(plan.color.transfer, eiviz_core::TransferFunction::Pq);
        assert_eq!(plan.field_at(1_000_000), FieldKind::Progressive);
        assert_eq!(
            plan.vram_bytes,
            3840_u64 * 2160 * 8 * 2,
            "empty plan retains output and staging surfaces"
        );

        project.video =
            eiviz_core::VideoFormat::hd_interlaced_5994(eiviz_core::FieldOrder::TopFieldFirst);
        let field_plan = plan_program(&project, &unit);
        assert_eq!(field_plan.field_at(0), FieldKind::Top);
        assert_eq!(field_plan.field_at(1), FieldKind::Bottom);
    }

    #[cfg(feature = "wgpu-backend")]
    #[test]
    fn wgpu_composites_layers_or_explicitly_has_no_hardware() {
        use crate::wgpu_backend::WgpuCompositor;
        let input = eiviz_core::InputId::new();
        let plan = RenderPlan {
            width: 16,
            height: 16,
            output_format: PixelFormat::Rgba8,
            color: eiviz_core::ColorSpace::Bt709Sdr.metadata(),
            field_order: None,
            color_conversion: ColorConversionPolicy::Exact,
            vram_bytes: RenderPlan::estimate_vram_bytes(16, 16, PixelFormat::Rgba8, 1),
            layers: vec![Layer {
                input,
                transform: Transform2D::fullscreen(),
                opacity: 1.0,
            }],
        };
        if let Ok(gpu) = WgpuCompositor::new_headless_hardware() {
            let source = VideoFrame::rgba_solid(0, MediaTime::ZERO, 4, 4, [255, 0, 0, 255]);
            let frame = gpu
                .composite(&plan, &HashMap::from([(input, source)]), MediaTime::ZERO, 1)
                .unwrap();
            assert_eq!(frame.pixel(8, 8), [255, 0, 0, 255]);
            assert_eq!(gpu.diagnostics().readbacks, 1);
        }
    }
}

#[cfg(feature = "wgpu-backend")]
mod wgpu_backend;

#[cfg(feature = "wgpu-backend")]
pub use wgpu_backend::{
    AdapterCapabilities, DeviceLossReport, SharedWgpuContext, WgpuCompositor, WgpuDiagnostics,
    WgpuError, WgpuTextureFrame,
};
