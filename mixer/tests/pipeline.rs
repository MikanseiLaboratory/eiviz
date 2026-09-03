use std::ffi::CString;
use std::thread;
use std::time::{Duration, Instant};

use eiviz_mixer::{
    EASING_IN_OUT, ERR_INVALID_ARGUMENT, ERR_IO, ERR_NOT_CREATED, INCOMING_PROGRAM, MixerRebarInfo,
    OK, OUT_DECKLINK, OUT_OMT, OverlayDesc, Rect, SCENE_BASE, SRC_BARS, SRC_BLUE, SRC_COLOR,
    SRC_KIND_MU_PREVIEW, SRC_KIND_MU_PROGRAM, TRANSITION_BLOOM, TRANSITION_CUBE,
    TRANSITION_CUBE_ZOOM, TRANSITION_DATAMOSH, TRANSITION_DIP, TRANSITION_FADE,
    TRANSITION_FLY_ROTATE, TRANSITION_GLITCH, TRANSITION_HEART, TRANSITION_LOREZ,
    TRANSITION_METAMIX, TRANSITION_MULTITASK, TRANSITION_OPTICAL_FLOW, TRANSITION_PAGE_CURL,
    TRANSITION_PARTS, TRANSITION_PIXEL_SORT, TRANSITION_SLIDE, TRANSITION_STAR, TRANSITION_SWIRL,
    TRANSITION_TILE, TRANSITION_VISUAL_DISSOLVE, TRANSITION_WIPE, UnitState, VideoCaptureInfo,
    mixer_audio_bus_count, mixer_copy_rebar_info, mixer_create, mixer_create_unit,
    mixer_define_mix_input, mixer_define_scene, mixer_destroy, mixer_omt_connect,
    mixer_omt_start_send, mixer_output_add, mixer_ping, mixer_set_live_save,
    mixer_set_ndi_gpu_upload, mixer_set_rebar_optimization, mixer_unit_acquire_frame,
    mixer_unit_auto, mixer_unit_cut, mixer_unit_get_state, mixer_unit_release_frame,
    mixer_unit_set_state, mixer_validate_custom_wgsl, mixer_video_enum_captures, mixer_video_start,
};
#[cfg(windows)]
use eiviz_mixer::{OUT_NDI, mixer_ndi_discover, mixer_output_remove};
use openmediatransport::{Codec, FrameType, MediaFrame, Sender};

#[test]
fn video_captures_enum_is_safe() {
    let mut devices = [VideoCaptureInfo::default(); 8];
    let n = unsafe { mixer_video_enum_captures(devices.as_mut_ptr(), devices.len() as u32) };
    assert!(n >= 0);
    assert!(n as usize <= devices.len());
    for item in devices.iter().take(n as usize) {
        assert_ne!(item.id[0], 0);
        assert_ne!(item.name[0], 0);
    }
}

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
    assert_eq!(mixer_set_ndi_gpu_upload(1), OK);
    assert_eq!(mixer_set_ndi_gpu_upload(0), OK);
    assert_eq!(mixer_set_ndi_gpu_upload(1), OK);
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
    assert_eq!(mixer_unit_cut(1, 1, 0), OK);
    assert_eq!(
        mixer_unit_auto(
            1,
            TRANSITION_FADE,
            200,
            1,
            1,
            0,
            0,
            0.0,
            0.0,
            0.0,
            1.0,
            0,
            0.02,
            0.0
        ),
        OK
    );

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

