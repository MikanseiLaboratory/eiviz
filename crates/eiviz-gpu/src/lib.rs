//! Demand-driven compositor. The CPU path is the CI/reference implementation.
//! A wgpu backend can be enabled with `--features wgpu-backend`.

use eiviz_core::{MixingUnit, Project, Transform2D};
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
    pub layers: Vec<Layer>,
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
    if let Some(id) = scene_id {
        if let Some(scene) = project.scenes.get(&id) {
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
    if include_overlays {
        let mut overlays = unit.overlays.clone();
        overlays.sort_by_key(|o| o.z_order);
        for overlay in overlays {
            if !overlay.enabled {
                continue;
            }
            if let Some(id) = overlay.scene {
                if let Some(scene) = project.scenes.get(&id) {
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
    }
    RenderPlan {
        width: project.video.width,
        height: project.video.height,
        layers,
    }
}

pub fn composite(
    plan: &RenderPlan,
    sources: &HashMap<eiviz_core::InputId, VideoFrame>,
    pts: MediaTime,
    frame_id: u64,
) -> VideoFrame {
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
        width: plan.width,
        height: plan.height,
        format: PixelFormat::Rgba8,
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
        width: a.width,
        height: a.height,
        format: PixelFormat::Rgba8,
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
        width,
        height,
        format: PixelFormat::Rgba8,
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
}
