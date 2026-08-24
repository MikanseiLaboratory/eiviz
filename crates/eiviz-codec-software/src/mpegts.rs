use crate::h264;
use eiviz_media::EncodedAccessUnit;

pub fn pat() -> [u8; 188] {
    let mut pkt = [0xffu8; 188];
    pkt[0] = 0x47;
    pkt[1] = 0x40;
    pkt[2] = 0x00;
    pkt[3] = 0x10;
    pkt[4] = 0x00;
    let mut payload = vec![
        0x00, 0xb0, 0x0d, 0x00, 0x01, 0xc1, 0x00, 0x00, 0x00, 0x01, 0xe1, 0x00,
    ];
    let crc = mpeg_crc32(&payload[1..]);
    payload.extend_from_slice(&crc.to_be_bytes());
    pkt[5..5 + payload.len()].copy_from_slice(&payload);
    pkt
}

pub fn pmt() -> [u8; 188] {
    let mut pkt = [0xffu8; 188];
    pkt[0] = 0x47;
    pkt[1] = 0x41;
    pkt[2] = 0x00;
    pkt[3] = 0x10;
    pkt[4] = 0x00;
    let mut section = vec![
        0x02, 0xb0, 0x12, 0x00, 0x01, 0xc1, 0x00, 0x00, 0xe1, 0x00, 0xf0, 0x00, 0x1b, 0xe1, 0x00,
        0xf0, 0x00,
    ];
    let crc = mpeg_crc32(&section[1..]);
    section.extend_from_slice(&crc.to_be_bytes());
    pkt[5..5 + section.len()].copy_from_slice(&section);
    pkt
}

pub fn pes_video(au: &EncodedAccessUnit, pts_90k: u64, cc: &mut u8) -> Vec<[u8; 188]> {
    let mut payload = Vec::new();
    for nal in h264::split_annexb(&au.bytes) {
        payload.extend_from_slice(&[0, 0, 0, 1]);
        payload.extend_from_slice(&nal);
    }
    let mut pes = vec![0x00, 0x00, 0x01, 0xe0, 0x00, 0x00, 0x80, 0x80, 0x05];
    pes.extend_from_slice(&pts_bytes(pts_90k));
    pes.extend_from_slice(&payload);
    packetize(0x0100, &pes, cc, true)
}

fn packetize(pid: u16, data: &[u8], cc: &mut u8, start: bool) -> Vec<[u8; 188]> {
    let mut out = Vec::new();
    let mut off = 0;
    let mut first = start;
    while off < data.len() {
        let mut pkt = [0xffu8; 188];
        pkt[0] = 0x47;
        let mut b1 = ((pid >> 8) as u8) & 0x1f;
        if first {
            b1 |= 0x40;
            first = false;
        }
        pkt[1] = b1;
        pkt[2] = pid as u8;
        *cc = (*cc + 1) & 0x0f;
        pkt[3] = 0x10 | *cc;
        let n = (188 - 4).min(data.len() - off);
        pkt[4..4 + n].copy_from_slice(&data[off..off + n]);
        off += n;
        out.push(pkt);
    }
    out
}

fn pts_bytes(pts: u64) -> [u8; 5] {
    [
        0x21 | (((pts >> 30) as u8 & 0x07) << 1),
        ((pts >> 22) as u8),
        0x01 | (((pts >> 15) as u8) << 1),
        ((pts >> 7) as u8),
        0x01 | ((pts as u8) << 1),
    ]
}

fn mpeg_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffffffffu32;
    for &b in data {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            if crc & 0x80000000 != 0 {
                crc = (crc << 1) ^ 0x04c11db7;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}
