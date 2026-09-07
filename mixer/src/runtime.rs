use std::ffi::CString;
use std::sync::{Mutex, OnceLock};

use eiviz_control::error::{ControlError, ControlResult};
use eiviz_control::live::{LiveState, UnitLiveState};
use eiviz_control::port::*;
use eiviz_control::service::{ControlService, RequestKey};
use eiviz_control::{Command, Incoming};

use crate::abi::{ERR_INVALID_ARGUMENT, GEN_BARS, GEN_SOLID, OK, OverlayDesc, Rect, UnitState};
use crate::{
    mixer_api_configure, mixer_audio_bus_remove, mixer_audio_bus_upsert, mixer_audio_set_bus_gain,
    mixer_audio_set_headphone_copy_master, mixer_audio_set_input, mixer_audio_set_unit_link,
    mixer_bind_multiview, mixer_create_unit, mixer_define_generator, mixer_define_mix_input,
    mixer_define_scene, mixer_destroy_scene, mixer_destroy_source, mixer_destroy_unit,
    mixer_load_still, mixer_ndi_connect, mixer_ndi_discover, mixer_omt_connect, mixer_omt_discover,
    mixer_omt_set_quality, mixer_output_add, mixer_output_remove, mixer_set_bus_colors,
    mixer_set_frame_buffer, mixer_set_live_save, mixer_set_mv_label, mixer_set_ndi_gpu_upload,
    mixer_set_rebar_optimization, mixer_snapshot, mixer_unit_configure, mixer_unit_get_state,
    mixer_video_seek, mixer_video_set_loop, mixer_video_set_playing, mixer_video_start,
};

pub(crate) fn control() -> &'static Mutex<ControlService> {
    static CONTROL: OnceLock<Mutex<ControlService>> = OnceLock::new();
    CONTROL.get_or_init(|| Mutex::new(ControlService::new(ProcessMixer)))
}

pub(crate) fn map_abi(code: i32) -> ControlResult<()> {
    if code == OK {
        Ok(())
    } else {
        Err(ControlError::from_abi(code, last_error_text()))
    }
}

pub(crate) fn last_error_text() -> String {
    let mut buf = vec![0u8; 512];
    let n = unsafe { crate::mixer_last_error(buf.as_mut_ptr(), buf.len()) };
    if n <= 0 {
        return String::new();
    }
    String::from_utf8_lossy(&buf[..n as usize]).into_owned()
}

pub struct ProcessMixer;

impl MixerPort for ProcessMixer {
    fn create(
        &mut self,
        fps_num: u32,
        fps_den: u32,
        renderer: eiviz_control::session::Renderer,
    ) -> ControlResult<()> {
        map_abi(crate::mixer_create_with_backend(
            renderer.create_abi(),
            0,
            fps_num,
            fps_den,
        ))
    }

    fn destroy(&mut self) -> ControlResult<()> {
        crate::mixer_destroy_inner();
        Ok(())
    }

    fn is_ready(&self) -> bool {
        crate::mixer_created()
    }

    fn create_unit(&mut self, id: u64, width: u32, height: u32) -> ControlResult<()> {
        map_abi(mixer_create_unit(id, width, height))
    }

    fn destroy_unit(&mut self, id: u64) -> ControlResult<()> {
        map_abi(mixer_destroy_unit(id))
    }

    fn configure_unit(
        &mut self,
        id: u64,
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
    ) -> ControlResult<()> {
        map_abi(mixer_unit_configure(id, width, height, fps_num, fps_den))
    }

