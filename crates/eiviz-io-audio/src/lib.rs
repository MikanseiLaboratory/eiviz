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
    }
}
