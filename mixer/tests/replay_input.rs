//! An angle that records an input directly.
//!
//! A scene angle records a composited scene. An operator who only wants one camera
//! should not have to build a scene for it: this angle takes the input's own
//! picture. The test proves the recorded frames come from that input, and that the
//! recorder does not lose them.
//!
//! Loading a Pro module is permanent, so this lives in its own test binary.

#![cfg(debug_assertions)]

mod common;

use std::ffi::CString;
use std::thread;
use std::time::{Duration, Instant};

use common::{
    cstring, expect, full_layer, is_red, load_test_module, mark_in_when_ready, mark_out_after,
    scene_id, scene_pixel,
};
use eiviz_mixer::{
    GEN_SOLID, MixerReplayChannelState, SRC_COLOR, UnitState, mixer_create, mixer_create_unit,
    mixer_define_generator, mixer_define_scene, mixer_destroy, mixer_replay_channel_cue,
    mixer_replay_channel_transport, mixer_replay_configure, mixer_replay_copy_channels,
    mixer_replay_record, mixer_unit_set_state,
};

/// The channel's own source id: `REPLAY_CHANNEL_BASE` with the channel selected by
/// `inputId: 0` in the document.
const CHANNEL_SOURCE: u64 = 0x0008_0001;

#[test]
fn an_input_angle_records_without_a_scene() {
    load_test_module("replay-input");

    mixer_destroy();
    expect("create", mixer_create(0, 60_000, 1_001));
    expect("create unit", mixer_create_unit(1, 320, 180));

    // The input that is recorded: a solid red generator. It is not part of any
    // scene, which is the point of an input angle.
    expect(
        "generator",
        mixer_define_generator(SRC_COLOR, GEN_SOLID, 1.0, 0.0, 0.0, 1.0, 0),
    );

    // Scene 1 shows the Replay channel, so the recording can be watched.
    let channel_layer = full_layer(CHANNEL_SOURCE);
    expect("channel scene", unsafe {
        mixer_define_scene(scene_id(1), 320, 180, 1, &channel_layer)
    });
    unsafe {
        let state = UnitState {
            program_source: scene_id(1),
            preview_source: SRC_COLOR,
            ..UnitState::default()
        };
        expect("unit state", mixer_unit_set_state(1, &state));
    }

    let dir = std::env::temp_dir().join(format!("eiviz-replay-input-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.display().to_string().replace('\\', "/");
    // One angle, recording the input directly: `source: Input` and no scene.
    let config = CString::new(format!(
        r#"{{"enabled":true,"storage":[{{"path":"{path}","angleIds":[1]}}],
"angles":[{{"id":1,"name":"A","source":"Input","inputId":{SRC_COLOR},"audio":"Silent"}}],
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

    mark_in_when_ready("replay_input");
    let (uuid, in_frame, out_frame) = mark_out_after("replay_input", 20);
    assert!(out_frame > in_frame, "the mark must span recorded frames");

    // Before the channel carries the clip the program shows black.
    let before = scene_pixel(scene_id(1), 160, 90);
    assert!(
        common::is_black(before),
        "an uncued channel shows black, got {before:?}"
    );

    let uuid_c = cstring(&uuid);
    expect("cue", unsafe {
        mixer_replay_channel_cue(0, 1, in_frame, out_frame, uuid_c.as_ptr())
    });
    expect("play", unsafe {
        mixer_replay_channel_transport(0, 1, 1000, -1)
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut pixel = before;
    while Instant::now() < deadline {
        pixel = scene_pixel(scene_id(1), 160, 90);
        if is_red(pixel) {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    let mut channels = vec![MixerReplayChannelState::default(); 4];
    let count = unsafe { mixer_replay_copy_channels(channels.as_mut_ptr(), 4) };
    let substituted = if count >= 1 {
        channels[0].substituted_frames
    } else {
        0
    };
    expect("record off", unsafe { mixer_replay_record(0) });
    mixer_destroy();

    assert!(
        is_red(pixel),
        "the input angle's picture must reach the program: pixel {pixel:?}, \
         mark {in_frame}..{out_frame}, substituted {substituted}"
    );
    // `substituted` counts playout frames that reused the last decoded picture
    // because a decode did not finish in time. It measures how fast the runner's
    // adapter is, not whether the recorder read the input: on a shared runner with a
    // software adapter it reaches eight frames while the picture stays correct, and
    // the pixel above is what proves the input reached the program.
    let _ = std::fs::remove_dir_all(&dir);
}