    fn define_scene(&mut self, spec: SceneApply) -> ControlResult<()> {
        let labels: Vec<CString> = spec
            .layers
            .iter()
            .map(|layer| {
                CString::new(layer.label.replace('\0', ""))
                    .unwrap_or_else(|_| CString::new("").unwrap())
            })
            .collect();
        let mut layers: Vec<OverlayDesc> = spec
            .layers
            .iter()
            .enumerate()
            .map(|(index, layer)| OverlayDesc {
                source_id: layer.source_id,
                rect: Rect {
                    x: layer.x,
                    y: layer.y,
                    width: layer.width,
                    height: layer.height,
                },
                crop: Rect {
                    x: layer.crop_x,
                    y: layer.crop_y,
                    width: layer.crop_width,
                    height: layer.crop_height,
                },
                opacity: layer.opacity,
                z: layer.z,
                audio_follow: u32::from(layer.audio_follow),
                hidden: u32::from(layer.hidden),
                label: if labels[index].as_bytes().is_empty() {
                    std::ptr::null()
                } else {
                    labels[index].as_ptr()
                },
            })
            .collect();
        let code = unsafe {
            mixer_define_scene(
                spec.id,
                spec.width,
                spec.height,
                layers.len() as u32,
                if layers.is_empty() {
                    std::ptr::null()
                } else {
                    layers.as_mut_ptr()
                },
            )
        };
        map_abi(code)
    }

    fn destroy_scene(&mut self, id: u64) -> ControlResult<()> {
        map_abi(mixer_destroy_scene(id))
    }

    fn define_generator(&mut self, spec: GeneratorApply) -> ControlResult<()> {
        let kind = if spec.bars { GEN_BARS } else { GEN_SOLID };
        map_abi(mixer_define_generator(
            spec.id,
            kind,
            spec.r,
            spec.g,
            spec.b,
            1.0,
            u32::from(spec.scroll),
        ))?;
        map_abi(crate::mixer_generator_set_tone(
            spec.id,
            spec.tone_hz,
            spec.tone_level_dbfs,
        ))
    }

    fn define_mix_input(&mut self, spec: MixInputApply) -> ControlResult<()> {
        map_abi(mixer_define_mix_input(
            spec.id,
            spec.target_id,
            spec.source_kind,
            spec.delay,
            spec.audio_bus_id,
        ))
    }

    fn load_still(&mut self, id: u64, path: &str) -> ControlResult<()> {
        let c_path =
            CString::new(path).map_err(|error| ControlError::invalid(error.to_string()))?;
        map_abi(unsafe { mixer_load_still(id, c_path.as_ptr()) })
    }

    fn video_start(&mut self, spec: VideoStartApply) -> ControlResult<()> {
        let c_path =
            CString::new(spec.path).map_err(|error| ControlError::invalid(error.to_string()))?;
        map_abi(unsafe {
            mixer_video_start(
                spec.id,
                c_path.as_ptr(),
                u32::from(spec.capture),
                crate::abi::FMT_UYVY,
                spec.width,
                spec.height,
                spec.fps_num,
                spec.fps_den,
                spec.frame_buffer_frames,
            )
        })?;
        map_abi(mixer_video_set_loop(spec.id, u32::from(spec.loop_playback)))?;
        map_abi(mixer_video_set_playing(spec.id, u32::from(spec.playing)))?;
        if spec.position_hns > 0 {
            map_abi(mixer_video_seek(spec.id, spec.position_hns))?;
        }
        Ok(())
    }

    fn omt_connect(&mut self, spec: LiveConnectApply) -> ControlResult<()> {
        let address =
            CString::new(spec.address).map_err(|error| ControlError::invalid(error.to_string()))?;
        map_abi(unsafe {
            mixer_omt_connect(
                spec.id,
                address.as_ptr(),
                u32::from(spec.use_gpu),
                spec.frame_buffer_frames,
                spec.quality_or_bandwidth,
            )
        })?;
        map_abi(mixer_set_live_save(
            spec.id,
            spec.save_mode,
            if spec.keep_full_on_multiview {
                crate::abi::SAVE_FLAG_MULTIVIEW
            } else {
                0
            },
        ))
    }

