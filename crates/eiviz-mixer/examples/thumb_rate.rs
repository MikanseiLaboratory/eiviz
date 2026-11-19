//! Measures how many distinct thumbnail buffers per second actually reach
//! `mixer_thumb_read`, versus the configured present interval. The bars
//! generator animates on every master frame, so every delivered buffer is a
//! new picture: a missed delivery is a lost capture, never a duplicate.
//!
//!   cargo run -p eiviz_mixer --release --example thumb_rate
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use eiviz_mixer::{
    GEN_BARS, MixerStats, OK, OverlayDesc, Rect, SCENE_BASE, UnitState, mixer_copy_stats,
    mixer_create, mixer_create_unit, mixer_define_generator, mixer_define_scene, mixer_destroy,
    mixer_thumb_read, mixer_thumb_set, mixer_unit_set_state,
};

/// Input id for the animated generator. Below `SCENE_BASE`, outside the
/// reserved replay bands: the same range session inputs use.
const GEN_ID: u64 = 10;

/// One scrolling-bars layer filling the scene, like the color inputs the UI
/// shows in scene tiles. The built-in `SRC_BARS` is static (`scroll=false`),
/// so a scene over it never changes and the hosts' change detection would
/// rightly report nothing.
fn layer(z: i32) -> OverlayDesc {
    OverlayDesc {
        source_id: GEN_ID,
        rect: Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        crop: Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        opacity: 1.0,
        z,
        audio_follow: 1,
        hidden: 0,
        label: std::ptr::null(),
        layer_id: z as u64 + 1,
    }
}

/// The same 64-point sample the hosts hash with, so the bench counts exactly
/// what the UI would count as a new picture.
fn sample_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 14_695_981_039_346_656_037;
    let step = (bytes.len() / 64).max(1);
    let mut i = 0;
    while i < bytes.len() {
        hash = (hash ^ u64::from(bytes[i])).wrapping_mul(1_099_511_628_211);
        i += step;
    }
    if !bytes.is_empty() {
        hash ^= u64::from(bytes[bytes.len() - 1]);
    }
    hash ^ bytes.len() as u64
}

struct Sub {
    source: u64,
    interval: u32,
    label: &'static str,
}

fn main() {
    let run_secs: u64 = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(12);
    // The intervals the UI offers, on sources the main window shows: two scenes
    // on the busses, one scene only a tile subscribes to, and the bars input.
    let subs: Vec<Sub> = vec![
        Sub {
            source: SCENE_BASE | 1,
            interval: 1,
            label: "scene1",
        },
        Sub {
            source: SCENE_BASE | 2,
            interval: 2,
            label: "scene2",
        },
        Sub {
            source: SCENE_BASE | 3,
            interval: 3,
            label: "scene3",
        },
        Sub {
            source: GEN_ID,
            interval: 4,
            label: "gen   ",
        },
    ];
    unsafe {
        assert_eq!(mixer_create(0, 60_000, 1_001), OK);
        assert_eq!(mixer_create_unit(1, 1920, 1080), OK);
        // A scrolling bars generator: its texture changes on every master
        // frame, so a delivered thumbnail is a genuinely new picture.
        assert_eq!(
            mixer_define_generator(GEN_ID, GEN_BARS, 0.0, 0.6, 1.0, 1.0, 1),
            OK
        );
        let layers = [layer(0)];
        for scene in 1..=3u64 {
            assert_eq!(
                mixer_define_scene(
                    SCENE_BASE | scene,
                    1920,
                    1080,
                    layers.len() as u32,
                    layers.as_ptr(),
                ),
                OK
            );
        }
        let state = UnitState {
            program_source: SCENE_BASE | 1,
            preview_source: SCENE_BASE | 2,
            mix: 1.0,
            ..UnitState::default()
        };
        assert_eq!(mixer_unit_set_state(1, &state), OK);
        for sub in &subs {
            assert_eq!(mixer_thumb_set(sub.source, 176, 90, sub.interval), OK);
        }

        let sources: Vec<u64> = subs.iter().map(|s| s.source).collect();
        let changes: Arc<Vec<AtomicU64>> =
            Arc::new((0..sources.len()).map(|_| AtomicU64::new(0)).collect());
        let running = Arc::new(AtomicBool::new(true));
        // Poll far faster than the hosts do, so the measurement shows the core's
        // own delivery rate and not the UI's poll cadence.
        let poller = {
            let sources = sources.clone();
            let changes = Arc::clone(&changes);
            let running = Arc::clone(&running);
            thread::spawn(move || {
                let cap = 960 * 540 * 4;
                let mut buf = vec![0u8; cap];
                let mut hashes = vec![0u64; sources.len()];
                let mut known = vec![false; sources.len()];
                let (mut w, mut h, mut stride) = (0u32, 0u32, 0u32);
                while running.load(Ordering::Relaxed) {
                    for (i, source) in sources.iter().enumerate() {
                        let n = mixer_thumb_read(
                            *source,
                            buf.as_mut_ptr(),
                            cap,
                            &mut w,
                            &mut h,
                            &mut stride,
                        );
                        if n <= 0 {
                            continue;
                        }
                        let hash = sample_hash(&buf[..n as usize]);
                        if known[i] && hash == hashes[i] {
                            continue;
                        }
                        hashes[i] = hash;
                        known[i] = true;
                        changes[i].fetch_add(1, Ordering::Relaxed);
                    }
                    thread::sleep(Duration::from_millis(2));
                }
            })
        };

        // Warm-up: fill the readback slots, then measure a steady window.
        thread::sleep(Duration::from_secs(2));
        let mut stats = MixerStats::default();
        mixer_copy_stats(&mut stats);
        let (start_frames, start_skips) = (stats.frames_rendered, stats.frames_skipped);
        for counter in changes.iter() {
            counter.store(0, Ordering::Relaxed);
        }
        let start = Instant::now();
        thread::sleep(Duration::from_secs(run_secs));
        let elapsed = start.elapsed().as_secs_f64();
        running.store(false, Ordering::Relaxed);
        poller.join().unwrap();
        mixer_copy_stats(&mut stats);
        let master_fps = (stats.frames_rendered - start_frames) as f64 / elapsed;
        let skipped = stats.frames_skipped - start_skips;

        println!("master render fps: {master_fps:.2} (skipped {skipped}) over {elapsed:.1}s");
        for (sub, counter) in subs.iter().zip(changes.iter()) {
            let got = counter.load(Ordering::Relaxed) as f64 / elapsed;
            let want = 59.94 / f64::from(sub.interval);
            let pct = 100.0 * got / want;
            println!(
                "{} interval={}: delivered {:.2} fps, configured {:.2} fps -> {:.0}%",
                sub.label, sub.interval, got, want, pct
            );
        }
    }
    mixer_destroy();
    println!("done");
}
