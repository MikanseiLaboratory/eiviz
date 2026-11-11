#![allow(clippy::missing_safety_doc)]
#![allow(unsafe_op_in_unsafe_fn)]

#[cfg(target_arch = "aarch64")]
use std::arch::aarch64::*;

use super::scalar;

pub fn yuv422_to_bgra(
    src: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    uyvy: bool,
    dst: &mut [u8],
) {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        yuv422_to_bgra_neon(src, width, height, stride, uyvy, dst);
        return;
    }
    #[cfg(not(target_arch = "aarch64"))]
    scalar::yuv422_to_bgra(src, width, height, stride, uyvy, dst);
}

#[cfg(target_arch = "aarch64")]
unsafe fn yuv422_to_bgra_neon(
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
    if w == 0
        || h == 0
        || src.len() < (h - 1) * stride + w * 2
        || dst.len() < (h - 1) * dst_stride + w * 4
    {
        return;
    }
    for y in 0..h {
        let s = y * stride;
        let d = y * dst_stride;
        let mut x = 0usize;
        while x + 16 <= w && s + x * 2 + 32 <= src.len() && d + x * 4 + 64 <= dst.len() {
            let p = vld4_u8(src.as_ptr().add(s + x * 2));
            let (y0, cb, cr) = if uyvy {
                (p.1, p.0, p.2)
            } else {
                (p.0, p.1, p.3)
            };
            let (y1, _, _) = if uyvy {
                (p.3, p.0, p.2)
            } else {
                (p.2, p.1, p.3)
            };
            vst4_u8(
                dst.as_mut_ptr().add(d + x * 4),
                yuv8_to_bgra(y0, y1, cb, cr),
            );

            let (y0, cb, cr) = if uyvy {
                (
                    vext_u8::<4>(p.1, p.1),
                    vext_u8::<4>(p.0, p.0),
                    vext_u8::<4>(p.2, p.2),
                )
            } else {
                (
                    vext_u8::<4>(p.0, p.0),
                    vext_u8::<4>(p.1, p.1),
                    vext_u8::<4>(p.3, p.3),
                )
            };
            let (y1, _, _) = if uyvy {
                (
                    vext_u8::<4>(p.3, p.3),
                    vext_u8::<4>(p.0, p.0),
                    vext_u8::<4>(p.2, p.2),
                )
            } else {
                (
                    vext_u8::<4>(p.2, p.2),
                    vext_u8::<4>(p.1, p.1),
                    vext_u8::<4>(p.3, p.3),
                )
            };
            vst4_u8(
                dst.as_mut_ptr().add(d + x * 4 + 32),
                yuv8_to_bgra(y0, y1, cb, cr),
            );
            x += 16;
        }
        if x < w {
            scalar::yuv422_to_bgra(
                &src[s + x * 2..],
                (w - x) as u32,
                1,
                (w - x) * 2,
                uyvy,
                &mut dst[d + x * 4..],
            );
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn yuv8_to_bgra(y0: uint8x8_t, y1: uint8x8_t, cb: uint8x8_t, cr: uint8x8_t) -> uint8x8x4_t {
    let (b0, g0, r0) = yuv4_to_bgra(y0, cb, cr);
    let (b1, g1, r1) = yuv4_to_bgra(y1, cb, cr);
    uint8x8x4_t(
        vzip1_u8(b0, b1),
        vzip1_u8(g0, g1),
        vzip1_u8(r0, r1),
        vdup_n_u8(255),
    )
}

#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn yuv4_to_bgra(
    y: uint8x8_t,
    cb: uint8x8_t,
    cr: uint8x8_t,
) -> (uint8x8_t, uint8x8_t, uint8x8_t) {
    let zero = vdupq_n_f32(0.0);
    let max = vdupq_n_f32(255.0);
    let yv = vmulq_n_f32(
        vsubq_f32(
            vcvtq_f32_u32(vmovl_u16(vget_low_u16(vmovl_u8(y)))),
            vdupq_n_f32(16.0),
        ),
        255.0 / 219.0,
    );
    let u = vmulq_n_f32(
        vsubq_f32(
            vcvtq_f32_u32(vmovl_u16(vget_low_u16(vmovl_u8(cb)))),
            vdupq_n_f32(128.0),
        ),
        255.0 / 224.0,
    );
    let v = vmulq_n_f32(
        vsubq_f32(
            vcvtq_f32_u32(vmovl_u16(vget_low_u16(vmovl_u8(cr)))),
            vdupq_n_f32(128.0),
        ),
        255.0 / 224.0,
    );
    let b = vminq_f32(vmaxq_f32(vaddq_f32(yv, vmulq_n_f32(u, 1.8556)), zero), max);
    let g = vminq_f32(
        vmaxq_f32(
            vsubq_f32(
                vsubq_f32(yv, vmulq_n_f32(u, 0.187324)),
                vmulq_n_f32(v, 0.468124),
            ),
            zero,
        ),
        max,
    );
    let r = vminq_f32(vmaxq_f32(vaddq_f32(yv, vmulq_n_f32(v, 1.5748)), zero), max);
    (
        vmovn_u16(vcombine_u16(
            vmovn_u32(vcvtq_u32_f32(b)),
            vmovn_u32(vcvtq_u32_f32(b)),
        )),
        vmovn_u16(vcombine_u16(
            vmovn_u32(vcvtq_u32_f32(g)),
            vmovn_u32(vcvtq_u32_f32(g)),
        )),
        vmovn_u16(vcombine_u16(
            vmovn_u32(vcvtq_u32_f32(r)),
            vmovn_u32(vcvtq_u32_f32(r)),
        )),
    )
}

pub fn yuy2_to_uyvy(src: &[u8], width: u32, height: u32, stride: usize, dst: &mut [u8]) {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        yuy2_to_uyvy_neon(src, width, height, stride, dst);
    }
    #[cfg(not(target_arch = "aarch64"))]
    scalar::yuy2_to_uyvy(src, width, height, stride, dst);
}

pub fn or_opaque_bgra(pixels: &mut [u8]) {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        or_opaque_neon(pixels);
    }
    #[cfg(not(target_arch = "aarch64"))]
    scalar::or_opaque_bgra(pixels);
}

