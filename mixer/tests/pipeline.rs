use std::ffi::CString;
use std::thread;
use std::time::Duration;

use eiviz_mixer::{
    ERR_INVALID_ARGUMENT, ERR_IO, ERR_NOT_CREATED, MixerRebarInfo, OK, OUT_DECKLINK, OUT_OMT,
    OverlayDesc, Rect, SCENE_BASE, SRC_BARS, SRC_BLUE, SRC_COLOR, SRC_KIND_MU_PROGRAM, UnitState,
    mixer_audio_bus_count, mixer_copy_rebar_info, mixer_create, mixer_create_unit, mixer_define_scene,
    mixer_destroy, mixer_omt_connect, mixer_omt_start_send, mixer_output_add, mixer_ping,
    mixer_set_live_save, mixer_set_rebar_optimization, mixer_unit_acquire_frame, mixer_unit_auto,
    mixer_unit_cut, mixer_unit_get_state, mixer_unit_release_frame, mixer_unit_set_state,
    mixer_video_start,
};
#[cfg(windows)]
use eiviz_mixer::{OUT_NDI, mixer_ndi_discover, mixer_output_remove};
use openmediatransport::{Codec, FrameType, MediaFrame, Sender};

#[test]
fn ping_and_invalid_clock() {
    assert_eq!(mixer_ping(), 0x4549_5649);
    assert_eq!(mixer_create(0, 60_000, 0), ERR_INVALID_ARGUMENT);
    mixer_destroy();
    assert_eq!(mixer_set_live_save(1, 2, 0), ERR_NOT_CREATED);
}

#[test]
fn rebar_info_and_toggle() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    let mut info = MixerRebarInfo::default();
    unsafe {
        assert_eq!(mixer_copy_rebar_info(&mut info), OK);
    }
    assert_eq!(mixer_set_rebar_optimization(0), OK);
    unsafe {
        assert_eq!(mixer_copy_rebar_info(&mut info), OK);
    }
    assert_eq!(info.active, 0);
    assert_eq!(mixer_set_rebar_optimization(1), OK);
    mixer_destroy();
}

#[test]
fn vmx_roundtrip_is_available() {
    let mut enc = vmx::Codec::new(vmx::Config {
        width: 64,
        height: 64,
        profile: vmx::Profile::OmtHq,
        color_space: Default::default(),
    })
    .expect("codec");
    let frame = vec![128u8; 64 * 64 * 2];
    enc.encode_uyvy(&frame, 128).expect("encode");
    let mut buf = vec![0u8; 1 << 20];
    let len = enc.save_to(&mut buf).expect("save");
    assert!(len > 0);
    let mut dec = vmx::Codec::new(vmx::Config::new(64, 64)).expect("dec");
    dec.load_from(&buf[..len]).expect("load");
    let mut out = vec![0u8; 64 * 64 * 2];
    dec.decode_uyvy(&mut out, 128).expect("decode");
}

