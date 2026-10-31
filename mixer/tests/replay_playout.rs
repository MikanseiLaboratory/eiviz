//! The Replay picture on an output, end to end.
//!
//! The recorder writes what the angle shows, the channel plays the marked range
//! back while the recording continues, and a scene that shows the channel puts
//! that picture on the program output. This is the operation itself, not just the
//! transport's idea of it.
//!
//! Loading a Pro module is permanent, so this lives in its own test binary.

mod common;

use std::ffi::CString;
use std::thread;
use std::time::{Duration, Instant};

use common::{
    cstring, expect, full_layer, is_black, is_red, load_test_module, mark_in_when_ready,
    mark_out_after, scene_id, scene_pixel,
};
use eiviz_mixer::{
    GEN_SOLID, MixerReplayChannelState, SRC_COLOR, UnitState, mixer_create, mixer_create_unit,
    mixer_define_generator, mixer_define_scene, mixer_destroy, mixer_last_error,
    mixer_replay_channel_cue, mixer_replay_channel_transport, mixer_replay_configure,
    mixer_replay_copy_channels, mixer_replay_mark_in, mixer_replay_mark_out, mixer_replay_record,
    mixer_unit_set_state,
};

/// The channel's own source id: `REPLAY_CHANNEL_BASE` with the channel selected by
/// `inputId: 0` in the document.
const CHANNEL_SOURCE: u64 = 0x0008_0001;

/// The reason the mixer gave for the last refusal.
fn last_error() -> String {
    let mut buffer = vec![0u8; 512];
    unsafe { mixer_last_error(buffer.as_mut_ptr(), buffer.len()) };
    String::from_utf8_lossy(&buffer)
        .trim_end_matches('\0')
        .to_string()
}

/// What the channel says it is doing, for the playback assertions.
fn channel_state() -> MixerReplayChannelState {
    let mut channels = vec![MixerReplayChannelState::default(); 4];
    let count = unsafe { mixer_replay_copy_channels(channels.as_mut_ptr(), 4) };
    if count >= 1 {
        channels[0]
    } else {
        MixerReplayChannelState::default()
    }
}

#[test]
fn the_replay_picture_reaches_the_program_scene() {
    load_test_module("replay-playout");

    mixer_destroy();
    expect("create", mixer_create(0, 60_000, 1_001));
    expect("create unit", mixer_create_unit(1, 320, 180));

    // The angle records a solid red scene.
    expect(
        "generator",
        mixer_define_generator(SRC_COLOR, GEN_SOLID, 1.0, 0.0, 0.0, 1.0, 0),
    );
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
    // The synchronous playout path: the queue is a jitter guard, so the path it
    // falls back to has to keep working on its own.
    let config = CString::new(format!(
        r#"{{"enabled":true,"storage":[{{"path":"{path}","angleIds":[1]}}],
"angles":[{{"id":1,"name":"A","sceneId":1,"audio":"Silent"}}],
"activeAngleId":1,
"channels":[{{"index":0,"playlistId":null,"inputId":0}}],
"playlists":[],"events":[],
"format":{{"width":320,"height":180,"fpsNum":60000,"fpsDen":1001,"profile":"Sq","segmentFrames":120}},
"buffering":{{"encodeQueueFrames":6,"writeBufferMb":64,"overflowPolicy":"DropFrames","reserveMb":0}},"decodeAhead":false}}"#
    ))
    .unwrap();
    expect("configure", unsafe {
        mixer_replay_configure(config.as_ptr())
    });

    // Recording stays on while the operator plays the mark back: that is what a
    // replay deck does, and the frames being marked are the ones just recorded.
    expect("record on", unsafe { mixer_replay_record(1) });
    thread::sleep(Duration::from_millis(1200));
    // A recording numbers its frames from its own first frame, so frame 0 is a
    // recorded frame: Mark In is allowed to sit on it.
    mark_in_when_ready("replay_playout");
    // A clip long enough to watch play: the rate assertions below need more than a
    // few frames of material, so the close waits for the recording to move on.
    let (uuid, in_frame, out_frame) = mark_out_after("replay_playout", 60);
    assert!(out_frame > in_frame, "the mark must span recorded frames");
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
    let position = if count >= 1 {
        channels[0].position_frame
    } else {
        0
    };
    assert!(
        is_red(pixel),
        "the recorded angle must reach the program through the channel: \
         pixel {pixel:?}, mark {in_frame}..{out_frame}, position {position}"
    );

    // Playback runs in real time: the channel moves through the clip at the
    // recording's frame rate, not one frame at a time. This is what an operator
    // sees, so it is measured rather than assumed. The band is wide because a
    // software adapter decodes slowly: the point is that it moves on its own and
    // does not run away, not that this machine hits the frame rate.
    let start = channel_state();
    thread::sleep(Duration::from_millis(600));
    let moved = channel_state();
    let advanced = moved.position_frame.saturating_sub(start.position_frame);
    assert!(
        (5..=90).contains(&advanced),
        "playback advances in real time: {advanced} frames in 600ms \
         ({start:?} -> {moved:?})"
    );

    // And it stops at the end of the clip instead of running past it. The budget is
    // generous: a clip of a second is several seconds of wall clock on a software
    // decoder under load, and a shared runner has taken minutes for this suite.
    let deadline = Instant::now() + Duration::from_secs(240);
    let mut ended = channel_state();
    while Instant::now() < deadline {
        ended = channel_state();
        if ended.playing == 0 {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(ended.playing, 0, "the clip ends instead of running on");
    assert!(
        ended.position_frame <= out_frame,
        "the channel parks inside the clip: {} > {out_frame}",
        ended.position_frame
    );

    // Recording stops, and the material it recorded stays a replay: marking is a
    // timeline operation, and a clip recorded a moment ago still plays back.
    expect("record off", unsafe { mixer_replay_record(0) });
    let mut mark_in_after_stop = 0u64;
    expect("mark in after stop", unsafe {
        mixer_replay_mark_in(&mut mark_in_after_stop)
    });
    assert!(
        mark_in_after_stop > 0,
        "a stopped recorder still has a timeline to mark"
    );
    // Nothing is recorded after that mark, so closing it would make a clip that can
    // never show anything. The refusal says so, and the mark stays open.
    let mut out_after_stop = 0u64;
    let mut uuid_after = vec![0u8; 64];
    let code = unsafe {
        mixer_replay_mark_out(
            &mut mark_in_after_stop,
            &mut out_after_stop,
            uuid_after.as_mut_ptr(),
            uuid_after.len(),
        )
    };
    assert_ne!(code, 0, "an empty mark must not become a clip");
    let message = last_error();
    assert!(
        message.contains("empty"),
        "the refusal explains the empty mark: {message}"
    );

    // Pause the channel so the next play starts from the cued clip again.
    expect("pause", unsafe {
        mixer_replay_channel_transport(0, 0, 1000, -1)
    });
    expect("cue again", unsafe {
        mixer_replay_channel_cue(0, 1, in_frame, out_frame, uuid_c.as_ptr())
    });
    expect("play again", unsafe {
        mixer_replay_channel_transport(0, 1, 1000, -1)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut after_stop = [0u8, 0, 0];
    while Instant::now() < deadline {
        after_stop = scene_pixel(scene_id(2), 160, 90);
        if is_red(after_stop) {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    mixer_destroy();

    assert!(
        is_red(after_stop),
        "a clip recorded before the stop must still play: {after_stop:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
