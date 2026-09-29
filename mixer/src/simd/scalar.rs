const Y_SCALE: f32 = 255.0 / 219.0;
const CR_R: f32 = 1.5748;
const CB_B: f32 = 1.8556;
const CB_G: f32 = 0.1873;
const CR_G: f32 = 0.4681;

#[inline]
fn yuv_to_bgra(luma: f32, u: f32, v: f32) -> [u8; 4] {
    let yv = (luma - 16.0) * Y_SCALE;
    [
        (yv + CB_B * u).clamp(0.0, 255.0) as u8,
        (yv - CB_G * u - CR_G * v).clamp(0.0, 255.0) as u8,
        (yv + CR_R * v).clamp(0.0, 255.0) as u8,
        255,
    ]
}

pub fn yuv422_to_bgra(
    src: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    uyvy: bool,
    dst: &mut [u8],
) {
    let w = (width as usize) & !1;
    let h = height as usize;
    let dst_stride = w * 4;
    for y in 0..h {
        let s = y * stride;
        let d = y * dst_stride;
        for x in (0..w).step_by(2) {
            let i = s + x * 2;
            if i + 3 >= src.len() || d + x * 4 + 7 >= dst.len() {
                break;
            }
            let (u, y0, v, y1) = if uyvy {
                (
                    src[i] as f32,
                    src[i + 1] as f32,
                    src[i + 2] as f32,
                    src[i + 3] as f32,
                )
            } else {
                (
                    src[i + 1] as f32,
                    src[i] as f32,
                    src[i + 3] as f32,
                    src[i + 2] as f32,
                )
            };
            let u = u - 128.0;
            let v = v - 128.0;
            dst[d + x * 4..d + x * 4 + 4].copy_from_slice(&yuv_to_bgra(y0, u, v));
            dst[d + (x + 1) * 4..d + (x + 1) * 4 + 4].copy_from_slice(&yuv_to_bgra(y1, u, v));
        }
    }
}

pub fn yuy2_to_uyvy(src: &[u8], width: u32, height: u32, stride: usize, dst: &mut [u8]) {
    let w = (width as usize) & !1;
    let h = height as usize;
    let dst_stride = w * 2;
    for y in 0..h {
        let s = y * stride;
        let d = y * dst_stride;
        for x in (0..w).step_by(2) {
            let i = s + x * 2;
            let o = d + x * 2;
            if i + 3 >= src.len() || o + 3 >= dst.len() {
                break;
            }
            dst[o] = src[i + 1];
            dst[o + 1] = src[i];
            dst[o + 2] = src[i + 3];
            dst[o + 3] = src[i + 2];
        }
    }
}

pub fn or_opaque_bgra(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = 0xFF;
    }
}

pub fn mix_stereo_gain(dest: &mut [f32], src: &[f32], gain: f32) {
    let n = dest.len().min(src.len()) & !1;
    for i in 0..n {
        dest[i] += src[i] * gain;
    }
}

pub fn scale_f32(samples: &mut [f32], gain: f32) {
    for sample in samples {
        *sample *= gain;
    }
}

pub fn peak_f32(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |acc, &s| acc.max(s.abs()))
}

pub fn peak_interleaved(samples: &[f32]) -> (f32, f32) {
    let mut left = 0.0f32;
    let mut right = 0.0f32;
    for chunk in samples.chunks_exact(2) {
        left = left.max(chunk[0].abs());
        right = right.max(chunk[1].abs());
    }
    (left.min(1.0), right.min(1.0))
}

pub fn sine_fill(out: &mut [f32], phase: f64, hz: f32, amplitude: f32, rate: f64) {
    let freq = f64::from(hz.max(0.0));
    let tau = std::f64::consts::TAU;
    let mut p = phase;
    for sample in out.iter_mut() {
        *sample = (tau * freq * p / rate).sin() as f32 * amplitude;
        p += 1.0;
        if p >= rate {
            p -= rate;
        }
    }
}

pub fn blend_u8(bg: u8, fg: u8, cover: u16) -> u8 {
    ((fg as u16 * cover + bg as u16 * (255 - cover)) / 255) as u8
}

pub fn copy_rows(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    row_bytes: usize,
    rows: usize,
) {
    for y in 0..rows {
        let s = y * src_stride;
        let d = y * dst_stride;
        if s + row_bytes > src.len() || d + row_bytes > dst.len() {
            break;
        }
        dst[d..d + row_bytes].copy_from_slice(&src[s..s + row_bytes]);
    }
}

#[cfg(test)]
mod opaque_tests {
    use super::*;

    #[test]
    fn or_opaque_handles_unaligned_slices() {
        let mut data = vec![0u8; 13];
        or_opaque_bgra(&mut data[1..9]);
        assert_eq!(data, vec![0, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 0]);
    }
}
