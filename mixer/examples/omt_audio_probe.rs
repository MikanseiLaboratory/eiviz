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
    let mut silent_runs: Vec<(u64, u64)> = Vec::new();
    let mut run_start: Option<u64> = None;
    let mut video_count = 0u64;
    let mut last_video_ts: Option<i64> = None;
    let mut offsets: Vec<f64> = Vec::new();
    let mut peak_all = 0.0f32;
    while Instant::now() < deadline {
        while let Some(video) = session.try_recv_video() {
            video_count += 1;
            last_video_ts = Some(video.timestamp);
        }
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
                if let Some(video_ts) = last_video_ts {
                    offsets.push((frame.timestamp - video_ts) as f64 / 10_000.0);
                }
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
                        if run_start.is_none() {
                            run_start = Some(count);
                        }
                    } else if let Some(start) = run_start.take() {
                        silent_runs.push((start, count.saturating_sub(1)));
                    }
                    if count < 40 {
                        println!("frame {count} rms={rms:.6}");
                    }
                }
            }
            None => thread::sleep(Duration::from_millis(2)),
        }
    }
    if let Some(start) = run_start {
        silent_runs.push((start, count.saturating_sub(1)));
    }
    println!(
        "audio frames={count} gaps={gaps} max_gap_100ns={max_gap} silent={silent} peak={peak_all:.4}"
    );
    println!("silent runs {:?}", silent_runs);
    if !offsets.is_empty() {
        let avg = offsets.iter().sum::<f64>() / offsets.len() as f64;
        let min = offsets.iter().copied().fold(f64::INFINITY, f64::min);
        let max = offsets.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let last = offsets.last().copied().unwrap_or(0.0);
        println!(
            "video frames={video_count} av_offset_ms avg={avg:.1} min={min:.1} max={max:.1} last={last:.1}"
        );
    } else {
        println!("video frames={video_count} (no audio/video pair)");
    }
}
