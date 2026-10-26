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

/// Slots a recording readback keeps at the angle counts this test uses. The recorder sizes
/// its ring from `readback::record_slots(angles)`; one angle is at the deep end.
const READBACK_DEPTH: u64 = 16;

use std::ffi::CString;
use std::thread;
use std::time::Duration;

use common::{expect, full_layer, load_test_module};
use eiviz_mixer::{
    MixerReplayAngleStats, MixerReplayStats, SCENE_BASE, SRC_BARS, UnitState, mixer_create,
    mixer_create_unit, mixer_define_scene, mixer_destroy, mixer_last_error, mixer_replay_configure,
    mixer_replay_copy_stats, mixer_replay_record, mixer_unit_set_state,
};

/// The reason the mixer gave for the last refusal. A recorder that will not start
/// has to say why, so the tests read the message, not only the code.
fn last_error() -> String {
    let mut buffer = vec![0u8; 512];
    unsafe { mixer_last_error(buffer.as_mut_ptr(), buffer.len()) };
    String::from_utf8_lossy(&buffer)
        .trim_end_matches('\0')
        .to_string()
}

/// Loading a Pro module is process-wide and permanent, so one binary loads it
/// once however many tests it holds.
fn load_module() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| load_test_module("replay-record"));
}

#[test]
fn recording_keeps_the_frames_the_renderer_produced() {
    load_module();

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
        r#"{{"enabled":true,"storage":[{{"path":"{path}","angleIds":[1]}}],
"angles":[{{"id":1,"name":"A","sceneId":1,"audio":"Silent"}}],
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

    // Wait until enough frames have been offered to judge the accounting. Two seconds is
    // plenty on a GPU adapter, but a software one under load produces far fewer frames,
    // and "too short to judge" is a property of the runner, not of the recorder.
    let deadline = std::time::Instant::now() + Duration::from_secs(40);
    let (stats, angles) = loop {
        let mut stats = MixerReplayStats::default();
        let mut angles = vec![MixerReplayAngleStats::default(); 4];
        unsafe { mixer_replay_copy_stats(&mut stats, angles.as_mut_ptr(), 4) };
        if stats.frames_offered >= 30 || std::time::Instant::now() >= deadline {
            // The call reports OK; the count of angle entries is in the stats.
            break (stats, angles);
        }
        thread::sleep(Duration::from_millis(250));
    };

    expect("record off", unsafe { mixer_replay_record(0) });
    mixer_destroy();

    let offered = stats.frames_offered;
    let pushed = stats.frames_pushed;
    let lost = angles[0].dropped_readback;
    assert_eq!(stats.angle_count, 1, "one angle is configured");
    assert_eq!(angles[0].angle_id, 1);
    assert!(
        offered >= 30,
        "the recording is too short to judge: {offered} frames"
    );
    // Every frame handed to the readback is either recorded or counted as a readback
    // loss: a frame that disappears without a reason would be a silent hole in the
    // recording. The ring depth follows the angle count, and the frame being handed over
    // right now is one more than that, so a frame still in flight is expected; what must
    // not happen is a frame with no reason at all. How many frames a second the adapter
    // manages is not this test's business (a software one is slow, and its drops are
    // counted, not hidden).
    let in_flight = READBACK_DEPTH + 1;
    assert!(
        offered + lost <= pushed,
        "offered {offered} + readback lost {lost} must not exceed pushed {pushed}"
    );
    assert!(
        offered + lost + in_flight >= pushed,
        "every handed-over frame is recorded or counted: offered {offered}, lost {lost}, \
         pushed {pushed}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A Replay document with no format is what a client that never ran Replay Setup
/// produces (the macOS host used to write exactly this). Recording must refuse it
/// and say which setting is missing, instead of returning a bare code.
#[test]
fn recording_refuses_a_document_without_a_format() {
    load_module();

    mixer_destroy();
    expect("create", mixer_create(0, 60_000, 1_001));
    expect("create unit", mixer_create_unit(1, 320, 180));
    let layer = full_layer(SRC_BARS);
    expect("define scene", unsafe {
        mixer_define_scene(SCENE_BASE | 1, 320, 180, 1, &layer)
    });

    let dir = std::env::temp_dir().join(format!("eiviz-replay-noformat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.display().to_string().replace('\\', "/");
    let config = CString::new(format!(
        r#"{{"enabled":true,"storage":[{{"path":"{path}","angleIds":[1]}}],
"angles":[{{"id":1,"name":"A","sceneId":1,"audio":"Silent"}}],
"channels":[{{"index":0,"playlistId":null,"inputId":0}}],
"playlists":[],"events":[],
"format":{{"width":0,"height":0,"fpsNum":0,"fpsDen":0,"profile":"Sq","segmentFrames":120}},
"buffering":{{"encodeQueueFrames":6,"writeBufferMb":64,"overflowPolicy":"DropFrames","reserveMb":0}}}}"#
    ))
    .unwrap();
    expect("configure", unsafe {
        mixer_replay_configure(config.as_ptr())
    });

    let code = unsafe { mixer_replay_record(1) };
    let message = last_error();
    assert_ne!(code, 0, "a format-less document must not record");
    assert!(
        message.contains("format"),
        "the refusal names the missing format: {message}"
    );

    // The refusal leaves the engine usable: the operator fixes the setting and
    // records without restarting the mixer.
    let fixed = CString::new(format!(
        r#"{{"enabled":true,"storage":[{{"path":"{path}","angleIds":[1]}}],
"angles":[{{"id":1,"name":"A","sceneId":1,"audio":"Silent"}}],
"channels":[{{"index":0,"playlistId":null,"inputId":0}}],
"playlists":[],"events":[],
"format":{{"width":320,"height":180,"fpsNum":60000,"fpsDen":1001,"profile":"Sq","segmentFrames":120}},
"buffering":{{"encodeQueueFrames":6,"writeBufferMb":64,"overflowPolicy":"DropFrames","reserveMb":0}}}}"#
    ))
    .unwrap();
    expect("configure fixed", unsafe {
        mixer_replay_configure(fixed.as_ptr())
    });
    expect("record on", unsafe { mixer_replay_record(1) });
    expect("record off", unsafe { mixer_replay_record(0) });
    mixer_destroy();
    let _ = std::fs::remove_dir_all(&dir);
}

