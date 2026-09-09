use std::collections::HashMap;

use crate::upload::AudioPacket;

pub(super) fn mix_mapped_f32(
    dest: &mut [f32],
    channels: usize,
    mapped: &HashMap<(i32, i32), Vec<(f32, f32)>>,
) {
    dest.fill(0.0);
    let channels = channels.max(1);
    for ((left, right), stereo) in mapped {
        let map_left = (*left).max(0) as usize;
        let map_right = (*right).max(0) as usize;
        for (i, (sl, sr)) in stereo.iter().enumerate() {
            let base = i * channels;
            if map_left < channels {
                if let Some(slot) = dest.get_mut(base + map_left) {
                    *slot += sl.clamp(-1.0, 1.0);
                }
            }
            if map_right != map_left && map_right < channels {
                if let Some(slot) = dest.get_mut(base + map_right) {
                    *slot += sr.clamp(-1.0, 1.0);
                }
            }
        }
    }
}

pub(super) fn f32_to_i16(src: &[f32], dest: &mut [i16]) {
    for (sample, slot) in src.iter().zip(dest.iter_mut()) {
        *slot = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
    }
}

pub(super) fn f32_to_i32(src: &[f32], dest: &mut [i32]) {
    for (sample, slot) in src.iter().zip(dest.iter_mut()) {
        *slot = (sample.clamp(-1.0, 1.0) * 2_147_483_647.0) as i32;
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn silent_packet(timestamp: i64, sample_rate: i32, frames: u32) -> AudioPacket {
    AudioPacket {
        timestamp,
        sample_rate,
        channels: 2,
        samples_per_channel: frames as i32,
        pcm_planar_f32: vec![0.0; frames as usize * 2],
    }
}

pub(super) fn interleaved_f32_packet(
    timestamp: i64,
    sample_rate: i32,
    interleaved: &[f32],
    channels: usize,
    map_left: usize,
    map_right: usize,
) -> AudioPacket {
    let channels = channels.max(1);
    let frames = interleaved.len() / channels;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    for i in 0..frames {
        let base = i * channels;
        if map_left < channels {
            left[i] = interleaved[base + map_left];
        }
        if map_right < channels {
            right[i] = interleaved[base + map_right];
        }
    }
    let mut pcm = left;
    pcm.extend(right);
    AudioPacket {
        timestamp,
        sample_rate,
        channels: 2,
        samples_per_channel: frames as i32,
        pcm_planar_f32: pcm,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn mapped_packet(
    timestamp: i64,
    sample_rate: i32,
    frames: u32,
    src: &[u8],
    channels: usize,
    bits: u16,
    float: bool,
    map_left: usize,
    map_right: usize,
) -> AudioPacket {
    let frames = frames as usize;
    let sample_bytes = (bits as usize / 8).max(1);
    let frame_bytes = channels * sample_bytes;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    for i in 0..frames {
        let base = i * frame_bytes;
        if base + frame_bytes > src.len() {
            break;
        }
        left[i] = read_sample(src, base, channels, sample_bytes, float, bits, map_left);
        right[i] = read_sample(src, base, channels, sample_bytes, float, bits, map_right);
    }
    let mut pcm = left;
    pcm.extend(right);
    AudioPacket {
        timestamp,
        sample_rate,
        channels: 2,
        samples_per_channel: frames as i32,
        pcm_planar_f32: pcm,
    }
}

fn read_sample(
    src: &[u8],
    base: usize,
    channels: usize,
    sample_bytes: usize,
    float: bool,
    bits: u16,
    channel: usize,
) -> f32 {
    if channel >= channels {
        return 0.0;
    }
    let offset = base + channel * sample_bytes;
    if offset + sample_bytes > src.len() {
        return 0.0;
    }
    if float && sample_bytes == 4 {
        return f32::from_le_bytes([
            src[offset],
            src[offset + 1],
            src[offset + 2],
            src[offset + 3],
        ]);
    }
    match bits {
        16 => {
            let value = i16::from_le_bytes([src[offset], src[offset + 1]]);
            value as f32 / 32768.0
        }
        24 if sample_bytes >= 3 => {
            let value =
                i32::from_le_bytes([src[offset], src[offset + 1], src[offset + 2], 0]) << 8 >> 8;
            value as f32 / 8_388_608.0
        }
        32 => {
            let value = i32::from_le_bytes([
                src[offset],
                src[offset + 1],
                src[offset + 2],
                src[offset + 3],
            ]);
            value as f32 / 2_147_483_648.0
        }
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{interleaved_f32_packet, mapped_packet, silent_packet};
    use crate::upload::AUDIO_RATE;

    #[test]
    fn silent_packet_is_stereo_planar() {
        let packet = silent_packet(10, AUDIO_RATE, 4);
        assert_eq!(packet.channels, 2);
        assert_eq!(packet.samples_per_channel, 4);
        assert_eq!(packet.pcm_planar_f32.len(), 8);
        assert!(packet.pcm_planar_f32.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn mapped_packet_reads_float_interleaved() {
        let mut src = Vec::new();
        for frame in 0..2u32 {
            src.extend((frame as f32).to_le_bytes());
            src.extend((frame as f32 + 0.5).to_le_bytes());
        }
        let packet = mapped_packet(0, 48_000, 2, &src, 2, 32, true, 0, 1);
        assert_eq!(packet.pcm_planar_f32, vec![0.0, 1.0, 0.5, 1.5]);
    }

    #[test]
    fn interleaved_f32_packet_maps_channels() {
        let packet = interleaved_f32_packet(0, 48_000, &[0.25, 0.5, 0.75, 1.0], 2, 0, 1);
        assert_eq!(packet.pcm_planar_f32, vec![0.25, 0.75, 0.5, 1.0]);
    }

    #[test]
    fn mix_mapped_f32_writes_stereo() {
        let mut dest = [0.0f32; 4];
        let mut mapped = HashMap::new();
        mapped.insert((0, 1), vec![(0.5, -0.5), (1.0, -1.0)]);
        super::mix_mapped_f32(&mut dest, 2, &mapped);
        assert_eq!(dest, [0.5, -0.5, 1.0, -1.0]);
    }
}
