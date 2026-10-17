//! The Replay picture on an output, end to end.
//!
//! The recorder writes what the angle shows, the channel plays the marked range
//! back while the recording continues, and a scene that shows the channel puts
//! that picture on the program output. This is the operation itself, not just the
//! transport's idea of it.
//!
//! Loading a Pro module is permanent, so this lives in its own test binary.

#![cfg(debug_assertions)]

mod common;

use std::ffi::CString;
use std::thread;
use std::time::{Duration, Instant};

use common::{cstring, expect, full_layer, is_black, is_red, load_test_module, scene_id, scene_pixel};
use eiviz_mixer::{
    GEN_SOLID, MixerReplayChannelState, SRC_COLOR, UnitState, mixer_create, mixer_create_unit,
    mixer_define_generator, mixer_define_scene, mixer_destroy, mixer_replay_channel_cue,
    mixer_replay_channel_transport, mixer_replay_configure, mixer_replay_copy_channels,
    mixer_replay_mark_in, mixer_replay_mark_out, mixer_replay_record, mixer_unit_set_state,
};

/// The channel's own source id: `REPLAY_CHANNEL_BASE` with the channel selected by
/// `inputId: 0` in the document.
const CHANNEL_SOURCE: u64 = 0x0008_0001;

#[test]
fn the_replay_picture_reaches_the_program_scene() {
    load_test_module("replay-playout");

    mixer_destroy();
    expect("create", mixer_create(0, 60_000, 1_001));
    expect("create unit", mixer_create_unit(1, 320, 180));

    // The angle records a solid red scene.
    expect("generator", mixer_define_generator(
        SRC_COLOR, GEN_SOLID, 1.0, 0.0, 0.0, 1.0, 0
    ));
    let angle_layer = full_layer(SRC_COLOR);
    expect("angle scene", unsafe {
        mixer_define_scene(scene_id(1), 320, 180, 1, &angle_layer)
    });

    // The program scene shows the Replay channel, which is how a channel goes out.
    let channel_layer = full_layer(CHANNEL_SOURCE);
    expect("program scene", unsafe {
        mixer_define_scene(scene_id(2), 320, 180, 1, &channel_layer)
    });
    unsafe {
        let state = UnitState {
            program_source: scene_id(2),
            preview_source: SRC_COLOR,
            ..UnitState::default()
        };
        expect("unit state", mixer_unit_set_state(1, &state));
    }

    let dir = std::env::temp_dir().join(format!("eiviz-replay-playout-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.display().to_string().replace('\\', "/");
    let config = CString::new(format!(
        r#"{{"enabled":true,"storage":[{{"path":"{path}"}}],
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

    // Recording stays on while the operator plays the mark back: that is what a
    // replay deck does, and the frames being marked are the ones just recorded.
    expect("record on", unsafe { mixer_replay_record(1) });
    thread::sleep(Duration::from_millis(1200));
    let mut in_frame = 0u64;
    expect("mark in", unsafe { mixer_replay_mark_in(&mut in_frame) });
    assert!(in_frame > 0, "Mark In should sit on a recorded frame");
    thread::sleep(Duration::from_millis(400));
    let mut out_frame = 0u64;
    let mut uuid = vec![0u8; 64];
    expect("mark out", unsafe {
        mixer_replay_mark_out(
            &mut in_frame,
            &mut out_frame,
            uuid.as_mut_ptr(),
            uuid.len(),
        )
    });
    assert!(out_frame > in_frame, "the mark must span recorded frames");
    let uuid = String::from_utf8(uuid)
        .unwrap()
        .trim_end_matches('\0')
        .to_string();
    assert!(!uuid.is_empty());

    // Before the channel carries a clip the program shows black, not the
    // recording and not a green placeholder.
    let before = scene_pixel(scene_id(2), 160, 90);
    assert!(
        is_black(before),
        "an uncued channel shows black, got {before:?}"
    );

    // Cue the marked range, play it, and wait for the picture to arrive.
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
        pixel = scene_pixel(scene_id(2), 160, 90);
        if is_red(pixel) {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    let mut channels = vec![MixerReplayChannelState::default(); 4];
    let count = unsafe { mixer_replay_copy_channels(channels.as_mut_ptr(), 4) };
    let position = if count >= 1 { channels[0].position_frame } else { 0 };
    expect("record off", unsafe { mixer_replay_record(0) });
    mixer_destroy();

    assert!(
        is_red(pixel),
        "the recorded angle must reach the program through the channel: \
         pixel {pixel:?}, mark {in_frame}..{out_frame}, position {position}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