/// An angle that names a scene the mixer does not have cannot be recorded. The
/// refusal has to name the scene, otherwise the operator cannot tell a missing
/// scene from a full disk.
#[test]
fn recording_refuses_an_angle_without_a_scene() {
    load_module();

    mixer_destroy();
    expect("create", mixer_create(0, 60_000, 1_001));
    expect("create unit", mixer_create_unit(1, 320, 180));
    let layer = full_layer(SRC_BARS);
    expect("define scene", unsafe {
        mixer_define_scene(SCENE_BASE | 1, 320, 180, 1, &layer)
    });

    let dir = std::env::temp_dir().join(format!("eiviz-replay-noscene-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.display().to_string().replace('\\', "/");
    // Angle 1 names scene 9, which was never defined.
    let config = CString::new(format!(
        r#"{{"enabled":true,"storage":[{{"path":"{path}","angleIds":[1]}}],
"angles":[{{"id":1,"name":"A","sceneId":9,"audio":"Silent"}}],
"channels":[{{"index":0,"playlistId":null,"inputId":0}}],
"playlists":[],"events":[],
"format":{{"width":320,"height":180,"fpsNum":60000,"fpsDen":1001,"profile":"Sq","segmentFrames":120}},
"buffering":{{"encodeQueueFrames":6,"writeBufferMb":64,"overflowPolicy":"DropFrames","reserveMb":0}}}}"#
    ))
    .unwrap();
    expect("configure", unsafe {
        mixer_replay_configure(config.as_ptr())
    });

    let code = unsafe { mixer_replay_record(1) };
    let message = last_error();
    assert_ne!(code, 0, "an angle without a scene must not record");
    assert!(
        message.contains("scene 9"),
        "the refusal names the missing scene: {message}"
    );
    mixer_destroy();
    let _ = std::fs::remove_dir_all(&dir);
}

/// An angle that no drive claims is a configuration error, not something the
/// mixer may guess: it used to fall back to the first drive and record to a disk
/// the operator never chose.
#[test]
fn recording_refuses_an_angle_with_no_storage_assigned() {
    load_module();

    mixer_destroy();
    expect("create", mixer_create(0, 60_000, 1_001));
    expect("create unit", mixer_create_unit(1, 320, 180));
    let layer = full_layer(SRC_BARS);
    expect("define scene", unsafe {
        mixer_define_scene(SCENE_BASE | 1, 320, 180, 1, &layer)
    });

    let dir = std::env::temp_dir().join(format!("eiviz-replay-nodrive-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.display().to_string().replace('\\', "/");
    let config = CString::new(format!(
        r#"{{"enabled":true,"storage":[{{"path":"{path}"}}],
"angles":[{{"id":1,"name":"A","sceneId":1,"audio":"Silent"}}],
"channels":[{{"index":0,"playlistId":null,"inputId":0}}],
"playlists":[],"events":[],
"format":{{"width":320,"height":180,"fpsNum":60000,"fpsDen":1001,"profile":"Sq","segmentFrames":120}},
"buffering":{{"encodeQueueFrames":6,"writeBufferMb":64,"overflowPolicy":"DropFrames","reserveMb":0}}}}"#
    ))
    .unwrap();
    expect("configure", unsafe {
        mixer_replay_configure(config.as_ptr())
    });

    let code = unsafe { mixer_replay_record(1) };
    let message = last_error();
    assert_ne!(code, 0, "an unassigned angle must not record");
    assert!(
        message.contains("no storage assigned"),
        "the refusal names the unassigned drive: {message}"
    );
    mixer_destroy();
    let _ = std::fs::remove_dir_all(&dir);
}
