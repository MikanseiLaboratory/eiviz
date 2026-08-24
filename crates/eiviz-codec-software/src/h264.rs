use crate::bitstream::{BitWriter, annexb};
use eiviz_media::{EncodedAccessUnit, EncodedKind, VideoFrame};

/// Baseline IDR with I_PCM macroblocks. Valid (uncompressed) H.264.
pub fn encode_idr(frame: &VideoFrame) -> EncodedAccessUnit {
    let mb_w = frame.width.div_ceil(16);
    let mb_h = frame.height.div_ceil(16);
    let pad_w = mb_w * 16;
    let pad_h = mb_h * 16;
    let mut yuv = vec![0u8; (pad_w * pad_h * 3 / 2) as usize];
    rgba_to_yuv420(
        &frame.data,
        frame.width,
        frame.height,
        pad_w,
        pad_h,
        &mut yuv,
    );
    let crop_r = pad_w - frame.width;
    let crop_b = pad_h - frame.height;
    let sps = sps(mb_w, mb_h, crop_r, crop_b);
    let pps = pps();
    let slice = idr_slice(&yuv, mb_w, mb_h, pad_w);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&annexb(7, 3, &sps));
    bytes.extend_from_slice(&annexb(8, 3, &pps));
    bytes.extend_from_slice(&annexb(5, 3, &slice));
    EncodedAccessUnit {
        pts: frame.pts,
        dts: Some(frame.pts),
        keyframe: true,
        bytes,
        kind: EncodedKind::Avc,
    }
}

pub fn extract_sps_pps(au: &EncodedAccessUnit) -> (Vec<u8>, Vec<u8>) {
    let mut sps = Vec::new();
    let mut pps = Vec::new();
    for nal in split_annexb(&au.bytes) {
        if nal.is_empty() {
            continue;
        }
        match nal[0] & 0x1f {
            7 => sps = nal,
            8 => pps = nal,
            _ => {}
        }
    }
    (sps, pps)
}

