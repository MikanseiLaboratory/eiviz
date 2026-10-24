//! A passthrough angle stores the frames its OMT input already sends.
//!
//! A composite angle reads the picture back from the GPU and encodes it. An OMT input
//! arrives encoded, so a passthrough angle hands that bitstream straight to the store:
//! the recording holds what the camera sent rather than a re-encode of it. This test
//! drives a real sender into a real OMT input and checks that the frames reach the
//! store.
//!
//! Loading a Pro module is permanent, so this lives in its own test binary.

#![cfg(debug_assertions)]

mod common;

use std::ffi::CString;
use std::thread;
use std::time::{Duration, Instant};

use common::{cstring, expect, full_layer, load_test_module, scene_id};
use eiviz_mixer::{
    MixerReplayStats, UnitState, mixer_create, mixer_create_unit, mixer_define_scene,
    mixer_destroy, mixer_omt_connect, mixer_replay_configure, mixer_replay_copy_stats,
    mixer_replay_record, mixer_unit_set_state,
};
use openmediatransport::{Codec, FrameType, MediaFrame, Sender};
use vmx::{Codec as VmxCodec, Config as VmxConfig, Profile};

/// The channel's own source id: `REPLAY_CHANNEL_BASE` with the channel selected by
/// `inputId: 0` in the document.
const CHANNEL_SOURCE: u64 = 0x0008_0001;
/// The OMT input the passthrough angle records.
const OMT_INPUT: u64 = 40;

/// One colour-bar frame encoded to VMX1, the way an OMT sender would.
fn bitstream(width: i32, height: i32, shift: u8) -> Vec<u8> {
    let stride = (width as usize) * 2;
    let mut uyvy = vec![128u8; stride * height as usize];
    for y in 0..height as usize {
        for x in (0..width as usize).step_by(2) {
            let o = y * stride + x * 2;
            uyvy[o] = 128;
            uyvy[o + 1] = 16 + ((x + y + usize::from(shift)) % 220) as u8;
            uyvy[o + 2] = 128;
            uyvy[o + 3] = 16 + ((x + 1 + y + usize::from(shift)) % 220) as u8;
        }
    }
    let mut enc = VmxCodec::new(VmxConfig {
        width,
        height,
        profile: Profile::OmtLq,
        color_space: Default::default(),
    })
    .unwrap();
    enc.encode_uyvy(&uyvy, stride).unwrap();
    let mut out = vec![0u8; 2 << 20];
    let len = enc.save_to(&mut out).unwrap();
    out.truncate(len);
    out
}

#[test]
fn a_passthrough_angle_stores_the_frames_the_input_sent() {
    load_test_module("replay-passthrough");

    mixer_destroy();
    expect("create", mixer_create(0, 60_000, 1_001));
    expect("create unit", mixer_create_unit(1, 320, 180));

    // The sender the OMT input receives from.
    let mut sender = Sender::create("PassthroughSrc", FrameType::VIDEO).expect("sender");
    let address = format!("omt://127.0.0.1:{}", sender.port());

    // Scene 1 shows the Replay channel, so the recording can be watched.
    let channel_layer = full_layer(CHANNEL_SOURCE);
    expect("channel scene", unsafe {
        mixer_define_scene(scene_id(1), 320, 180, 1, &channel_layer)
    });
    unsafe {
        let state = UnitState {
            program_source: scene_id(1),
            preview_source: channel_layer.source_id,
            ..UnitState::default()
        };
        expect("unit state", mixer_unit_set_state(1, &state));
    }

    let address_c = cstring(&address);
    expect("omt connect", unsafe {
        mixer_omt_connect(OMT_INPUT, address_c.as_ptr(), 0, 1, 0)
    });

    let dir = std::env::temp_dir().join(format!("eiviz-replay-passthrough-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.display().to_string().replace('\\', "/");
    // One angle, passing the input's frames through: no scene, no readback.
    let config = CString::new(format!(
        r#"{{"enabled":true,"storage":[{{"path":"{path}","angleIds":[1]}}],
"angles":[{{"id":1,"name":"A","source":"Input","inputId":{OMT_INPUT},"audio":"Silent","recordMode":"Passthrough"}}],
"channels":[{{"index":0,"playlistId":null,"inputId":0}}],
"playlists":[],"events":[],
"format":{{"width":320,"height":180,"fpsNum":60000,"fpsDen":1001,"profile":"Sq","segmentFrames":120}},
"buffering":{{"encodeQueueFrames":6,"writeBufferMb":64,"overflowPolicy":"DropFrames","reserveMb":0}}}}"#
    ))
    .unwrap();
    expect("configure", unsafe {
        mixer_replay_configure(config.as_ptr())
    });

    // Let the receiver accept the sender before recording starts.
    for _ in 0..100 {
        let _ = sender.poll_accept();
        let _ = sender.poll_peer_metadata();
        if sender.video_subscribed() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    if !sender.video_subscribed() {
        sender.force_subscribe(true, false, false);
    }

    expect("record on", unsafe { mixer_replay_record(1) });

    // Frames the sender produced while the recorder is running must reach the store.
    let width = 320i32;
    let height = 180i32;
    let mut sent = 0u64;
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut stats = MixerReplayStats::default();
    while Instant::now() < deadline {
        let frame = MediaFrame {
            frame_type: FrameType::VIDEO,
            timestamp: 1_000_000 + sent as i64 * 166_667,
            codec: Codec::Vmx1 as i32,
            width,
            height,
            frame_rate_n: 60_000,
            frame_rate_d: 1_001,
            aspect_ratio: 16.0 / 9.0,
            data: bitstream(width, height, (sent % 32) as u8),
            ..Default::default()
        };
        let _ = sender.send_video(frame);
        sent += 1;
        // The sender keeps accepting while it sends: a connection that is only
        // polled before the first frame is not enough for the receiver to subscribe.
        let _ = sender.poll_accept();
        let _ = sender.poll_peer_metadata();
        if !sender.video_subscribed() {
            sender.force_subscribe(true, false, false);
        }
        thread::sleep(Duration::from_millis(40));
        unsafe { mixer_replay_copy_stats(&mut stats, std::ptr::null_mut(), 0) };
        if stats.encoded_frames >= 5 {
            break;
        }
    }
    expect("record off", unsafe { mixer_replay_record(0) });
    mixer_destroy();

    assert!(
        stats.encoded_frames >= 5,
        "the passthrough angle must store what the input sent: sent {sent}, stored {}",
        stats.encoded_frames
    );
    assert_eq!(
        stats.encoded_dropped, 0,
        "the store accepted every frame it was handed"
    );
    // A passthrough angle never goes through the readback, so nothing was pushed.
    assert_eq!(
        stats.frames_pushed, 0,
        "a passthrough angle must not be read back from the GPU"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
