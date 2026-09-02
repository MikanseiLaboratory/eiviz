//! Micro-benches for CPU hot paths. Ignored by default; run Release:
//! `cargo test --manifest-path mixer/Cargo.toml --release --test cpu_hotpath_bench -- --ignored --nocapture`

use std::thread;
use std::time::{Duration, Instant};

use eiviz_mixer::simd;
use eiviz_mixer::{
    mixer_copy_stats, mixer_create, mixer_create_unit, mixer_define_generator,
    mixer_define_scene, mixer_destroy, mixer_generator_set_tone, mixer_unit_configure,
    mixer_unit_set_state, mixer_video_set_loop, mixer_video_set_playing, mixer_video_start,
    MixerStats, OverlayDesc, Rect, UnitState, GEN_BARS, GEN_SOLID, OK, SCENE_BASE,
};

const FRAMES: usize = 1601;
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

fn report(name: &str, iters: u32, elapsed: std::time::Duration) {
    let ns = elapsed.as_nanos() / u128::from(iters.max(1));
    println!(
        "{name:<28} path={:<6} iters={iters} {ns} ns/iter ({:.3} ms)",
        simd::path(),
        ns as f64 / 1_000_000.0
    );
}

#[test]
#[ignore]
fn cpu_hotpath_kernels() {
    println!("simd path {}", simd::path());

    let packed_len = (WIDTH * HEIGHT * 2) as usize;
    let mut yuy2 = vec![0u8; packed_len];
    for (i, b) in yuy2.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    let mut bgra = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    let mut uyvy = vec![0u8; packed_len];

    let start = Instant::now();
    const YUV_ITERS: u32 = 8;
    for _ in 0..YUV_ITERS {
        simd::yuv422_to_bgra(&yuy2, WIDTH, HEIGHT, (WIDTH * 2) as usize, false, &mut bgra);
    }
    report("yuv422_to_bgra 1080p", YUV_ITERS, start.elapsed());

    let start = Instant::now();
    const SHUF_ITERS: u32 = 20;
    for _ in 0..SHUF_ITERS {
        simd::yuy2_to_uyvy(&yuy2, WIDTH, HEIGHT, (WIDTH * 2) as usize, &mut uyvy);
    }
    report("yuy2_to_uyvy 1080p", SHUF_ITERS, start.elapsed());

    let start = Instant::now();
    const ALPHA_ITERS: u32 = 20;
    for _ in 0..ALPHA_ITERS {
        simd::or_opaque_bgra(&mut bgra);
    }
    report("or_opaque_bgra 1080p", ALPHA_ITERS, start.elapsed());

    let src: Vec<f32> = (0..FRAMES * 2).map(|i| (i as f32) * 0.0001).collect();
    let mut dest = vec![0.0f32; FRAMES * 2];
    let start = Instant::now();
    const MIX_ITERS: u32 = 400;
    for _ in 0..MIX_ITERS {
        dest.fill(0.0);
        simd::mix_stereo_gain(&mut dest, &src, 0.75);
        simd::mix_stereo_gain(&mut dest, &src, 0.25);
        simd::mix_stereo_gain(&mut dest, &src, 0.5);
        simd::scale_f32(&mut dest, 0.9);
    }
    report("mix 3src+fader 1601", MIX_ITERS, start.elapsed());

    let start = Instant::now();
    const PEAK_ITERS: u32 = 2000;
    let mut peak = 0.0f32;
    for _ in 0..PEAK_ITERS {
        peak = simd::peak_f32(&src);
    }
    report("peak_f32 1601", PEAK_ITERS, start.elapsed());
    let _ = peak;

    let mut sine = vec![0.0f32; FRAMES];
    let start = Instant::now();
    const SINE_ITERS: u32 = 200;
    for _ in 0..SINE_ITERS {
        simd::sine_fill(&mut sine, 0.0, 1000.0, 0.1, 48_000.0);
    }
    report("sine_fill 1kHz 1601", SINE_ITERS, start.elapsed());

    let planar: Vec<f32> = (0..FRAMES * 2).map(|i| i as f32 * 0.001).collect();
    let mut stereo = Vec::new();
    let start = Instant::now();
    const RES_ITERS: u32 = 400;
    for _ in 0..RES_ITERS {
        simd::resample_planar_to_stereo(&planar, FRAMES, 2, 48_000, 48_000, &mut stereo);
    }
    report("resample 48k passthrough", RES_ITERS, start.elapsed());

    let start = Instant::now();
    for _ in 0..RES_ITERS {
        simd::resample_planar_to_stereo(&planar, FRAMES, 2, 44_100, 48_000, &mut stereo);
    }
    report("resample 44k1->48k", RES_ITERS, start.elapsed());

    let start = Instant::now();
    const COPY_ITERS: u32 = 20;
    let mut packed = vec![0u8; packed_len];
    for _ in 0..COPY_ITERS {
        simd::copy_rows(
            &yuy2,
            (WIDTH * 2) as usize,
            &mut packed,
            (WIDTH * 2) as usize,
            (WIDTH * 2) as usize,
            HEIGHT as usize,
        );
    }
    report("copy_rows uyvy 1080p", COPY_ITERS, start.elapsed());
}

