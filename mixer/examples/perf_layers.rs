//! Measures the core render loop with static scenes of N layers, with and without
//! the scene cache. Run under `ps` for process CPU:
//!   EIVIZ_SCENE_CACHE=0 cargo run --release --example perf_layers -- 20
use std::{env, thread, time::Duration};

use eiviz_mixer::{
    OK, OverlayDesc, Rect, SCENE_BASE, SRC_BARS, UnitState, mixer_create, mixer_create_unit,
    mixer_define_scene, mixer_destroy, mixer_unit_set_state,
};

fn layer(z: i32) -> OverlayDesc {
    OverlayDesc {
        source_id: SRC_BARS,
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

fn main() {
    let count: usize = env::args().nth(1).and_then(|v| v.parse().ok()).unwrap_or(1);
    unsafe {
        assert_eq!(mixer_create(0, 60_000, 1_001), OK);
        assert_eq!(mixer_create_unit(1, 1920, 1080), OK);
        let layers: Vec<OverlayDesc> = (0..count).map(|z| layer(z as i32)).collect();
        for scene in [1u64, 2] {
            assert_eq!(
                mixer_define_scene(
                    SCENE_BASE | scene,
                    1920,
                    1080,
                    layers.len() as u32,
                    layers.as_ptr()
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
    }
    thread::sleep(Duration::from_secs(20));
    unsafe { mixer_destroy() };
    println!("layers={count} done");
}
