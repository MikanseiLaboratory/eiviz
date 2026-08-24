use crate::h264;
use eiviz_media::EncodedAccessUnit;

pub fn flv_header() -> Vec<u8> {
    let mut v = b"FLV\x01\x01".to_vec();
    v.extend_from_slice(&9u32.to_be_bytes());
    v.extend_from_slice(&0u32.to_be_bytes());
    v
}

pub fn flv_avc_sequence_header(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut avcc = vec![1, 66, 0, 31, 0xff, 0xe1];
    avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    avcc.extend_from_slice(sps);
    avcc.push(1);
    avcc.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    avcc.extend_from_slice(pps);
    video_tag(0, true, 0, &avcc)
}

pub fn flv_avc_nalu(au: &EncodedAccessUnit, dts_ms: u32) -> Vec<u8> {
    let mut payload = Vec::new();
    for nal in h264::split_annexb(&au.bytes) {
        if nal.is_empty() {
            continue;
        }
        let t = nal[0] & 0x1f;
        if t == 7 || t == 8 {
            continue;
        }
        payload.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        payload.extend_from_slice(&nal);
    }
    video_tag(dts_ms, au.keyframe, 1, &payload)
}

fn video_tag(dts_ms: u32, key: bool, avc_packet_type: u8, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(if key { 0x17 } else { 0x27 });
    body.push(avc_packet_type);
    body.extend_from_slice(&[0, 0, 0]);
    body.extend_from_slice(data);
    let mut tag = Vec::new();
    tag.push(9);
    tag.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    tag.extend_from_slice(&dts_ms.to_be_bytes()[1..]);
    tag.push(0);
    tag.extend_from_slice(&[0, 0, 0]);
    tag.extend_from_slice(&body);
    let prev = (tag.len() as u32).to_be_bytes();
    tag.extend_from_slice(&prev);
    tag
}
