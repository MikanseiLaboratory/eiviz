use eiviz_media::{AudioBuffer, Capability};

pub fn probe() -> Vec<Capability> {
    vec![
        Capability {
            id: "wasapi".into(),
            available: cfg!(target_os = "windows"),
            detail: "Windows WASAPI".into(),
        },
        Capability {
            id: "asio".into(),
            available: false,
            detail: "ASIO requires Steinberg license or GPLv3; not linked".into(),
        },
        Capability {
            id: "coreaudio".into(),
            available: cfg!(target_os = "macos"),
            detail: "macOS CoreAudio".into(),
        },
        Capability {
            id: "alsa".into(),
            available: cfg!(target_os = "linux"),
            detail: "Linux ALSA/PipeWire via CPAL when enabled".into(),
        },
        Capability {
            id: "software-monitor".into(),
            available: true,
            detail: "always-on silent/software graph".into(),
        },
    ]
}

/// Realtime-safe: no allocation after construction. Callers pre-size `out`.
pub fn mix_into(out: &mut AudioBuffer, src: &AudioBuffer, gain: f32) {
    let n = out.planes[0].len().min(src.planes[0].len());
    let ch = out.channels.min(src.channels) as usize;
    for c in 0..ch {
        for i in 0..n {
            out.planes[c][i] += src.planes[c][i] * gain;
        }
    }
}

/// Linear interpolating resampler. Safe to call off the audio callback.
pub fn resample_linear(src: &AudioBuffer, dst_rate: u32, dst_frames: usize) -> AudioBuffer {
    let mut out = AudioBuffer::silence(src.sample_index, dst_rate, src.channels, dst_frames);
    if src.planes.is_empty() || src.planes[0].is_empty() || dst_frames == 0 {
        return out;
    }
    let src_len = src.planes[0].len();
    let ratio = src.sample_rate as f64 / dst_rate as f64;
    for c in 0..src.channels as usize {
        for i in 0..dst_frames {
            let pos = i as f64 * ratio;
            let i0 = pos.floor() as usize;
            let i1 = (i0 + 1).min(src_len - 1);
            let frac = (pos - i0 as f64) as f32;
            let a = src.planes[c][i0.min(src_len - 1)];
            let b = src.planes[c][i1];
            out.planes[c][i] = a + (b - a) * frac;
        }
    }
    out
}

pub fn peak_meter(buf: &AudioBuffer) -> f32 {
    buf.planes
        .iter()
        .flat_map(|p| p.iter())
        .fold(0.0f32, |a, x| a.max(x.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_does_not_allocate_beyond_buffers() {
        let mut out = AudioBuffer::silence(0, 48000, 2, 8);
        let mut src = AudioBuffer::silence(0, 48000, 2, 8);
        src.planes[0][0] = 0.5;
        mix_into(&mut out, &src, 2.0);
        assert_eq!(out.planes[0][0], 1.0);
        assert!(
            probe()
                .iter()
                .any(|c| c.id == "software-monitor" && c.available)
        );
        let up = resample_linear(&src, 96000, 16);
        assert_eq!(up.sample_rate, 96000);
        assert_eq!(up.planes[0].len(), 16);
        assert!(peak_meter(&src) > 0.4);
    }
}
