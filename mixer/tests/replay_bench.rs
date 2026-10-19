//! Replay stage-0 bench. Ignored by default. Release:
//! `cargo test --manifest-path mixer/Cargo.toml --release --test replay_bench -- --ignored --nocapture`
//!
//! Optional `EIVIZ_REPLAY_BENCH_DIR` selects the disk target. Results decide
//! `segment_frames` (default 120), profile SQ, encode queue 6, and write buffer 512 MiB.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use vmx::{Codec, ColorSpace, Config, Profile};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

fn report(name: &str, frames: u32, bytes: u64, elapsed: std::time::Duration) {
    let ms = elapsed.as_secs_f64() * 1000.0 / f64::from(frames.max(1));
    let mbps = if elapsed.as_secs_f64() > 0.0 {
        bytes as f64 * 8.0 / elapsed.as_secs_f64() / 1_000_000.0
    } else {
        0.0
    };
    println!(
        "{name:<36} frames={frames:<4} {ms:.2} ms/frame  {mbps:.1} Mbps  bytes={bytes}"
    );
}

fn uyvy_frame() -> Vec<u8> {
    let mut pixels = vec![0u8; (WIDTH * HEIGHT * 2) as usize];
    for (index, byte) in pixels.iter_mut().enumerate() {
        *byte = (index % 251) as u8;
    }
    pixels
}

fn encode_frames(profile: Profile, threads: Option<usize>, frames: u32) -> (u64, std::time::Duration) {
    let mut codec = Codec::new(Config {
        width: WIDTH as i32,
        height: HEIGHT as i32,
        profile,
        color_space: ColorSpace::Bt709,
    })
    .expect("codec");
    if let Some(threads) = threads {
        codec.set_threads(threads);
    }
    let pixels = uyvy_frame();
    let mut packet = vec![0u8; 8 * 1024 * 1024];
    let started = Instant::now();
    let mut bytes = 0u64;
    for _ in 0..frames {
        codec
            .encode_uyvy(&pixels, (WIDTH * 2) as usize)
            .expect("encode");
        loop {
            match codec.save_to(&mut packet) {
                Ok(n) => {
                    bytes += n as u64;
                    break;
                }
                Err(_) => packet.resize(packet.len() * 2, 0),
            }
        }
    }
    (bytes, started.elapsed())
}

fn decode_frames(profile: Profile, frames: u32) -> std::time::Duration {
    let mut encoder = Codec::new(Config {
        width: WIDTH as i32,
        height: HEIGHT as i32,
        profile,
        color_space: ColorSpace::Bt709,
    })
    .expect("encoder");
    let pixels = uyvy_frame();
    encoder
        .encode_uyvy(&pixels, (WIDTH * 2) as usize)
        .expect("encode");
    let mut packet = vec![0u8; 8 * 1024 * 1024];
    let n = loop {
        match encoder.save_to(&mut packet) {
            Ok(n) => break n,
            Err(_) => packet.resize(packet.len() * 2, 0),
        }
    };
    let mut decoder = Codec::new(Config {
        width: WIDTH as i32,
        height: HEIGHT as i32,
        profile,
        color_space: ColorSpace::Bt709,
    })
    .expect("decoder");
    let mut dst = vec![0u8; pixels.len()];
    let started = Instant::now();
    for _ in 0..frames {
        decoder.load_from(&packet[..n]).expect("load");
        decoder
            .decode_uyvy(&mut dst, (WIDTH * 2) as usize)
            .expect("decode");
    }
    started.elapsed()
}

