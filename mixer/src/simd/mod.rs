mod dispatch;
mod scalar;

#[cfg(target_arch = "x86_64")]
mod x86;
#[cfg(target_arch = "aarch64")]
mod neon;

pub use dispatch::{path, SimdPath};

pub fn yuv422_to_bgra(src: &[u8], width: u32, height: u32, stride: usize, uyvy: bool, dst: &mut [u8]) {
    #[cfg(target_arch = "x86_64")]
    {
        x86::yuv422_to_bgra(src, width, height, stride, uyvy, dst);
        return;
    }
    #[cfg(target_arch = "aarch64")]
    {
        neon::yuv422_to_bgra(src, width, height, stride, uyvy, dst);
        return;
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    scalar::yuv422_to_bgra(src, width, height, stride, uyvy, dst);
}

pub fn yuy2_to_uyvy(src: &[u8], width: u32, height: u32, stride: usize, dst: &mut [u8]) {
    #[cfg(target_arch = "x86_64")]
    {
        x86::yuy2_to_uyvy(src, width, height, stride, dst);
        return;
    }
    #[cfg(target_arch = "aarch64")]
    {
        neon::yuy2_to_uyvy(src, width, height, stride, dst);
        return;
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    scalar::yuy2_to_uyvy(src, width, height, stride, dst);
}

pub fn or_opaque_bgra(pixels: &mut [u8]) {
    #[cfg(target_arch = "x86_64")]
    {
        x86::or_opaque_bgra(pixels);
        return;
    }
    #[cfg(target_arch = "aarch64")]
    {
        neon::or_opaque_bgra(pixels);
        return;
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    scalar::or_opaque_bgra(pixels);
}

pub fn mix_stereo_gain(dest: &mut [f32], src: &[f32], gain: f32) {
    #[cfg(target_arch = "x86_64")]
    {
        x86::mix_stereo_gain(dest, src, gain);
        return;
    }
    #[cfg(target_arch = "aarch64")]
    {
        neon::mix_stereo_gain(dest, src, gain);
        return;
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    scalar::mix_stereo_gain(dest, src, gain);
}

pub fn scale_f32(samples: &mut [f32], gain: f32) {
    #[cfg(target_arch = "x86_64")]
    {
        x86::scale_f32(samples, gain);
        return;
    }
    #[cfg(target_arch = "aarch64")]
    {
        neon::scale_f32(samples, gain);
        return;
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    scalar::scale_f32(samples, gain);
}

pub fn peak_f32(samples: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        return x86::peak_f32(samples);
    }
    #[cfg(target_arch = "aarch64")]
    {
        return neon::peak_f32(samples);
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    scalar::peak_f32(samples)
}

pub fn peak_interleaved(samples: &[f32]) -> (f32, f32) {
    scalar::peak_interleaved(samples)
}

pub fn sine_fill(out: &mut [f32], phase: f64, hz: f32, amplitude: f32, rate: f64) {
    scalar::sine_fill(out, phase, hz, amplitude, rate);
}

pub fn planar_to_stereo(planar: &[f32], frames: usize, channels: usize, out: &mut Vec<f32>) {
    scalar::planar_to_stereo(planar, frames, channels, out);
}

pub fn resample_planar_to_stereo(
    planar: &[f32],
    src_frames: usize,
    channels: usize,
    src_rate: usize,
    dst_rate: usize,
    out: &mut Vec<f32>,
) {
    scalar::resample_planar_to_stereo(planar, src_frames, channels, src_rate, dst_rate, out);
}

pub fn resample_stereo(src: &[f32], src_rate: u32, dst_frames: usize, dst_rate: u32, out: &mut [f32]) {
    scalar::resample_stereo(src, src_rate, dst_frames, dst_rate, out);
}

pub fn blend_u8(bg: u8, fg: u8, cover: u16) -> u8 {
    scalar::blend_u8(bg, fg, cover)
}

pub fn copy_rows(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    row_bytes: usize,
    rows: usize,
) {
    scalar::copy_rows(src, src_stride, dst, dst_stride, row_bytes, rows);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packed_yuy2(width: usize, height: usize) -> Vec<u8> {
        let mut src = vec![0u8; width * height * 2];
        for (i, b) in src.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        src
    }

    #[test]
    fn yuy2_shuffle_matches_scalar() {
        let w = 64u32;
        let h = 8u32;
        let src = packed_yuy2(w as usize, h as usize);
        let mut simd = vec![0u8; (w * h * 2) as usize];
        let mut scalar_out = vec![0u8; simd.len()];
        yuy2_to_uyvy(&src, w, h, (w * 2) as usize, &mut simd);
        scalar::yuy2_to_uyvy(&src, w, h, (w * 2) as usize, &mut scalar_out);
        assert_eq!(simd, scalar_out);
    }

    #[test]
    fn yuv_matches_scalar() {
        let w = 32u32;
        let h = 4u32;
        let src = packed_yuy2(w as usize, h as usize);
        let mut simd = vec![0u8; (w * h * 4) as usize];
        let mut scalar_out = vec![0u8; simd.len()];
        yuv422_to_bgra(&src, w, h, (w * 2) as usize, false, &mut simd);
        scalar::yuv422_to_bgra(&src, w, h, (w * 2) as usize, false, &mut scalar_out);
        assert_eq!(simd, scalar_out);
    }

    #[test]
    fn opaque_or_matches_scalar() {
        let mut simd = vec![0x11u8; 64];
        let mut scalar_out = simd.clone();
        or_opaque_bgra(&mut simd);
        scalar::or_opaque_bgra(&mut scalar_out);
        assert_eq!(simd, scalar_out);
        assert!(simd.chunks_exact(4).all(|px| px[3] == 0xFF));
    }

    #[test]
    fn mix_gain_matches_scalar() {
        let src: Vec<f32> = (0..64).map(|i| i as f32 * 0.01).collect();
        let mut simd = vec![0.5f32; 64];
        let mut scalar_out = vec![0.5f32; 64];
        mix_stereo_gain(&mut simd, &src, 0.25);
        scalar::mix_stereo_gain(&mut scalar_out, &src, 0.25);
        for (a, b) in simd.iter().zip(scalar_out.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn peak_matches_scalar() {
        let samples: Vec<f32> = (0..127).map(|i| (i as f32 - 60.0) * 0.01).collect();
        let a = peak_f32(&samples);
        let b = scalar::peak_f32(&samples);
        assert!((a - b).abs() < 1e-6);
    }
}