pub fn split_annexb(buf: &[u8]) -> Vec<Vec<u8>> {
    let mut idx = Vec::new();
    let mut i = 0;
    while i + 2 < buf.len() {
        if buf[i..].starts_with(&[0, 0, 0, 1]) {
            idx.push(i + 4);
            i += 4;
        } else if buf[i..].starts_with(&[0, 0, 1]) {
            idx.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    for (n, start) in idx.iter().copied().enumerate() {
        let end = idx.get(n + 1).copied().unwrap_or(buf.len());
        let mut e = end;
        while e > start + 3
            && (buf[e - 4..e].starts_with(&[0, 0, 0, 1]) || buf[e - 3..e].starts_with(&[0, 0, 1]))
        {
            e -= if buf[e - 4..e].starts_with(&[0, 0, 0, 1]) {
                4
            } else {
                3
            };
        }
        if start < buf.len() {
            out.push(buf[start..start.max(e.min(buf.len()))].to_vec());
        }
    }
    // simpler: scan again
    let mut nals = Vec::new();
    let mut starts = Vec::new();
    i = 0;
    while i + 3 <= buf.len() {
        if i + 4 <= buf.len() && buf[i..i + 4] == [0, 0, 0, 1] {
            starts.push(i + 4);
            i += 4;
        } else if buf[i..i + 3] == [0, 0, 1] {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    for (n, s) in starts.iter().copied().enumerate() {
        let e = starts
            .get(n + 1)
            .map(|next| {
                let mut p = *next;
                if p >= 4 && buf[p - 4..p] == [0, 0, 0, 1] {
                    p -= 4;
                } else if p >= 3 && buf[p - 3..p] == [0, 0, 1] {
                    p -= 3;
                }
                p
            })
            .unwrap_or(buf.len());
        if s < e {
            nals.push(buf[s..e].to_vec());
        }
    }
    let _ = out;
    nals
}

fn sps(mb_w: u32, mb_h: u32, crop_r: u32, crop_b: u32) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(66, 8);
    w.write_bits(0, 8);
    w.write_bits(31, 8);
    w.write_ue(0);
    w.write_ue(0);
    w.write_ue(0);
    w.write_ue(0);
    w.write_ue(0);
    w.write_bit(0);
    w.write_ue(mb_w.saturating_sub(1));
    w.write_ue(mb_h.saturating_sub(1));
    w.write_bit(1);
    w.write_bit(1);
    if crop_r > 0 || crop_b > 0 {
        w.write_bit(1);
        w.write_ue(0);
        w.write_ue(crop_r * 2);
        w.write_ue(0);
        w.write_ue(crop_b * 2);
    } else {
        w.write_bit(0);
    }
    w.write_bit(0);
    w.into_rbsp()
}

fn pps() -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0);
    w.write_ue(0);
    w.write_bit(0);
    w.write_bit(0);
    w.write_ue(0);
    w.write_ue(0);
    w.write_ue(0);
    w.write_bit(0);
    w.write_bits(0, 2);
    w.write_se(0);
    w.write_se(0);
    w.write_se(0);
    w.write_bit(0);
    w.write_bit(0);
    w.write_bit(0);
    w.into_rbsp()
}

fn idr_slice(yuv: &[u8], mb_w: u32, mb_h: u32, stride_y: u32) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0);
    w.write_ue(2);
    w.write_ue(0);
    w.write_bits(0, 4);
    w.write_ue(0);
    w.write_bits(0, 4);
    w.write_bit(0);
    w.write_bit(0);
    w.write_se(0);
    let y_size = (stride_y * mb_h * 16) as usize;
    let u_off = y_size;
    let v_off = y_size + (stride_y * mb_h * 16 / 4) as usize;
    for my in 0..mb_h {
        for mx in 0..mb_w {
            w.write_ue(25);
            w.align_byte();
            let x0 = mx * 16;
            let y0 = my * 16;
            for yy in 0..16u32 {
                let row = ((y0 + yy) * stride_y + x0) as usize;
                w.bytes.extend_from_slice(&yuv[row..row + 16]);
            }
            let cstride = stride_y / 2;
            for base in [u_off, v_off] {
                for yy in 0..8u32 {
                    let row = base + ((y0 / 2 + yy) * cstride + x0 / 2) as usize;
                    w.bytes.extend_from_slice(&yuv[row..row + 8]);
                }
            }
        }
    }
    w.rbsp_trailing();
    w.bytes
}

fn rgba_to_yuv420(rgba: &[u8], w: u32, h: u32, pw: u32, ph: u32, out: &mut [u8]) {
    let y_size = (pw * ph) as usize;
    let c_size = (pw * ph / 4) as usize;
    for y in 0..ph {
        for x in 0..pw {
            let sx = x.min(w.saturating_sub(1));
            let sy = y.min(h.saturating_sub(1));
            let i = ((sy * w + sx) * 4) as usize;
            let r = rgba[i] as i32;
            let g = rgba[i + 1] as i32;
            let b = rgba[i + 2] as i32;
            let yv = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
            out[(y * pw + x) as usize] = yv.clamp(0, 255) as u8;
        }
    }
    for y in 0..ph / 2 {
        for x in 0..pw / 2 {
            let sx = (x * 2).min(w.saturating_sub(1));
            let sy = (y * 2).min(h.saturating_sub(1));
            let i = ((sy * w + sx) * 4) as usize;
            let r = rgba[i] as i32;
            let g = rgba[i + 1] as i32;
            let b = rgba[i + 2] as i32;
            let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
            let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
            out[y_size + (y * (pw / 2) + x) as usize] = u.clamp(0, 255) as u8;
            out[y_size + c_size + (y * (pw / 2) + x) as usize] = v.clamp(0, 255) as u8;
        }
    }
}
