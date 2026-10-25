//! Replay audio on the way out.
//!
//! The recorder stores an angle's sound next to its picture, and a channel that
//! plays the clip hands that sound to the mixer as if it came from a live input.
//! This follows the whole way: a tone in the angle's audio source, recorded,
//! played back, seen on the channel's input meter, and gone again when the
//! operator pauses.
//!
//! Loading a Pro module is permanent, so this lives in its own test binary.

#![cfg(debug_assertions)]

mod common;

use std::ffi::CString;
use std::thread;
use std::time::{Duration, Instant};

use common::{
    cstring, expect, full_layer, load_test_module, mark_in_when_ready, mark_out_after, scene_id,
};
use eiviz_mixer::{
    AudioPeak, GEN_SOLID, OverlayDesc, SRC_COLOR, UnitState, mixer_audio_set_input,
    mixer_copy_audio_peaks, mixer_create, mixer_create_unit, mixer_define_generator,
    mixer_define_scene, mixer_destroy, mixer_generator_set_tone, mixer_replay_channel_cue,
    mixer_replay_channel_transport, mixer_replay_configure, mixer_replay_record,
    mixer_unit_set_state,
};

/// The channel's own source id, which is also the id its audio arrives under.
const CHANNEL_SOURCE: u64 = 0x0008_0001;

/// Peak id of unit 1's program bus, where the channel's sound goes out.
const PROGRAM_BUS: u64 = 0x0004_0001;

/// The tone the recorder hears: 1 kHz at -12 dBFS, which is about 0.25 peak.
const TONE_HZ: f32 = 1000.0;
const TONE_DBFS: f32 = -12.0;

/// Loudest peak the mixer reports for one source or bus, or zero when it has none.
fn peak_for(id: u64) -> f32 {
    let mut peaks = vec![AudioPeak::default(); 64];
    let count = unsafe { mixer_copy_audio_peaks(peaks.as_mut_ptr(), peaks.len() as u32) };
    if count <= 0 {
        return 0.0;
    }
    peaks[..count as usize]
        .iter()
        .filter(|peak| peak.source_id == id)
        .map(|peak| peak.left.max(peak.right))
        .fold(0.0, f32::max)
}

fn channel_peak() -> f32 {
    peak_for(CHANNEL_SOURCE)
}

/// Waits for a peak to pass `level`, and returns what it settled on.
fn wait_for_peak(id: u64, level: f32, within: Duration) -> f32 {
    let deadline = Instant::now() + within;
    let mut peak = 0.0;
    while Instant::now() < deadline {
        peak = peak_for(id);
        if peak > level {
            return peak;
        }
        thread::sleep(Duration::from_millis(50));
    }
    peak
}

