use crate::live::LiveState;
use crate::session::{
    AudioCaptureMode, AudioDeviceKind, AudioLinkMode, BandwidthSave, Document, InputDto, InputKind,
    MixSource, MultiviewDto, MvSlotKind, NdiBandwidth, OmtQuality, OutputDto, OutputSourceKind,
    OutputTransport, SceneDto, VideoPlayWhen,
};
use crate::{geometry::MultiviewPane, ids, port::*};

#[derive(Debug, Clone, PartialEq)]
pub enum ReconcileOp {
    Settings,
    CreateUnit {
        id: u64,
        width: u32,
        height: u32,
    },
    ConfigureUnit {
        id: u64,
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
    },
    DestroyUnit {
        id: u64,
    },
    UpsertBus(BusApply),
    RemoveBus {
        id: u64,
    },
    DefineGenerator(GeneratorApply),
    DefineMixInput(MixInputApply),
    LoadStill {
        id: u64,
        path: String,
    },
    StartVideo(VideoStartApply),
    ConnectOmt(LiveConnectApply),
    ConnectNdi(LiveConnectApply),
    StartAudioCapture(AudioCaptureApply),
    DestroySource {
        id: u64,
    },
    FailInput {
        id: u64,
        message: String,
    },
    DefineScene(SceneApply),
    DestroyScene {
        id: u64,
    },
    DefineMultiview {
        spec: SceneApply,
        preview_unit: u64,
        program_unit: u64,
    },
    SetLiveState {
        unit_id: u64,
        program: u64,
        preview: u64,
    },
    AddOutput(OutputApply),
    RemoveOutput {
        id: u64,
    },
    AudioInput {
        id: u64,
        bus_mask: u32,
        gain: f32,
        mute: bool,
    },
    AudioUnitLink {
        unit_id: u64,
        bus_id: u64,
        mode: u32,
    },
    HeadphoneCopyMaster {
        enabled: bool,
    },
    ConfigureVmixApi {
        http_enabled: bool,
        tcp_enabled: bool,
        port: u32,
        user: String,
        pass: String,
    },
    ConfigureNativeApi {
        enabled: bool,
        port: u32,
    },
}

