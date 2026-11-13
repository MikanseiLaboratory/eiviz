//! Receives OMT audio for a few seconds and reports timestamp gaps.
//! Usage: omt_audio_probe "omt://127.0.0.1:6400" [seconds]
use std::thread;
use std::time::{Duration, Instant};

use openmediatransport::{FrameType, ReceiverConfig, ReceiverSession};

fn main() {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "omt://127.0.0.1:6400".into());
    let seconds: f64 = std::env::args()
        .nth(2)
        .and_then(|v| v.parse().ok())
        .unwrap_or(10.0);
    let session = match ReceiverSession::connect(
        &url,
        ReceiverConfig {
            frame_types: FrameType::VIDEO | FrameType::AUDIO,
            ..ReceiverConfig::default()
        },
    ) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("connect {url}: {error}");
            std::process::exit(1);
        }
    };
    let deadline = Instant::now() + Duration::from_secs_f64(seconds);
    let mut count = 0u64;
    let mut last_ts: Option<i64> = None;
    let mut gaps = 0u64;
    let mut max_gap = 0i64;
    let mut total_frames = 0i64;
    let mut silent = 0u64;
    let mut peak_all = 0.0f32;
    while Instant::now() < deadline {
        match session.try_recv_audio() {
            Some(frame) => {
                if let Some(prev) = last_ts {
                    let delta = frame.timestamp - prev;
                    if delta <= 0 || delta > 150_000 {
                        gaps += 1;
                        max_gap = max_gap.max(delta);
                        println!("gap #{gaps}: delta={delta} ns at pts={}", frame.timestamp);
                    }
                }
                last_ts = Some(frame.timestamp);
                total_frames += 1;
                count += 1;
                let bytes = &frame.pcm_planar_f32;
                let mut sum = 0.0f64;
                let mut n = 0u64;
                for chunk in bytes.chunks_exact(4) {
                    let sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    sum += f64::from(sample * sample);
                    n += 1;
                    peak_all = peak_all.max(sample.abs());
                }
                if n > 0 {
                    let rms = (sum / n as f64).sqrt() as f32;
                    if rms < 1e-5 {
                        silent += 1;
                    }
                }
            }
            None => thread::sleep(Duration::from_millis(2)),
        }
    }
    println!(
        "audio frames={count} gaps={gaps} max_gap_100ns={max_gap} silent={silent} peak={peak_all:.4}"
    );
}
