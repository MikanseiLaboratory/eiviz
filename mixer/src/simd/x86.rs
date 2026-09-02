#![allow(clippy::missing_safety_doc)]
#![allow(unsafe_op_in_unsafe_fn)]

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

use super::scalar;

pub fn yuv422_to_bgra(src: &[u8], width: u32, height: u32, stride: usize, uyvy: bool, dst: &mut [u8]) {
    if is_x86_feature_detected!("avx2") {
        unsafe { yuv422_to_bgra_avx2(src, width, height, stride, uyvy, dst) }
    } else if is_x86_feature_detected!("sse2") {
        unsafe { yuv422_to_bgra_sse2(src, width, height, stride, uyvy, dst) }
    } else {
        scalar::yuv422_to_bgra(src, width, height, stride, uyvy, dst);
    }
}

pub fn yuy2_to_uyvy(src: &[u8], width: u32, height: u32, stride: usize, dst: &mut [u8]) {
    if is_x86_feature_detected!("ssse3") {
        unsafe { yuy2_to_uyvy_ssse3(src, width, height, stride, dst) }
    } else {
        scalar::yuy2_to_uyvy(src, width, height, stride, dst);
    }
}

pub fn or_opaque_bgra(pixels: &mut [u8]) {
    if is_x86_feature_detected!("avx2") {
        unsafe { or_opaque_avx2(pixels) }
    } else if is_x86_feature_detected!("sse2") {
        unsafe { or_opaque_sse2(pixels) }
    } else {
        scalar::or_opaque_bgra(pixels);
    }
}

pub fn mix_stereo_gain(dest: &mut [f32], src: &[f32], gain: f32) {
    if is_x86_feature_detected!("avx2") {
        unsafe { mix_gain_avx2(dest, src, gain) }
    } else if is_x86_feature_detected!("sse2") {
        unsafe { mix_gain_sse2(dest, src, gain) }
    } else {
        scalar::mix_stereo_gain(dest, src, gain);
    }
}

pub fn scale_f32(samples: &mut [f32], gain: f32) {
    if is_x86_feature_detected!("avx2") {
        unsafe { scale_avx2(samples, gain) }
    } else if is_x86_feature_detected!("sse2") {
        unsafe { scale_sse2(samples, gain) }
    } else {
        scalar::scale_f32(samples, gain);
    }
}

pub fn peak_f32(samples: &[f32]) -> f32 {
    if is_x86_feature_detected!("avx2") {
        unsafe { peak_avx2(samples) }
    } else if is_x86_feature_detected!("sse2") {
        unsafe { peak_sse2(samples) }
    } else {
        scalar::peak_f32(samples)
    }
}

#[allow(dead_code)]
pub fn peak_interleaved(samples: &[f32]) -> (f32, f32) {
    scalar::peak_interleaved(samples)
}

#[target_feature(enable = "sse2")]
unsafe fn yuv422_to_bgra_sse2(
    src: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    uyvy: bool,
    dst: &mut [u8],
) {
    scalar::yuv422_to_bgra(src, width, height, stride, uyvy, dst);
}

#[target_feature(enable = "avx2")]
unsafe fn yuv422_to_bgra_avx2(
    src: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    uyvy: bool,
    dst: &mut [u8],
) {
    scalar::yuv422_to_bgra(src, width, height, stride, uyvy, dst);
}