#[inline(never)]
pub fn plan(previous: Option<&Document>, next: &Document) -> Vec<ReconcileOp> {
    let mut ops = Vec::new();
    ops.push(ReconcileOp::Settings);
    let prev_units: Vec<u64> = previous
        .map(|doc| doc.units.iter().map(|item| item.id).collect())
        .unwrap_or_default();
    let next_units: Vec<u64> = next.units.iter().map(|item| item.id).collect();
    for unit in &next.units {
        let prev_unit = previous.and_then(|doc| doc.units.iter().find(|item| item.id == unit.id));
        if prev_unit.is_none() {
            ops.push(ReconcileOp::CreateUnit {
                id: unit.id,
                width: unit.width,
                height: unit.height,
            });
        }
        if prev_unit.is_none_or(|prev| {
            prev.width != unit.width
                || prev.height != unit.height
                || prev.fps_num != unit.fps_num
                || prev.fps_den != unit.fps_den
        }) {
            ops.push(ReconcileOp::ConfigureUnit {
                id: unit.id,
                width: unit.width,
                height: unit.height,
                fps_num: unit.fps_num,
                fps_den: unit.fps_den,
            });
        }
    }
    for bus in &next.buses {
        ops.push(ReconcileOp::UpsertBus(BusApply {
            id: bus.id,
            name: bus.name.clone(),
            role: bus.role as u32,
            device_kind: match bus.device_kind {
                AudioDeviceKind::None => 0,
                AudioDeviceKind::Wasapi => 1,
                AudioDeviceKind::Asio => 2,
                AudioDeviceKind::CoreAudio => 3,
            },
            device_id: bus.device_id.clone(),
            map_left: bus.map_left.max(0) as u32,
            map_right: bus.map_right.max(0) as u32,
            gain: bus.gain,
            mute: bus.mute,
        }));
    }
    if let Some(prev) = previous {
        for bus in &prev.buses {
            if !next.buses.iter().any(|item| item.id == bus.id) {
                ops.push(ReconcileOp::RemoveBus { id: bus.id });
            }
        }
    }

    let prev_inputs = previous.map(|doc| doc.inputs.as_slice()).unwrap_or(&[]);
    for input in &next.inputs {
        let prev = prev_inputs.iter().find(|item| item.id == input.id);
        if let Some(prev) = prev {
            if input_desired_equal(prev, input) {
                continue;
            }
            if !crate::session::media_file_missing(prev) {
                ops.push(ReconcileOp::DestroySource { id: input.id });
            }
        }
        ops.extend(input_ops(input));
    }
    for prev in prev_inputs {
        if !next.inputs.iter().any(|item| item.id == prev.id) {
            ops.push(ReconcileOp::DestroySource { id: prev.id });
        }
    }

    let width = next.settings.default_width;
    let height = next.settings.default_height;
    let prev_scenes = previous.map(|doc| doc.scenes.as_slice()).unwrap_or(&[]);
    for scene in &next.scenes {
        let apply = scene_apply(scene, width, height);
        if prev_scenes
            .iter()
            .find(|item| item.id == scene.id)
            .is_some_and(|prev| scene_apply(prev, width, height) == apply)
        {
            continue;
        }
        ops.push(ReconcileOp::DefineScene(apply));
    }
    for prev in prev_scenes {
        if !next.scenes.iter().any(|item| item.id == prev.id) {
            ops.push(ReconcileOp::DestroyScene {
                id: ids::scene_gpu_id(prev.id),
            });
        }
    }

    let prev_mv = previous.map(|doc| doc.multiviews.as_slice()).unwrap_or(&[]);
    for layout in &next.multiviews {
        let op = multiview_op(layout, next, width, height);
        if prev_mv.iter().any(|prev| {
            prev.id == layout.id
                && multiview_op(prev, previous.unwrap_or(next), width, height) == op
        }) {
            continue;
        }
        ops.push(op);
    }
    for prev in prev_mv {
        if !next.multiviews.iter().any(|item| item.id == prev.id) {
            ops.push(ReconcileOp::DestroyScene {
                id: ids::multiview_gpu_id(prev.id),
            });
        }
    }

    let fallback_preview = next
        .scenes
        .first()
        .map(|scene| ids::scene_gpu_id(scene.id))
        .unwrap_or(0);
    let fallback_program = next
        .scenes
        .get(1)
        .map(|scene| ids::scene_gpu_id(scene.id))
        .unwrap_or(fallback_preview);
    for unit in &next.units {
        // Seed buses when the Mixing Unit itself is new. Stored scene IDs win
        // over the first/second-scene default when they are present.
        if !prev_units.contains(&unit.id) {
            let preview = if unit.preview_scene_id != 0 {
                ids::scene_gpu_id(unit.preview_scene_id)
            } else {
                fallback_preview
            };
            let program = if unit.program_scene_id != 0 {
                ids::scene_gpu_id(unit.program_scene_id)
            } else {
                fallback_program
            };
            ops.push(ReconcileOp::SetLiveState {
                unit_id: unit.id,
                program,
                preview,
            });
        }
        ops.push(ReconcileOp::AudioUnitLink {
            unit_id: unit.id,
            bus_id: if unit.audio_bus_id == 0 {
                1
            } else {
                unit.audio_bus_id
            },
            mode: match unit.audio_link {
                AudioLinkMode::Follow => 0,
                AudioLinkMode::Independent => 1,
            },
        });
    }

    for input in &next.inputs {
        ops.push(ReconcileOp::AudioInput {
            id: input.id,
            bus_mask: input.bus_mask,
            gain: input.gain,
            mute: input.mute,
        });
    }
    ops.push(ReconcileOp::HeadphoneCopyMaster {
        enabled: next.headphone_copy_master,
    });

    let prev_outputs = previous.map(|doc| doc.outputs.as_slice()).unwrap_or(&[]);
    for output in &next.outputs {
        if !output.enabled || matches!(output.transport, OutputTransport::DeckLink) {
            if prev_outputs.iter().any(|item| item.id == output.id) {
                ops.push(ReconcileOp::RemoveOutput { id: output.id });
            }
            continue;
        }
        let prev = prev_outputs.iter().find(|item| item.id == output.id);
        if prev.is_some_and(|item| output_equal(item, output)) {
            continue;
        }
        if prev.is_some() {
            ops.push(ReconcileOp::RemoveOutput { id: output.id });
        }
        ops.push(ReconcileOp::AddOutput(output_apply(output)));
    }
    for prev in prev_outputs {
        if !next
            .outputs
            .iter()
            .any(|item| item.id == prev.id && prev.enabled)
        {
            ops.push(ReconcileOp::RemoveOutput { id: prev.id });
        }
    }

    if previous.is_none_or(|prev| {
        prev.settings.vmix_api_enabled != next.settings.vmix_api_enabled
            || prev.settings.vmix_tcp_enabled != next.settings.vmix_tcp_enabled
            || prev.settings.vmix_api_port != next.settings.vmix_api_port
            || prev.settings.vmix_api_user != next.settings.vmix_api_user
            || prev.settings.vmix_api_password != next.settings.vmix_api_password
    }) {
        ops.push(ReconcileOp::ConfigureVmixApi {
            http_enabled: next.settings.vmix_api_enabled,
            tcp_enabled: next.settings.vmix_tcp_enabled,
            port: next.settings.vmix_api_port,
            user: next.settings.vmix_api_user.clone(),
            pass: next.settings.vmix_api_password.clone(),
        });
    }

    for id in prev_units {
        if !next_units.contains(&id) {
            ops.push(ReconcileOp::DestroyUnit { id });
        }
    }
    ops
}

