//! A transition between two clips, as it reaches the program output.
//!
//! The transport's own tests pin the timeline (Overwrap shares the duration,
//! Additive inserts it). This one goes one step further and asks the mixer for the
//! picture: two clips of different colours on one channel with a dissolve between
//! them must show a blend on the program while the transition runs, and end on the
//! second clip. Without this, "the transition is rendered" would only be a claim
//! about frame counts.
//!
//! Loading a Pro module is permanent, so this lives in its own test binary.

#![cfg(debug_assertions)]

mod common;

use std::ffi::CString;
use std::thread;
use std::time::{Duration, Instant};

use common::{
    expect, full_layer, load_test_module, mark_in_when_ready, mark_out_after, scene_id, scene_pixel,
};
use eiviz_mixer::{
    EASING_LINEAR, EivizMotion, EivizSceneCamera, EivizSceneSequenceDesc, EivizSceneStateDesc,
    EivizSequenceStepDesc, GEN_SOLID, MixerReplayChannelState, OverlayDesc, SCENE_SEQ_PLAY,
    SRC_COLOR, UnitState, mixer_create, mixer_create_unit, mixer_define_generator,
    mixer_define_scene, mixer_destroy, mixer_replay_channel_transport, mixer_replay_configure,
    mixer_replay_copy_channels, mixer_replay_record, mixer_scene_sequence,
    mixer_scene_sequences_define, mixer_scene_states_define, mixer_unit_set_state,
};

/// The channel's own source id: `REPLAY_CHANNEL_BASE` for channel A.
const CHANNEL_SOURCE: u64 = 0x0008_0001;

/// Frames the dissolve lasts. Thirty frames is half a second of picture, which is
/// long enough to be sampled while it runs.
const DISSOLVE_FRAMES: u32 = 30;

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

/// The second clip's picture: solid green. A dissolve near its end still carries a
/// little red, so the pure-colour test has to exclude that rather than call it green.
fn is_green(pixel: [u8; 3]) -> bool {
    pixel[1] > 200 && pixel[0] < 20 && pixel[2] < 20
}

/// Both clips on screen at once, which is what a dissolve is. The two clips are pure
/// red and pure green, so a pixel carrying a real amount of both came from the blend.
/// The floor sits above the codec's noise on a solid frame rather than at zero: a blend
/// that is four fifths of the way to green still holds a fifth of the red clip.
fn is_dissolve_mix(pixel: [u8; 3]) -> bool {
    pixel[0] > 20 && pixel[1] > 20 && pixel[0] < 235 && pixel[1] < 235 && pixel[2] < 140
}

fn is_red(pixel: [u8; 3]) -> bool {
    pixel[0] > 200 && pixel[1] < 20 && pixel[2] < 20
}

