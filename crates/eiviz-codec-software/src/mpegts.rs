use crate::h264;
use eiviz_media::EncodedAccessUnit;

pub const PMT_PID: u16 = 0x1000;
pub const VIDEO_PID: u16 = 0x0100;
pub const AUDIO_PID: u16 = 0x0101;

pub fn pat() -> [u8; 188] {
    let mut section = vec![
        0x00,
        0xb0,
        0x0d,
        0x00,
        0x01,
        0xc1,
        0x00,
        0x00,
        0x00,
        0x01,
        0xe0 | ((PMT_PID >> 8) as u8 & 0x1f),
        PMT_PID as u8,
    ];
    append_crc(&mut section);
    psi_packet(0, &section)
}

pub fn pmt() -> [u8; 188] {
    let mut section = vec![
        0x02,
        0xb0,
        0x17, // section length: fixed fields + two streams + CRC
        0x00,
        0x01,
        0xc1,
        0x00,
        0x00,
        0xe0 | ((VIDEO_PID >> 8) as u8 & 0x1f),
        VIDEO_PID as u8,
        0xf0,
        0x00,
        0x1b, // AVC
        0xe0 | ((VIDEO_PID >> 8) as u8 & 0x1f),
        VIDEO_PID as u8,
        0xf0,
        0x00,
        0x0f, // AAC with ADTS
        0xe0 | ((AUDIO_PID >> 8) as u8 & 0x1f),
        AUDIO_PID as u8,
        0xf0,
        0x00,
    ];
    append_crc(&mut section);
    psi_packet(PMT_PID, &section)
}

pub fn pes_video(au: &EncodedAccessUnit, pts_90k: u64, cc: &mut u8) -> Vec<[u8; 188]> {
    let mut payload = Vec::new();
    for nal in h264::split_annexb(&au.bytes) {
        payload.extend_from_slice(&[0, 0, 0, 1]);
        payload.extend_from_slice(&nal);
    }
    let pes = pes(0xe0, pts_90k, &payload);
    packetize(VIDEO_PID, &pes, cc)
}

pub fn pes_aac(
    au: &EncodedAccessUnit,
    pts_90k: u64,
    sample_rate: u32,
    channels: u16,
    cc: &mut u8,
) -> Result<Vec<[u8; 188]>, &'static str> {
    let mut payload = adts_header(au.bytes.len(), sample_rate, channels)?.to_vec();
    payload.extend_from_slice(&au.bytes);
    let pes = pes(0xc0, pts_90k, &payload);
    Ok(packetize(AUDIO_PID, &pes, cc))
}

pub fn media_time_90k(time: eiviz_time::MediaTime) -> u64 {
    let base = time.timebase();
    let value =
        time.ticks() as i128 * base.numerator() as i128 * 90_000 / base.denominator() as i128;
    value.max(0).min(u64::MAX as i128) as u64
}

fn adts_header(
    payload_len: usize,
    sample_rate: u32,
    channels: u16,
) -> Result<[u8; 7], &'static str> {
    let frequency_index = match sample_rate {
        96000 => 0,
        88200 => 1,
        64000 => 2,
        48000 => 3,
        44100 => 4,
        32000 => 5,
        24000 => 6,
        22050 => 7,
        16000 => 8,
        12000 => 9,
        11025 => 10,
        8000 => 11,
        7350 => 12,
        _ => return Err("unsupported AAC sample rate for ADTS"),
    };
    if channels == 0 || channels > 7 {
        return Err("ADTS supports channel configurations 1 through 7");
    }
    let frame_len = payload_len
        .checked_add(7)
        .filter(|length| *length <= 0x1fff)
        .ok_or("AAC access unit is too large for ADTS")?;
    let profile = 1u8; // AAC Low Complexity: object type minus one.
    Ok([
        0xff,
        0xf1,
        (profile << 6) | ((frequency_index as u8) << 2) | ((channels >> 2) as u8),
        (((channels & 3) as u8) << 6) | ((frame_len >> 11) as u8),
        (frame_len >> 3) as u8,
        ((frame_len as u8 & 7) << 5) | 0x1f,
        0xfc,
    ])
}

