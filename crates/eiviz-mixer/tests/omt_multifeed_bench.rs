//! Multi-feed ingest scaling bench. Ignored by default; run Release:
//!
//! ```text
//! cargo test -p eiviz_mixer --release --test omt_multifeed_bench -- --ignored --nocapture
//! ```
//!
//! Every feed is a local OMT sender pushing one pre-encoded 1080p moving-bars
//! VMX1 frame at 60 fps, so the send side stays cheap and the measurement
//! tracks the mixer: ingest, VMX decode, compose and the render thread.
//!
//! - `EIVIZ_MULTIFEED_FEEDS`: comma-separated feed counts (default `1,2,4,8`)
//! - `EIVIZ_MULTIFEED_GPU`: `1` (default) for the GPU texture path, `0` for the
//!   CPU BGRA path whose uploads run on the render thread
//! - `EIVIZ_MULTIFEED_SECS`: measurement window per count (default `8`)

use std::ffi::CString;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use eiviz_mixer::{
    MixerStats, OK, OverlayDesc, Rect, SCENE_BASE, SRC_COLOR, UnitState, mixer_copy_stats,
    mixer_create, mixer_create_unit, mixer_define_scene, mixer_destroy, mixer_omt_connect,
    mixer_unit_set_state,
};
use openmediatransport::{Codec, FrameType, MediaFrame, Sender};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const SRC_BASE: u64 = 20;
const FPS: f64 = 60.0;

/// UYVY color bars with a shifted cut, so the bitstream carries real AC
/// coefficients; a still frame would let the wavelet codec encode a nearly
/// empty picture and understate the decode cost.
fn moving_bars(width: usize, height: usize, out: &mut [u8]) {
    const BARS: [(u8, u8, u8); 8] = [
        (235, 128, 128),
        (210, 16, 146),
        (170, 166, 16),
        (145, 54, 34),
        (106, 202, 222),
        (81, 90, 240),
        (41, 240, 110),
        (16, 128, 128),
    ];
    let stride = width * 2;
    let bar_width = (width / BARS.len()).max(1);
    let shift = width / 3;
    for row in 0..height {
        for x in (0..width).step_by(2) {
            let index = ((x + shift) / bar_width) % BARS.len();
            let (luma, u, v) = BARS[index];
            let next = ((x + 1 + shift) / bar_width) % BARS.len();
            let luma_next = BARS[next].0;
            let i = row * stride + x * 2;
            out[i] = u;
            out[i + 1] = luma;
            out[i + 2] = v;
            out[i + 3] = luma_next;
        }
    }
}

/// One 1080p frame of moving bars, VMX1-encoded once. Senders replay this
/// bitstream as a passthrough frame, so no feed pays an encoder on the way in.
fn encode_feed() -> Vec<u8> {
    let mut encoder = vmx::Codec::new(vmx::Config {
        width: WIDTH as i32,
        height: HEIGHT as i32,
        profile: vmx::Profile::OmtHq,
        color_space: vmx::ColorSpace::Bt709,
    })
    .expect("vmx encoder");
    let stride = (WIDTH * 2) as usize;
    let mut frame = vec![0u8; stride * HEIGHT as usize];
    moving_bars(WIDTH as usize, HEIGHT as usize, &mut frame);
    encoder.encode_uyvy(&frame, stride).expect("encode");
    let mut buf = vec![0u8; 8 << 20];
    let len = encoder.save_to(&mut buf).expect("save");
    buf.truncate(len);
    buf
}

/// Paces one feed at 60 fps on its own thread, the way a live source would.
fn pump_feed(
    mut sender: Sender,
    bitstream: Arc<Vec<u8>>,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut next = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            let _ = sender.poll_accept();
            let _ = sender.poll_peer_metadata();
            if !sender.video_subscribed() {
                sender.force_subscribe(true, false, false);
            }
            let _ = sender.send_video(MediaFrame {
                frame_type: FrameType::VIDEO,
                codec: Codec::Vmx1 as i32,
                width: WIDTH as i32,
                height: HEIGHT as i32,
                frame_rate_n: 60,
                frame_rate_d: 1,
                data: bitstream.as_ref().clone(),
                ..MediaFrame::default()
            });
            next += Duration::from_secs_f64(1.0 / FPS);
            let now = Instant::now();
            if next > now {
                thread::sleep(next - now);
            } else {
                next = now;
            }
        }
    })
}