    fn ndi_connect(&mut self, spec: LiveConnectApply) -> ControlResult<()> {
        let address =
            CString::new(spec.address).map_err(|error| ControlError::invalid(error.to_string()))?;
        map_abi(unsafe {
            mixer_ndi_connect(
                spec.id,
                address.as_ptr(),
                spec.frame_buffer_frames,
                spec.quality_or_bandwidth,
            )
        })
    }

    fn destroy_source(&mut self, id: u64) -> ControlResult<()> {
        map_abi(mixer_destroy_source(id))
    }

    fn set_live_save(&mut self, id: u64, mode: u32, flags: u32) -> ControlResult<()> {
        map_abi(mixer_set_live_save(id, mode, flags))
    }

    fn set_omt_quality(&mut self, id: u64, quality: u32) -> ControlResult<()> {
        map_abi(mixer_omt_set_quality(id, quality))
    }

    fn output_add(&mut self, spec: OutputApply) -> ControlResult<()> {
        let name = CString::new(spec.name).unwrap_or_else(|_| CString::new("").unwrap());
        map_abi(unsafe {
            mixer_output_add(
                spec.id,
                spec.transport,
                name.as_ptr(),
                spec.source_kind,
                spec.source_id,
                spec.unit_id,
                u32::from(spec.use_gpu),
                spec.audio_bus_id,
                u32::from(spec.skip_encode_when_no_receivers),
            )
        })
    }

    fn output_remove(&mut self, id: u64) -> ControlResult<()> {
        map_abi(mixer_output_remove(id))
    }

    fn audio_bus_upsert(&mut self, spec: BusApply) -> ControlResult<()> {
        let name = CString::new(spec.name).unwrap_or_else(|_| CString::new("").unwrap());
        let device = CString::new(spec.device_id).unwrap_or_else(|_| CString::new("").unwrap());
        map_abi(unsafe {
            mixer_audio_bus_upsert(
                spec.id,
                name.as_ptr(),
                spec.role,
                spec.device_kind,
                device.as_ptr(),
                spec.map_left as i32,
                spec.map_right as i32,
                u32::from(spec.exclusive),
            )
        })?;
        map_abi(mixer_audio_set_bus_gain(
            spec.id,
            spec.gain,
            u32::from(spec.mute),
        ))
    }

    fn audio_bus_remove(&mut self, id: u64) -> ControlResult<()> {
        map_abi(mixer_audio_bus_remove(id))
    }

    fn audio_set_input(
        &mut self,
        id: u64,
        bus_mask: u32,
        gain: f32,
        mute: bool,
    ) -> ControlResult<()> {
        map_abi(mixer_audio_set_input(id, bus_mask, gain, u32::from(mute)))
    }

    fn audio_set_bus_gain(&mut self, id: u64, gain: f32, mute: bool) -> ControlResult<()> {
        map_abi(mixer_audio_set_bus_gain(id, gain, u32::from(mute)))
    }

    fn audio_set_unit_link(&mut self, unit_id: u64, bus_id: u64, mode: u32) -> ControlResult<()> {
        map_abi(mixer_audio_set_unit_link(unit_id, bus_id, mode))
    }

    fn audio_set_headphone_copy_master(&mut self, enabled: bool) -> ControlResult<()> {
        map_abi(mixer_audio_set_headphone_copy_master(u32::from(enabled)))
    }

    fn set_frame_buffer(&mut self, frames: u32) -> ControlResult<()> {
        map_abi(mixer_set_frame_buffer(frames))
    }

    fn set_rebar_optimization(&mut self, enabled: bool) -> ControlResult<()> {
        map_abi(mixer_set_rebar_optimization(u32::from(enabled)))
    }

    fn set_ndi_gpu_upload(&mut self, enabled: bool) -> ControlResult<()> {
        map_abi(mixer_set_ndi_gpu_upload(u32::from(enabled)))
    }

    fn set_bus_colors(
        &mut self,
        preview: [u8; 3],
        program: [u8; 3],
        inactive: [u8; 3],
    ) -> ControlResult<()> {
        map_abi(mixer_set_bus_colors(
            preview[0],
            preview[1],
            preview[2],
            program[0],
            program[1],
            program[2],
            inactive[0],
            inactive[1],
            inactive[2],
        ))
    }

