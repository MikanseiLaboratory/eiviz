use crate::upload::{AUDIO_RATE, AudioPacket};

pub fn sine_packet(phase: &mut f64, hz: f32, level_dbfs: f32, frames: usize, pts: i64) -> AudioPacket {
    let amplitude = 10f32.powf(level_dbfs.clamp(-120.0, 0.0) / 20.0);
    let rate = f64::from(AUDIO_RATE);
    let mut left = vec![0.0f32; frames];
    crate::simd::sine_fill(&mut left, *phase, hz, amplitude, rate);
    let mut pcm = Vec::with_capacity(frames * 2);
    pcm.extend_from_slice(&left);
    pcm.extend_from_slice(&left);
    *phase += frames as f64;
    if *phase >= rate {
        *phase %= rate;
    }
    AudioPacket {
        timestamp: pts,
        sample_rate: AUDIO_RATE,
        channels: 2,
        samples_per_channel: frames as i32,
        pcm_planar_f32: pcm,
    }
}