#[test]
fn a_dissolve_between_two_clips_reaches_the_program() {
    load_test_module("replay-transition");

    mixer_destroy();
    expect("create", mixer_create(0, 60_000, 1_001));
    expect("create unit", mixer_create_unit(1, 320, 180));

    // The angle records whatever its scene shows, and the program shows the
    // channel. Starting red and switching to green gives the two clips their own
    // colour without a second angle.
    expect(
        "generator red",
        mixer_define_generator(SRC_COLOR, GEN_SOLID, 1.0, 0.0, 0.0, 1.0, 0),
    );
    let angle_layer = full_layer(SRC_COLOR);
    expect("angle scene", unsafe {
        mixer_define_scene(scene_id(1), 320, 180, 1, &angle_layer)
    });
    // The animation needs a layer it can name, so the program layer carries an id.
    let channel_layer = OverlayDesc {
        layer_id: 1,
        ..full_layer(CHANNEL_SOURCE)
    };
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

    let dir = std::env::temp_dir().join(format!("eiviz-replay-dissolve-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.display().to_string().replace('\\', "/");
    // The channel starts with no playlist: the clips only exist once they are
    // marked, so the playlist is written after the marks are taken.
    let open = CString::new(format!(
        r#"{{"enabled":true,"storage":[{{"path":"{path}","angleIds":[1]}}],
"angles":[{{"id":1,"name":"A","sceneId":1,"audio":"Silent","recordMode":"Composite"}}],
"activeAngleId":1,
"channels":[{{"index":0,"playlistId":null,"inputId":0}}],
"playlists":[],"events":[],
"format":{{"width":320,"height":180,"fpsNum":60000,"fpsDen":1001,"profile":"Sq","segmentFrames":120}},
"buffering":{{"encodeQueueFrames":6,"writeBufferMb":64,"overflowPolicy":"DropFrames","reserveMb":0}}}}"#
    ))
    .unwrap();
    expect("configure", unsafe {
        mixer_replay_configure(open.as_ptr())
    });
    expect("record on", unsafe { mixer_replay_record(1) });

    // Clip A: the red stretch. Mark Out is taken before the colour changes, so the
    // clip holds one colour end to end, and it has to be longer than the transition
    // that will dissolve it.
    thread::sleep(Duration::from_millis(1200));
    let in_a = mark_in_when_ready("replay_transition A");
    let (uuid, in_a, out_a) = mark_out_after("replay_transition A", u64::from(DISSOLVE_FRAMES) + 2);
    assert!(
        out_a > in_a + u64::from(DISSOLVE_FRAMES),
        "clip A must be longer than the transition: {in_a}..{out_a}"
    );

    // Clip B: the green stretch that follows.
    expect(
        "generator green",
        mixer_define_generator(SRC_COLOR, GEN_SOLID, 0.0, 1.0, 0.0, 1.0, 0),
    );
    thread::sleep(Duration::from_millis(400));
    let in_b = mark_in_when_ready("replay_transition B");
    let (_dropped, in_b, out_b) = mark_out_after("replay_transition B", 20);
    assert!(out_b > in_b, "clip B must span recorded frames");

    // The playlist: A dissolves into B. The angle set is unchanged, so the
    // recording keeps running while the clips are filed. The same recording is
    // played twice, once per timing, so both are proven with one pass of frames.
    let playlist = |timing: &str| {
        CString::new(format!(
            r#"{{"enabled":true,"storage":[{{"path":"{path}","angleIds":[1]}}],
"angles":[{{"id":1,"name":"A","sceneId":1,"audio":"Silent","recordMode":"Composite"}}],
"activeAngleId":1,
"channels":[{{"index":0,"playlistId":7,"inputId":0}}],
"playlists":[{{"id":7,"name":"Replay","revision":1,"clips":[
  {{"uid":"clip-a","source":"Angle","angleId":1,"recordingUuid":"{uuid}","inFrame":{in_a},"outFrame":{out_a},
    "speed":1.0,"gainDb":0.0,"transition":{{"kind":"Dissolve","timing":"{timing}","durationFrames":{DISSOLVE_FRAMES}}}}},
  {{"uid":"clip-b","source":"Angle","angleId":1,"recordingUuid":"{uuid}","inFrame":{in_b},"outFrame":{out_b},
    "speed":1.0,"gainDb":0.0,"transition":{{"kind":"Cut","timing":"Overwrap","durationFrames":0}}}}]}}],
"events":[],
"format":{{"width":320,"height":180,"fpsNum":60000,"fpsDen":1001,"profile":"Sq","segmentFrames":120}},
"buffering":{{"encodeQueueFrames":6,"writeBufferMb":64,"overflowPolicy":"DropFrames","reserveMb":0}}}}"#
        ))
        .unwrap()
    };
    expect("configure playlist", unsafe {
        mixer_replay_configure(playlist("Overwrap").as_ptr())
    });

    // An animation step cues the first clip and starts it. Going through the step
    // means the transport takes the playlist's own clip, so the dissolve between
    // the two is the one the playlist asked for.
    let anim_layers = [channel_layer];
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
    let clip_uid = b"clip-a";
    // A sequence needs two steps, so the cue rides on the second one. The first
    // step only poses the scene, which leaves the picture alone.
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

    // Both timings run over the same recorded frames: Overwrap shares the dissolve
    // between the clips, Additive inserts it. Either way the picture the operator
    // sees must blend and then settle on the second clip.
    //
    // The sampling follows the channel's own position instead of the clock: the
    // whole suite runs in parallel, and a busy machine stretches playback without
    // moving the transition. Where the channel is decides what the picture must be.
    let mut held: Vec<usize> = Vec::new();
    for timing in ["Overwrap", "Additive"] {
        // A transition lasts a fixed number of frames, while the composed picture is
        // sampled by rendering a scene and reading it back — much slower than the
        // renderer runs. On a software adapter a whole transition can pass between two
        // samples, so the run is repeated when the blend was missed entirely. The
        // picture the operator sees is what is measured either way.
        let mut held_here = 0;
        let mut last = [0, 0, 0];
        // Real-time playout can move a whole clip past between two scene readbacks,
        // especially in a debug build. The counters are accumulated across attempts so
        // a run that sampled only the transition still proves clip A played.
        let mut red_total = 0;
        let mut inside_total = 0;
        let mut mixed_total = 0;
        let attempts = 4;
        // A's last frame and the frames the transition covers. Overwrap runs the
        // transition over the tail of A; Additive holds A's last frame for it.
        let window_first = out_a - u64::from(DISSOLVE_FRAMES) + 1;
        let mut inside_seen = 0;
        for _ in 0..attempts {
            let run = play_once(&playlist(timing), in_a, out_a, window_first, NORMAL_SPEED);
            red_total += run.red;
            inside_total += run.inside;
            inside_seen = inside_seen.max(run.inside);
            mixed_total += run.mixed;
            held_here = run.held;
            last = run.last;
            if run.mixed > 0 && run.red > 0 && run.reached_end {
                break;
            }
        }
        // A software adapter's snapshot can take longer than a full-speed dissolve
        // lasts, so the blend falls between two samples however many runs are made.
        // The same thirty frames at a tenth of the speed cover five seconds and can
        // be sampled; it is used only when no run saw a blend at all.
        if mixed_total == 0 {
            let run = play_once(&playlist(timing), in_a, out_a, window_first, SLOW_SPEED);
            red_total += run.red;
            inside_total += run.inside;
            inside_seen = inside_seen.max(run.inside);
            mixed_total += run.mixed;
            last = run.last;
        }
        let _ = inside_seen;

        // Additive holds the outgoing clip's last frame while it inserts the
        // transition, so the channel reports clip A's out frame for its whole
        // duration. Overwrap keeps moving, because the outgoing clip is still
        // playing underneath. Both are measured the same way, so they are compared
        // with each other rather than against a sample count the frame rate decides.
        held.push(held_here);
        assert!(
            red_total > 0,
            "{timing}: the first clip must play before the transition: last {last:?}"
        );
        assert!(
            inside_total > 0,
            "{timing}: the transition window must be sampled: out_a {out_a}, last {last:?}"
        );
        assert!(
            mixed_total > 0,
            "{timing}: the dissolve must put both clips on screen: {inside_total} samples in \
             {window_first}..={out_a}, last {last:?}"
        );
        assert!(
            is_green(last),
            "{timing}: the playlist must end on the second clip: last {last:?}, mixed {mixed_total}"
        );
    }
    // "Overwrap" then "Additive": the insertion is the whole difference, because it
    // holds the outgoing edge instead of playing through it.
    assert!(
        held[1] > held[0],
        "Additive must hold the outgoing edge longer than Overwrap: {held:?}"
    );

    expect("record off", unsafe { mixer_replay_record(0) });
    mixer_destroy();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Channel speeds, in thousandths. The slow run exists for machines where one
