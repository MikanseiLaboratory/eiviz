//! Recording an angle that is not on air.
//!
//! A replay operator records every camera, not only the one on the program bus, so
//! the recorder must get frames for a scene that no unit is showing. This test
//! covers exactly that, because the first attempt at it produced no frames at all.
//!
//! Loading a Pro module is permanent, so this lives in its own test binary.

#![cfg(debug_assertions)]

mod common;

use std::ffi::CString;
use std::thread;
use std::time::Duration;

use common::{expect, full_layer, load_test_module, scene_id};
use eiviz_mixer::{
    MixerReplayAngleStats, MixerReplayStats, SRC_BARS, SRC_BLUE, SRC_COLOR, UnitState,
    mixer_create, mixer_create_unit, mixer_define_scene, mixer_destroy, mixer_replay_configure,
    mixer_replay_copy_stats, mixer_replay_record, mixer_unit_set_state,
};

#[test]
fn an_angle_that_is_not_on_air_still_records() {
    load_test_module("replay-offair");

    mixer_destroy();
    expect("create", mixer_create(0, 60_000, 1_001));
    expect("create unit", mixer_create_unit(1, 320, 180));

    // Scene 1 is the recording angle. It is deliberately not the program source:
    // the unit shows scene 2 instead.
    let angle_layer = full_layer(SRC_COLOR);
    expect("angle scene", unsafe {
        mixer_define_scene(scene_id(1), 320, 180, 1, &angle_layer)
    });
    let program_layer = full_layer(SRC_BARS);
    expect("program scene", unsafe {
        mixer_define_scene(scene_id(2), 320, 180, 1, &program_layer)
    });
    unsafe {
        let state = UnitState {
            program_source: scene_id(2),
            preview_source: SRC_BLUE,
            ..UnitState::default()
        };
        expect("unit state", mixer_unit_set_state(1, &state));
    }

    let dir = std::env::temp_dir().join(format!("eiviz-replay-offair-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.display().to_string().replace('\\', "/");
    let config = CString::new(format!(
        r#"{{"enabled":true,"storage":[{{"path":"{path}","angleIds":[1]}}],
"angles":[{{"id":1,"name":"A","sceneId":1,"audio":"Silent","recordMode":"Composite"}}],
"activeAngleId":1,
"channels":[{{"index":0,"playlistId":null,"inputId":0}}],
"playlists":[],"events":[],
"format":{{"width":320,"height":180,"fpsNum":60000,"fpsDen":1001,"profile":"Sq","segmentFrames":120}},
"buffering":{{"encodeQueueFrames":6,"writeBufferMb":64,"overflowPolicy":"DropFrames","reserveMb":0}}}}"#
    ))
    .unwrap();
    expect("configure", unsafe {
        mixer_replay_configure(config.as_ptr())
    });
    expect("record on", unsafe { mixer_replay_record(1) });
    thread::sleep(Duration::from_secs(2));

    let mut stats = MixerReplayStats::default();
    let mut angles = vec![MixerReplayAngleStats::default(); 4];
    unsafe { mixer_replay_copy_stats(&mut stats, angles.as_mut_ptr(), 4) };
    expect("record off", unsafe { mixer_replay_record(0) });
    mixer_destroy();

    // The stats are read while recording is still on, so state is recording.
    assert_eq!(stats.angle_count, 1);
    assert!(
        stats.frames_pushed > 0,
        "the angle scene must be rendered and handed to the readback; \
         pushed {} offered {} readback lost {}",
        stats.frames_pushed,
        stats.frames_offered,
        angles[0].dropped_readback
    );
    assert!(
        stats.frames_offered + 3 >= stats.frames_pushed,
        "offered {} of {} frames handed to the readback",
        stats.frames_offered,
        stats.frames_pushed
    );
    let _ = std::fs::remove_dir_all(&dir);
}


