use std::path::Path;
use std::sync::OnceLock;

pub const LABEL_WIDTH: u32 = 512;
pub const LABEL_HEIGHT: u32 = 32;

pub fn contrast_rgb(rgb: [u8; 3]) -> [u8; 3] {
    let luma = 0.299 * f32::from(rgb[0]) + 0.587 * f32::from(rgb[1]) + 0.114 * f32::from(rgb[2]);
    if luma >= 140.0 {
        [0x11, 0x11, 0x11]
    } else {
        [255, 255, 255]
    }
}

pub fn raster(text: &str, background: [u8; 3]) -> Vec<u8> {
    let width = LABEL_WIDTH as usize;
    let height = LABEL_HEIGHT as usize;
    let mut pixels = vec![0u8; width * height * 4];
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[0] = background[0];
        pixel[1] = background[1];
        pixel[2] = background[2];
        pixel[3] = 255;
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return pixels;
    }
    let Some(font) = font() else {
        return pixels;
    };
    let fg = contrast_rgb(background);
    let px = 18.0;
    let mut pen_x = 8.0f32;
    let baseline = 23.0f32;
    for ch in trimmed.chars() {
        let (metrics, coverage) = font.rasterize(ch, px);
        if pen_x + metrics.advance_width > width as f32 - 4.0 {
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
    pixels
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
            pixels[i] = mix(bg[0], fg[0], cover);
            pixels[i + 1] = mix(bg[1], fg[1], cover);
            pixels[i + 2] = mix(bg[2], fg[2], cover);
            pixels[i + 3] = 255;
        }
    }
}

fn mix(bg: u8, fg: u8, cover: u16) -> u8 {
    ((fg as u16 * cover + bg as u16 * (255 - cover)) / 255) as u8
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
                scale: 40.0,
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
