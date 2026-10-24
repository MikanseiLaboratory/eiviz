//! The operator flow end to end, through the same C ABI the hosts call:
//! record, mark in and out, cue the clip on a channel, and play it out.
//!
//! Loading a Pro module is permanent, so this lives in its own test binary.

#![cfg(debug_assertions)]

mod common;

use std::ffi::CString;
use std::thread;
use std::time::Duration;

use common::{
    cstring, expect, full_layer, load_test_module, mark_in_when_ready, mark_out_after,
    mark_out_when_ready,
};
use eiviz_mixer::{
    MixerReplayChannelState, SCENE_BASE, SRC_BARS, UnitState, mixer_create, mixer_create_unit,
    mixer_define_scene, mixer_destroy, mixer_replay_channel_cue, mixer_replay_channel_transport,
    mixer_replay_configure, mixer_replay_copy_channels, mixer_replay_mark_in,
    mixer_replay_mark_out, mixer_replay_record, mixer_unit_set_state,
};

#[test]
fn an_operator_records_marks_and_plays_back() {
    load_test_module("replay-flow");

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

    let dir = std::env::temp_dir().join(format!("eiviz-replay-flow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.display().to_string().replace('\\', "/");
    let config = CString::new(format!(
        r#"{{"enabled":true,"storage":[{{"path":"{path}","angleIds":[1]}}],
"angles":[{{"id":1,"name":"A","sceneId":1,"audio":"Silent"}}],
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
    thread::sleep(Duration::from_millis(1200));

    // Mark In, let more of the recording land, then Mark Out. Both wait for the
    // recording to reach the frame they need: a software adapter takes longer than
    // any fixed sleep to produce one.
    // A recording numbers its frames from its own first frame, so frame 0 is a
    // recorded frame: Mark In is allowed to sit on it.
    let in_frame = mark_in_when_ready("replay_flow");
    let (uuid, in_frame, out_frame) = mark_out_after("replay_flow", 40);
    assert!(
        out_frame > in_frame,
        "Mark Out {out_frame} must sit after Mark In {in_frame}"
    );
    assert!(!uuid.is_empty(), "an event must name its recording");
    expect("record off", unsafe { mixer_replay_record(0) });

    // Cue the clip the operator just marked, then play it.
    let uuid_c = cstring(&uuid);
    expect("cue", unsafe {
        mixer_replay_channel_cue(0, 1, in_frame, out_frame, uuid_c.as_ptr())
    });
    let mut channels = vec![MixerReplayChannelState::default(); 4];
    let count = unsafe { mixer_replay_copy_channels(channels.as_mut_ptr(), 4) };
    assert!(count >= 1, "the channel must exist");
    assert_eq!(channels[0].channel_index, 0);
    assert_eq!(channels[0].in_frame, in_frame);
    assert_eq!(channels[0].out_frame, out_frame);
    assert_eq!(channels[0].angle_id, 1);
    assert_eq!(channels[0].playing, 0, "a cue leaves the channel paused");

    // Play. A negative seek frame means "do not move the position".
    expect("play", unsafe {
        mixer_replay_channel_transport(0, 1, 1000, -1)
    });
    thread::sleep(Duration::from_millis(400));
    let count = unsafe { mixer_replay_copy_channels(channels.as_mut_ptr(), 4) };
    assert!(count >= 1);
    assert_eq!(channels[0].playing, 1, "the channel should be playing");
    assert!(
        channels[0].position_frame > in_frame,
        "playback should advance past Mark In: position {} in {}",
        channels[0].position_frame,
        in_frame
    );

    // Pause and check the transport is where it was told to stay.
    expect("pause", unsafe {
        mixer_replay_channel_transport(0, 0, 1000, -1)
    });
    let count = unsafe { mixer_replay_copy_channels(channels.as_mut_ptr(), 4) };
    assert!(count >= 1);
    assert_eq!(channels[0].playing, 0, "the channel should be paused");

    mixer_destroy();
    let _ = std::fs::remove_dir_all(&dir);
}
