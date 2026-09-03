use std::path::Path;
use std::sync::OnceLock;

const BAND_ALPHA: u8 = 168;
const TEXT_ALPHA: u8 = 220;
const AA: f32 = 2.0;
const MIN_PX: f32 = 8.0;
const MAX_PX: f32 = 128.0;

pub fn contrast_rgb(rgb: [u8; 3]) -> [u8; 3] {
    let luma = 0.299 * f32::from(rgb[0]) + 0.587 * f32::from(rgb[1]) + 0.114 * f32::from(rgb[2]);
    if luma >= 140.0 {
        [0x11, 0x11, 0x11]
    } else {
        [255, 255, 255]
    }
}

pub fn clamp_size(size: f32) -> f32 {
    if !size.is_finite() {
        return 18.0;
    }
    size.clamp(1.0, 200.0)
}

pub fn font_px(size: f32, percent: bool, tile_height_px: f32) -> f32 {
    let px = if percent {
        tile_height_px.max(1.0) * (clamp_size(size) / 100.0)
    } else {
        clamp_size(size)
    };
    let snapped = (px * 2.0).round() / 2.0;
    snapped.clamp(MIN_PX, MAX_PX)
}

pub fn band_height(font_px: f32) -> u32 {
    let (ascent, descent, pad) = line_box(font_px);
    (ascent + descent + pad * 2.0).ceil().max(1.0) as u32
}

pub struct Raster {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub fn raster(text: &str, background: [u8; 3], font_px: f32, dest_w: u32, dest_h: u32) -> Raster {
    let width = ((dest_w.max(1) as f32) * AA).round().max(1.0) as usize;
    let height = ((dest_h.max(1) as f32) * AA).round().max(1.0) as usize;
    let mut pixels = vec![0u8; width * height * 4];
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[0] = background[0];
        pixel[1] = background[1];
        pixel[2] = background[2];
        pixel[3] = BAND_ALPHA;
    }
    let trimmed = text.trim();
    let Some(font) = font() else {
        return Raster {
            pixels,
            width: width as u32,
            height: height as u32,
        };
    };
    if trimmed.is_empty() {
        return Raster {
            pixels,
            width: width as u32,
            height: height as u32,
        };
    }
    let fg = contrast_rgb(background);
    let px = font_px * AA;
    let (ascent, _, pad) = line_box(font_px);
    let mut pen_x = (6.0 * AA).max(2.0);
    let baseline = (pad + ascent) * AA;
    for ch in trimmed.chars() {
        let (metrics, coverage) = font.rasterize(ch, px);
        if pen_x + metrics.advance_width > width as f32 - 2.0 * AA {
            break;
        }
        blit_glyph(
            &mut pixels,
            width,
            height,
            &coverage,
            metrics.width,
            metrics.height,
            pen_x + metrics.xmin as f32,
            baseline - metrics.height as f32 - metrics.ymin as f32,
            fg,
            background,
        );
        pen_x += metrics.advance_width;
    }
    Raster {
        pixels,
        width: width as u32,
        height: height as u32,
    }
}

fn line_box(font_px: f32) -> (f32, f32, f32) {
    let pad = (font_px * 0.1).clamp(1.0, 4.0);
    if let Some(font) = font() {
        if let Some(metrics) = font.horizontal_line_metrics(font_px) {
            return (metrics.ascent.max(0.0), metrics.descent.abs(), pad);
        }
    }
    (font_px * 0.8, font_px * 0.22, pad)
}

fn blit_glyph(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    coverage: &[u8],
    glyph_w: usize,
    glyph_h: usize,
    origin_x: f32,
    origin_y: f32,
    fg: [u8; 3],
    bg: [u8; 3],
) {
    let ox = origin_x.round() as i32;
    let oy = origin_y.round() as i32;
    for gy in 0..glyph_h {
        for gx in 0..glyph_w {
            let dx = ox + gx as i32;
            let dy = oy + gy as i32;
            if dx < 0 || dy < 0 || dx >= width as i32 || dy >= height as i32 {
                continue;
            }
            let cover = coverage[gy * glyph_w + gx] as u16;
            if cover == 0 {
                continue;
            }
            let i = (dy as usize * width + dx as usize) * 4;
            pixels[i] = crate::simd::blend_u8(bg[0], fg[0], cover);
            pixels[i + 1] = crate::simd::blend_u8(bg[1], fg[1], cover);
            pixels[i + 2] = crate::simd::blend_u8(bg[2], fg[2], cover);
            pixels[i + 3] = crate::simd::blend_u8(BAND_ALPHA, TEXT_ALPHA, cover);
        }
    }
}

fn font() -> Option<&'static fontdue::Font> {
    static FONT: OnceLock<Option<fontdue::Font>> = OnceLock::new();
    FONT.get_or_init(load_font).as_ref()
}

fn load_font() -> Option<fontdue::Font> {
    for path in font_paths() {
        if let Some(font) = load_path(&path) {
            return Some(font);
        }
    }
    None
}

fn load_path(path: &Path) -> Option<fontdue::Font> {
    let bytes = std::fs::read(path).ok()?;
    for index in 0..8u32 {
        if let Ok(font) = fontdue::Font::from_bytes(
            bytes.as_slice(),
            fontdue::FontSettings {
                collection_index: index,
                scale: 160.0,
                ..Default::default()
            },
        ) {
            return Some(font);
        }
    }
    None
}

fn font_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    #[cfg(windows)]
    {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        let fonts = Path::new(&windir).join("Fonts");
        for name in [
            "YuGothM.ttc",
            "YuGothR.ttc",
            "meiryo.ttc",
            "msgothic.ttc",
            "segoeui.ttf",
            "arial.ttf",
        ] {
            paths.push(fonts.join(name));
        }
    }
    #[cfg(target_os = "macos")]
    {
        for name in [
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
            "/System/Library/Fonts/ヒラギノ角ゴシック W6.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/ヒラギノ角ゴ ProN W3.otf",
            "/Library/Fonts/Arial Unicode.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/AppleSDGothicNeo.ttc",
            "/System/Library/Fonts/Helvetica.ttc",
        ] {
            paths.push(Path::new(name).to_path_buf());
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_px_units() {
        assert_eq!(font_px(18.0, false, 540.0), 18.0);
        assert_eq!(font_px(4.0, true, 500.0), 20.0);
        assert_eq!(font_px(1.0, false, 100.0), MIN_PX);
    }

    #[test]
    fn band_tracks_font() {
        let small = band_height(12.0);
        let large = band_height(36.0);
        assert!(small >= 8 && small <= 28);
        assert!(large > small);
        assert!(large <= 64);
    }
}