fn pes(stream_id: u8, pts: u64, payload: &[u8]) -> Vec<u8> {
    let pes_length = payload.len().saturating_add(8);
    let encoded_length = u16::try_from(pes_length).unwrap_or(0);
    let mut packet = vec![0x00, 0x00, 0x01, stream_id];
    packet.extend_from_slice(&encoded_length.to_be_bytes());
    packet.extend_from_slice(&[0x80, 0x80, 0x05]);
    packet.extend_from_slice(&pts_bytes(pts));
    packet.extend_from_slice(payload);
    packet
}

fn packetize(pid: u16, data: &[u8], cc: &mut u8) -> Vec<[u8; 188]> {
    let mut output = Vec::new();
    let mut offset = 0;
    let mut first = true;
    while offset < data.len() {
        let remaining = data.len() - offset;
        let payload_len = remaining.min(184);
        let needs_adaptation = payload_len < 184;
        let mut packet = [0xff; 188];
        packet[0] = 0x47;
        packet[1] = ((pid >> 8) as u8) & 0x1f;
        if first {
            packet[1] |= 0x40;
            first = false;
        }
        packet[2] = pid as u8;
        packet[3] = if needs_adaptation { 0x30 } else { 0x10 } | (*cc & 0x0f);
        *cc = (*cc + 1) & 0x0f;

        let payload_start = if needs_adaptation {
            let adaptation_len = 183 - payload_len;
            packet[4] = adaptation_len as u8;
            if adaptation_len > 0 {
                packet[5] = 0;
            }
            5 + adaptation_len
        } else {
            4
        };
        packet[payload_start..payload_start + payload_len]
            .copy_from_slice(&data[offset..offset + payload_len]);
        offset += payload_len;
        output.push(packet);
    }
    output
}

fn psi_packet(pid: u16, section: &[u8]) -> [u8; 188] {
    let mut packet = [0xff; 188];
    packet[0] = 0x47;
    packet[1] = 0x40 | (((pid >> 8) as u8) & 0x1f);
    packet[2] = pid as u8;
    packet[3] = 0x10;
    packet[4] = 0; // pointer field
    packet[5..5 + section.len()].copy_from_slice(section);
    packet
}

fn pts_bytes(pts: u64) -> [u8; 5] {
    let pts = pts & ((1u64 << 33) - 1);
    [
        0x20 | (((pts >> 30) as u8 & 0x07) << 1) | 1,
        (pts >> 22) as u8,
        (((pts >> 15) as u8 & 0x7f) << 1) | 1,
        (pts >> 7) as u8,
        ((pts as u8 & 0x7f) << 1) | 1,
    ]
}

fn append_crc(section: &mut Vec<u8>) {
    let crc = mpeg_crc32(section);
    section.extend_from_slice(&crc.to_be_bytes());
}

fn mpeg_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= (byte as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pmt_advertises_avc_and_aac() {
        let packet = pmt();
        assert_eq!(packet[0], 0x47);
        assert!(packet.windows(5).any(|bytes| bytes[0] == 0x1b));
        assert!(packet.windows(5).any(|bytes| bytes[0] == 0x0f));
    }

    #[test]
    fn final_packet_uses_adaptation_stuffing() {
        let mut cc = 0;
        let au = EncodedAccessUnit {
            pts: eiviz_time::MediaTime::ZERO,
            dts: Some(eiviz_time::MediaTime::ZERO),
            keyframe: false,
            bytes: vec![1, 2, 3].into(),
            kind: eiviz_media::EncodedKind::Aac,
        };
        let packets = pes_aac(&au, 0, 48_000, 2, &mut cc).unwrap();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0][3] & 0x30, 0x30);
    }
}
