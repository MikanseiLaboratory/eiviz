use std::f64::consts::TAU;

use crate::upload::{AUDIO_RATE, AudioPacket};

pub fn sine_packet(phase: &mut f64, hz: f32, level_dbfs: f32, frames: usize, pts: i64) -> AudioPacket {
    let amplitude = 10f32.powf(level_dbfs.clamp(-120.0, 0.0) / 20.0);
    let rate = f64::from(AUDIO_RATE);
    let freq = f64::from(hz.max(0.0));
    let mut pcm = Vec::with_capacity(frames * 2 * 4);
    let start = *phase;
    for _channel in 0..2 {
        let mut p = start;
        for _ in 0..frames {
            let sample = (TAU * freq * p / rate).sin() as f32 * amplitude;
            pcm.extend_from_slice(&sample.to_le_bytes());
            p += 1.0;
            if p >= rate {
                p -= rate;
            }
        }
    }
    *phase = start + frames as f64;
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
