//! Compare OMT GPU decode vs CPU decode + ingest upload. Ignored by default.

use std::ffi::CString;
use std::thread;
use std::time::{Duration, Instant};

use eiviz_mixer::{
    mixer_copy_stats, mixer_create, mixer_create_unit, mixer_destroy, mixer_last_error,
    mixer_omt_connect, mixer_set_omt_cpu_decode_ingest, mixer_unit_set_state, MixerStats, OK,
    UnitState,
};
use openmediatransport::{Codec, FrameType, MediaFrame, Sender};

const SOURCE_ID: u64 = 50;
const UNIT_ID: u64 = 1;
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const WARMUP: Duration = Duration::from_secs(2);
const SAMPLE: Duration = Duration::from_secs(10);

#[test]
#[ignore]
fn omt_cpu_decode_ingest_ab() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK, "{}", last_error());
    assert_eq!(mixer_create_unit(UNIT_ID, WIDTH, HEIGHT), OK);

    let mut sender =
        Sender::create("eiviz-omt-ab", FrameType::VIDEO | FrameType::AUDIO).expect("sender");
    let url = format!("omt://127.0.0.1:{}", sender.port());
    let address = CString::new(url).unwrap();
    let uyvy = vec![0x80u8, 0xEB, 0x80, 0x10].repeat((WIDTH * HEIGHT / 2) as usize);

    let pump = thread::spawn(move || {
        let deadline = Instant::now() + WARMUP + SAMPLE + SAMPLE + Duration::from_secs(4);
        while Instant::now() < deadline {
            let _ = sender.poll_accept();
            let _ = sender.poll_peer_metadata();
            if !sender.video_subscribed() {
                sender.force_subscribe(true, true, false);
            }
            let _ = sender.send_video(MediaFrame {
                frame_type: FrameType::VIDEO,
                codec: Codec::Uyvy as i32,
                width: WIDTH as i32,
                height: HEIGHT as i32,
                stride: (WIDTH * 2) as i32,
                frame_rate_n: 60,
                frame_rate_d: 1,
                data: uyvy.clone(),
                ..Default::default()
            });
            thread::sleep(Duration::from_millis(16));
        }
    });

    for (name, cpu_ingest, use_gpu) in [
        ("gpu_decode", 0u32, 1u32),
        ("cpu_ingest", 1, 0),
    ] {
        assert_eq!(mixer_set_omt_cpu_decode_ingest(cpu_ingest), OK);
        unsafe {
            assert_eq!(
                mixer_omt_connect(SOURCE_ID, address.as_ptr(), use_gpu, 3, 0),
                OK,
                "{}",
                last_error()
            );
            let state = UnitState {
                program_source: SOURCE_ID,
                preview_source: SOURCE_ID,
                mix: 0.0,
                ..UnitState::default()
            };
            assert_eq!(mixer_unit_set_state(UNIT_ID, &state), OK);
        }
        thread::sleep(WARMUP);
        let stats = sample_render();
        println!(
            "=== {name} === n={} mean={:.3} p50={:.3} p95={:.3} min={:.3} max={:.3} over={}/{} budget={:.3}",
            stats.n,
            stats.mean,
            stats.p50,
            stats.p95,
            stats.min,
            stats.max,
            stats.overruns,
            stats.n,
            stats.budget
        );
    }

    mixer_destroy();
    let _ = pump.join();
}

struct RenderStats {
    n: usize,
    mean: f32,
    p50: f32,
    p95: f32,
    min: f32,
    max: f32,
    overruns: usize,
    budget: f32,
}

fn sample_render() -> RenderStats {
    let deadline = Instant::now() + SAMPLE;
    let mut samples = Vec::new();
    let mut budget = 16.68f32;
    while Instant::now() < deadline {
        let mut stats = MixerStats::default();
        unsafe {
            assert_eq!(mixer_copy_stats(&mut stats), OK);
        }
        if stats.render_ms > 0.0 {
            samples.push(stats.render_ms);
            budget = stats.frame_budget_ms;
        }
        thread::sleep(Duration::from_millis(16));
    }
    assert!(samples.len() >= 30, "samples={}", samples.len());
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = samples.len();
    RenderStats {
        n,
        mean: samples.iter().sum::<f32>() / n as f32,
        p50: samples[n / 2],
        p95: samples[((n - 1) * 95) / 100],
        min: samples[0],
        max: *samples.last().unwrap(),
        overruns: samples.iter().filter(|ms| **ms > budget).count(),
        budget,
    }
}

fn last_error() -> String {
    let mut buf = vec![0u8; 512];
    let n = unsafe { mixer_last_error(buf.as_mut_ptr(), buf.len()) };
    if n > 0 {
        String::from_utf8_lossy(&buf[..n as usize]).into_owned()
    } else {
        String::new()
    }
}
