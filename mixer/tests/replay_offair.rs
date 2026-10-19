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
    thread::sleep(Duration::from_millis(1500));

    let mut first = MixerReplayStats::default();
    let mut first_angles = vec![MixerReplayAngleStats::default(); 4];
    unsafe { mixer_replay_copy_stats(&mut first, first_angles.as_mut_ptr(), 4) };

    thread::sleep(Duration::from_millis(1500));
    let mut stats = MixerReplayStats::default();
    let mut angles = vec![MixerReplayAngleStats::default(); 4];
    unsafe { mixer_replay_copy_stats(&mut stats, angles.as_mut_ptr(), 4) };
    expect("record off", unsafe { mixer_replay_record(0) });
    mixer_destroy();

    assert_eq!(stats.angle_count, 1);
    assert!(
        stats.frames_pushed > 0,
        "the angle scene must be rendered and handed to the readback; \
         pushed {} offered {} readback lost {}",
        stats.frames_pushed,
        stats.frames_offered,
        angles[0].dropped_readback
    );
    // An off-air angle keeps recording: the recorder receives its frames and keeps
    // receiving them. How many frames per second arrive depends on the adapter (a
    // software one is slow), so this measures that recording progresses rather than
    // a rate.
    assert!(
        stats.frames_offered > first.frames_offered,
        "the recorder must keep receiving frames from an off-air angle: \
         offered {} then {} of {} handed over",
        first.frames_offered,
        stats.frames_offered,
        stats.frames_pushed
    );
    // Nothing is invented: every frame the recorder reports was handed to the
    // readback, and what it could not take is counted as a readback loss. The
    // readback keeps a small ring of copies in flight, so a few frames may still be
    // on their way; what must not happen is a frame that disappears without a reason.
    assert!(
        stats.frames_offered + u64::from(angles[0].dropped_readback) <= stats.frames_pushed,
        "offered {} + readback lost {} must not exceed pushed {}",
        stats.frames_offered,
        angles[0].dropped_readback,
        stats.frames_pushed
    );
    assert!(
        stats.frames_offered + u64::from(angles[0].dropped_readback) + 8 >= stats.frames_pushed,
        "every handed-over frame is recorded or counted: offered {}, lost {}, pushed {}",
        stats.frames_offered,
        angles[0].dropped_readback,
        stats.frames_pushed
    );
    let _ = std::fs::remove_dir_all(&dir);
}