fn process_cpu_percent(pid: u32) -> Option<f32> {
    let output = Command::new("ps")
        .args(["-o", "pcpu=", "-p", &pid.to_string()])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    text.parse::<f32>().ok()
}

fn percentile(values: &[f32], which: usize) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values[(values.len() - 1) * which / 100]
}

fn run_count(feeds: u64, use_gpu: u32, secs: u64) {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, WIDTH, HEIGHT), OK);

    let bitstream = Arc::new(encode_feed());
    let stop = Arc::new(AtomicBool::new(false));
    let mut urls = Vec::new();
    let mut pumps = Vec::new();
    for i in 0..feeds {
        let sender =
            Sender::create(format!("eiviz-multifeed-{i}"), FrameType::VIDEO).expect("sender");
        urls.push(format!("omt://127.0.0.1:{}", sender.port()));
        pumps.push(pump_feed(sender, Arc::clone(&bitstream), Arc::clone(&stop)));
    }

    // One scene tiles every feed across the program picture, so compose cost
    // and the number of live sources both grow with the feed count.
    let cols = ((feeds as f64).sqrt().ceil() as u64).max(1);
    let rows = feeds.div_ceil(cols);
    let layers: Vec<OverlayDesc> = (0..feeds)
        .map(|i| OverlayDesc {
            source_id: SRC_BASE + i,
            rect: Rect {
                x: (i % cols) as f32 / cols as f32,
                y: (i / cols) as f32 / rows as f32,
                width: 1.0 / cols as f32,
                height: 1.0 / rows as f32,
            },
            opacity: 1.0,
            z: i as i32,
            ..OverlayDesc::default()
        })
        .collect();
    let scene = SCENE_BASE | 1;
    unsafe {
        assert_eq!(
            mixer_define_scene(scene, WIDTH, HEIGHT, layers.len() as u32, layers.as_ptr()),
            OK
        );
    }
    for (i, url) in urls.iter().enumerate() {
        let address = CString::new(url.as_str()).expect("url");
        unsafe {
            assert_eq!(
                mixer_omt_connect(SRC_BASE + i as u64, address.as_ptr(), use_gpu, 3, 0, 0),
                OK
            );
        }
    }
    let state = UnitState {
        program_source: scene,
        preview_source: SRC_COLOR,
        mix: 0.0,
        ..UnitState::default()
    };
    unsafe {
        assert_eq!(mixer_unit_set_state(1, &state), OK);
    }

    // Loopback feeds come up fast; give the receivers a moment to subscribe.
    thread::sleep(Duration::from_secs(4));

    let pid = std::process::id();
    let mut render = Vec::new();
    let mut cpu = Vec::new();
    let mut budget = 0.0f32;
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        let mut stats = MixerStats::default();
        unsafe {
            mixer_copy_stats(&mut stats);
        }
        budget = stats.frame_budget_ms;
        if stats.render_ms > 0.0 {
            render.push(stats.render_ms);
        }
        if let Some(value) = process_cpu_percent(pid) {
            cpu.push(value);
        }
        thread::sleep(Duration::from_millis(100));
    }
    render.sort_by(|a, b| a.partial_cmp(b).unwrap());
    cpu.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "multifeed feeds={feeds} gpu={use_gpu} render p50={:.3}ms p95={:.3}ms max={:.3}ms budget={budget:.2}ms cpu p50={:.1}% samples={}",
        percentile(&render, 50),
        percentile(&render, 95),
        render.last().copied().unwrap_or(0.0),
        percentile(&cpu, 50),
        render.len(),
    );

    stop.store(true, Ordering::Release);
    for pump in pumps {
        let _ = pump.join();
    }
    mixer_destroy();
}

#[test]
#[ignore]
fn multifeed_scaling() {
    let counts: Vec<u64> = std::env::var("EIVIZ_MULTIFEED_FEEDS")
        .ok()
        .and_then(|value| {
            let parsed: Option<Vec<u64>> =
                value.split(',').map(|c| c.trim().parse().ok()).collect();
            parsed
        })
        .unwrap_or_else(|| vec![1, 2, 4, 8]);
    let use_gpu: u32 = std::env::var("EIVIZ_MULTIFEED_GPU")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    let secs: u64 = std::env::var("EIVIZ_MULTIFEED_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8);
    println!("multifeed bench: counts={counts:?} gpu={use_gpu} secs={secs}");
    for feeds in counts {
        run_count(feeds, use_gpu, secs);
    }
}
