//! The vMix-compatible HTTP API driving Replay: cue the clip the operator just
//! marked on a channel, then play it. This is the surface an automation panel or a
//! Stream Deck uses, so it is exercised over real HTTP.
//!
//! Replay needs a plan that allows it, so the test module is loaded and this lives
//! in its own test binary.

#![cfg(debug_assertions)]

mod common;

use std::ffi::CString;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

use common::{expect, load_test_module};
use eiviz_mixer::{
    MixerReplayChannelState, mixer_api_configure, mixer_create, mixer_create_unit, mixer_destroy,
    mixer_replay_configure, mixer_replay_copy_channels, mixer_replay_copy_recording_uuid,
    mixer_replay_record, mixer_session_replace,
};

const PORT: u32 = 18731;

/// The recording the recorder has open, or an empty string.
fn recording_uuid() -> String {
    let buffer = [0u8; 128];
    let mut buffer = buffer;
    let n = unsafe { mixer_replay_copy_recording_uuid(buffer.as_mut_ptr(), buffer.len()) };
    if n <= 0 {
        return String::new();
    }
    String::from_utf8_lossy(&buffer[..n as usize]).into_owned()
}
/// The replay document, as `mixer_replay_configure` takes it. The clips live in the
/// session document; this opens the drive and the angle so the store has somewhere
/// to write and the channel has something to read.
fn replay_configure_json() -> String {
    format!(
        r#"{{"enabled":true,"storage":[{{"path":"{}","angleIds":[1]}}],
"angles":[{{"id":1,"name":"Cam 1","sceneId":1,"audio":"Silent"}}],
"activeAngleId":1,
"channels":[{{"index":0,"playlistId":7,"inputId":524289}}],
"playlists":[{{"id":7,"name":"Replay","revision":1,"clips":[
{{"uid":"clip-1","angleId":1,"recordingUuid":"rec","inFrame":10,"outFrame":40}},
{{"uid":"clip-2","angleId":1,"recordingUuid":"rec","inFrame":100,"outFrame":160}}
]}}],"events":[],
"format":{{"width":320,"height":180,"fpsNum":60000,"fpsDen":1001,"profile":"Sq","segmentFrames":120}},
"buffering":{{"encodeQueueFrames":6,"writeBufferMb":64,"overflowPolicy":"DropFrames","reserveMb":0}}}}"#,
        storage_path()
    )
}
/// A drive this test may record to. It is wiped first so a rerun starts empty.
fn storage_path() -> String {
    let dir = std::env::temp_dir().join(format!("eiviz-vmix-replay-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.display().to_string().replace('\\', "/")
}

#[test]
fn the_http_api_cues_and_plays_a_marked_clip() {
    load_test_module("vmix-replay");

    mixer_destroy();
    expect("create", mixer_create(0, 60_000, 1_001));
    expect("create unit", mixer_create_unit(1, 320, 180));

    // One Replay channel whose playlist holds two clips. A recording is opened first:
    // a clip whose recording is not open cannot be played, so the channel needs one.
    let json = r##"{
  "version": 2,
  "inputs": [
    { "id": 10, "name": "Bars", "kind": "Bars" },
    { "id": 524289, "name": "Replay A", "kind": "Replay" }
  ],
  "scenes": [
    { "id": 1, "name": "Scene 1", "layers": [{ "inputId": 10, "width": 1, "height": 1 }] }
  ],
  "units": [{ "id": 1, "name": "MU1" }],
  "replay": {
    "enabled": false,
    "angles": [{ "id": 1, "name": "Cam 1", "sceneId": 1, "audio": "Silent" }],
    "channels": [{ "index": 0, "playlistId": 7, "inputId": 524289 }],
    "playlists": [{ "id": 7, "name": "Replay", "revision": 1, "clips": [
      { "uid": "clip-1", "angleId": 1, "recordingUuid": "rec", "inFrame": 10, "outFrame": 40 },
      { "uid": "clip-2", "angleId": 1, "recordingUuid": "rec", "inFrame": 100, "outFrame": 160 }
    ] }]
  }
}"##;
    let code = unsafe { mixer_session_replace(json.as_ptr(), json.len(), 0) };
    expect("session replace", code);
    // Open a recording: a clip whose recording is not open cannot be cued, so the
    // channel needs a store before the API asks it to play.
    let configure = replay_configure_json();
    let configure = CString::new(configure).unwrap();
    expect("configure", unsafe {
        mixer_replay_configure(configure.as_ptr())
    });
    expect("record on", unsafe { mixer_replay_record(1) });
    thread::sleep(Duration::from_millis(600));
    // The clips name the recording the recorder actually opened: a clip whose
    // recording is not open is refused, and the store names its own id.
    let uuid = recording_uuid();
    assert!(!uuid.is_empty(), "a recording must be open before cueing");
    let json = json.replace("\"rec\"", &format!("\"{uuid}\""));
    let code = unsafe { mixer_session_replace(json.as_ptr(), json.len(), 0) };
    expect("session replace with the recording", code);
    expect("api on", unsafe {
        mixer_api_configure(1, PORT, c"user".as_ptr(), c"secret".as_ptr())
    });
    thread::sleep(Duration::from_millis(120));

    // An empty Value cues the newest clip on the channel.
    let cue = http_get("/api?Function=ReplayCue&Channel=A");
    assert!(!cue.contains("404"), "cue was refused: {cue}");
    let channels = replay_channels();
    assert_eq!(channels[0].in_frame, 100, "{channels:?}");
    assert_eq!(channels[0].out_frame, 160, "{channels:?}");
    assert_eq!(channels[0].angle_id, 1, "{channels:?}");
    assert_eq!(channels[0].playing, 0, "a cue leaves the channel paused");

    // Play at half speed, then pause.
    let play = http_get("/api?Function=ReplayPlay&Channel=A&Value=0.5");
    assert!(!play.contains("404"), "play was refused: {play}");
    let channels = replay_channels();
    assert_eq!(channels[0].playing, 1, "the channel must be playing");
    assert_eq!(channels[0].speed_milli, 500);

    let pause = http_get("/api?Function=ReplayPause&Channel=A");
    assert!(!pause.contains("404"), "pause was refused: {pause}");
    let channels = replay_channels();
    assert_eq!(channels[0].playing, 0, "the channel must be paused");

    // A channel nobody has is refused instead of silently playing channel A.
    let bad = http_get("/api?Function=ReplayPlay&Channel=abc");
    assert!(
        bad.contains("400") || bad.contains("bad replay channel"),
        "a bad channel must be refused: {bad}"
    );

    expect("api off", unsafe {
        mixer_api_configure(0, PORT, c"user".as_ptr(), c"secret".as_ptr())
    });
    mixer_destroy();
}

fn replay_channels() -> Vec<MixerReplayChannelState> {
    let mut channels = vec![MixerReplayChannelState::default(); 2];
    let count = unsafe { mixer_replay_copy_channels(channels.as_mut_ptr(), 2) };
    assert!(count >= 1, "the session has a Replay channel");
    channels.truncate(count as usize);
    channels
}

/// One HTTP GET against the vMix API, with the credentials the test configured.
fn http_get(path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", PORT as u16)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    let token = base64_encode("user:secret");
    let headers =
        format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\nAuthorization: Basic {token}\r\n\r\n");
    stream.write_all(headers.as_bytes()).expect("write");
    let mut body = String::new();
    stream.read_to_string(&mut body).ok();
    body
}

fn base64_encode(text: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = text.as_bytes();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}