/// snapshot takes longer than a full-speed dissolve.
const NORMAL_SPEED: i32 = 1000;
const SLOW_SPEED: i32 = 100;

/// What one playthrough of the playlist showed.
struct Playthrough {
    red: usize,
    inside: usize,
    mixed: usize,
    held: usize,
    last: [u8; 3],
    reached_end: bool,
}

/// Plays the playlist once from the sequence step, sampling the composed program
/// pixel as often as the snapshot allows. `rate` is the channel speed in
/// thousandths; the frame counts are the same at any speed.
fn play_once(
    playlist: &CString,
    in_a: u64,
    out_a: u64,
    window_first: u64,
    rate: i32,
) -> Playthrough {
    expect("configure playlist", unsafe {
        mixer_replay_configure(playlist.as_ptr())
    });
    expect("play sequence", unsafe {
        mixer_scene_sequence(scene_id(2), 1, SCENE_SEQ_PLAY)
    });

    let mut run = Playthrough {
        red: 0,
        inside: 0,
        mixed: 0,
        held: 0,
        last: [0, 0, 0],
        reached_end: false,
    };
    // The previous run leaves the channel on its last clip until the step cues the
    // first one again, so the run has not started until the channel reports a frame
    // of clip A.
    let mut started = false;
    let mut slowed = false;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let pixel = scene_pixel(scene_id(2), 160, 90);
        let mut channels = vec![MixerReplayChannelState::default(); 2];
        let count = unsafe { mixer_replay_copy_channels(channels.as_mut_ptr(), 2) };
        let position = if count >= 1 {
            channels[0].position_frame
        } else {
            0
        };
        if !started {
            if position < in_a || position > out_a {
                continue;
            }
            started = true;
        }
        // Slow down before the transition rather than during it, so the frames the
        // dissolve covers are the same thirty either way.
        if rate < NORMAL_SPEED && !slowed && position + 5 < window_first {
            let _ = unsafe { mixer_replay_channel_transport(0, 1, rate, -1) };
            slowed = true;
        }
        run.last = pixel;
        if position == out_a {
            run.held += 1;
        }
        if position < window_first {
            // Clip A plays on its own before the transition.
            if is_red(pixel) {
                run.red += 1;
            }
        } else {
            // The transition window is `window_first..=out_a`.
            if position <= out_a {
                run.inside += 1;
            }
            if is_green(pixel) {
                // B is on screen: the playlist ran to its end.
                run.reached_end = true;
                break;
            }
        }
        // The blend is looked for in every sample of the run rather than tied to the
        // reported position: the composed picture is read back through the GPU, and
        // on a software adapter that lags the transport by more than a transition
        // lasts, so the two do not line up. Before the transition the picture is one
        // solid colour, so a blend can only come from it.
        if is_dissolve_mix(pixel) {
            run.mixed += 1;
        }
    }
    run
}