#[test]
fn omt_gpu_send_1080p_rgba_does_not_overflow() {
    use openmediatransport::{GpuVideoContext, VideoTextureMeta};
    use std::sync::Arc;

    let Some((_, _, device, queue)) = vmx::gpu::request_headless_device() else {
        eprintln!("skip: no wgpu adapter");
        return;
    };
    let ctx = GpuVideoContext {
        device: Arc::new(device.clone()),
        queue: Arc::new(queue.clone()),
        gpu_lock: None,
    };
    let width = 1920u32;
    let height = 1080u32;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("eiviz omt 1080p rgba"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    for (i, b) in pixels.iter_mut().enumerate() {
        *b = (i.wrapping_mul(1103515245).wrapping_add(12345) >> 16) as u8;
    }
    queue.write_texture(
        texture.as_image_copy(),
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        texture.size(),
    );

    let mut sender = Sender::create("eiviz-1080p-gpu", FrameType::VIDEO).expect("sender");
    sender.force_subscribe(true, false, false);
    sender
        .send_video_texture(
            &ctx,
            &texture,
            VideoTextureMeta {
                width,
                height,
                timestamp: 1,
                frame_rate_n: 60,
                frame_rate_d: 1,
                ..Default::default()
            },
        )
        .expect("1080p GPU OMT send");
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
        assert_eq!(mixer_unit_cut(1, 1, 0), OK);
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
fn mix_input_define_and_self_cycle_rejected() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);
    assert_eq!(mixer_create_unit(2, 320, 180), OK);
    assert_eq!(mixer_define_mix_input(20, 1, SRC_KIND_MU_PROGRAM, 2, 0), OK);
    assert_eq!(mixer_define_mix_input(21, 2, SRC_KIND_MU_PREVIEW, 1, 1), OK);
    assert_eq!(
        mixer_define_mix_input(0, 1, SRC_KIND_MU_PROGRAM, 1, 0),
        ERR_INVALID_ARGUMENT
    );

    let mut cycle = UnitState {
        program_source: 20,
        preview_source: SRC_BARS,
        ..UnitState::default()
    };
    unsafe {
        assert_eq!(mixer_unit_set_state(1, &cycle), ERR_INVALID_ARGUMENT);
    }

    let nested = UnitState {
        program_source: 20,
        preview_source: SRC_COLOR,
        ..UnitState::default()
    };
    unsafe {
        assert_eq!(mixer_unit_set_state(2, &nested), OK);
    }

    cycle.program_source = SRC_BARS;
    unsafe {
        assert_eq!(mixer_unit_set_state(1, &cycle), OK);
    }
    mixer_destroy();
}

#[test]
fn missing_video_file_returns_io_error() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    let path = CString::new(r"C:\eiviz-missing-file-does-not-exist.mp4").unwrap();
    unsafe {
        assert_eq!(
            mixer_video_start(99, path.as_ptr(), 0, 0, 0, 0, 0, 0, 0),
            ERR_IO
        );
    }
    mixer_destroy();
}

#[test]
fn omt_connect_returns_before_unreachable_timeout() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    let address = CString::new("omt://127.0.0.1:1/missing").unwrap();
    let started = Instant::now();
    unsafe {
        assert_eq!(mixer_omt_connect(20, address.as_ptr(), 0, 1, 0), OK);
    }
    assert!(
        started.elapsed() < Duration::from_millis(750),
        "omt connect blocked for {:?}",
        started.elapsed()
    );
    mixer_destroy();
}

#[test]
fn reload_mixer_then_preview_and_cut() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);
    let bars = full_layer(SRC_BARS);
    let color = full_layer(SRC_COLOR);
    unsafe {
        assert_eq!(mixer_define_scene(scene_id(1), 320, 180, 1, &bars), OK);
        assert_eq!(mixer_define_scene(scene_id(2), 320, 180, 1, &color), OK);
        let state = UnitState {
            program_source: scene_id(1),
            preview_source: scene_id(2),
            mix: 0.0,
            ..UnitState::default()
        };
        assert_eq!(mixer_unit_set_state(1, &state), OK);
    }
    mixer_destroy();

    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);
    unsafe {
        assert_eq!(mixer_define_scene(scene_id(1), 320, 180, 1, &bars), OK);
        assert_eq!(mixer_define_scene(scene_id(2), 320, 180, 1, &color), OK);
        let mut state = UnitState {
            program_source: scene_id(1),
            preview_source: scene_id(1),
            mix: 0.0,
            ..UnitState::default()
        };
        assert_eq!(mixer_unit_set_state(1, &state), OK);
        state.preview_source = scene_id(2);
        assert_eq!(mixer_unit_set_state(1, &state), OK);
        let mut previewing = UnitState::default();
        assert_eq!(mixer_unit_get_state(1, &mut previewing), OK);
        assert_eq!(previewing.preview_source, scene_id(2));
        assert_eq!(previewing.program_source, scene_id(1));
        assert_eq!(mixer_unit_cut(1, 1, 0), OK);
        let mut after = UnitState::default();
        assert_eq!(mixer_unit_get_state(1, &mut after), OK);
        assert_eq!(after.program_source, scene_id(2));
        assert_eq!(after.preview_source, scene_id(1));
        assert_eq!(after.mix, 0.0);
    }
    mixer_destroy();
}

