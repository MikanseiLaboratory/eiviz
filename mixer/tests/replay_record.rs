//! Recording delivery: the recorder must keep nearly every frame the renderer
//! produced.
//!
//! The readback used to map one slot per frame and hand over a single frame, so a
//! recording followed the map latency (roughly one frame in three) instead of the
//! render rate. This test drives a real mixer and compares what the renderer
//! produced with what was handed to the readback.
//!
//! Loading a Pro module is permanent, so this lives in its own test binary.

#![cfg(debug_assertions)]

mod common;

use std::ffi::CString;
use std::thread;
use std::time::Duration;

use common::{expect, full_layer, load_test_module};
use eiviz_mixer::{
    MixerReplayAngleStats, MixerReplayStats, SCENE_BASE, SRC_BARS, UnitState, mixer_create,
    mixer_create_unit, mixer_define_scene, mixer_destroy, mixer_replay_configure,
    mixer_replay_copy_stats, mixer_replay_record, mixer_unit_set_state,
};

#[test]
fn recording_keeps_the_frames_the_renderer_produced() {
    load_test_module("replay-record");

    mixer_destroy();
    expect("create", mixer_create(0, 60_000, 1_001));
    expect("create unit", mixer_create_unit(1, 320, 180));
    let layer = full_layer(SRC_BARS);
    expect("define scene", unsafe {
        mixer_define_scene(SCENE_BASE | 1, 320, 180, 1, &layer)
    });
    unsafe {
        let state = UnitState {
            program_source: SCENE_BASE | 1,
            preview_source: SRC_BARS,
            ..UnitState::default()
        };
        expect("unit state", mixer_unit_set_state(1, &state));
    }

    // One angle on the scene above, recording to a scratch directory.
    let dir = std::env::temp_dir().join(format!("eiviz-replay-record-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.display().to_string().replace('\\', "/");
    let config = CString::new(format!(
        r#"{{"enabled":true,"storage":[{{"path":"{path}"}}],
"angles":[{{"id":1,"name":"A","sceneId":1,"audio":"Silent","recordMode":"Composite"}}],
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

    // Let it render and record for a couple of seconds.
    thread::sleep(Duration::from_secs(2));

    let mut stats = MixerReplayStats::default();
    let mut angles = vec![MixerReplayAngleStats::default(); 4];
    // The call reports OK; the count of angle entries is in the stats.
    unsafe { mixer_replay_copy_stats(&mut stats, angles.as_mut_ptr(), 4) };
    expect("record off", unsafe { mixer_replay_record(0) });
    mixer_destroy();

    let offered = stats.frames_offered;
    let pushed = stats.frames_pushed;
    assert_eq!(stats.angle_count, 1, "one angle is configured");
    assert_eq!(angles[0].angle_id, 1);
    assert!(
        offered >= 30,
        "the recording is too short to judge: {offered} frames"
    );
    // The last couple of copies may still be in flight when the stats are read,
    // so allow the map latency but not a systematic loss.
    assert!(
        offered + 3 >= pushed,
        "offered {offered} of {pushed} frames handed to the readback"
    );
    assert!(
        angles[0].dropped_readback <= 2,
        "the readback dropped {} frames",
        angles[0].dropped_readback
    );
    let _ = std::fs::remove_dir_all(&dir);
}