/// Retarget PGM/PVW only when the live source disappeared from the next
/// document. Does not invent a new bus layout for settings-only replaces.
pub fn repair_live(next: &Document, live: &LiveState) -> Vec<ReconcileOp> {
    let preview = next
        .scenes
        .first()
        .map(|scene| ids::scene_gpu_id(scene.id))
        .unwrap_or(0);
    let program = next
        .scenes
        .get(1)
        .map(|scene| ids::scene_gpu_id(scene.id))
        .unwrap_or(preview);
    let mut ops = Vec::new();
    for unit in &next.units {
        let Some(state) = live.units.get(&unit.id) else {
            continue;
        };
        let next_program = if source_still_live(next, state.program_source) {
            state.program_source
        } else {
            program
        };
        let next_preview = if source_still_live(next, state.preview_source) {
            state.preview_source
        } else {
            preview
        };
        if next_program != state.program_source || next_preview != state.preview_source {
            ops.push(ReconcileOp::SetLiveState {
                unit_id: unit.id,
                program: next_program,
                preview: next_preview,
            });
        }
    }
    ops
}

fn source_still_live(doc: &Document, source: u64) -> bool {
    if source == 0 {
        return true;
    }
    if source & ids::MU_SOURCE_FLAG == ids::MU_SOURCE_FLAG {
        let unit_id = source & ids::MU_ID_MASK;
        return doc.units.iter().any(|unit| unit.id == unit_id);
    }
    if source & ids::MULTIVIEW_BASE == ids::MULTIVIEW_BASE {
        return doc
            .multiviews
            .iter()
            .any(|layout| ids::multiview_gpu_id(layout.id) == source);
    }
    if source & ids::SCENE_BASE == ids::SCENE_BASE {
        return doc
            .scenes
            .iter()
            .any(|scene| ids::scene_gpu_id(scene.id) == source);
    }
    doc.inputs.iter().any(|input| input.id == source)
}

#[inline(never)]
pub fn apply_one<P: crate::port::MixerPort + ?Sized>(
    port: &mut P,
    next: &Document,
    op: &ReconcileOp,
    statuses: &mut Vec<crate::live::ResourceStatus>,
) -> crate::error::ControlResult<()> {
    match op {
        ReconcileOp::Settings => apply_settings(port, next),
        ReconcileOp::CreateUnit { .. }
        | ReconcileOp::ConfigureUnit { .. }
        | ReconcileOp::DestroyUnit { .. }
        | ReconcileOp::UpsertBus(_)
        | ReconcileOp::RemoveBus { .. } => apply_units(port, op),
        ReconcileOp::DefineGenerator(_)
        | ReconcileOp::DefineMixInput(_)
        | ReconcileOp::LoadStill { .. }
        | ReconcileOp::StartVideo(_)
        | ReconcileOp::ConnectOmt(_)
        | ReconcileOp::ConnectNdi(_)
        | ReconcileOp::StartAudioCapture(_)
        | ReconcileOp::DestroySource { .. }
        | ReconcileOp::FailInput { .. } => apply_inputs(port, op, statuses),
        ReconcileOp::DefineScene(_)
        | ReconcileOp::DestroyScene { .. }
        | ReconcileOp::DefineMultiview { .. } => apply_scenes(port, next, op),
        _ => apply_live(port, op, statuses),
    }
}

#[inline(never)]
fn apply_units<P: crate::port::MixerPort + ?Sized>(
    port: &mut P,
    op: &ReconcileOp,
) -> crate::error::ControlResult<()> {
    match op {
        ReconcileOp::CreateUnit { id, width, height } => port.create_unit(*id, *width, *height),
        ReconcileOp::ConfigureUnit {
            id,
            width,
            height,
            fps_num,
            fps_den,
        } => port.configure_unit(*id, *width, *height, *fps_num, *fps_den),
        ReconcileOp::DestroyUnit { id } => port.destroy_unit(*id),
        ReconcileOp::UpsertBus(spec) => port.audio_bus_upsert(spec.clone()),
        ReconcileOp::RemoveBus { id } => port.audio_bus_remove(*id),
        _ => Ok(()),
    }
}

#[inline(never)]
fn apply_inputs<P: crate::port::MixerPort + ?Sized>(
    port: &mut P,
    op: &ReconcileOp,
    statuses: &mut Vec<crate::live::ResourceStatus>,
) -> crate::error::ControlResult<()> {
    match op {
        ReconcileOp::DefineGenerator(spec) => accept_io(
            port.define_generator(spec.clone()),
            statuses,
            crate::ids::ResourceKind::Input,
            spec.id,
        ),
        ReconcileOp::DefineMixInput(spec) => {
            port.define_mix_input(spec.clone())?;
            statuses.push(ready(crate::ids::ResourceKind::Input, spec.id));
            Ok(())
        }
        ReconcileOp::LoadStill { id, path } => accept_io(
            port.load_still(*id, path),
            statuses,
            crate::ids::ResourceKind::Input,
            *id,
        ),
        ReconcileOp::StartVideo(spec) => accept_io(
            port.video_start(spec.clone()),
            statuses,
            crate::ids::ResourceKind::Input,
            spec.id,
        ),
        ReconcileOp::ConnectOmt(spec) => accept_io(
            port.omt_connect(spec.clone()),
            statuses,
            crate::ids::ResourceKind::Input,
            spec.id,
        ),
        ReconcileOp::ConnectNdi(spec) => accept_io(
            port.ndi_connect(spec.clone()),
            statuses,
            crate::ids::ResourceKind::Input,
            spec.id,
        ),
        ReconcileOp::StartAudioCapture(spec) => accept_io(
            port.audio_capture_start(spec.clone()),
            statuses,
            crate::ids::ResourceKind::Input,
            spec.id,
        ),
        ReconcileOp::DestroySource { id } => port.destroy_source(*id),
        ReconcileOp::FailInput { id, message } => {
            statuses.push(failed(crate::ids::ResourceKind::Input, *id, message));
            Ok(())
        }
        _ => Ok(()),
    }
}