#[test]
fn the_recorded_sound_reaches_the_channel_meter() {
    load_test_module("replay-audio");

    mixer_destroy();
    expect("create", mixer_create(0, 60_000, 1_001));
    expect("create unit", mixer_create_unit(1, 320, 180));
    expect("create audio unit", mixer_create_unit(2, 320, 180));

    // The tone source is the angle's picture and the sound the angle records. A
    // layer carries the sound only when it follows it, so the unit that feeds the
    // angle's audio has that layer.
    expect(
        "generator",
        mixer_define_generator(SRC_COLOR, GEN_SOLID, 1.0, 0.0, 0.0, 1.0, 0),
    );
    expect(
        "tone",
        mixer_generator_set_tone(SRC_COLOR, TONE_HZ, TONE_DBFS),
    );

    let angle_layer = OverlayDesc {
        layer_id: 1,
        audio_follow: 1,
        ..full_layer(SRC_COLOR)
    };
    expect("angle scene", unsafe {
        mixer_define_scene(scene_id(1), 320, 180, 1, &angle_layer)
    });
    // The program shows the Replay channel, which is how its sound goes out.
    let channel_layer = OverlayDesc {
        layer_id: 1,
        audio_follow: 1,
        ..full_layer(CHANNEL_SOURCE)
    };
    expect("program scene", unsafe {
        mixer_define_scene(scene_id(2), 320, 180, 1, &channel_layer)
    });
    // Unit 2 is the angle's audio source: its program mix is the tone.
    let tone_layer = OverlayDesc {
        layer_id: 1,
        audio_follow: 1,
        ..full_layer(SRC_COLOR)
    };
    expect("tone scene", unsafe {
        mixer_define_scene(scene_id(3), 320, 180, 1, &tone_layer)
    });
    unsafe {
        let program = UnitState {
            program_source: scene_id(2),
            preview_source: scene_id(1),
            ..UnitState::default()
        };
        expect("unit state", mixer_unit_set_state(1, &program));
        let audio = UnitState {
            program_source: scene_id(3),
            preview_source: scene_id(3),
            ..UnitState::default()
        };
        expect("audio unit state", mixer_unit_set_state(2, &audio));
    }

    let dir = std::env::temp_dir().join(format!("eiviz-replay-audio-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.display().to_string().replace('\\', "/");
    let config = CString::new(format!(
        r#"{{"enabled":true,"storage":[{{"path":"{path}","angleIds":[1]}}],
"angles":[{{"id":1,"name":"A","sceneId":1,"audio":"Unit","audioUnitId":2}}],
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

    // The audio graph only mixes inputs the operator has routed, so the tone is
    // routed to the unit that feeds the angle's sound, and the channel is routed
    // to the unit whose program carries it out.
    let tone_route = [2u64];
    expect("route tone", unsafe {
        mixer_audio_set_input(
            SRC_COLOR,
            tone_route.as_ptr(),
            tone_route.len() as u32,
            1.0,
            0,
        )
    });
    let channel_route = [1u64];
    expect("route channel", unsafe {
        mixer_audio_set_input(
            CHANNEL_SOURCE,
            channel_route.as_ptr(),
            channel_route.len() as u32,
            1.0,
            0,
        )
    });

    // An idle channel has no sound of its own.
    assert_eq!(channel_peak(), 0.0, "an uncued channel must be silent");

    expect("record on", unsafe { mixer_replay_record(1) });
    thread::sleep(Duration::from_millis(1500));
    mark_in_when_ready("replay_audio");
    let (uuid, in_frame, out_frame) = mark_out_after("replay_audio", 30);
    assert!(out_frame > in_frame, "the mark must span recorded frames");

    let uuid_c = cstring(&uuid);
    expect("cue", unsafe {
        mixer_replay_channel_cue(0, 1, in_frame, out_frame, uuid_c.as_ptr())
    });
    expect("play", unsafe {
        mixer_replay_channel_transport(0, 1, 1000, -1)
    });

    // The recorded tone must arrive as the channel's audio, and reach the program
    // the operator is sending. A wrong layout or a dropped block leaves both empty.
    let peak = wait_for_peak(CHANNEL_SOURCE, 0.05, Duration::from_secs(5));
    let program = wait_for_peak(PROGRAM_BUS, 0.05, Duration::from_secs(2));
    expect("pause", unsafe {
        mixer_replay_channel_transport(0, 0, 1000, -1)
    });
    // A paused deck is silent: the meter drops instead of freezing on the tail. The
    // decay follows the audio blocks the mixer produces, so a slow adapter needs more
    // wall clock for the same number of blocks; the deadline is generous for that.
    let quiet_deadline = Instant::now() + Duration::from_secs(30);
    let mut quiet = channel_peak();
    while Instant::now() < quiet_deadline && quiet > 0.01 {
        thread::sleep(Duration::from_millis(50));
        quiet = channel_peak();
    }

    expect("record off", unsafe { mixer_replay_record(0) });

    assert!(
        peak > 0.05,
        "the recorded tone must reach the channel's meter, got {peak}"
    );
    assert!(
        peak < 0.8,
        "the tone must arrive at roughly its recorded level, got {peak}"
    );
    assert!(
        program > 0.05,
        "the channel's sound must reach the program bus, got {program}"
    );
    assert!(
        quiet <= 0.01,
        "a paused channel must be silent, meter still reads {quiet}"
    );

    // A second recording is exercised at the engine level by
    // `sound_is_filed_on_the_recordings_own_timeline`: the test module keeps one audio
    // track per angle rather than one per recording, so it cannot model two recordings
    // faithfully.

    expect("record off", unsafe { mixer_replay_record(0) });
    mixer_destroy();
    let _ = std::fs::remove_dir_all(&dir);
}