fn bench_dir() -> PathBuf {
    std::env::var_os("EIVIZ_REPLAY_BENCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("eiviz-replay-bench"))
}

fn write_segment(path: &std::path::Path, bytes: usize, fsync: bool) -> std::time::Duration {
    let started = Instant::now();
    let mut file = File::create(path).expect("create");
    let chunk = vec![0xA5u8; 1024 * 1024];
    let mut left = bytes;
    while left > 0 {
        let n = left.min(chunk.len());
        file.write_all(&chunk[..n]).expect("write");
        left -= n;
    }
    if fsync {
        file.sync_all().expect("fsync");
    }
    started.elapsed()
}

#[test]
#[ignore]
fn replay_stage0() {
    let frames = 8u32;
    for (name, profile) in [("HQ", Profile::Hq), ("SQ", Profile::Sq), ("LQ", Profile::Lq)] {
        let (bytes, elapsed) = encode_frames(profile, Some(1), frames);
        report(&format!("encode {name} 1080p threads=1"), frames, bytes, elapsed);
        let (bytes, elapsed) = encode_frames(profile, None, frames);
        report(&format!("encode {name} 1080p threads=auto"), frames, bytes, elapsed);
        let elapsed = decode_frames(profile, frames);
        report(&format!("decode {name} 1080p"), frames, 0, elapsed);
    }

    if let Some((instance, _adapter, device, queue)) = vmx::gpu::request_headless_device() {
        let _ = instance;
        let bgra = vec![0x40u8; (WIDTH * HEIGHT * 4) as usize];
        let texture = vmx::gpu::upload_bgra_texture(&device, &queue, WIDTH, HEIGHT, &bgra);
        let mut codec = Codec::new(Config {
            width: WIDTH as i32,
            height: HEIGHT as i32,
            profile: Profile::Sq,
            color_space: ColorSpace::Bt709,
        })
        .expect("gpu codec");
        let started = Instant::now();
        let gpu_frames = 4u32;
        let mut packet = vec![0u8; 8 * 1024 * 1024];
        let mut bytes = 0u64;
        for _ in 0..gpu_frames {
            codec
                .encode_from_texture_opaque(&device, &queue, &texture)
                .expect("gpu encode");
            loop {
                match codec.save_to(&mut packet) {
                    Ok(n) => {
                        bytes += n as u64;
                        break;
                    }
                    Err(_) => packet.resize(packet.len() * 2, 0),
                }
            }
        }
        report("gpu encode SQ 1080p opaque", gpu_frames, bytes, started.elapsed());
    } else {
        println!("gpu encode skipped: no headless device");
    }

    let dir = bench_dir();
    std::fs::create_dir_all(&dir).expect("bench dir");
    // SQ 1080p60 is about 130 Mbps, 16.25 MiB/s.
    let per_second = 16 * 1024 * 1024;
    for seconds in [1usize, 2, 4] {
        let path = dir.join(format!("seg-{seconds}s.bin"));
        let elapsed = write_segment(&path, per_second * seconds, true);
        let mb_s = (per_second * seconds) as f64 / elapsed.as_secs_f64() / (1024.0 * 1024.0);
        println!(
            "disk sequential {seconds}s + fsync  {mb_s:.1} MiB/s  {:?}",
            elapsed
        );
        let _ = std::fs::remove_file(&path);
    }
    let total = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    let mut joins = Vec::new();
    for index in 0..4 {
        let dir = dir.clone();
        let total = Arc::clone(&total);
        joins.push(thread::spawn(move || {
            let path = dir.join(format!("par-{index}.bin"));
            let elapsed = write_segment(&path, per_second, true);
            total.fetch_add(per_second as u64, Ordering::Relaxed);
            let _ = std::fs::remove_file(path);
            elapsed
        }));
    }
    for join in joins {
        let _ = join.join();
    }
    let elapsed = started.elapsed();
    let mb_s = total.load(Ordering::Relaxed) as f64 / elapsed.as_secs_f64() / (1024.0 * 1024.0);
    println!("disk 4-way 1s + fsync  {mb_s:.1} MiB/s aggregate");

    let pixels = uyvy_frame();
    let started = Instant::now();
    let copies = 30u32;
    let mut sink = Vec::new();
    for _ in 0..copies {
        sink = pixels.clone();
    }
    report(
        "memcpy uyvy 1080p",
        copies,
        pixels.len() as u64 * u64::from(copies),
        started.elapsed(),
    );
    let _ = sink;

    println!("stage0 defaults: segment_frames=120 profile=SQ encode_queue_frames=6 write_buffer_mb=512");
}