#[inline(never)]
fn apply_scenes<P: crate::port::MixerPort + ?Sized>(
    port: &mut P,
    next: &Document,
    op: &ReconcileOp,
) -> crate::error::ControlResult<()> {
    match op {
        ReconcileOp::DefineScene(spec) => port.define_scene(spec.clone()),
        ReconcileOp::DestroyScene { id } => port.destroy_scene(*id),
        ReconcileOp::DefineMultiview {
            spec,
            preview_unit,
            program_unit,
        } => apply_multiview(port, next, spec, *preview_unit, *program_unit),
        _ => Ok(()),
    }
}

#[inline(never)]
fn apply_live<P: crate::port::MixerPort + ?Sized>(
    port: &mut P,
    op: &ReconcileOp,
    statuses: &mut Vec<crate::live::ResourceStatus>,
) -> crate::error::ControlResult<()> {
    match op {
        ReconcileOp::SetLiveState {
            unit_id,
            program,
            preview,
        } => port.unit_set_state(*unit_id, *program, *preview, 0.0),
        ReconcileOp::AddOutput(spec) => accept_io(
            port.output_add(spec.clone()),
            statuses,
            crate::ids::ResourceKind::Output,
            spec.id,
        ),
        ReconcileOp::RemoveOutput { id } => port.output_remove(*id),
        ReconcileOp::AudioInput {
            id,
            bus_mask,
            gain,
            mute,
        } => port.audio_set_input(*id, *bus_mask, *gain, *mute),
        ReconcileOp::AudioUnitLink {
            unit_id,
            bus_id,
            mode,
        } => port.audio_set_unit_link(*unit_id, *bus_id, *mode),
        ReconcileOp::HeadphoneCopyMaster { enabled } => {
            port.audio_set_headphone_copy_master(*enabled)
        }
        ReconcileOp::ConfigureVmixApi {
            http_enabled,
            tcp_enabled,
            port: api_port,
            user,
            pass,
        } => port.configure_vmix_api(*http_enabled, *tcp_enabled, *api_port, user, pass),
        ReconcileOp::ConfigureNativeApi {
            enabled,
            port: ws_port,
        } => port.configure_native_api(*enabled, *ws_port),
        _ => Ok(()),
    }
}

#[inline(never)]
fn apply_settings<P: crate::port::MixerPort + ?Sized>(
    port: &mut P,
    next: &Document,
) -> crate::error::ControlResult<()> {
    port.set_master_fps(next.settings.master_fps_num, next.settings.master_fps_den)?;
    port.set_frame_buffer(next.settings.frame_buffer_frames)?;
    port.set_rebar_optimization(next.settings.rebar_optimization)?;
    port.set_ndi_gpu_upload(next.settings.ndi_gpu_upload)?;
    port.set_bus_colors(
        [
            next.settings.preview_color.r,
            next.settings.preview_color.g,
            next.settings.preview_color.b,
        ],
        [
            next.settings.program_color.r,
            next.settings.program_color.g,
            next.settings.program_color.b,
        ],
        [
            next.settings.inactive_color.r,
            next.settings.inactive_color.g,
            next.settings.inactive_color.b,
        ],
    )
}

#[inline(never)]
fn apply_multiview<P: crate::port::MixerPort + ?Sized>(
    port: &mut P,
    next: &Document,
    spec: &crate::port::SceneApply,
    preview_unit: u64,
    program_unit: u64,
) -> crate::error::ControlResult<()> {
    port.define_scene(spec.clone())?;
    port.bind_multiview(spec.id, preview_unit, program_unit)?;
    let percent = matches!(
        next.settings.multiview_label_unit,
        crate::session::MvLabelUnit::Percent
    );
    let top = matches!(
        next.settings.multiview_label_anchor,
        crate::session::MvLabelAnchor::Top
    );
    port.set_mv_label(spec.id, next.settings.multiview_label_size, percent, top)
}

