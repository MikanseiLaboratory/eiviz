//! An animation step cues Replay: the session names a channel and a clip, and the
//! sequence fires it on its own timecode.
//!
//! The mixer only renders while something asks for picture, so the session records
//! during the run, exactly as a real one does. Loading a Pro module is permanent,
//! so this lives in its own test binary.

#![cfg(debug_assertions)]

mod common;

use std::ffi::CString;
use std::thread;
use std::time::Duration;

use common::{expect, full_layer, load_test_module, scene_id};
use eiviz_mixer::{
    EASING_LINEAR, EivizMotion, EivizSceneCamera, EivizSceneSequenceDesc, EivizSceneStateDesc,
    EivizSequenceStepDesc, MixerReplayChannelState, MixerReplayStats, OverlayDesc, SCENE_SEQ_PLAY,
    SRC_BARS, SRC_BLUE, SRC_COLOR, UnitState, mixer_create, mixer_create_unit, mixer_define_scene,
    mixer_destroy, mixer_replay_configure, mixer_replay_copy_channels, mixer_replay_copy_stats,
    mixer_replay_record, mixer_scene_sequence, mixer_scene_sequences_define,
    mixer_scene_states_define, mixer_unit_set_state,
};

/// Linear motion over the given frames.
fn linear(frames: u32) -> EivizMotion {
    EivizMotion {
        duration_frames: frames,
        easing: EASING_LINEAR,
        x1: 0.0,
        y1: 0.0,
        x2: 1.0,
        y2: 1.0,
        has_bezier: 0,
    }
}

#[test]
fn an_animation_step_cues_a_replay_clip() {
    load_test_module("replay-animation");

    mixer_destroy();
    expect("create", mixer_create(0, 60_000, 1_001));
    expect("create unit", mixer_create_unit(1, 320, 180));

    // Scene 1 is the angle's picture, scene 2 is the animated program.
    let angle_layer = full_layer(SRC_COLOR);
    expect("angle scene", unsafe {
        mixer_define_scene(scene_id(1), 320, 180, 1, &angle_layer)
    });
    let program_layer = OverlayDesc {
        layer_id: 1,
        ..full_layer(SRC_BARS)
    };
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

    // A Replay document whose channel playlist holds the clip the animation names.
    let dir = std::env::temp_dir().join(format!("eiviz-replay-anim-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.display().to_string().replace('\\', "/");
    let config = CString::new(format!(
        r#"{{"enabled":true,"storage":[{{"path":"{path}","angleIds":[1]}}],
"angles":[{{"id":1,"name":"A","sceneId":1,"audio":"Silent"}}],
"activeAngleId":1,
"channels":[{{"index":0,"playlistId":7,"inputId":0}}],
"playlists":[{{"id":7,"name":"Replay","revision":1,"clips":[
  {{"uid":"clip-1","source":"Angle","angleId":1,"recordingUuid":"rec","inFrame":0,"outFrame":90,
    "speed":1.0,"gainDb":0.0,"transition":{{"kind":"Cut","timing":"Overwrap","durationFrames":0}}}}]}}],
"events":[],
"format":{{"width":320,"height":180,"fpsNum":60000,"fpsDen":1001,"profile":"Sq","segmentFrames":120}},
"buffering":{{"encodeQueueFrames":6,"writeBufferMb":64,"overflowPolicy":"DropFrames","reserveMb":0}}}}"#
    ))
    .unwrap();
    expect("configure", unsafe {
        mixer_replay_configure(config.as_ptr())
    });
    expect("record on", unsafe { mixer_replay_record(1) });
    thread::sleep(Duration::from_millis(1500));

    // One state, then a two-step sequence whose second step cues the clip and
    // starts it. The first step is a plain step, so only the second one acts.
    let anim_layers = [program_layer];
    let states = [EivizSceneStateDesc {
        id: 1,
        layers: anim_layers.as_ptr(),
        layer_count: anim_layers.len() as u32,
        enter: linear(1),
        camera: EivizSceneCamera::IDENTITY,
        has_camera: 0,
    }];
    expect("define states", unsafe {
        mixer_scene_states_define(scene_id(2), states.as_ptr(), states.len() as u32)
    });
    let clip_uid = b"clip-1";
    let steps = [
        EivizSequenceStepDesc {
            state_id: 1,
            motion: linear(2),
            has_motion: 1,
            hold_frames: 4,
            replay_channel: 0,
            replay_active: 0,
            replay_play: 0,
            replay_clip: std::ptr::null(),
            replay_clip_len: 0,
        },
        EivizSequenceStepDesc {
            state_id: 1,
            motion: linear(2),
            has_motion: 1,
            hold_frames: 4,
            replay_channel: 0,
            replay_active: 1,
            replay_play: 1,
            replay_clip: clip_uid.as_ptr(),
            replay_clip_len: clip_uid.len(),
        },
    ];
    let sequences = [EivizSceneSequenceDesc {
        id: 1,
        steps: steps.as_ptr(),
        step_count: steps.len() as u32,
    }];
    expect("define sequences", unsafe {
        mixer_scene_sequences_define(scene_id(2), sequences.as_ptr(), 1)
    });

    expect("play sequence", unsafe {
        mixer_scene_sequence(scene_id(2), 1, SCENE_SEQ_PLAY)
    });
    // The sequence advances on rendered frames, and a CI runner without a GPU renders
    // them slowly. Wait for the cue with a deadline instead of assuming half a second
    // of wall clock is enough, which made this test fail on slow machines only.
    let mut channels = vec![MixerReplayChannelState::default(); 4];
    let mut count = 0i32;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(200));
        channels = vec![MixerReplayChannelState::default(); 4];
        count = unsafe { mixer_replay_copy_channels(channels.as_mut_ptr(), 4) };
        if count >= 1 && channels[0].out_frame == 90 {
            break;
        }
    }
    if count >= 1 && channels[0].out_frame != 90 {
        // One more read after a short settle, so a frame that landed between the poll
        // and the assertion is still seen.
        thread::sleep(Duration::from_millis(500));
        count = unsafe { mixer_replay_copy_channels(channels.as_mut_ptr(), 4) };
    }
    let mut stats = MixerReplayStats::default();
    unsafe { mixer_replay_copy_stats(&mut stats, std::ptr::null_mut(), 0) };
    expect("record off", unsafe { mixer_replay_record(0) });
    mixer_destroy();

    assert!(count >= 1, "the channel must exist");
    assert!(
        stats.frames_pushed > 0,
        "the session should have been recording while the animation ran"
    );
    assert_eq!(
        channels[0].in_frame, 0,
        "the animation should cue the clip the session named"
    );
    assert_eq!(
        channels[0].out_frame, 90,
        "the cue must carry the clip's own range"
    );
    assert_eq!(channels[0].angle_id, 1);
    assert_eq!(channels[0].playing, 1, "the step asked for playback");
    let _ = std::fs::remove_dir_all(&dir);
}
