use std::ffi::CString;
use std::thread;
use std::time::{Duration, Instant};

use eiviz_mixer::{
    OK, OUTPUT_PROGRAM, OverlayDesc, Rect, SCENE_BASE, SRC_BARS, SRC_BLUE, UnitState, mixer_create,
    mixer_create_unit, mixer_define_scene, mixer_destroy, mixer_snapshot, mixer_unit_set_state,
};

fn scene_id(n: u64) -> u64 {
    SCENE_BASE | n
}

fn full(src: u64) -> OverlayDesc {
    OverlayDesc {
        source_id: src,
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
        z: 0,
        audio_follow: 1,
        hidden: 0,
        label: std::ptr::null(),
        layer_id: 1,
    }
}

fn mean_rb(unit: u64, path: &std::path::Path) -> Option<(f32, f32)> {
    let _ = std::fs::remove_file(path);
    let cpath = CString::new(path.to_string_lossy().as_bytes()).ok()?;
    let code = unsafe { mixer_snapshot(unit, OUTPUT_PROGRAM, cpath.as_ptr()) };
    if code != OK {
        return None;
    }
    let img = image::open(path).ok()?.to_rgb8();
    let n = img.pixels().len().max(1) as f32;
    let (mut r, mut b) = (0.0f32, 0.0f32);
    for p in img.pixels() {
        r += f32::from(p[0]);
        b += f32::from(p[2]);
    }
    Some((r / n, b / n))
}

fn wait(unit: u64, path: &std::path::Path, want: impl Fn(f32, f32) -> bool) -> Option<(f32, f32)> {
    let start = Instant::now();
    let mut last = None;
    while start.elapsed() < Duration::from_secs(3) {
        if let Some(sample) = mean_rb(unit, path) {
            last = Some(sample);
            if want(sample.0, sample.1) {
                return Some(sample);
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    last
}

#[test]
fn redefining_a_scene_updates_the_program_picture() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);
    let bars = full(SRC_BARS);
    assert_eq!(
        unsafe { mixer_define_scene(scene_id(1), 320, 180, 1, &bars) },
        OK
    );
    let state = UnitState {
        program_source: scene_id(1),
        preview_source: SRC_BLUE,
        mix: 1.0,
        transition_kind: 0,
        mv_slot_count: 0,
        mv_slots: [0; 16],
        transition_easing: 0,
        transition_direction: 0,
        keep_preview: 0,
        pad: 0,
        dip_r: 0.0,
        dip_g: 0.0,
        dip_b: 0.0,
        dip_a: 0.0,
        incoming_source: 0,
        softness: 0.0,
        param: 0.0,
    };
    assert_eq!(unsafe { mixer_unit_set_state(1, &state) }, OK);
    let path =
        std::env::temp_dir().join(format!("eiviz-scene-redefine-{}.png", std::process::id()));
    let first = wait(1, &path, |_, _| false).expect("bars snapshot");
    assert!(
        first.0 > 40.0 || first.1 > 40.0,
        "bars should have colour, got {first:?}"
    );

    let blue = full(SRC_BLUE);
    assert_eq!(
        unsafe { mixer_define_scene(scene_id(1), 320, 180, 1, &blue) },
        OK
    );
    let second = wait(1, &path, |r, b| b > 120.0 && b > r + 30.0).expect("snapshot after redefine");
    assert!(
        second.1 > 120.0 && second.1 > second.0 + 30.0,
        "program did not follow the redefined scene layers: bars={first:?} after={second:?}"
    );
    let _ = std::fs::remove_file(&path);
    mixer_destroy();
}
