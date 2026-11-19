#![allow(clippy::missing_safety_doc)]
#![allow(unsafe_op_in_unsafe_fn)]

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

use super::scalar;

pub fn yuv422_to_bgra(
    src: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    uyvy: bool,
    dst: &mut [u8],
) {
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

pub fn bgra_to_rgba_rows(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    width: u32,
    height: u32,
) {
    if is_x86_feature_detected!("ssse3") {
        unsafe { bgra_to_rgba_rows_ssse3(src, src_stride, dst, dst_stride, width, height) }
    } else {
        scalar::bgra_to_rgba_rows(src, src_stride, dst, dst_stride, width, height);
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

/// Four pixels per step of 4:2:2 to BGRA.
///
/// The float expression is the scalar one, lane for lane: `(y-16)*Y_SCALE`,
/// `(u-128)*C_SCALE`, `(v-128)*C_SCALE`, then the scalar's add/sub order.
/// Every step is IEEE f32 per lane and the clamp+truncate matches
/// `clamp(0.0, 255.0) as u8`, so the result is bit-identical to the scalar
/// path (the `*_matches_scalar` oracle tests assert it).
#[target_feature(enable = "sse2")]
unsafe fn yuv422_to_bgra_sse2(
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
    let y_scale = _mm_set1_ps(scalar::Y_SCALE);
    let c_scale = _mm_set1_ps(scalar::C_SCALE);
    let cb_b = _mm_set1_ps(scalar::CB_B);
    let cb_g = _mm_set1_ps(scalar::CB_G);
    let cr_g = _mm_set1_ps(scalar::CR_G);
    let cr_r = _mm_set1_ps(scalar::CR_R);
    let k16 = _mm_set1_ps(16.0);
    let k128 = _mm_set1_ps(128.0);
    let zero = _mm_setzero_ps();
    let max = _mm_set1_ps(255.0);
    let opaque = _mm_set1_epi8(-1);
    for y in 0..h {
        let s = y * stride;
        let d = y * dst_stride;
        let mut x = 0usize;
        while x + 4 <= w {
            let i = s + x * 2;
            let o = d + x * 4;
            if i + 7 >= src.len() || o + 15 >= dst.len() {
                break;
            }
            let p = src.as_ptr().add(i);
            let b0 = *p as i32;
            let b1 = *p.add(1) as i32;
            let b2 = *p.add(2) as i32;
            let b3 = *p.add(3) as i32;
            let b4 = *p.add(4) as i32;
            let b5 = *p.add(5) as i32;
            let b6 = *p.add(6) as i32;
            let b7 = *p.add(7) as i32;
            let (y0, y1, y2, y3, u0, u1, v0, v1) = if uyvy {
                (b1, b3, b5, b7, b0, b4, b2, b6)
            } else {
                (b0, b2, b4, b6, b1, b5, b3, b7)
            };
            let yv = _mm_mul_ps(
                _mm_sub_ps(_mm_cvtepi32_ps(_mm_setr_epi32(y0, y1, y2, y3)), k16),
                y_scale,
            );
            let uu = _mm_mul_ps(
                _mm_sub_ps(_mm_cvtepi32_ps(_mm_setr_epi32(u0, u0, u1, u1)), k128),
                c_scale,
            );
            let vv = _mm_mul_ps(
                _mm_sub_ps(_mm_cvtepi32_ps(_mm_setr_epi32(v0, v0, v1, v1)), k128),
                c_scale,
            );
            let bb = _mm_cvttps_epi32(_mm_min_ps(
                _mm_max_ps(_mm_add_ps(yv, _mm_mul_ps(cb_b, uu)), zero),
                max,
            ));
            let gg = _mm_cvttps_epi32(_mm_min_ps(
                _mm_max_ps(
                    _mm_sub_ps(_mm_sub_ps(yv, _mm_mul_ps(cb_g, uu)), _mm_mul_ps(cr_g, vv)),
                    zero,
                ),
                max,
            ));
            let rr = _mm_cvttps_epi32(_mm_min_ps(
                _mm_max_ps(_mm_add_ps(yv, _mm_mul_ps(cr_r, vv)), zero),
                max,
            ));
            // i32 -> i16 -> u8, then interleave B,G,R,255 into 4 BGRA pixels.
            let b8 = _mm_packus_epi16(_mm_packs_epi32(bb, bb), _mm_setzero_si128());
            let g8 = _mm_packus_epi16(_mm_packs_epi32(gg, gg), _mm_setzero_si128());
            let r8 = _mm_packus_epi16(_mm_packs_epi32(rr, rr), _mm_setzero_si128());
            let bg = _mm_unpacklo_epi8(b8, g8);
            let ra = _mm_unpacklo_epi8(r8, opaque);
            let bgra = _mm_unpacklo_epi16(bg, ra);
            _mm_storeu_si128(dst.as_mut_ptr().add(o).cast(), bgra);
            x += 4;
        }
        // Tail (odd widths, short buffers): the scalar expression, so the
        // bounds handling and the values match the reference path exactly.
        while x + 1 < w {
            let i = s + x * 2;
            let o = d + x * 4;
            if i + 3 >= src.len() || o + 7 >= dst.len() {
                break;
            }
            let (u, y0, v, y1) = if uyvy {
                (src[i], src[i + 1], src[i + 2], src[i + 3])
            } else {
                (src[i + 1], src[i], src[i + 3], src[i + 2])
            };
            let u = u as f32 - 128.0;
            let v = v as f32 - 128.0;
            dst[o..o + 4].copy_from_slice(&scalar::yuv_to_bgra(y0 as f32, u, v));
            dst[o + 4..o + 8].copy_from_slice(&scalar::yuv_to_bgra(y1 as f32, u, v));
            x += 2;
        }
    }
}

/// Clamp 8 f32 lanes to 0..=255 and truncate them into 8 packed u8 values.
///
/// `clamp` + `as u8` in the scalar reference is exactly "clamp, then truncate
/// toward zero"; `cvttps` truncates the same way and `shuffle_epi8` keeps the
/// low byte of each dword (already in 0..=255).
#[target_feature(enable = "avx2")]
unsafe fn lanes_to_u8(v: __m256, zero: __m256, max255: __m256, pack_ctrl: __m256i) -> __m128i {
    let clamped = _mm256_min_ps(_mm256_max_ps(v, zero), max255);
    let ints = _mm256_cvttps_epi32(clamped);
    // Per 128-bit lane: four bytes (lane value 0..3 / 4..7) land in bytes 0..3.
    let bytes = _mm256_shuffle_epi8(ints, pack_ctrl);
    // Interleave the two lanes' low dwords: bytes 0..3 = lanes 0..3, 4..7 = 4..7.
    _mm_unpacklo_epi32(
        _mm256_castsi256_si128(bytes),
        _mm256_extracti128_si256(bytes, 1),
    )
}

/// Gather the eight bytes a per-lane `shuffle_epi8` left in bytes 0..3 of each
/// 128-bit lane into one contiguous 8-byte value.
#[target_feature(enable = "avx2")]
unsafe fn gather_lane_bytes(bytes: __m256i) -> __m128i {
    _mm_unpacklo_epi32(
        _mm256_castsi256_si128(bytes),
        _mm256_extracti128_si256(bytes, 1),
    )
}

/// Planar-free YUV 4:2:2 -> BGRA, 16 pixels per iteration.
///
/// The arithmetic mirrors `scalar::yuv_to_bgra` operation for operation
/// (same f32 constants, same association, no FMA: `avx2` does not imply `fma`
/// in Rust, and the scalar path is compiled without it), so the output is
/// byte-identical to the scalar reference.
#[target_feature(enable = "avx2")]
unsafe fn yuv422_to_bgra_avx2(
    src: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    uyvy: bool,
    dst: &mut [u8],
) {
    let y_scale = _mm256_set1_ps(scalar::Y_SCALE);
    let c_scale = _mm256_set1_ps(scalar::C_SCALE);
    let cr_r = _mm256_set1_ps(scalar::CR_R);
    let cb_b = _mm256_set1_ps(scalar::CB_B);
    let cb_g = _mm256_set1_ps(scalar::CB_G);
    let cr_g = _mm256_set1_ps(scalar::CR_G);

    let w = (width as usize) & !1;
    let h = height as usize;
    let dst_stride = w * 4;
    if w == 0 || h == 0 {
        return;
    }

    // Per-128-bit-lane pshufb controls (same bytes in both lanes).
    // UYVY: [U Y V Y] -> luma 1,3,5,7..; U 0,4,8,12; V 2,6,10,14
    // YUY2: [Y U Y V] -> luma 0,2,4,6..; U 1,5,9,13; V 3,7,11,15
    let (luma_ctrl, u_ctrl, v_ctrl) = if uyvy {
        (
            _mm_setr_epi8(1, 3, 5, 7, 9, 11, 13, 15, -1, -1, -1, -1, -1, -1, -1, -1),
            _mm_setr_epi8(0, 4, 8, 12, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1),
            _mm_setr_epi8(2, 6, 10, 14, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1),
        )
    } else {
        (
            _mm_setr_epi8(0, 2, 4, 6, 8, 10, 12, 14, -1, -1, -1, -1, -1, -1, -1, -1),
            _mm_setr_epi8(1, 5, 9, 13, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1),
            _mm_setr_epi8(3, 7, 11, 15, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1),
        )
    };
    let luma_ctrl = _mm256_broadcastsi128_si256(luma_ctrl);
    let u_ctrl = _mm256_broadcastsi128_si256(u_ctrl);
    let v_ctrl = _mm256_broadcastsi128_si256(v_ctrl);
    // Gather byte 0 of each i32 lane (0,4,8,12) per 128-bit lane.
    let pack_ctrl = _mm256_broadcastsi128_si256(_mm_setr_epi8(
        0, 4, 8, 12, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    ));

    let luma_bias = _mm256_set1_ps(16.0);
    let chroma_bias = _mm256_set1_ps(128.0);
    let zero = _mm256_setzero_ps();
    let max255 = _mm256_set1_ps(255.0);
    // Chroma pair i feeds pixels 2i and 2i+1.
    let dup_lo = _mm256_setr_epi32(0, 0, 1, 1, 2, 2, 3, 3);
    let dup_hi = _mm256_setr_epi32(4, 4, 5, 5, 6, 6, 7, 7);
    let alpha = _mm_set1_epi8(-1i8);

    for y in 0..h {
        let row_src = y * stride;
        let row_dst = y * dst_stride;
        let mut x = 0usize;
        // 16 pixels consume 32 source bytes and write 64 bytes.
        while x + 16 <= w && row_src + x * 2 + 32 <= src.len() && row_dst + x * 4 + 64 <= dst.len()
        {
            let packed = _mm256_loadu_si256(src.as_ptr().add(row_src + x * 2).cast());
            let luma = _mm256_shuffle_epi8(packed, luma_ctrl);
            let u = _mm256_shuffle_epi8(packed, u_ctrl);
            let v = _mm256_shuffle_epi8(packed, v_ctrl);
            // 8 luma bytes live in bytes 0..7 of each lane: q0 || q2 is contiguous.
            let luma = _mm256_permute4x64_epi64::<0x08>(luma);
            // 4 chroma bytes live in bytes 0..3 of each lane.
            let u = gather_lane_bytes(u);
            let v = gather_lane_bytes(v);
            let luma128 = _mm256_castsi256_si128(luma);

            let y_lo = _mm256_cvtepi32_ps(_mm256_cvtepu8_epi32(luma128));
            let y_hi = _mm256_cvtepi32_ps(_mm256_cvtepu8_epi32(_mm_srli_si128::<8>(luma128)));
            let u_f = _mm256_mul_ps(
                _mm256_sub_ps(_mm256_cvtepi32_ps(_mm256_cvtepu8_epi32(u)), chroma_bias),
                c_scale,
            );
            let v_f = _mm256_mul_ps(
                _mm256_sub_ps(_mm256_cvtepi32_ps(_mm256_cvtepu8_epi32(v)), chroma_bias),
                c_scale,
            );

            let mut b = _mm_setzero_si128();
            let mut g = _mm_setzero_si128();
            let mut r = _mm_setzero_si128();
            for (half, yv) in [(0usize, y_lo), (1usize, y_hi)] {
                let u_p = if half == 0 {
                    _mm256_permutevar8x32_ps(u_f, dup_lo)
                } else {
                    _mm256_permutevar8x32_ps(u_f, dup_hi)
                };
                let v_p = if half == 0 {
                    _mm256_permutevar8x32_ps(v_f, dup_lo)
                } else {
                    _mm256_permutevar8x32_ps(v_f, dup_hi)
                };
                let yv = _mm256_mul_ps(_mm256_sub_ps(yv, luma_bias), y_scale);
                let bb = _mm256_add_ps(yv, _mm256_mul_ps(cb_b, u_p));
                let gg = _mm256_sub_ps(
                    _mm256_sub_ps(yv, _mm256_mul_ps(cb_g, u_p)),
                    _mm256_mul_ps(cr_g, v_p),
                );
                let rr = _mm256_add_ps(yv, _mm256_mul_ps(cr_r, v_p));
                let bb = lanes_to_u8(bb, zero, max255, pack_ctrl);
                let gg = lanes_to_u8(gg, zero, max255, pack_ctrl);
                let rr = lanes_to_u8(rr, zero, max255, pack_ctrl);
                if half == 0 {
                    b = bb;
                    g = gg;
                    r = rr;
                } else {
                    b = _mm_unpacklo_epi64(b, bb);
                    g = _mm_unpacklo_epi64(g, gg);
                    r = _mm_unpacklo_epi64(r, rr);
                }
            }

            let bg_lo = _mm_unpacklo_epi8(b, g);
            let bg_hi = _mm_unpackhi_epi8(b, g);
            let ra_lo = _mm_unpacklo_epi8(r, alpha);
            let ra_hi = _mm_unpackhi_epi8(r, alpha);
            let out = dst.as_mut_ptr().add(row_dst + x * 4);
            _mm_storeu_si128(out.cast(), _mm_unpacklo_epi16(bg_lo, ra_lo));
            _mm_storeu_si128(out.add(16).cast(), _mm_unpackhi_epi16(bg_lo, ra_lo));
            _mm_storeu_si128(out.add(32).cast(), _mm_unpacklo_epi16(bg_hi, ra_hi));
            _mm_storeu_si128(out.add(48).cast(), _mm_unpackhi_epi16(bg_hi, ra_hi));
            x += 16;
        }

        if x < w {
            // A truncated source or destination ends the row at the same
            // sample the scalar path stops at; a row that starts past the
            // buffer is left untouched, as the old per-pixel loop did.
            let si = row_src + x * 2;
            let di = row_dst + x * 4;
            if si < src.len() && di < dst.len() {
                scalar::yuv422_to_bgra(
                    &src[si..],
                    (w - x) as u32,
                    1,
                    stride,
                    uyvy,
                    &mut dst[di..],
                );
            }
        }
    }
    _mm256_zeroupper();
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

/// BGRA -> RGBA repack with independent source/destination strides, 16 bytes
/// (4 pixels) per iteration. Pure byte permutation, so it is byte-identical to
/// the scalar loop by construction.
#[target_feature(enable = "ssse3")]
unsafe fn bgra_to_rgba_rows_ssse3(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    width: u32,
    height: u32,
) {
    let row_bytes = (width as usize) * 4;
    let mask = _mm_setr_epi8(2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15);
    for y in 0..height as usize {
        let s = y * src_stride;
        let d = y * dst_stride;
        if s >= src.len() || d >= dst.len() {
            continue;
        }
        let n = row_bytes.min(src.len() - s).min(dst.len() - d) / 4 * 4;
        let mut x = 0usize;
        while x + 16 <= n {
            let v = _mm_loadu_si128(src.as_ptr().add(s + x).cast());
            _mm_storeu_si128(
                dst.as_mut_ptr().add(d + x).cast(),
                _mm_shuffle_epi8(v, mask),
            );
            x += 16;
        }
        while x + 4 <= n {
            let o = s + x;
            let p = d + x;
            dst[p] = src[o + 2];
            dst[p + 1] = src[o + 1];
            dst[p + 2] = src[o];
            dst[p + 3] = src[o + 3];
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
        _mm256_storeu_ps(
            dest.as_mut_ptr().add(i),
            _mm256_add_ps(d, _mm256_mul_ps(s, g)),
        );
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