    fn set_mv_label(
        &mut self,
        scene_id: u64,
        size: f32,
        percent: bool,
        top: bool,
    ) -> ControlResult<()> {
        map_abi(mixer_set_mv_label(
            scene_id,
            size,
            u32::from(percent),
            u32::from(top),
        ))
    }

    fn bind_multiview(
        &mut self,
        scene_id: u64,
        preview_unit: u64,
        program_unit: u64,
    ) -> ControlResult<()> {
        map_abi(mixer_bind_multiview(scene_id, preview_unit, program_unit))
    }

    fn unit_cut(&mut self, unit_id: u64, swap: bool, incoming: u64) -> ControlResult<()> {
        map_abi(crate::unit_cut_inner(unit_id, u32::from(swap), incoming))
    }

    fn unit_auto(&mut self, spec: AutoApply) -> ControlResult<()> {
        map_abi(crate::unit_auto_inner(
            spec.unit_id,
            spec.kind,
            spec.duration_ms,
            u32::from(spec.swap),
            u32::from(spec.keep_preview),
            spec.easing,
            spec.direction,
            spec.dip_r,
            spec.dip_g,
            spec.dip_b,
            spec.dip_a,
            spec.incoming,
            spec.softness,
            spec.param,
        ))
    }

    fn unit_set_preview(&mut self, unit_id: u64, scene_gpu_id: u64) -> ControlResult<()> {
        let mut state = UnitState::default();
        map_abi(unsafe { mixer_unit_get_state(unit_id, &mut state) })?;
        state.preview_source = scene_gpu_id;
        map_abi(crate::unit_set_state_inner(unit_id, &state))
    }

    fn unit_set_mix(&mut self, unit_id: u64, mix: f32) -> ControlResult<()> {
        let mut state = UnitState::default();
        map_abi(unsafe { mixer_unit_get_state(unit_id, &mut state) })?;
        state.mix = mix;
        map_abi(crate::unit_set_state_inner(unit_id, &state))
    }

    fn overlay_auto(&mut self, spec: OverlayAutoApply) -> ControlResult<()> {
        let desc = OverlayDesc {
            source_id: spec.source_id,
            rect: Rect {
                x: spec.x,
                y: spec.y,
                width: spec.width,
                height: spec.height,
            },
            crop: Rect::default(),
            opacity: spec.opacity,
            z: spec.z,
            audio_follow: u32::from(spec.audio_follow),
            hidden: u32::from(spec.hidden),
            label: std::ptr::null(),
        };
        map_abi(crate::overlay_auto_inner(
            spec.unit_id,
            u32::from(spec.to_on),
            spec.duration_ms,
            desc,
        ))
    }

    fn unit_set_state(
        &mut self,
        unit_id: u64,
        program: u64,
        preview: u64,
        _mix: f32,
    ) -> ControlResult<()> {
        let mut state = UnitState::default();
        map_abi(unsafe { mixer_unit_get_state(unit_id, &mut state) })?;
        state.program_source = program;
        state.preview_source = preview;
        map_abi(crate::unit_set_state_inner(unit_id, &state))
    }

    fn unit_live(&self, unit_id: u64) -> ControlResult<UnitLiveState> {
        crate::live_unit(unit_id)
            .map(|live| UnitLiveState {
                program_source: live.program_source,
                preview_source: live.preview_source,
                mix: 0.0,
                transitioning: false,
                incoming_source: 0,
                overlay_sources: live.overlay_sources,
            })
            .ok_or_else(|| ControlError::not_found(format!("unit {unit_id}")))
    }

    fn live_state(&self) -> ControlResult<LiveState> {
        Ok(crate::all_live_state())
    }