/// Headless contract: compose + OMT + cut/auto without attach / HWND.
#[test]
fn dx12_compose_omt_and_program_out() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert!(mixer_audio_bus_count() >= 2);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);

    let mut state = UnitState {
        program_source: SRC_COLOR,
        preview_source: SRC_BLUE,
        mix: 0.5,
        transition_kind: 1,
        overlay_count: 1,
        ..UnitState::default()
    };
    state.overlays[0].source_id = SRC_BARS;
    state.overlays[0].rect.width = 0.3;
    state.overlays[0].rect.height = 0.3;
    state.overlays[0].opacity = 0.8;
    unsafe {
        assert_eq!(mixer_unit_set_state(1, &state), OK);
    }
    thread::sleep(Duration::from_millis(250));
    unsafe {
        try_acquire(1);
    }
    assert_eq!(mixer_unit_cut(1, 1), OK);
    assert_eq!(mixer_unit_auto(1, 200, 1), OK);

    let mut sender =
        Sender::create("eiviz-test-src", FrameType::VIDEO | FrameType::AUDIO).expect("sender");
    let url = format!("omt://127.0.0.1:{}", sender.port());
    let address = CString::new(url).unwrap();
    unsafe {
        assert_eq!(mixer_omt_connect(20, address.as_ptr(), 0, 1, 0), OK);
        assert_eq!(
            mixer_omt_start_send(1, CString::new("eiviz-test-pgm").unwrap().as_ptr()),
            OK
        );
        state.program_source = 20;
        state.mix = 0.0;
        assert_eq!(mixer_unit_set_state(1, &state), OK);
    }

    let uyvy = vec![0x80u8, 0xEB, 0x80, 0x10].repeat((64 * 64 / 2) as usize);
    for _ in 0..24 {
        let _ = sender.poll_accept();
        let _ = sender.poll_peer_metadata();
        if !sender.video_subscribed() {
            sender.force_subscribe(true, true, false);
        }
        sender
            .send_video(MediaFrame {
                frame_type: FrameType::VIDEO,
                codec: Codec::Uyvy as i32,
                width: 64,
                height: 64,
                stride: 128,
                frame_rate_n: 60,
                frame_rate_d: 1,
                data: uyvy.clone(),
                ..Default::default()
            })
            .expect("send");
        thread::sleep(Duration::from_millis(16));
    }
    thread::sleep(Duration::from_millis(120));
    unsafe {
        assert_eq!(
            mixer_output_add(
                101,
                OUT_OMT,
                CString::new("eiviz-out-a").unwrap().as_ptr(),
                SRC_KIND_MU_PROGRAM,
                0,
                1,
                0
            ),
            OK
        );
        assert_eq!(
            mixer_output_add(
                102,
                OUT_OMT,
                CString::new("eiviz-out-b").unwrap().as_ptr(),
                SRC_KIND_MU_PROGRAM,
                0,
                1,
                0
            ),
            OK
        );
    }
    mixer_destroy();
}

#[test]
fn dx12_omt_gpu_in_and_out() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);

    let mut sender =
        Sender::create("eiviz-test-gpu-src", FrameType::VIDEO | FrameType::AUDIO).expect("sender");
    let url = format!("omt://127.0.0.1:{}", sender.port());
    let address = CString::new(url).unwrap();
    unsafe {
        assert_eq!(mixer_omt_connect(21, address.as_ptr(), 1, 3, 0), OK);
        assert_eq!(
            mixer_output_add(
                201,
                OUT_OMT,
                CString::new("eiviz-gpu-pgm").unwrap().as_ptr(),
                SRC_KIND_MU_PROGRAM,
                0,
                1,
                1
            ),
            OK
        );
        let state = UnitState {
            program_source: 21,
            preview_source: SRC_BLUE,
            mix: 0.0,
            ..UnitState::default()
        };
        assert_eq!(mixer_unit_set_state(1, &state), OK);
    }

    let uyvy = vec![0x80u8, 0xEB, 0x80, 0x10].repeat((64 * 64 / 2) as usize);
    for _ in 0..16 {
        let _ = sender.poll_accept();
        let _ = sender.poll_peer_metadata();
        if !sender.video_subscribed() {
            sender.force_subscribe(true, true, false);
        }
        sender
            .send_video(MediaFrame {
                frame_type: FrameType::VIDEO,
                codec: Codec::Uyvy as i32,
                width: 64,
                height: 64,
                stride: 128,
                frame_rate_n: 60,
                frame_rate_d: 1,
                data: uyvy.clone(),
                ..Default::default()
            })
            .expect("send");
        thread::sleep(Duration::from_millis(16));
    }
    thread::sleep(Duration::from_millis(120));
    mixer_destroy();
}

fn scene_id(id: u64) -> u64 {
    SCENE_BASE | id
}

fn full_layer(source_id: u64) -> OverlayDesc {
    OverlayDesc {
        source_id,
        rect: Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        opacity: 1.0,
        z: 0,
        ..Default::default()
    }
}

