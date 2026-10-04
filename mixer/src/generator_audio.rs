use crate::upload::{AUDIO_RATE, AudioPacket};

/// One block of a stereo test tone. `phase` is the oscillator position in cycles
/// (`0.0..1.0`), so the wave stays continuous across blocks and across frequency changes.
pub fn sine_packet(
    phase: &mut f64,
    hz: f32,
    level_dbfs: f32,
    frames: usize,
    pts: i64,
) -> AudioPacket {
    let rate = f64::from(AUDIO_RATE);
    let hz = if hz.is_finite() {
        f64::from(hz).clamp(0.0, rate / 2.0)
    } else {
        0.0
    };
    let level = if level_dbfs.is_finite() {
        level_dbfs.clamp(-120.0, 0.0)
    } else {
        -120.0
    };
    let amplitude = 10f32.powf(level / 20.0);
    let step = hz / rate;
    let mut position = if phase.is_finite() {
        phase.rem_euclid(1.0)
    } else {
        0.0
    };
    let mut pcm = vec![0.0f32; frames * 2];
    let (left, right) = pcm.split_at_mut(frames);
    for (l, r) in left.iter_mut().zip(right.iter_mut()) {
        let sample = (std::f64::consts::TAU * position).sin() as f32 * amplitude;
        *l = sample;
        *r = sample;
        position += step;
        if position >= 1.0 {
            position -= 1.0;
        }
    }
    *phase = position;
    AudioPacket {
        timestamp: pts,
        sample_rate: AUDIO_RATE,
        channels: 2,
        samples_per_channel: frames as i32,
        pcm_planar_f32: pcm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractional_frequency_is_continuous_across_blocks() {
        let mut phase = 0.0;
        let mut samples = Vec::new();
        // 1234.5 Hz does not repeat within one second, the old sample-counter wrap glitched.
        for _ in 0..600 {
            let packet = sine_packet(&mut phase, 1234.5, -6.0, 480, 0);
            samples.extend_from_slice(&packet.pcm_planar_f32[..480]);
        }
        let worst = samples
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0f32, f32::max);
        // 1234.5 Hz at -6 dBFS moves at most ~0.2 per sample.
        assert!(worst < 0.25, "step {worst}");
    }

    #[test]
    fn frequency_change_keeps_the_waveform_continuous() {
        let mut phase = 0.0;
        let first = sine_packet(&mut phase, 440.0, 0.0, 333, 0);
        let second = sine_packet(&mut phase, 880.0, 0.0, 333, 0);
        let last = first.pcm_planar_f32[332];
        let next = second.pcm_planar_f32[0];
        assert!((next - last).abs() < 0.2, "{last} -> {next}");
    }

    #[test]
    fn non_finite_parameters_produce_silence() {
        let mut phase = f64::NAN;
        let packet = sine_packet(&mut phase, f32::NAN, f32::NAN, 64, 0);
        assert!(
            packet
                .pcm_planar_f32
                .iter()
                .all(|v| v.is_finite() && v.abs() < 1e-5)
        );
        assert!(phase.is_finite());
    }
}
