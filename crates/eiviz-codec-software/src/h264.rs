#[cfg(test)]
use crate::bitstream::{BitWriter, annexb};
use eiviz_media::EncodedAccessUnit;
#[cfg(test)]
use eiviz_media::{EncodedKind, VideoFrame};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AvccError {
    #[error("AVCC NAL length size must be 1, 2, or 4")]
    InvalidLengthSize,
    #[error("AVCC sample has a truncated NAL length")]
    TruncatedLength,
    #[error("AVCC NAL length is zero")]
    ZeroLength,
    #[error("AVCC NAL payload is truncated")]
    TruncatedNal,
    #[error("Annex-B output exceeds configured limit {0}")]
    OutputLimit(usize),
    #[error("AVC configuration has no {0}")]
    MissingParameterSet(&'static str),
}

const ANNEX_B_START_CODE: [u8; 4] = [0, 0, 0, 1];

/// Convert length-prefixed MP4 sample NALs to Annex-B with strict bounds.
pub fn avcc_sample_to_annexb(
    sample: &[u8],
    length_size: usize,
    max_output: usize,
) -> Result<Vec<u8>, AvccError> {
    if !matches!(length_size, 1 | 2 | 4) {
        return Err(AvccError::InvalidLengthSize);
    }
    let mut cursor = 0usize;
    let mut output = Vec::new();
    while cursor < sample.len() {
        let length_end = cursor
            .checked_add(length_size)
            .ok_or(AvccError::TruncatedLength)?;
        let length_bytes = sample
            .get(cursor..length_end)
            .ok_or(AvccError::TruncatedLength)?;
        let mut length = 0usize;
        for byte in length_bytes {
            length = length
                .checked_mul(256)
                .and_then(|value| value.checked_add(*byte as usize))
                .ok_or(AvccError::OutputLimit(max_output))?;
        }
        if length == 0 {
            return Err(AvccError::ZeroLength);
        }
        let nal_end = length_end
            .checked_add(length)
            .ok_or(AvccError::TruncatedNal)?;
        let nal = sample
            .get(length_end..nal_end)
            .ok_or(AvccError::TruncatedNal)?;
        append_annexb_nal(&mut output, nal, max_output)?;
        cursor = nal_end;
    }
    Ok(output)
}

/// Build the decoder reset preamble from out-of-band avcC SPS/PPS NALs.
pub fn avcc_parameter_sets_to_annexb(
    sequence_parameter_sets: &[Vec<u8>],
    picture_parameter_sets: &[Vec<u8>],
    max_output: usize,
) -> Result<Vec<u8>, AvccError> {
    if sequence_parameter_sets.is_empty() {
        return Err(AvccError::MissingParameterSet("SPS"));
    }
    if picture_parameter_sets.is_empty() {
        return Err(AvccError::MissingParameterSet("PPS"));
    }
    let mut output = Vec::new();
    for nal in sequence_parameter_sets
        .iter()
        .chain(picture_parameter_sets.iter())
    {
        if nal.is_empty() {
            return Err(AvccError::ZeroLength);
        }
        append_annexb_nal(&mut output, nal, max_output)?;
    }
    Ok(output)
}

fn append_annexb_nal(output: &mut Vec<u8>, nal: &[u8], max_output: usize) -> Result<(), AvccError> {
    let required = output
        .len()
        .checked_add(ANNEX_B_START_CODE.len())
        .and_then(|value| value.checked_add(nal.len()))
        .ok_or(AvccError::OutputLimit(max_output))?;
    if required > max_output {
        return Err(AvccError::OutputLimit(max_output));
    }
    output.extend_from_slice(&ANNEX_B_START_CODE);
    output.extend_from_slice(nal);
    Ok(())
}

/// Baseline IDR with I_PCM macroblocks. Valid (uncompressed) H.264.
#[cfg(test)]
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
        bytes: bytes.into(),
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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
