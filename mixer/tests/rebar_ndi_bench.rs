//! One-off ReBAR bench for four LAN NDI sources. Ignored by default.

use std::ffi::CString;
use std::thread;
use std::time::{Duration, Instant};

use eiviz_mixer::{
    mixer_copy_rebar_info, mixer_copy_source_usage, mixer_copy_stats, mixer_create,
    mixer_create_unit, mixer_define_scene, mixer_destroy, mixer_last_error, mixer_ndi_connect,
    mixer_ndi_discover, mixer_set_ndi_gpu_upload, mixer_set_rebar_optimization,
    mixer_unit_configure, mixer_unit_set_state, MixerRebarInfo, MixerStats, OK, OverlayDesc, Rect,
    SCENE_BASE, SourceUsage, UnitState,
};

const SOURCE_BASE: u64 = 40;
const UNIT_ID: u64 = 1;
const SCENE_ID: u64 = SCENE_BASE | 1;
const WARMUP: Duration = Duration::from_secs(4);
const SAMPLE: Duration = Duration::from_secs(10);

#[test]
#[ignore]
fn lan_ndi_rebar_modes() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK, "{}", last_error());

    let mut info = MixerRebarInfo::default();
    unsafe {
        assert_eq!(mixer_copy_rebar_info(&mut info), OK);
    }
    let adapter = cstr_field(&info.adapter);
    println!("=== ReBAR probe ===");
    println!("adapter            : {adapter}");
    println!("available          : {}", info.available != 0);
    println!("gpu_upload_heaps   : {}", info.gpu_upload_heaps != 0);

    let sources = discover_ndi(Duration::from_secs(10));
    println!("=== NDI sources ===");
    for source in &sources {
        println!("  {source}");
    }
    let picked = pick_lan_sources(&sources, 4);
    assert!(
        picked.len() >= 4,
        "need 4 LAN NDI sources, got {} (last_error={})",
        picked.len(),
        last_error()
    );
    for (i, name) in picked.iter().enumerate() {
        println!("using[{i}]            : {name}");
        let address = CString::new(name.as_str()).unwrap();
        unsafe {
            assert_eq!(
                mixer_ndi_connect(SOURCE_BASE + i as u64, address.as_ptr(), 3, 0),
                OK,
                "{}",
                last_error()
            );
        }
    }

    let usages = wait_for_frames(4);
    let mut width = 3840u32;
    let mut height = 2160u32;
    for (i, usage) in usages.iter().enumerate() {
        println!("frame[{i}]           : {}x{}", usage.width, usage.height);
        width = width.max(usage.width);
        height = height.max(usage.height);
    }

    let mut layers = [OverlayDesc::default(); 4];
    for i in 0..4u32 {
        layers[i as usize] = OverlayDesc {
            source_id: SOURCE_BASE + u64::from(i),
            rect: Rect {
                x: (i % 2) as f32 * 0.5,
                y: (i / 2) as f32 * 0.5,
                width: 0.5,
                height: 0.5,
            },
            opacity: 1.0,
            z: i as i32,
            ..OverlayDesc::default()
        };
    }
    unsafe {
        assert_eq!(
            mixer_define_scene(SCENE_ID, width, height, 4, layers.as_ptr()),
            OK
        );
    }
    assert_eq!(mixer_create_unit(UNIT_ID, width, height), OK);
    assert_eq!(
        mixer_unit_configure(UNIT_ID, width, height, 60_000, 1_001),
        OK
    );
    let state = UnitState {
        program_source: SCENE_ID,
        preview_source: SCENE_ID,
        mix: 0.0,
        ..UnitState::default()
    };
    unsafe {
        assert_eq!(mixer_unit_set_state(UNIT_ID, &state), OK);
    }
    println!("compose             : {width}x{height} 2x2");

    for (name, gpu, rebar) in [
        ("cpu", 0u32, 0u32),
        ("gpu", 1, 0),
        ("gpu_rebar", 1, 1),
    ] {
        assert_eq!(mixer_set_ndi_gpu_upload(gpu), OK);
        assert_eq!(mixer_set_rebar_optimization(rebar), OK);
        thread::sleep(WARMUP);
        let stats = sample_render();
        println!(
            "=== {name} === n={} mean={:.3} p50={:.3} p95={:.3} min={:.3} max={:.3} over={}/{} budget={:.3}",
            stats.n, stats.mean, stats.p50, stats.p95, stats.min, stats.max, stats.overruns, stats.n, stats.budget
        );
    }
    mixer_destroy();
}

fn discover_ndi(limit: Duration) -> Vec<String> {
    let mut buf = vec![0u8; 16 * 1024];
    let deadline = Instant::now() + limit;
    let mut last = Vec::new();
    while Instant::now() < deadline {
        let n = unsafe { mixer_ndi_discover(buf.as_mut_ptr(), buf.len()) };
        if n > 0 {
            last = String::from_utf8_lossy(&buf[..n as usize])
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect();
            if last.len() >= 4 {
                return last;
            }
        }
        thread::sleep(Duration::from_millis(400));
    }
    last
}

fn pick_lan_sources(sources: &[String], want: usize) -> Vec<String> {
    let mut remote: Vec<String> = sources
        .iter()
        .filter(|source| {
            let lower = source.to_ascii_lowercase();
            !lower.contains("127.0.0.1") && !lower.contains("localhost")
        })
        .cloned()
        .collect();
    if remote.len() < want {
        remote = sources.to_vec();
    }
    remote.into_iter().take(want).collect()
}

fn wait_for_frames(count: usize) -> Vec<SourceUsage> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut items = [SourceUsage::default(); 16];
        let n = unsafe { mixer_copy_source_usage(items.as_mut_ptr(), items.len() as u32) };
        let mut found = Vec::new();
        if n > 0 {
            for i in 0..count {
                let id = SOURCE_BASE + i as u64;
                if let Some(item) = items[..n as usize]
                    .iter()
                    .find(|item| item.source_id == id && item.width >= 640)
                {
                    found.push(*item);
                }
            }
        }
        if found.len() == count {
            return found;
        }
        if Instant::now() >= deadline {
            panic!("need {count} NDI frames, got {} ({})", found.len(), last_error());
        }
        thread::sleep(Duration::from_millis(80));
    }
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

fn cstr_field(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}