#[test]
#[ignore]
fn eightme_scroll_tone_render() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(
        mixer_define_generator(1, GEN_SOLID, 1.0, 0.0, 0.0, 1.0, 0),
        OK
    );
    assert_eq!(
        mixer_define_generator(2, GEN_BARS, 1.0, 0.0, 0.0, 1.0, 1),
        OK
    );
    assert_eq!(mixer_generator_set_tone(2, 1000.0, -20.0), OK);
    let layers = [
        OverlayDesc {
            source_id: 2,
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            opacity: 1.0,
            ..OverlayDesc::default()
        },
        OverlayDesc {
            source_id: 1,
            rect: Rect {
                x: 0.7,
                y: 0.7,
                width: 0.3,
                height: 0.3,
            },
            opacity: 1.0,
            z: 1,
            ..OverlayDesc::default()
        },
    ];
    let scene = SCENE_BASE | 1;
    unsafe {
        assert_eq!(mixer_define_scene(scene, 1920, 1080, 2, layers.as_ptr()), OK);
    }
    assert_eq!(mixer_create_unit(1, 1920, 1080), OK);
    assert_eq!(mixer_unit_configure(1, 1920, 1080, 60_000, 1_001), OK);
    let state = UnitState {
        program_source: scene,
        preview_source: 2,
        mix: 0.0,
        ..UnitState::default()
    };
    unsafe {
        assert_eq!(mixer_unit_set_state(1, &state), OK);
    }
    if let Some(path) = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from).map(|home| {
        home.join("Videos").join("機銃.mp4")
    }) {
        if path.is_file() {
            let cpath = std::ffi::CString::new(path.to_string_lossy().as_ref()).unwrap();
            unsafe {
                if mixer_video_start(15, cpath.as_ptr(), 0, 0, 0, 0, 0, 0, 0) == OK {
                    let _ = mixer_video_set_loop(15, 1);
                    let _ = mixer_video_set_playing(15, 1);
                    println!("video file attached {}", path.display());
                }
            }
        }
    }
    thread::sleep(Duration::from_secs(4));
    let deadline = Instant::now() + Duration::from_secs(10);
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
    mixer_destroy();
    assert!(samples.len() >= 30, "samples={}", samples.len());
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = samples.len();
    let mean = samples.iter().sum::<f32>() / n as f32;
    let p50 = samples[n / 2];
    let p95 = samples[((n - 1) * 95) / 100];
    let over = samples.iter().filter(|ms| **ms > budget).count();
    println!(
        "eightme-like scroll+tone n={n} mean={mean:.3} p50={p50:.3} p95={p95:.3} min={:.3} max={:.3} over={over}/{n} budget={budget:.3}",
        samples[0],
        samples[n - 1]
    );
}