    fn video_set_playing(&mut self, id: u64, playing: bool) -> ControlResult<()> {
        map_abi(mixer_video_set_playing(id, u32::from(playing)))
    }

    fn video_set_loop(&mut self, id: u64, looping: bool) -> ControlResult<()> {
        map_abi(mixer_video_set_loop(id, u32::from(looping)))
    }

    fn video_seek(&mut self, id: u64, position_hns: i64) -> ControlResult<()> {
        map_abi(mixer_video_seek(id, position_hns))
    }

    fn snapshot(&mut self, unit_id: u64, kind: u32, path: &str) -> ControlResult<()> {
        let c_path =
            CString::new(path).map_err(|error| ControlError::invalid(error.to_string()))?;
        map_abi(unsafe { mixer_snapshot(unit_id, kind, c_path.as_ptr()) })
    }

    fn publish_session(&mut self, document: &eiviz_control::session::Document) -> ControlResult<()> {
        let bytes = eiviz_control::session::to_vec(document)
            .map_err(|error| ControlError::invalid(error))?;
        crate::vmix_api::publish_bytes(&bytes);
        Ok(())
    }

    fn configure_vmix_api(
        &mut self,
        enabled: bool,
        tcp_enabled: bool,
        port: u32,
        user: &str,
        pass: &str,
    ) -> ControlResult<()> {
        let user = CString::new(user).unwrap_or_else(|_| CString::new("").unwrap());
        let pass = CString::new(pass).unwrap_or_else(|_| CString::new("").unwrap());
        let http =
            unsafe { mixer_api_configure(u32::from(enabled), port, user.as_ptr(), pass.as_ptr()) };
        let tcp = crate::vmix_tcp::configure(tcp_enabled);
        if http == crate::abi::ERR_IO || tcp == crate::abi::ERR_IO {
            return Ok(());
        }
        map_abi(http).and_then(|_| map_abi(tcp))
    }

    fn configure_native_api(&mut self, _enabled: bool, _port: u32) -> ControlResult<()> {
        Ok(())
    }

    fn discover_omt(&self) -> ControlResult<String> {
        let mut buf = vec![0u8; 4096];
        let n = unsafe { mixer_omt_discover(buf.as_mut_ptr(), buf.len()) };
        if n < 0 {
            return Err(ControlError::from_abi(-n, last_error_text()));
        }
        Ok(String::from_utf8_lossy(&buf[..n as usize]).into_owned())
    }

    fn discover_ndi(&self) -> ControlResult<String> {
        let mut buf = vec![0u8; 4096];
        let n = unsafe { mixer_ndi_discover(buf.as_mut_ptr(), buf.len()) };
        if n < 0 {
            return Err(ControlError::from_abi(-n, last_error_text()));
        }
        Ok(String::from_utf8_lossy(&buf[..n as usize]).into_owned())
    }

    fn apply_reconcile(
        &mut self,
        next: &eiviz_control::session::Document,
        op: &eiviz_control::session::reconcile::ReconcileOp,
        statuses: &mut Vec<eiviz_control::live::ResourceStatus>,
    ) -> ControlResult<()> {
        eiviz_control::session::reconcile::apply_one(self, next, op, statuses)
    }
}

pub(crate) fn replace_session_bytes(bytes: &[u8], expected_revision: u64) -> i32 {
    let document = match eiviz_control::session::parse(bytes) {
        Ok(doc) => doc,
        Err(error) => {
            crate::diag::error(&error);
            return -ERR_INVALID_ARGUMENT;
        }
    };
    let expected = if expected_revision == 0 {
        None
    } else {
        Some(expected_revision)
    };
    match control().lock() {
        Ok(mut svc) => match svc.execute(
            RequestKey {
                client_instance_id: "c-abi".into(),
                request_id: String::new(),
            },
            Command::ReplaceSession {
                document: Box::new(document),
                expected_revision: expected,
            },
        ) {
            Ok(_) => {
                let _ = crate::vmix_api::publish_bytes(bytes);
                OK
            }
            Err(error) => {
                crate::diag::error(error.message());
                -error.to_abi()
            }
        },
        Err(_) => -ERR_INVALID_ARGUMENT,
    }
}