unsafe fn try_acquire(unit: u64) {
    for _ in 0..12 {
        let mut ptr = std::ptr::null();
        let mut stride = 0u32;
        let mut pts = 0i64;
        let mut length = 0u32;
        let acquired =
            unsafe { mixer_unit_acquire_frame(unit, &mut ptr, &mut stride, &mut pts, &mut length) };
        if acquired == OK {
            assert!(!ptr.is_null());
            assert!(length > 0);
            mixer_unit_release_frame(unit);
            return;
        }
        thread::sleep(Duration::from_millis(40));
    }
}

#[test]
fn scene_compose_overlay_after_mix_multiview_and_tbar_take() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);
    assert_eq!(mixer_create_unit(2, 320, 180), OK);

    let bars = full_layer(SRC_BARS);
    let color = full_layer(SRC_COLOR);
    unsafe {
        assert_eq!(mixer_define_scene(scene_id(1), 320, 180, 1, &bars), OK);
        assert_eq!(mixer_define_scene(scene_id(2), 320, 180, 1, &color), OK);
        assert_eq!(
            mixer_define_scene(scene_id(3), 320, 180, 0, std::ptr::null()),
            OK
        );
    }

    let mut state = UnitState {
        program_source: scene_id(2),
        preview_source: scene_id(1),
        mix: 0.0,
        overlay_count: 1,
        ..UnitState::default()
    };
    state.overlays[0] = OverlayDesc {
        source_id: scene_id(3),
        rect: Rect {
            x: 0.6,
            y: 0.1,
            width: 0.3,
            height: 0.3,
        },
        opacity: 1.0,
        z: 0,
        ..OverlayDesc::default()
    };
    state.mv_slots[0] = scene_id(1);
    state.mv_slots[1] = SRC_BLUE;
    unsafe {
        assert_eq!(mixer_unit_set_state(1, &state), OK);
    }

    let other = UnitState {
        program_source: scene_id(1),
        preview_source: scene_id(2),
        mix: 0.0,
        ..UnitState::default()
    };
    unsafe {
        assert_eq!(mixer_unit_set_state(2, &other), OK);
    }

    thread::sleep(Duration::from_millis(250));
    unsafe {
        try_acquire(1);
        try_acquire(2);
    }

    unsafe {
        state.mix = 1.0;
        assert_eq!(mixer_unit_set_state(1, &state), OK);
        assert_eq!(mixer_unit_cut(1, 1), OK);
        let mut after = UnitState::default();
        assert_eq!(mixer_unit_get_state(1, &mut after), OK);
        assert_eq!(after.program_source, scene_id(1));
        assert_eq!(after.preview_source, scene_id(2));
        assert_eq!(after.mix, 0.0);
        assert_eq!(after.mv_slots[0], scene_id(1));
        assert_eq!(after.overlay_count, 1);

        let mut still = UnitState::default();
        assert_eq!(mixer_unit_get_state(2, &mut still), OK);
        assert_eq!(still.program_source, scene_id(1));
        assert_eq!(still.preview_source, scene_id(2));
        assert_eq!(still.mix, 0.0);
    }

    unsafe {
        #[cfg(windows)]
        {
            assert_eq!(
                mixer_output_add(
                    99,
                    OUT_NDI,
                    CString::new("eiviz-ndi-test").unwrap().as_ptr(),
                    SRC_KIND_MU_PROGRAM,
                    0,
                    1,
                    0
                ),
                OK
            );
            assert_eq!(mixer_output_remove(99), OK);
            let mut ndi_names = vec![0u8; 4096];
            let discovered = mixer_ndi_discover(ndi_names.as_mut_ptr(), ndi_names.len());
            assert!(discovered >= 0);
        }
        assert_eq!(
            mixer_output_add(
                98,
                OUT_DECKLINK,
                CString::new("decklink-unlinked").unwrap().as_ptr(),
                SRC_KIND_MU_PROGRAM,
                0,
                1,
                0
            ),
            ERR_IO
        );
    }
    mixer_destroy();
}

#[test]
fn missing_video_file_returns_io_error() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    let path = CString::new(r"C:\eiviz-missing-file-does-not-exist.mp4").unwrap();
    unsafe {
        assert_eq!(mixer_video_start(99, path.as_ptr(), 0, 0), ERR_IO);
    }
    mixer_destroy();
}
