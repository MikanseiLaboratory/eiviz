use crate::h264;
use eiviz_media::EncodedAccessUnit;

pub fn flv_header() -> Vec<u8> {
    let mut v = b"FLV\x01\x05".to_vec();
    v.extend_from_slice(&9u32.to_be_bytes());
    v.extend_from_slice(&0u32.to_be_bytes());
    v
}

pub fn flv_avc_sequence_header(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    video_tag_payload(0, avc_sequence_header_payload(sps, pps))
}

/// RTMP video-message payload for an AVC sequence header.
pub fn avc_sequence_header_payload(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let profile = sps.get(1).copied().unwrap_or(66);
    let compatibility = sps.get(2).copied().unwrap_or(0);
    let level = sps.get(3).copied().unwrap_or(31);
    let mut avcc = vec![1, profile, compatibility, level, 0xff, 0xe1];
    avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    avcc.extend_from_slice(sps);
    avcc.push(1);
    avcc.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    avcc.extend_from_slice(pps);
    let mut body = vec![0x17, 0, 0, 0, 0];
    body.extend_from_slice(&avcc);
    body
}

pub fn flv_avc_nalu(au: &EncodedAccessUnit, dts_ms: u32) -> Vec<u8> {
    video_tag_payload(dts_ms, avc_nalu_payload(au))
}

/// RTMP video-message payload for one Annex-B AVC access unit.
pub fn avc_nalu_payload(au: &EncodedAccessUnit) -> Vec<u8> {
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
    let composition_ms =
        media_time_ms(au.pts).saturating_sub(media_time_ms(au.dts.unwrap_or(au.pts)));
    let composition = composition_ms.clamp(-8_388_608, 8_388_607) as i32;
    let bytes = composition.to_be_bytes();
    let mut body = vec![if au.keyframe { 0x17 } else { 0x27 }, 1];
    body.extend_from_slice(&bytes[1..]);
    body.extend_from_slice(&payload);
    body
}

pub fn aac_sequence_header_payload(audio_specific_config: &[u8]) -> Vec<u8> {
    let mut body = vec![0xaf, 0];
    body.extend_from_slice(audio_specific_config);
    body
}

pub fn aac_raw_payload(au: &EncodedAccessUnit) -> Vec<u8> {
    let mut body = vec![0xaf, 1];
    body.extend_from_slice(&au.bytes);
    body
}

pub fn flv_aac_sequence_header(audio_specific_config: &[u8]) -> Vec<u8> {
    audio_tag(0, &aac_sequence_header_payload(audio_specific_config))
}

pub fn flv_aac_raw(au: &EncodedAccessUnit, timestamp_ms: u32) -> Vec<u8> {
    audio_tag(timestamp_ms, &aac_raw_payload(au))
}

fn video_tag_payload(dts_ms: u32, body: Vec<u8>) -> Vec<u8> {
    tag(9, dts_ms, &body)
}

fn audio_tag(timestamp_ms: u32, body: &[u8]) -> Vec<u8> {
    tag(8, timestamp_ms, body)
}

fn tag(kind: u8, timestamp_ms: u32, body: &[u8]) -> Vec<u8> {
    let mut tag = Vec::new();
    tag.push(kind);
    tag.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    tag.extend_from_slice(&timestamp_ms.to_be_bytes()[1..]);
    tag.push((timestamp_ms >> 24) as u8);
    tag.extend_from_slice(&[0, 0, 0]);
    tag.extend_from_slice(body);
    let prev = (tag.len() as u32).to_be_bytes();
    tag.extend_from_slice(&prev);
    tag
}

fn media_time_ms(time: eiviz_time::MediaTime) -> i64 {
    let ticks = time.ticks() as i128;
    let base = time.timebase();
    let value = ticks * base.numerator() as i128 * 1_000 / base.denominator() as i128;
    value.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}