pub fn mix_stereo_gain(dest: &mut [f32], src: &[f32], gain: f32) {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        mix_gain_neon(dest, src, gain);
    }
    #[cfg(not(target_arch = "aarch64"))]
    scalar::mix_stereo_gain(dest, src, gain);
}

pub fn scale_f32(samples: &mut [f32], gain: f32) {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        scale_neon(samples, gain);
    }
    #[cfg(not(target_arch = "aarch64"))]
    scalar::scale_f32(samples, gain);
}

pub fn peak_f32(samples: &[f32]) -> f32 {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        return peak_neon(samples);
    }
    #[cfg(not(target_arch = "aarch64"))]
    scalar::peak_f32(samples)
}

pub fn peak_interleaved(samples: &[f32]) -> (f32, f32) {
    scalar::peak_interleaved(samples)
}

#[cfg(target_arch = "aarch64")]
unsafe fn yuy2_to_uyvy_neon(src: &[u8], width: u32, height: u32, stride: usize, dst: &mut [u8]) {
    let w = (width as usize) & !1;
    let h = height as usize;
    let dst_stride = w * 2;
    for y in 0..h {
        let s = y * stride;
        let d = y * dst_stride;
        let mut x = 0;
        while x + 16 <= w * 2 && s + x + 16 <= src.len() && d + x + 16 <= dst.len() {
            let v = vld1q_u8(src.as_ptr().add(s + x));
            let even = vshrq_n_u16(vreinterpretq_u16_u8(v), 8);
            let odd = vshlq_n_u16(vreinterpretq_u16_u8(v), 8);
            let swapped = vorrq_u16(even, odd);
            vst1q_u8(dst.as_mut_ptr().add(d + x), vreinterpretq_u8_u16(swapped));
            x += 16;
        }
        while x + 4 <= w * 2 && s + x + 3 < src.len() && d + x + 3 < dst.len() {
            dst[d + x] = src[s + x + 1];
            dst[d + x + 1] = src[s + x];
            dst[d + x + 2] = src[s + x + 3];
            dst[d + x + 3] = src[s + x + 2];
            x += 4;
        }
    }
}

#[cfg(target_arch = "aarch64")]
unsafe fn or_opaque_neon(pixels: &mut [u8]) {
    let alpha = vdupq_n_u32(0xFF00_0000);
    let mut i = 0;
    let n = pixels.len() / 16 * 16;
    while i < n {
        let v = vld1q_u32(pixels.as_ptr().add(i).cast());
        vst1q_u32(pixels.as_mut_ptr().add(i).cast(), vorrq_u32(v, alpha));
        i += 16;
    }
    if i < pixels.len() {
        scalar::or_opaque_bgra(&mut pixels[i..]);
    }
}

#[cfg(target_arch = "aarch64")]
unsafe fn mix_gain_neon(dest: &mut [f32], src: &[f32], gain: f32) {
    let n = dest.len().min(src.len());
    let g = vdupq_n_f32(gain);
    let mut i = 0;
    let end = n / 4 * 4;
    while i < end {
        let d = vld1q_f32(dest.as_ptr().add(i));
        let s = vld1q_f32(src.as_ptr().add(i));
        vst1q_f32(dest.as_mut_ptr().add(i), vfmaq_f32(d, s, g));
        i += 4;
    }
    for j in i..n {
        dest[j] += src[j] * gain;
    }
}

#[cfg(target_arch = "aarch64")]
unsafe fn scale_neon(samples: &mut [f32], gain: f32) {
    let g = vdupq_n_f32(gain);
    let mut i = 0;
    let end = samples.len() / 4 * 4;
    while i < end {
        let v = vld1q_f32(samples.as_ptr().add(i));
        vst1q_f32(samples.as_mut_ptr().add(i), vmulq_f32(v, g));
        i += 4;
    }
    for sample in &mut samples[i..] {
        *sample *= gain;
    }
}

#[cfg(target_arch = "aarch64")]
unsafe fn peak_neon(samples: &[f32]) -> f32 {
    let mut acc = vdupq_n_f32(0.0);
    let mut i = 0;
    let end = samples.len() / 4 * 4;
    while i < end {
        let v = vabsq_f32(vld1q_f32(samples.as_ptr().add(i)));
        acc = vmaxq_f32(acc, v);
        i += 4;
    }
    let mut tmp = [0.0f32; 4];
    vst1q_f32(tmp.as_mut_ptr(), acc);
    let mut max = tmp[0].max(tmp[1]).max(tmp[2]).max(tmp[3]);
    for sample in &samples[i..] {
        max = max.max(sample.abs());
    }
    max
}