#[target_feature(enable = "ssse3")]
unsafe fn yuy2_to_uyvy_ssse3(src: &[u8], width: u32, height: u32, stride: usize, dst: &mut [u8]) {
    let w = (width as usize) & !1;
    let h = height as usize;
    let dst_stride = w * 2;
    let mask = _mm_setr_epi8(1, 0, 3, 2, 5, 4, 7, 6, 9, 8, 11, 10, 13, 12, 15, 14);
    for y in 0..h {
        let s = y * stride;
        let d = y * dst_stride;
        let mut x = 0;
        while x + 16 <= w * 2 && s + x + 16 <= src.len() && d + x + 16 <= dst.len() {
            let v = _mm_loadu_si128(src.as_ptr().add(s + x).cast());
            let shuffled = _mm_shuffle_epi8(v, mask);
            _mm_storeu_si128(dst.as_mut_ptr().add(d + x).cast(), shuffled);
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

#[target_feature(enable = "sse2")]
unsafe fn or_opaque_sse2(pixels: &mut [u8]) {
    let alpha = _mm_set1_epi32(0xFF00_0000u32 as i32);
    let mut i = 0;
    let n = pixels.len() / 16 * 16;
    while i < n {
        let v = _mm_loadu_si128(pixels.as_ptr().add(i).cast());
        _mm_storeu_si128(pixels.as_mut_ptr().add(i).cast(), _mm_or_si128(v, alpha));
        i += 16;
    }
    if i < pixels.len() {
        scalar::or_opaque_bgra(&mut pixels[i..]);
    }
}

#[target_feature(enable = "avx2")]
unsafe fn or_opaque_avx2(pixels: &mut [u8]) {
    let alpha = _mm256_set1_epi32(0xFF00_0000u32 as i32);
    let mut i = 0;
    let n = pixels.len() / 32 * 32;
    while i < n {
        let v = _mm256_loadu_si256(pixels.as_ptr().add(i).cast());
        _mm256_storeu_si256(pixels.as_mut_ptr().add(i).cast(), _mm256_or_si256(v, alpha));
        i += 32;
    }
    if i < pixels.len() {
        or_opaque_sse2(&mut pixels[i..]);
    }
}

#[target_feature(enable = "sse2")]
unsafe fn mix_gain_sse2(dest: &mut [f32], src: &[f32], gain: f32) {
    let n = dest.len().min(src.len());
    let g = _mm_set1_ps(gain);
    let mut i = 0;
    let end = n / 4 * 4;
    while i < end {
        let d = _mm_loadu_ps(dest.as_ptr().add(i));
        let s = _mm_loadu_ps(src.as_ptr().add(i));
        _mm_storeu_ps(dest.as_mut_ptr().add(i), _mm_add_ps(d, _mm_mul_ps(s, g)));
        i += 4;
    }
    for j in i..n {
        dest[j] += src[j] * gain;
    }
}

#[target_feature(enable = "avx2")]
unsafe fn mix_gain_avx2(dest: &mut [f32], src: &[f32], gain: f32) {
    let n = dest.len().min(src.len());
    let g = _mm256_set1_ps(gain);
    let mut i = 0;
    let end = n / 8 * 8;
    while i < end {
        let d = _mm256_loadu_ps(dest.as_ptr().add(i));
        let s = _mm256_loadu_ps(src.as_ptr().add(i));
        _mm256_storeu_ps(dest.as_mut_ptr().add(i), _mm256_add_ps(d, _mm256_mul_ps(s, g)));
        i += 8;
    }
    if i < n {
        mix_gain_sse2(&mut dest[i..], &src[i..], gain);
    }
}

#[target_feature(enable = "sse2")]
unsafe fn scale_sse2(samples: &mut [f32], gain: f32) {
    let g = _mm_set1_ps(gain);
    let mut i = 0;
    let end = samples.len() / 4 * 4;
    while i < end {
        let v = _mm_loadu_ps(samples.as_ptr().add(i));
        _mm_storeu_ps(samples.as_mut_ptr().add(i), _mm_mul_ps(v, g));
        i += 4;
    }
    for sample in &mut samples[i..] {
        *sample *= gain;
    }
}

#[target_feature(enable = "avx2")]
unsafe fn scale_avx2(samples: &mut [f32], gain: f32) {
    let g = _mm256_set1_ps(gain);
    let mut i = 0;
    let end = samples.len() / 8 * 8;
    while i < end {
        let v = _mm256_loadu_ps(samples.as_ptr().add(i));
        _mm256_storeu_ps(samples.as_mut_ptr().add(i), _mm256_mul_ps(v, g));
        i += 8;
    }
    if i < samples.len() {
        scale_sse2(&mut samples[i..], gain);
    }
}

#[target_feature(enable = "sse2")]
unsafe fn peak_sse2(samples: &[f32]) -> f32 {
    let sign = _mm_set1_ps(-0.0);
    let mut acc = _mm_setzero_ps();
    let mut i = 0;
    let end = samples.len() / 4 * 4;
    while i < end {
        let v = _mm_andnot_ps(sign, _mm_loadu_ps(samples.as_ptr().add(i)));
        acc = _mm_max_ps(acc, v);
        i += 4;
    }
    let mut tmp = [0.0f32; 4];
    _mm_storeu_ps(tmp.as_mut_ptr(), acc);
    let mut max = tmp[0].max(tmp[1]).max(tmp[2]).max(tmp[3]);
    for sample in &samples[i..] {
        max = max.max(sample.abs());
    }
    max
}

#[target_feature(enable = "avx2")]
unsafe fn peak_avx2(samples: &[f32]) -> f32 {
    let sign = _mm256_set1_ps(-0.0);
    let mut acc = _mm256_setzero_ps();
    let mut i = 0;
    let end = samples.len() / 8 * 8;
    while i < end {
        let v = _mm256_andnot_ps(sign, _mm256_loadu_ps(samples.as_ptr().add(i)));
        acc = _mm256_max_ps(acc, v);
        i += 8;
    }
    let mut tmp = [0.0f32; 8];
    _mm256_storeu_ps(tmp.as_mut_ptr(), acc);
    let mut max = tmp.iter().copied().fold(0.0f32, f32::max);
    for sample in &samples[i..] {
        max = max.max(sample.abs());
    }
    max
}