fn accept_io(
    result: crate::error::ControlResult<()>,
    statuses: &mut Vec<crate::live::ResourceStatus>,
    kind: crate::ids::ResourceKind,
    id: u64,
) -> crate::error::ControlResult<()> {
    match result {
        Ok(()) => {
            statuses.push(ready(kind, id));
            Ok(())
        }
        Err(error) if matches!(error, crate::error::ControlError::Io { .. }) => {
            statuses.push(retrying(kind, id, error.message()));
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn ready(kind: crate::ids::ResourceKind, id: u64) -> crate::live::ResourceStatus {
    crate::live::ResourceStatus {
        kind,
        id,
        phase: crate::live::ResourcePhase::Ready,
        message: String::new(),
    }
}

fn retrying(kind: crate::ids::ResourceKind, id: u64, message: &str) -> crate::live::ResourceStatus {
    crate::live::ResourceStatus {
        kind,
        id,
        phase: crate::live::ResourcePhase::Retrying,
        message: message.to_string(),
    }
}

fn failed(kind: crate::ids::ResourceKind, id: u64, message: &str) -> crate::live::ResourceStatus {
    crate::live::ResourceStatus {
        kind,
        id,
        phase: crate::live::ResourcePhase::Failed,
        message: message.to_string(),
    }
}

fn input_ops(input: &InputDto) -> Vec<ReconcileOp> {
    match input.kind {
        InputKind::Color | InputKind::Bars | InputKind::Black => {
            vec![ReconcileOp::DefineGenerator(GeneratorApply {
                id: input.id,
                bars: input.kind == InputKind::Bars,
                r: input.color_r,
                g: input.color_g,
                b: input.color_b,
                scroll: input.scroll,
                tone_hz: input.tone_hz,
                tone_level_dbfs: input.tone_level_dbfs,
            })]
        }
        InputKind::Still => vec![match crate::session::missing_media_message(input) {
            Some(message) => ReconcileOp::FailInput {
                id: input.id,
                message,
            },
            None => ReconcileOp::LoadStill {
                id: input.id,
                path: input.path_or_address.clone().unwrap_or_default(),
            },
        }],
        InputKind::Video => vec![match crate::session::missing_media_message(input) {
            Some(message) => ReconcileOp::FailInput {
                id: input.id,
                message,
            },
            None => ReconcileOp::StartVideo(VideoStartApply {
                id: input.id,
                path: input.path_or_address.clone().unwrap_or_default(),
                capture: false,
                loop_playback: input.video_loop,
                playing: matches!(
                    input.video_play_when,
                    VideoPlayWhen::Never | VideoPlayWhen::Always
                ),
                width: input.capture_width,
                height: input.capture_height,
                fps_num: input.capture_fps_num,
                fps_den: input.capture_fps_den,
                frame_buffer_frames: input.frame_buffer_frames,
                position_hns: 0,
            }),
        }],
        InputKind::UVC => vec![ReconcileOp::StartVideo(VideoStartApply {
            id: input.id,
            path: input.path_or_address.clone().unwrap_or_default(),
            capture: true,
            loop_playback: input.video_loop,
            playing: matches!(
                input.video_play_when,
                VideoPlayWhen::Never | VideoPlayWhen::Always
            ),
            width: input.capture_width,
            height: input.capture_height,
            fps_num: input.capture_fps_num,
            fps_den: input.capture_fps_den,
            frame_buffer_frames: input.frame_buffer_frames,
            position_hns: 0,
        })],
        InputKind::OMT => vec![ReconcileOp::ConnectOmt(live_connect(input))],
        InputKind::NDI => vec![ReconcileOp::ConnectNdi(live_connect(input))],
        InputKind::Mix => {
            let mut target_id = input.mix_target_id;
            let source_kind = match input.mix_source {
                MixSource::MuPreview => 1,
                MixSource::MuProgram => 2,
                MixSource::SessionMultiview => 3,
            };
            if input.mix_source == MixSource::SessionMultiview
                && target_id != 0
                && target_id < ids::MULTIVIEW_BASE
            {
                target_id = ids::multiview_gpu_id(target_id);
            }
            vec![ReconcileOp::DefineMixInput(MixInputApply {
                id: input.id,
                target_id,
                source_kind,
                delay: input.frame_buffer_frames,
                audio_bus_id: input.mix_audio_bus_id,
            })]
        }
        InputKind::Audio => vec![ReconcileOp::StartAudioCapture(AudioCaptureApply {
            id: input.id,
            kind: match input.audio_device_kind {
                AudioDeviceKind::Wasapi => 1,
                AudioDeviceKind::Asio => 2,
                AudioDeviceKind::CoreAudio => 3,
                AudioDeviceKind::None => 0,
            },
            device_id: if input.audio_device_id.is_empty() {
                input.path_or_address.clone().unwrap_or_default()
            } else {
                input.audio_device_id.clone()
            },
            mode: match input.audio_capture_mode {
                AudioCaptureMode::Mic => 0,
                AudioCaptureMode::EndpointLoopback => 1,
                AudioCaptureMode::ProcessLoopback => 2,
            },
            map_left: input.audio_map_left,
            map_right: input.audio_map_right,
            process_exe: input.audio_process_exe.clone(),
            process_aumid: input.audio_process_aumid.clone(),
        })],
    }
}

pub(crate) fn omt_quality_to_abi(quality: OmtQuality) -> u32 {
    match quality {
        OmtQuality::Default => 0,
        OmtQuality::Low => 1,
        OmtQuality::Medium => 50,
        OmtQuality::High => 100,
    }
}

fn live_connect(input: &InputDto) -> LiveConnectApply {
    LiveConnectApply {
        id: input.id,
        address: input.path_or_address.clone().unwrap_or_default(),
        use_gpu: input.use_gpu,
        frame_buffer_frames: input.frame_buffer_frames,
        quality_or_bandwidth: match input.kind {
            InputKind::NDI => {
                if input.ndi_bandwidth == NdiBandwidth::Lowest {
                    1
                } else {
                    0
                }
            }
            _ => omt_quality_to_abi(input.omt_quality),
        },
        save_mode: match input.bandwidth_save {
            BandwidthSave::AlwaysLow => 0,
            BandwidthSave::NotOnProgram => 1,
            BandwidthSave::NotOnPreviewOrProgram => 2,
            BandwidthSave::AlwaysFull => 3,
        },
        keep_full_on_multiview: input.keep_full_on_multiview,
    }
}

fn scene_apply(scene: &SceneDto, width: u32, height: u32) -> SceneApply {
    SceneApply {
        id: ids::scene_gpu_id(scene.id),
        width,
        height,
        layers: scene
            .layers
            .iter()
            .map(|layer| OverlayLayer {
                source_id: layer.input_id,
                x: layer.x,
                y: layer.y,
                width: layer.width,
                height: layer.height,
                crop_x: layer.crop_x,
                crop_y: layer.crop_y,
                crop_width: layer.crop_width,
                crop_height: layer.crop_height,
                opacity: layer.opacity,
                z: layer.z,
                audio_follow: layer.audio_follow,
                hidden: layer.hidden,
                label: String::new(),
            })
            .collect(),
    }
}

fn encode_slot(kind: MvSlotKind, source_id: u64) -> u64 {
    match kind {
        MvSlotKind::None => 0,
        MvSlotKind::Input => source_id,
        MvSlotKind::Scene => {
            if source_id < ids::SCENE_BASE {
                ids::scene_gpu_id(source_id)
            } else {
                source_id
            }
        }
        MvSlotKind::MuPreview => ids::mu_preview(source_id),
        MvSlotKind::MuProgram => ids::mu_program(source_id),
    }
}

fn tile_label(layout: &MultiviewDto, index: usize, doc: &Document) -> String {
    let Some(tile) = layout.tiles.get(index) else {
        return String::new();
    };
    if !tile.label_follow {
        return tile.label.clone();
    }
    match tile.kind {
        MvSlotKind::Input => doc
            .inputs
            .iter()
            .find(|item| item.id == tile.source_id)
            .map(|item| item.name.clone())
            .unwrap_or_default(),
        MvSlotKind::Scene => doc
            .scenes
            .iter()
            .find(|item| ids::scene_gpu_id(item.id) == tile.source_id || item.id == tile.source_id)
            .map(|item| item.name.clone())
            .unwrap_or_default(),
        MvSlotKind::MuPreview => format!(
            "PRV  {}",
            doc.units
                .iter()
                .find(|item| item.id == tile.source_id)
                .map(|item| item.name.as_str())
                .unwrap_or("")
        ),
        MvSlotKind::MuProgram => format!(
            "PGM  {}",
            doc.units
                .iter()
                .find(|item| item.id == tile.source_id)
                .map(|item| item.name.as_str())
                .unwrap_or("")
        ),
        MvSlotKind::None => String::new(),
    }
}

fn multiview_op(layout: &MultiviewDto, doc: &Document, width: u32, height: u32) -> ReconcileOp {
    let panes: Vec<MultiviewPane> = layout.template.panes();
    let layers = panes
        .iter()
        .enumerate()
        .map(|(index, pane)| {
            let tile = layout.tiles.get(index);
            OverlayLayer {
                source_id: tile
                    .map(|tile| encode_slot(tile.kind, tile.source_id))
                    .unwrap_or(0),
                x: pane.x,
                y: pane.y,
                width: pane.width,
                height: pane.height,
                crop_x: 0.0,
                crop_y: 0.0,
                crop_width: 1.0,
                crop_height: 1.0,
                opacity: 1.0,
                z: index as i32,
                audio_follow: false,
                hidden: false,
                label: tile_label(layout, index, doc),
            }
        })
        .collect();
    let preview = layout
        .tiles
        .iter()
        .find(|tile| tile.kind == MvSlotKind::MuPreview)
        .map(|tile| tile.source_id)
        .unwrap_or(layout.preview_unit_id)
        .max(1);
    let program = layout
        .tiles
        .iter()
        .find(|tile| tile.kind == MvSlotKind::MuProgram)
        .map(|tile| tile.source_id)
        .unwrap_or(layout.program_unit_id)
        .max(1);
    ReconcileOp::DefineMultiview {
        spec: SceneApply {
            id: ids::multiview_gpu_id(layout.id),
            width,
            height,
            layers,
        },
        preview_unit: preview,
        program_unit: program,
    }
}

fn output_apply(output: &OutputDto) -> OutputApply {
    let mut source_id = output.source_id;
    let source_kind = match output.source_kind {
        OutputSourceKind::Scene => 0,
        OutputSourceKind::MuPreview => 1,
        OutputSourceKind::MuProgram => 2,
        OutputSourceKind::Multiview => 3,
        OutputSourceKind::Input => 4,
    };
    if output.source_kind == OutputSourceKind::Multiview
        && source_id != 0
        && source_id < ids::MULTIVIEW_BASE
    {
        source_id = ids::multiview_gpu_id(source_id);
    } else if output.source_kind == OutputSourceKind::Scene
        && source_id != 0
        && source_id < ids::SCENE_BASE
    {
        source_id = ids::scene_gpu_id(source_id);
    }
    OutputApply {
        id: output.id,
        transport: match output.transport {
            OutputTransport::Omt => 0,
            OutputTransport::Ndi => 1,
            OutputTransport::DeckLink => 2,
        },
        name: output.name.clone(),
        source_kind,
        source_id,
        unit_id: output.unit_id,
        use_gpu: output.use_gpu,
        audio_bus_id: if output.source_kind == OutputSourceKind::Multiview {
            0
        } else {
            output.audio_bus_id
        },
        skip_encode_when_no_receivers: output.skip_encode_when_no_receivers,
        width: output.width,
        height: output.height,
        fps_num: output.fps_num,
        fps_den: output.fps_den,
    }
}

fn input_desired_equal(a: &InputDto, b: &InputDto) -> bool {
    a.kind == b.kind
        && a.path_or_address == b.path_or_address
        && a.color_r == b.color_r
        && a.color_g == b.color_g
        && a.color_b == b.color_b
        && a.scroll == b.scroll
        && a.use_gpu == b.use_gpu
        && a.frame_buffer_frames == b.frame_buffer_frames
        && a.mix_target_id == b.mix_target_id
        && a.mix_source == b.mix_source
        && a.capture_width == b.capture_width
        && a.capture_height == b.capture_height
        && a.capture_fps_num == b.capture_fps_num
        && a.capture_fps_den == b.capture_fps_den
}

fn output_equal(a: &OutputDto, b: &OutputDto) -> bool {
    a.transport == b.transport
        && a.name == b.name
        && a.source_kind == b.source_kind
        && a.source_id == b.source_id
        && a.unit_id == b.unit_id
        && a.use_gpu == b.use_gpu
        && a.enabled == b.enabled
        && a.audio_bus_id == b.audio_bus_id
        && a.skip_encode_when_no_receivers == b.skip_encode_when_no_receivers
        && a.width == b.width
        && a.height == b.height
        && a.fps_num == b.fps_num
        && a.fps_den == b.fps_den
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::parse;

    #[test]
    fn cold_apply_creates_graph_in_order() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Bars", "kind": "Bars" }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let doc = parse(src).unwrap();
        let ops = plan(None, &doc);
        assert!(matches!(ops.first(), Some(ReconcileOp::Settings)));
        assert!(
            ops.iter()
                .any(|op| matches!(op, ReconcileOp::CreateUnit { id: 1, .. }))
        );
        assert!(
            ops.iter()
                .any(|op| matches!(op, ReconcileOp::DefineGenerator(_)))
        );
        assert!(
            ops.iter()
                .any(|op| matches!(op, ReconcileOp::DefineScene(_)))
        );
        assert!(
            ops.iter()
                .any(|op| matches!(op, ReconcileOp::SetLiveState { .. }))
        );
    }

    #[test]
    fn cold_apply_uses_stored_preview_program() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Bars", "kind": "Bars" }],
          "scenes": [
            { "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] },
            { "id": 2, "name": "Scene 2", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }
          ],
          "units": [{ "id": 1, "name": "MU 1", "previewSceneId": 2, "programSceneId": 1 }]
        }"#;
        let doc = parse(src).unwrap();
        let ops = plan(None, &doc);
        assert!(ops.iter().any(|op| matches!(
            op,
            ReconcileOp::SetLiveState {
                unit_id: 1,
                program,
                preview,
            } if *program == ids::scene_gpu_id(1) && *preview == ids::scene_gpu_id(2)
        )));
    }

    #[test]
    fn reapply_same_document_skips_input_restart() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Bars", "kind": "Bars" }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let doc = parse(src).unwrap();
        let ops = plan(Some(&doc), &doc);
        assert!(
            !ops.iter()
                .any(|op| matches!(op, ReconcileOp::DefineGenerator(_)))
        );
        assert!(
            !ops.iter()
                .any(|op| matches!(op, ReconcileOp::DestroySource { .. }))
        );
        assert!(
            !ops.iter()
                .any(|op| matches!(op, ReconcileOp::SetLiveState { .. }))
        );
        assert!(
            !ops.iter()
                .any(|op| matches!(op, ReconcileOp::StartVideo(_)))
        );
        assert!(
            !ops.iter()
                .any(|op| matches!(op, ReconcileOp::DefineScene(_)))
        );
        assert!(
            !ops.iter()
                .any(|op| matches!(op, ReconcileOp::ConfigureUnit { .. }))
        );
    }

    #[test]
    fn settings_only_replace_keeps_live_and_media() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 10, "name": "Clip", "kind": "Video", "pathOrAddress": "clip.mp4" }],
          "scenes": [
            { "id": 1, "name": "Scene 1", "layers": [{ "inputId": 10, "width": 1, "height": 1 }] },
            { "id": 2, "name": "Scene 2", "layers": [{ "inputId": 10, "width": 1, "height": 1 }] }
          ],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let doc = parse(src).unwrap();
        let mut next = doc.clone();
        next.settings.vmix_api_port = 9099;
        let ops = plan(Some(&doc), &next);
        assert!(
            !ops.iter()
                .any(|op| matches!(op, ReconcileOp::SetLiveState { .. }))
        );
        assert!(
            !ops.iter()
                .any(|op| matches!(op, ReconcileOp::StartVideo(_)))
        );
        assert!(
            !ops.iter()
                .any(|op| matches!(op, ReconcileOp::DestroySource { .. }))
        );
        assert!(
            ops.iter()
                .any(|op| matches!(op, ReconcileOp::ConfigureVmixApi { port: 9099, .. }))
        );
    }

    #[test]
    fn missing_still_is_failed_not_loaded() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Card", "kind": "Still", "pathOrAddress": "/no/such/card.png" }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let doc = parse(src).unwrap();
        let ops = plan(None, &doc);
        assert!(
            ops.iter()
                .any(|op| matches!(op, ReconcileOp::FailInput { id: 2, .. }))
        );
        assert!(
            !ops.iter()
                .any(|op| matches!(op, ReconcileOp::LoadStill { .. }))
        );
    }

    #[test]
    fn mix_session_multiview_promotes_layout_id() {
        let src = br#"{
          "version": 2,
          "inputs": [
            { "id": 2, "name": "Bars", "kind": "Bars" },
            { "id": 20, "name": "MV", "kind": "Mix", "mixSource": "SessionMultiview", "mixTargetId": 1 }
          ],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 20, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }],
          "multiviews": [{ "id": 1, "name": "MV 1" }]
        }"#;
        let doc = parse(src).unwrap();
        let ops = plan(None, &doc);
        assert!(ops.iter().any(|op| matches!(
            op,
            ReconcileOp::DefineMixInput(spec)
                if spec.id == 20
                    && spec.source_kind == 3
                    && spec.target_id == ids::multiview_gpu_id(1)
        )));
    }

    #[test]
    fn omt_quality_maps_to_mixer_abi_values() {
        assert_eq!(omt_quality_to_abi(OmtQuality::Default), 0);
        assert_eq!(omt_quality_to_abi(OmtQuality::Low), 1);
        assert_eq!(omt_quality_to_abi(OmtQuality::Medium), 50);
        assert_eq!(omt_quality_to_abi(OmtQuality::High), 100);
        for (label, expected) in [("Default", 0u32), ("Low", 1), ("Medium", 50), ("High", 100)] {
            let src = format!(
                r#"{{
                  "version": 2,
                  "inputs": [{{
                    "id": 10,
                    "name": "Cam",
                    "kind": "Omt",
                    "pathOrAddress": "omt://studio/cam",
                    "omtQuality": "{label}"
                  }}],
                  "scenes": [{{ "id": 1, "name": "Scene 1", "layers": [{{ "inputId": 10, "width": 1, "height": 1 }}] }}],
                  "units": [{{ "id": 1, "name": "MU 1" }}]
                }}"#
            );
            let doc = parse(src.as_bytes()).unwrap();
            let ops = plan(None, &doc);
            assert!(
                ops.iter().any(|op| matches!(
                    op,
                    ReconcileOp::ConnectOmt(spec)
                        if spec.id == 10 && spec.quality_or_bandwidth == expected
                )),
                "omtQuality {label} should map to ABI {expected}"
            );
        }
    }

    #[test]
    fn repair_live_retargets_deleted_program_scene() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Bars", "kind": "Bars" }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let doc = parse(src).unwrap();
        let mut live = crate::live::LiveState::default();
        live.units.insert(
            1,
            crate::live::UnitLiveState {
                program_source: ids::scene_gpu_id(2),
                preview_source: ids::scene_gpu_id(1),
                ..crate::live::UnitLiveState::default()
            },
        );
        let ops = repair_live(&doc, &live);
        assert!(ops.iter().any(|op| matches!(
            op,
            ReconcileOp::SetLiveState {
                unit_id: 1,
                program,
                preview
            } if *program == ids::scene_gpu_id(1) && *preview == ids::scene_gpu_id(1)
        )));
    }
}
