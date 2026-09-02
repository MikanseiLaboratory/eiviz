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

pub fn yuv422_to_bgra(src: &[u8], width: u32, height: u32, stride: usize, uyvy: bool, dst: &mut [u8]) {
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
    let words = pixels.len() / 4;
    let ptr = pixels.as_mut_ptr();
    unsafe {
        for i in 0..words {
            *ptr.add(i * 4).cast::<u32>() |= 0xFF00_0000;
        }
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

pub fn planar_to_stereo(planar: &[f32], frames: usize, channels: usize, out: &mut Vec<f32>) {
    out.clear();
    out.reserve(frames * 2);
    for i in 0..frames {
        let left = planar.get(i).copied().unwrap_or(0.0);
        let right = if channels > 1 {
            planar.get(frames + i).copied().unwrap_or(left)
        } else {
            left
        };
        out.push(left);
        out.push(right);
    }
}

pub fn resample_planar_to_stereo(
    planar: &[f32],
    src_frames: usize,
    channels: usize,
    src_rate: usize,
    dst_rate: usize,
    out: &mut Vec<f32>,
) {
    if src_rate == dst_rate {
        planar_to_stereo(planar, src_frames, channels, out);
        return;
    }
    let dst_frames = (src_frames * dst_rate + src_rate / 2) / src_rate.max(1);
    out.clear();
    out.reserve(dst_frames * 2);
    let last = src_frames.saturating_sub(1);
    for i in 0..dst_frames {
        let src = i as f64 * src_rate as f64 / dst_rate.max(1) as f64;
        let idx = (src.floor() as usize).min(last);
        let frac = (src - idx as f64) as f32;
        let nxt = (idx + 1).min(last);
        let left = planar.get(idx).copied().unwrap_or(0.0) * (1.0 - frac)
            + planar.get(nxt).copied().unwrap_or(0.0) * frac;
        let right = if channels > 1 {
            let a = planar.get(src_frames + idx).copied().unwrap_or(left);
            let b = planar.get(src_frames + nxt).copied().unwrap_or(a);
            a * (1.0 - frac) + b * frac
        } else {
            left
        };
        out.push(left);
        out.push(right);
    }
}

pub fn resample_stereo(src: &[f32], src_rate: u32, dst_frames: usize, dst_rate: u32, out: &mut [f32]) {
    let src_frames = src.len() / 2;
    if dst_frames == 0 || out.len() < dst_frames * 2 {
        return;
    }
    if src_frames == 0 {
        out[..dst_frames * 2].fill(0.0);
        return;
    }
    if src_rate == dst_rate && src_frames == dst_frames {
        out[..dst_frames * 2].copy_from_slice(&src[..dst_frames * 2]);
        return;
    }
    let last = src_frames.saturating_sub(1);
    for i in 0..dst_frames {
        let src_pos = i as f64 * f64::from(src_rate) / f64::from(dst_rate.max(1));
        let idx = (src_pos.floor() as usize).min(last);
        let frac = (src_pos - idx as f64) as f32;
        let nxt = (idx + 1).min(last);
        out[i * 2] = src[idx * 2] * (1.0 - frac) + src[nxt * 2] * frac;
        out[i * 2 + 1] = src[idx * 2 + 1] * (1.0 - frac) + src[nxt * 2 + 1] * frac;
    }
}

pub fn blend_u8(bg: u8, fg: u8, cover: u16) -> u8 {
    ((fg as u16 * cover + bg as u16 * (255 - cover)) / 255) as u8
}

pub fn copy_rows(src: &[u8], src_stride: usize, dst: &mut [u8], dst_stride: usize, row_bytes: usize, rows: usize) {
    for y in 0..rows {
        let s = y * src_stride;
        let d = y * dst_stride;
        if s + row_bytes > src.len() || d + row_bytes > dst.len() {
            break;
        }
        dst[d..d + row_bytes].copy_from_slice(&src[s..s + row_bytes]);
    }
}