#[test]
fn keep_preview_freezes_incoming_source() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);
    let mut state = UnitState {
        program_source: SRC_COLOR,
        preview_source: SRC_BLUE,
        mix: 0.0,
        transition_kind: TRANSITION_FADE,
        ..UnitState::default()
    };
    unsafe {
        assert_eq!(mixer_unit_set_state(1, &state), OK);
        assert_eq!(
            mixer_unit_auto(
                1,
                TRANSITION_FADE,
                400,
                1,
                1,
                0,
                0,
                0.0,
                0.0,
                0.0,
                1.0,
                0,
                0.02,
                0.0
            ),
            OK
        );
        state.preview_source = SRC_BARS;
        assert_eq!(mixer_unit_set_state(1, &state), OK);
    }
    thread::sleep(Duration::from_millis(550));
    unsafe {
        let mut out = UnitState::default();
        assert_eq!(mixer_unit_get_state(1, &mut out), OK);
        assert_eq!(out.program_source, SRC_BLUE);
        assert_eq!(out.mix, 0.0);
    }
    mixer_destroy();
}

#[test]
fn easing_completes_with_cut() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);
    let state = UnitState {
        program_source: SRC_COLOR,
        preview_source: SRC_BLUE,
        mix: 0.0,
        transition_kind: TRANSITION_FADE,
        ..UnitState::default()
    };
    unsafe {
        assert_eq!(mixer_unit_set_state(1, &state), OK);
        assert_eq!(
            mixer_unit_auto(
                1,
                TRANSITION_FADE,
                200,
                1,
                1,
                EASING_IN_OUT,
                0,
                0.0,
                0.0,
                0.0,
                1.0,
                0,
                0.02,
                0.0
            ),
            OK
        );
    }
    thread::sleep(Duration::from_millis(350));
    unsafe {
        let mut out = UnitState::default();
        assert_eq!(mixer_unit_get_state(1, &mut out), OK);
        assert_eq!(out.mix, 0.0);
        assert_eq!(out.program_source, SRC_BLUE);
    }
    mixer_destroy();
}

#[test]
fn wipe_and_slide_emit_frames() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);
    let mut state = UnitState {
        program_source: SRC_COLOR,
        preview_source: SRC_BLUE,
        mix: 0.45,
        transition_kind: TRANSITION_WIPE,
        ..UnitState::default()
    };
    unsafe {
        assert_eq!(mixer_unit_set_state(1, &state), OK);
        thread::sleep(Duration::from_millis(80));
        try_acquire(1);
        state.transition_kind = TRANSITION_SLIDE;
        assert_eq!(mixer_unit_set_state(1, &state), OK);
        thread::sleep(Duration::from_millis(80));
        try_acquire(1);
    }
    mixer_destroy();
}

#[test]
fn shader_transitions_emit_frames() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);
    let kinds = [
        TRANSITION_CUBE,
        TRANSITION_CUBE_ZOOM,
        TRANSITION_FLY_ROTATE,
        TRANSITION_LOREZ,
        TRANSITION_METAMIX,
        TRANSITION_TILE,
        TRANSITION_PARTS,
        TRANSITION_SWIRL,
        TRANSITION_MULTITASK,
        TRANSITION_HEART,
        TRANSITION_STAR,
        TRANSITION_GLITCH,
        TRANSITION_PAGE_CURL,
        TRANSITION_PIXEL_SORT,
        TRANSITION_DATAMOSH,
        TRANSITION_VISUAL_DISSOLVE,
        TRANSITION_OPTICAL_FLOW,
        TRANSITION_BLOOM,
    ];
    for kind in kinds {
        let state = UnitState {
            program_source: SRC_COLOR,
            preview_source: SRC_BLUE,
            mix: 0.45,
            transition_kind: kind,
            softness: 0.02,
            param: 0.0,
            ..UnitState::default()
        };
        unsafe {
            assert_eq!(mixer_unit_set_state(1, &state), OK);
            thread::sleep(Duration::from_millis(80));
            try_acquire(1);
        }
    }
    mixer_destroy();
}

#[test]
fn custom_wgsl_can_sample_prev_and_time() {
    let src = r#"
fn user_compute(id: vec3<u32>, dim: vec2<u32>) {
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(dim);
    let c = textureSampleLevel(pgm_tex, src_samp, uv, 0.0);
    user_store(vec2<i32>(id.xy), c);
}
fn user_transition(uv: vec2<f32>, t: f32) -> vec4<f32> {
    let a = textureSample(pgm_tex, src_samp, uv);
    let p = textureSample(prev_tex, src_samp_n, uv);
    let flow = textureSample(flow_tex, src_samp, uv);
    let bloom = textureSample(bloom_tex, src_samp, uv);
    let aux = textureSample(aux_tex, src_samp, uv);
    return mix(mix(a, p, fract(params.time) * t), aux + bloom * 0.1 + flow, 0.0);
}
"#;
    let cstr = CString::new(src).unwrap();
    unsafe {
        assert_eq!(mixer_validate_custom_wgsl(cstr.as_ptr()), OK);
    }
}