pub(crate) fn poll_event(after: u64, out: &mut [u8]) -> i32 {
    let Ok(svc) = control().lock() else {
        return -ERR_INVALID_ARGUMENT;
    };
    let events = svc.events_after(after);
    if events.is_empty() {
        return 0;
    }
    let json = serde_json::to_vec(&events_json(&events)).unwrap_or_default();
    let n = json.len().min(out.len());
    out[..n].copy_from_slice(&json[..n]);
    n as i32
}

fn events_json(events: &[eiviz_control::Event]) -> serde_json::Value {
    serde_json::json!({
        "events": events.iter().map(|event| {
            let mut value = serde_json::json!({
                "sequence": event.meta().sequence,
                "revision": event.meta().session_revision,
                "kind": event.kind_name(),
                "requestId": event.meta().request_id,
            });
            if let eiviz_control::Event::SessionChanged { document, .. } = event {
                if let Ok(json) = eiviz_control::session::to_vec(document) {
                    value["documentJson"] = serde_json::Value::String(String::from_utf8_lossy(&json).into_owned());
                }
            }
            if let eiviz_control::Event::LiveChanged { live, .. } = event {
                if let Ok(json) = serde_json::to_vec(live) {
                    value["liveJson"] = serde_json::Value::String(String::from_utf8_lossy(&json).into_owned());
                }
            }
            value
        }).collect::<Vec<_>>()
    })
}

pub(crate) fn copy_snapshot_bytes(out: &mut [u8]) -> i32 {
    let Ok(svc) = control().lock() else {
        return -ERR_INVALID_ARGUMENT;
    };
    let Ok(snap) = svc.snapshot() else {
        return -ERR_INVALID_ARGUMENT;
    };
    let json = eiviz_control::session::to_vec(&snap.document).unwrap_or_default();
    let n = json.len().min(out.len());
    if json.len() > out.len() {
        return -1;
    }
    out[..n].copy_from_slice(&json[..n]);
    n as i32
}

pub(crate) fn c_auto(
    unit_id: u64,
    kind: u32,
    duration_ms: u32,
    swap: u32,
    keep_preview: u32,
    easing: u32,
    direction: u32,
    dip_r: f32,
    dip_g: f32,
    dip_b: f32,
    dip_a: f32,
    incoming: u64,
    softness: f32,
    param: f32,
) -> i32 {
    match control().lock() {
        Ok(mut svc) => match svc.execute(
            RequestKey {
                client_instance_id: "c-abi".into(),
                request_id: String::new(),
            },
            Command::Auto {
                unit_id,
                kind,
                duration_ms,
                swap: swap != 0,
                keep_preview: keep_preview != 0,
                easing,
                direction,
                dip_r,
                dip_g,
                dip_b,
                dip_a,
                incoming: Incoming::from_u64(incoming),
                softness,
                param,
            },
        ) {
            Ok(_) => OK,
            Err(error) => -error.to_abi(),
        },
        Err(_) => -ERR_INVALID_ARGUMENT,
    }
}

pub(crate) fn note_live(name: &str) {
    if let Ok(mut svc) = control().try_lock() {
        svc.note_external_live(name, "c-abi");
    }
}

pub(crate) fn c_cut(unit_id: u64, swap: u32, incoming: u64) -> i32 {
    match control().lock() {
        Ok(mut svc) => match svc.execute(
            RequestKey {
                client_instance_id: "c-abi".into(),
                request_id: String::new(),
            },
            Command::Cut {
                unit_id,
                swap: swap != 0,
                incoming: Incoming::from_u64(incoming),
            },
        ) {
            Ok(_) => OK,
            Err(error) => -error.to_abi(),
        },
        Err(_) => -ERR_INVALID_ARGUMENT,
    }
}
