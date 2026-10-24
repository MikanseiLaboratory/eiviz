//! Replay without a Pro module.
//!
//! The Community build has no Replay backend, so a project that asks for one must
//! be told that, not handed a code with no sentence. This binary deliberately never
//! loads the test module: loading one is permanent, so it needs its own process.
//!
//! A module that grants the plan but has no Replay backend would hit the mixer's
//! `replay requires the Pro module` branch; today every module that grants the plan
//! also implements Replay, so this test covers the plan refusal.

#![cfg(debug_assertions)]

mod common;

use std::ffi::CString;

use common::full_layer;
use eiviz_mixer::{
    SCENE_BASE, SRC_BARS, mixer_create, mixer_create_unit, mixer_define_scene, mixer_destroy,
    mixer_last_error, mixer_replay_configure, mixer_replay_record,
};

fn last_error() -> String {
    let mut buffer = vec![0u8; 512];
    unsafe { mixer_last_error(buffer.as_mut_ptr(), buffer.len()) };
    String::from_utf8_lossy(&buffer)
        .trim_end_matches('\0')
        .to_string()
}

#[test]
fn replay_refuses_with_a_reason_without_a_pro_module() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), 0, "create");
    assert_eq!(mixer_create_unit(1, 320, 180), 0, "create unit");
    let layer = full_layer(SRC_BARS);
    assert_eq!(
        unsafe { mixer_define_scene(SCENE_BASE | 1, 320, 180, 1, &layer) },
        0,
        "define scene"
    );

    let dir = std::env::temp_dir().join(format!("eiviz-replay-nomodule-{}", std::process::id()));
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

    // The document is accepted so the operator can open the deck and read why it
    // will not record; the refusal moves to the operations themselves.
    let code = unsafe { mixer_replay_configure(config.as_ptr()) };
    let message = last_error();
    assert_eq!(
        code, 0,
        "a plan without Replay keeps the document: {message}"
    );

    // Recording is refused, and the reason names the plan or the module.
    let record_code = unsafe { mixer_replay_record(1) };
    let record_message = last_error();
    assert_ne!(record_code, 0, "recording must not start");
    assert!(
        record_message.contains("plan")
            || record_message.contains("Pro module")
            || record_message.contains("Professional"),
        "the refusal names the plan or the module: {record_message}"
    );
    assert!(
        !record_message.contains("replay command was rejected"),
        "the recording refusal keeps its reason: {record_message}"
    );
    mixer_destroy();
    let _ = std::fs::remove_dir_all(&dir);
}