#[test]
fn dip_uses_preset_color() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);
    let state = UnitState {
        program_source: SRC_COLOR,
        preview_source: SRC_BLUE,
        mix: 0.0,
        transition_kind: TRANSITION_DIP,
        ..UnitState::default()
    };
    unsafe {
        assert_eq!(mixer_unit_set_state(1, &state), OK);
        assert_eq!(
            mixer_unit_auto(
                1,
                TRANSITION_DIP,
                300,
                1,
                1,
                0,
                0,
                0.2,
                0.4,
                0.8,
                1.0,
                0,
                0.02,
                0.0
            ),
            OK
        );
        let mut out = UnitState::default();
        assert_eq!(mixer_unit_get_state(1, &mut out), OK);
        assert!((out.dip_r - 0.2).abs() < 0.001);
        assert!((out.dip_g - 0.4).abs() < 0.001);
        assert!((out.dip_b - 0.8).abs() < 0.001);
        thread::sleep(Duration::from_millis(80));
        try_acquire(1);
    }
    mixer_destroy();
}

#[test]
fn auto_uses_incoming_source_instead_of_preview() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);
    let state = UnitState {
        program_source: SRC_COLOR,
        preview_source: SRC_BLUE,
        mix: 0.0,
        transition_kind: TRANSITION_FADE,
        ..UnitState::default()
    };
    unsafe {
        assert_eq!(mixer_unit_set_state(1, &state), OK);
        assert_eq!(
            mixer_unit_auto(
                1,
                TRANSITION_FADE,
                200,
                1,
                0,
                0,
                0,
                0.0,
                0.0,
                0.0,
                1.0,
                SRC_BARS,
                0.02,
                0.0
            ),
            OK
        );
    }
    thread::sleep(Duration::from_millis(350));
    unsafe {
        let mut out = UnitState::default();
        assert_eq!(mixer_unit_get_state(1, &mut out), OK);
        assert_eq!(out.program_source, SRC_BARS);
        assert_eq!(out.preview_source, SRC_BLUE);
        assert_eq!(out.incoming_source, 0);
        assert_eq!(out.mix, 0.0);
    }
    mixer_destroy();
}

#[test]
fn cut_uses_incoming_source_instead_of_preview() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);
    let state = UnitState {
        program_source: SRC_COLOR,
        preview_source: SRC_BLUE,
        mix: 0.0,
        ..UnitState::default()
    };
    unsafe {
        assert_eq!(mixer_unit_set_state(1, &state), OK);
        assert_eq!(mixer_unit_cut(1, 1, SRC_BARS), OK);
        let mut out = UnitState::default();
        assert_eq!(mixer_unit_get_state(1, &mut out), OK);
        assert_eq!(out.program_source, SRC_BARS);
        assert_eq!(out.preview_source, SRC_BLUE);
        assert_eq!(out.incoming_source, 0);
    }
    mixer_destroy();
}

#[test]
fn leftover_incoming_source_does_not_override_preview() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);
    let state = UnitState {
        program_source: SRC_COLOR,
        preview_source: SRC_BLUE,
        incoming_source: SRC_BARS,
        mix: 0.0,
        ..UnitState::default()
    };
    unsafe {
        assert_eq!(mixer_unit_set_state(1, &state), OK);
        assert_eq!(mixer_unit_cut(1, 1, 0), OK);
        let mut out = UnitState::default();
        assert_eq!(mixer_unit_get_state(1, &mut out), OK);
        assert_eq!(out.program_source, SRC_BLUE);
        assert_eq!(out.preview_source, SRC_COLOR);
        assert_eq!(out.incoming_source, 0);
    }
    mixer_destroy();
}

#[test]
fn cut_program_sentinel_keeps_program() {
    mixer_destroy();
    assert_eq!(mixer_create(0, 60_000, 1_001), OK);
    assert_eq!(mixer_create_unit(1, 320, 180), OK);
    let state = UnitState {
        program_source: SRC_COLOR,
        preview_source: SRC_BLUE,
        mix: 0.0,
        ..UnitState::default()
    };
    unsafe {
        assert_eq!(mixer_unit_set_state(1, &state), OK);
        assert_eq!(mixer_unit_cut(1, 1, INCOMING_PROGRAM), OK);
        let mut out = UnitState::default();
        assert_eq!(mixer_unit_get_state(1, &mut out), OK);
        assert_eq!(out.program_source, SRC_COLOR);
        assert_eq!(out.preview_source, SRC_BLUE);
        assert_eq!(out.incoming_source, 0);
    }
    mixer_destroy();
}
