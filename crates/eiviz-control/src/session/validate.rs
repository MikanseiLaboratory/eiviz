use crate::session::replay::{AngleSourceKind, SEGMENT_FRAMES_MAX, SEGMENT_FRAMES_MIN};
use crate::session::{
    AudioCaptureMode, Document, InputKind, MixSource, OutputSourceKind, OutputTransport,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub message: String,
}

impl ValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub fn validate(doc: &Document) -> Result<(), ValidationError> {
    if doc.version != 2 {
        return Err(ValidationError::new(format!(
            "unsupported session version {}",
            doc.version
        )));
    }
    if doc.scenes.is_empty() {
        return Err(ValidationError::new("session needs at least one Scene"));
    }
    if doc.units.is_empty() {
        return Err(ValidationError::new(
            "session needs at least one Mixing Unit",
        ));
    }
    unique_ids(doc.inputs.iter().map(|item| item.id), "input")?;
    unique_ids(doc.scenes.iter().map(|item| item.id), "scene")?;
    unique_ids(doc.units.iter().map(|item| item.id), "unit")?;
    unique_ids(doc.outputs.iter().map(|item| item.id), "output")?;
    unique_ids(doc.multiviews.iter().map(|item| item.id), "multiview")?;
    validate_replay(doc)?;
    unique_guids(doc.inputs.iter().map(|item| item.guid.as_str()), "input")?;
    unique_guids(doc.scenes.iter().map(|item| item.guid.as_str()), "scene")?;

    let input_ids: Vec<u64> = doc.inputs.iter().map(|item| item.id).collect();
    let unit_ids: Vec<u64> = doc.units.iter().map(|item| item.id).collect();
    let scene_ids: Vec<u64> = doc.scenes.iter().map(|item| item.id).collect();
    let mv_ids: Vec<u64> = doc.multiviews.iter().map(|item| item.id).collect();

    for unit in &doc.units {
        if unit.preview_scene_id != 0 && !scene_ids.contains(&unit.preview_scene_id) {
            return Err(ValidationError::new(format!(
                "mixing unit {} preview references missing scene {}",
                unit.id, unit.preview_scene_id
            )));
        }
        if unit.program_scene_id != 0 && !scene_ids.contains(&unit.program_scene_id) {
            return Err(ValidationError::new(format!(
                "mixing unit {} program references missing scene {}",
                unit.id, unit.program_scene_id
            )));
        }
        if !valid_rate(unit.fps_num, unit.fps_den) {
            return Err(ValidationError::new(format!(
                "mixing unit {} frame rate {}/{} is invalid",
                unit.id, unit.fps_num, unit.fps_den
            )));
        }
    }

    for scene in &doc.scenes {
        for layer in &scene.layers {
            if !input_ids.contains(&layer.input_id) {
                return Err(ValidationError::new(format!(
                    "scene {} references missing input {}",
                    scene.id, layer.input_id
                )));
            }
            if doc
                .inputs
                .iter()
                .find(|input| input.id == layer.input_id)
                .is_some_and(|input| !input.kind.has_video())
            {
                return Err(ValidationError::new(format!(
                    "scene {} cannot place audio-only input {}",
                    scene.id, layer.input_id
                )));
            }
        }
    }
    for input in &doc.inputs {
        if input.kind != InputKind::Audio {
            continue;
        }
        if input.audio_capture_mode == AudioCaptureMode::ProcessLoopback
            && input.audio_process_exe.trim().is_empty()
            && input.audio_process_aumid.trim().is_empty()
        {
            return Err(ValidationError::new(format!(
                "audio input {} needs a process exe or AUMID",
                input.id
            )));
        }
        if input.audio_device_kind == crate::session::AudioDeviceKind::Asio
            && input.audio_capture_mode == AudioCaptureMode::ProcessLoopback
        {
            return Err(ValidationError::new(format!(
                "audio input {} cannot use ASIO with process loopback",
                input.id
            )));
        }
        if input.audio_device_kind == crate::session::AudioDeviceKind::Asio
            && (input.audio_map_left < 0 || input.audio_map_right < 0)
        {
            return Err(ValidationError::new(format!(
                "audio input {} ASIO L/R map is invalid",
                input.id
            )));
        }
    }
    for input in &doc.inputs {
        if input.kind != InputKind::Mix {
            continue;
        }
        if input.mix_target_id == 0 {
            return Err(ValidationError::new(format!(
                "mix input {} has no target",
                input.id
            )));
        }
        match input.mix_source {
            MixSource::SessionMultiview => {
                if !mv_ids.iter().any(|&id| {
                    id == input.mix_target_id
                        || crate::ids::multiview_gpu_id(id) == input.mix_target_id
                }) {
                    return Err(ValidationError::new(format!(
                        "mix input {} references missing multiview {}",
                        input.id, input.mix_target_id
                    )));
                }
            }
            MixSource::MuProgram | MixSource::MuPreview => {
                if !unit_ids.contains(&input.mix_target_id) {
                    return Err(ValidationError::new(format!(
                        "mix input {} references missing unit {}",
                        input.id, input.mix_target_id
                    )));
                }
            }
        }
    }
    detect_mix_cycles(doc)?;
    for output in &doc.outputs {
        match output.source_kind {
            OutputSourceKind::Scene
                if output.source_id != 0
                    && !scene_ids.contains(&output.source_id)
                    && output.source_id & crate::ids::SCENE_BASE == 0 =>
            {
                // hosts may store either raw id or gpu id; accept both
            }
            OutputSourceKind::MuPreview | OutputSourceKind::MuProgram => {
                if output.unit_id != 0 && !unit_ids.contains(&output.unit_id) {
                    return Err(ValidationError::new(format!(
                        "output {} references missing unit {}",
                        output.id, output.unit_id
                    )));
                }
            }
            _ => {}
        }
        if output.transport == OutputTransport::DeckLink
            || output.transport == OutputTransport::Rtmp
        {
            // The mixer accepts or rejects the resource. A plan error does not abort the session.
        }
        if output.audio_unit_id != 0 && !unit_ids.contains(&output.audio_unit_id) {
            return Err(ValidationError::new(format!(
                "output {} references missing mixing unit {}",
                output.id, output.audio_unit_id
            )));
        }
        if (output.width == 0) != (output.height == 0) {
            return Err(ValidationError::new(format!(
                "output {} size must set both width and height, or neither",
                output.id
            )));
        }
        if output.width != 0 {
            if output.width < 16 || output.height < 16 || output.width % 2 != 0 {
                return Err(ValidationError::new(format!(
                    "output {} size {}x{} is invalid",
                    output.id, output.width, output.height
                )));
            }
        }
        if (output.fps_num == 0) != (output.fps_den == 0) {
            return Err(ValidationError::new(format!(
                "output {} frame rate must set both fps_num and fps_den, or neither",
                output.id
            )));
        }
        if output.fps_num > 0 && !valid_rate(output.fps_num, output.fps_den) {
            return Err(ValidationError::new(format!(
                "output {} frame rate {}/{} is invalid",
                output.id, output.fps_num, output.fps_den
            )));
        }
    }
    if !valid_rate(doc.settings.master_fps_num, doc.settings.master_fps_den) {
        return Err(ValidationError::new("master fps is invalid"));
    }
    for preset in &doc.transitions {
        check_curve(preset.easing, preset.bezier.as_ref(), false, "transition")?;
    }
    for overlay in &doc.overlays {
        check_curve(overlay.easing, overlay.bezier.as_ref(), false, "overlay")?;
    }
    for scene in &doc.scenes {
        let layer_ids: Vec<u64> = scene.layers.iter().map(|layer| layer.layer_id).collect();
        let state_ids: Vec<u64> = scene.states.iter().map(|state| state.id).collect();
        check_camera(&scene.camera, &format!("scene {}", scene.id))?;
        for state in &scene.states {
            if state.id == 0 {
                return Err(ValidationError::new(format!(
                    "scene {} state id 0 is reserved for the saved layout",
                    scene.id
                )));
            }
            for key in &state.layers {
                if !layer_ids.contains(&key.layer_id) {
                    return Err(ValidationError::new(format!(
                        "scene {} state {} references missing layer {}",
                        scene.id, state.id, key.layer_id
                    )));
                }
            }
            check_curve(
                state.enter.easing,
                state.enter.bezier.as_ref(),
                true,
                "state",
            )?;
            if state.enter.duration_frames == 0 {
                return Err(ValidationError::new(format!(
                    "scene {} state {} duration is zero",
                    scene.id, state.id
                )));
            }
            if let Some(camera) = &state.camera {
                check_camera(camera, &format!("scene {} state {}", scene.id, state.id))?;
            }
        }
        for seq in &scene.sequences {
            for step in &seq.steps {
                if !state_ids.contains(&step.state_id) {
                    return Err(ValidationError::new(format!(
                        "scene {} sequence {} references missing state {}",
                        scene.id, seq.id, step.state_id
                    )));
                }
                if let Some(motion) = &step.motion {
                    check_curve(motion.easing, motion.bezier.as_ref(), true, "sequence step")?;
                    if motion.duration_frames == 0 {
                        return Err(ValidationError::new(format!(
                            "scene {} sequence {} step duration is zero",
                            scene.id, seq.id
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

fn check_camera(camera: &crate::session::SceneCamera, what: &str) -> Result<(), ValidationError> {
    let finite = camera.x.is_finite() && camera.y.is_finite() && camera.zoom.is_finite();
    if !finite {
        return Err(ValidationError::new(format!("{what} camera is not finite")));
    }
    if !(1.0..=8.0).contains(&camera.zoom) {
        return Err(ValidationError::new(format!(
            "{what} camera zoom is out of range"
        )));
    }
    let margin = 0.5 / camera.zoom;
    let inside =
        (margin..=1.0 - margin).contains(&camera.x) && (margin..=1.0 - margin).contains(&camera.y);
    if !inside {
        return Err(ValidationError::new(format!(
            "{what} camera center is out of range"
        )));
    }
    Ok(())
}

fn check_curve(
    easing: u32,
    bezier: Option<&crate::session::BezierHandles>,
    allow_hold: bool,
    what: &str,
) -> Result<(), ValidationError> {
    let max = if allow_hold { 6 } else { 5 };
    if easing > max {
        return Err(ValidationError::new(format!(
            "{what} easing {easing} is not supported"
        )));
    }
    if easing == 5 {
        let Some(handles) = bezier else {
            return Err(ValidationError::new(format!(
                "{what} bezier easing requires handles"
            )));
        };
        let finite = [handles.x1, handles.y1, handles.x2, handles.y2]
            .iter()
            .all(|value| value.is_finite());
        if !finite || !handles.x_in_range() {
            return Err(ValidationError::new(format!(
                "{what} bezier handles are outside the supported range"
            )));
        }
    }
    Ok(())
}

fn valid_rate(num: u32, den: u32) -> bool {
    if num == 0 || den == 0 || num > 240_000 || den > 100_000 {
        return false;
    }
    (u64::from(den).saturating_mul(10_000_000) / u64::from(num)) > 0
}

pub fn validate_for_apply(doc: &Document) -> Result<(), ValidationError> {
    validate(doc)
}

fn unique_ids(ids: impl Iterator<Item = u64>, kind: &str) -> Result<(), ValidationError> {
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if id == 0 {
            return Err(ValidationError::new(format!("{kind} id 0 is reserved")));
        }
        if !seen.insert(id) {
            return Err(ValidationError::new(format!("duplicate {kind} id {id}")));
        }
    }
    Ok(())
}

fn unique_guids<'a>(
    guids: impl Iterator<Item = &'a str>,
    kind: &str,
) -> Result<(), ValidationError> {
    let mut seen = std::collections::HashSet::new();
    for guid in guids {
        if guid.is_empty() {
            continue;
        }
        let key = guid.to_ascii_lowercase();
        if !seen.insert(key) {
            return Err(ValidationError::new(format!(
                "duplicate {kind} guid {guid}"
            )));
        }
    }
    Ok(())
}

fn detect_mix_cycles(doc: &Document) -> Result<(), ValidationError> {
    use std::collections::{HashMap, HashSet};
    let mut edges: HashMap<u64, u64> = HashMap::new();
    for input in &doc.inputs {
        if input.kind == InputKind::Mix
            && input.mix_source != MixSource::SessionMultiview
            && input.mix_target_id != 0
        {
            edges.insert(input.id, input.mix_target_id);
        }
    }
    for &start in edges.keys() {
        let mut seen = HashSet::new();
        let mut cur = start;
        while let Some(unit) = edges.get(&cur).copied() {
            if !seen.insert(cur) {
                return Err(ValidationError::new(format!(
                    "mix input cycle involving {start}"
                )));
            }
            // Mix edges are input -> unit. A cycle through mix inputs would
            // require a mix input id equal to a unit id, which we reject.
            if edges.contains_key(&unit) {
                cur = unit;
            } else {
                break;
            }
        }
    }
    Ok(())
}

/// Longest transition the timeline accepts. Ten seconds at 60 fps is already far
/// beyond a replay dissolve.
const MAX_TRANSITION_FRAMES: u32 = 600;

fn validate_clip_transition(
    transition: &crate::session::replay::ClipTransitionDto,
    owner: &str,
) -> Result<(), ValidationError> {
    if transition.duration_frames > MAX_TRANSITION_FRAMES {
        return Err(ValidationError::new(format!(
            "replay transition for {owner} is {} frames; the limit is {MAX_TRANSITION_FRAMES}",
            transition.duration_frames
        )));
    }
    if transition.kind == crate::session::replay::ReplayTransitionKind::Cut
        && transition.duration_frames != 0
    {
        return Err(ValidationError::new(format!(
            "replay transition for {owner} is a cut and cannot carry a duration"
        )));
    }
    Ok(())
}

fn validate_replay(doc: &Document) -> Result<(), ValidationError> {
    let replay_inputs: Vec<u64> = doc
        .inputs
        .iter()
        .filter(|input| input.kind == InputKind::Replay)
        .map(|input| input.id)
        .collect();
    let Some(replay) = doc.replay.as_ref() else {
        if !replay_inputs.is_empty() {
            return Err(ValidationError::new(
                "Replay input requires a Replay document",
            ));
        }
        return Ok(());
    };
    // Several drives may share the recording load, so one or more paths are
    // allowed. An angle that names a drive records there; the rest use the first.
    if replay.enabled {
        if replay.storage.is_empty() {
            return Err(ValidationError::new(
                "replay needs at least one storage path",
            ));
        }
        if replay
            .storage
            .iter()
            .any(|storage| storage.path.trim().is_empty())
        {
            return Err(ValidationError::new("every replay storage needs a path"));
        }
        // Recording is one format for every angle, and a recording cannot start
        // without one. Leaving these at zero used to reach the mixer as an opaque
        // refusal, so the document is refused here with the missing piece named.
        let format = &replay.format;
        if format.width == 0 || format.height == 0 {
            return Err(ValidationError::new(
                "replay format needs a resolution; apply Replay Setup",
            ));
        }
        if format.fps_num == 0 || format.fps_den == 0 {
            return Err(ValidationError::new(
                "replay format needs a frame rate; apply Replay Setup",
            ));
        }
        if !(SEGMENT_FRAMES_MIN..=SEGMENT_FRAMES_MAX).contains(&format.segment_frames) {
            return Err(ValidationError::new(format!(
                "replay segment frames must be between {SEGMENT_FRAMES_MIN} and {SEGMENT_FRAMES_MAX}"
            )));
        }
        if replay.angles.is_empty() {
            return Err(ValidationError::new(
                "replay is enabled but has no angles; add one in Replay Setup",
            ));
        }
    } else if !replay.storage.is_empty() && !replay.angles.is_empty() {
        // The flag is not a separate switch: a document with a drive and an angle
        // is a recording setup, and saying otherwise hides that from the operator.
        return Err(ValidationError::new(
            "replay has a drive and an angle, so it must be enabled",
        ));
    }
    let mut sources = std::collections::HashSet::new();
    let mut angles = std::collections::HashSet::new();
    for angle in &replay.angles {
        if angle.id == 0 || !angles.insert(angle.id) {
            return Err(ValidationError::new(format!(
                "replay angle id {} is missing or duplicated",
                angle.id
            )));
        }
        match angle.source {
            AngleSourceKind::Scene => {
                if angle.input_id != 0 {
                    return Err(ValidationError::new(format!(
                        "replay angle {} records a scene, so it cannot name input {}",
                        angle.id, angle.input_id
                    )));
                }
                if !doc.scenes.iter().any(|scene| scene.id == angle.scene_id) {
                    return Err(ValidationError::new(format!(
                        "replay angle {} scene {} does not exist",
                        angle.id, angle.scene_id
                    )));
                }
                if !sources.insert((0u8, angle.scene_id)) {
                    return Err(ValidationError::new(format!(
                        "replay angle {} records scene {}, which another angle already records",
                        angle.id, angle.scene_id
                    )));
                }
            }
            AngleSourceKind::Input => {
                if angle.scene_id != 0 {
                    return Err(ValidationError::new(format!(
                        "replay angle {} records an input, so it cannot name scene {}",
                        angle.id, angle.scene_id
                    )));
                }
                let Some(input) = doc.inputs.iter().find(|input| input.id == angle.input_id) else {
                    return Err(ValidationError::new(format!(
                        "replay angle {} input {} does not exist",
                        angle.id, angle.input_id
                    )));
                };
                if input.kind == InputKind::Replay {
                    // A Replay channel recording itself is a loop, not a camera.
                    return Err(ValidationError::new(format!(
                        "replay angle {} cannot record a Replay channel",
                        angle.id
                    )));
                }
                if !input.kind.has_video() {
                    return Err(ValidationError::new(format!(
                        "replay angle {} input {} has no picture",
                        angle.id, angle.input_id
                    )));
                }
                if !sources.insert((1u8, angle.input_id)) {
                    return Err(ValidationError::new(format!(
                        "replay angle {} records input {}, which another angle already records",
                        angle.id, angle.input_id
                    )));
                }
            }
        }
        // Choosing an audio source that is not there would record silence while
        // claiming otherwise.
        if angle.audio == crate::session::replay::AngleAudio::Unit
            && !doc.units.iter().any(|unit| unit.id == angle.audio_unit_id)
        {
            return Err(ValidationError::new(format!(
                "replay angle {} selects mixing unit {}, which does not exist",
                angle.id, angle.audio_unit_id
            )));
        }
    }
    // A clip that names another angle for its sound must name one that exists,
    // otherwise the channel would play silence with no explanation.
    for list in &replay.playlists {
        for clip in &list.clips {
            if clip.audio != crate::session::replay::ClipAudio::Angle {
                continue;
            }
            if !angles.contains(&clip.audio_angle_id) {
                return Err(ValidationError::new(format!(
                    "replay clip {} takes audio from angle {}, which does not exist",
                    clip.uid, clip.audio_angle_id
                )));
            }
        }
    }
    if replay.channels.is_empty() {
        return Err(ValidationError::new(
            "Replay document requires an explicitly added Replay input",
        ));
    }
    // An animation that cues a clip must name a channel that exists, and a clip
    // that is really on it. Cueing nothing silently would be worse than refusing
    // the session.
    for scene in &doc.scenes {
        for sequence in &scene.sequences {
            for step in &sequence.steps {
                let Some(action) = step.replay.as_ref() else {
                    continue;
                };
                let Some(channel) = replay
                    .channels
                    .iter()
                    .find(|channel| channel.index == action.channel)
                else {
                    return Err(ValidationError::new(format!(
                        "scene {} sequence {} step cues replay channel {}, which does not exist",
                        scene.id, sequence.id, action.channel
                    )));
                };
                if action.clip_uid.is_empty() {
                    continue;
                }
                let on_channel = replay
                    .playlists
                    .iter()
                    .filter(|list| Some(list.id) == channel.playlist_id)
                    .any(|list| list.clips.iter().any(|clip| clip.uid == action.clip_uid));
                if !on_channel {
                    return Err(ValidationError::new(format!(
                        "scene {} sequence {} step cues clip {}, which is not on channel {}",
                        scene.id, sequence.id, action.clip_uid, action.channel
                    )));
                }
            }
        }
    }
    // An angle can belong to only one drive, and only to drives that exist. Every
    // angle needs one: recording to a drive the operator did not choose is worse
    // than refusing the document.
    let mut claimed: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for (index, storage) in replay.storage.iter().enumerate() {
        for angle_id in &storage.angle_ids {
            if !angles.contains(angle_id) {
                return Err(ValidationError::new(format!(
                    "replay storage {index} lists angle {angle_id}, which does not exist"
                )));
            }
            if claimed.insert(*angle_id, index).is_some() {
                return Err(ValidationError::new(format!(
                    "replay angle {angle_id} is assigned to more than one storage"
                )));
            }
        }
    }
    if replay.enabled {
        for angle in &replay.angles {
            if !claimed.contains_key(&angle.id) {
                return Err(ValidationError::new(format!(
                    "replay angle {} has no storage assigned",
                    angle.id
                )));
            }
        }
    }
    if let Some(active) = replay.active_angle_id {
        if !angles.contains(&active) {
            return Err(ValidationError::new(format!(
                "replay active angle {active} does not exist"
            )));
        }
    }
    if replay.channels.len() > 26 {
        return Err(ValidationError::new("replay allows at most 26 channels"));
    }
    let mut channels = std::collections::HashSet::new();
    for channel in &replay.channels {
        if channel.index >= 26 || !channels.insert(channel.index) {
            return Err(ValidationError::new(
                "Replay channel index is missing or duplicated",
            ));
        }
        if channel.input_id == 0
            || !doc
                .inputs
                .iter()
                .any(|input| input.id == channel.input_id && input.kind == InputKind::Replay)
        {
            return Err(ValidationError::new(format!(
                "Replay channel {} has no matching Replay input",
                channel.index
            )));
        }
        if let Some(playlist_id) = channel.playlist_id {
            if !replay.playlists.iter().any(|list| list.id == playlist_id) {
                return Err(ValidationError::new(format!(
                    "Replay channel {} selects playlist {playlist_id}, which does not exist",
                    channel.index
                )));
            }
        }
    }
    // A Replay input that no channel owns is a dead input: the mixer emits no
    // source for it, so it can never record or play back.
    for input_id in &replay_inputs {
        if !replay
            .channels
            .iter()
            .any(|channel| channel.input_id == *input_id)
        {
            return Err(ValidationError::new(format!(
                "Replay input {input_id} is not assigned to a channel"
            )));
        }
    }
    let mut events = std::collections::HashSet::new();
    for event in &replay.events {
        if event.id == 0 || !events.insert(event.id) {
            return Err(ValidationError::new(format!(
                "replay event id {} is missing or duplicated",
                event.id
            )));
        }
    }
    let mut playlists = std::collections::HashSet::new();
    let mut clip_uids = std::collections::HashSet::new();
    for list in &replay.playlists {
        if list.id == 0 || !playlists.insert(list.id) {
            return Err(ValidationError::new(format!(
                "replay playlist id {} is missing or duplicated",
                list.id
            )));
        }
        validate_clip_transition(&list.default_transition, &format!("playlist {}", list.id))?;
        for clip in &list.clips {
            // Clip uid is the only stable handle a playing channel has, so it
            // has to be unique across every playlist.
            if clip.uid.trim().is_empty() || !clip_uids.insert(clip.uid.as_str()) {
                return Err(ValidationError::new(
                    "replay clip uid is empty or duplicated",
                ));
            }
            validate_clip_transition(&clip.transition, &clip.uid)?;
            match clip.source {
                crate::session::replay::ClipSourceKind::Angle => {
                    if !angles.contains(&clip.angle_id) {
                        return Err(ValidationError::new(format!(
                            "replay clip {} uses angle {}, which does not exist",
                            clip.uid, clip.angle_id
                        )));
                    }
                }
                crate::session::replay::ClipSourceKind::Asset => {
                    // Nothing plays an asset clip yet, so accepting one would
                    // silently drop it from the channel.
                    return Err(ValidationError::new(format!(
                        "replay clip {} uses an asset source, which is not implemented",
                        clip.uid
                    )));
                }
            }
            if !(0.1..=8.0).contains(&clip.speed) {
                return Err(ValidationError::new(format!(
                    "replay clip {} speed {} is outside 0.1..=8.0",
                    clip.uid, clip.speed
                )));
            }
        }
    }
    let queue = replay.buffering.encode_queue_frames;
    if queue != 0 && !(2..=60).contains(&queue) {
        return Err(ValidationError::new(
            "replay encode queue must be 2..=60 frames",
        ));
    }
    let write = replay.buffering.write_buffer_mb;
    if write != 0 && !(64..=8192).contains(&write) {
        return Err(ValidationError::new(
            "replay write buffer must be 64..=8192 MiB",
        ));
    }
    let warn = replay.warnings.remaining_warn_secs;
    let alert = replay.warnings.remaining_alert_secs;
    let critical = replay.warnings.remaining_critical_secs;
    if !(warn >= alert && alert >= critical) {
        return Err(ValidationError::new(
            "replay warning thresholds must be warn >= alert >= critical",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::parse;

    #[test]
    fn rejects_empty_graph() {
        let doc = parse(br#"{"version":2}"#).unwrap();
        assert!(validate(&doc).is_err());
    }

    #[test]
    fn accepts_minimal_bars_session() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Bars", "kind": "Bars" }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let doc = parse(src).unwrap();
        validate(&doc).unwrap();
    }

    #[test]
    fn rejects_unknown_preview_scene() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Bars", "kind": "Bars" }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1", "previewSceneId": 9 }]
        }"#;
        let doc = parse(src).unwrap();
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("preview"), "{}", err.message);
    }

    #[test]
    fn apply_keeps_still_with_missing_file() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Card", "kind": "Still", "pathOrAddress": "/no/such/card.png" }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let doc = parse(src).unwrap();
        validate(&doc).unwrap();
        validate_for_apply(&doc).unwrap();
        assert_eq!(
            doc.inputs[0].path_or_address.as_deref(),
            Some("/no/such/card.png")
        );
    }

    #[test]
    fn mix_session_multiview_accepts_layout_or_gpu_id() {
        let src = br#"{
          "version": 2,
          "inputs": [
            { "id": 2, "name": "Bars", "kind": "Bars" },
            { "id": 20, "name": "MV", "kind": "Mix", "mixSource": "SessionMultiview", "mixTargetId": 1 }
          ],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }],
          "multiviews": [{ "id": 1, "name": "MV 1" }]
        }"#;
        let doc = parse(src).unwrap();
        validate(&doc).unwrap();

        let mut gpu = doc.clone();
        gpu.inputs[1].mix_target_id = crate::ids::multiview_gpu_id(1);
        validate(&gpu).unwrap();

        let mut missing = doc;
        missing.inputs[1].mix_target_id = 9;
        let err = validate(&missing).unwrap_err();
        assert!(err.message.contains("multiview"), "{}", err.message);
    }

    #[test]
    fn mix_mu_program_rejects_gpu_multiview_id_as_unit() {
        let src = br#"{
          "version": 2,
          "inputs": [
            { "id": 2, "name": "Bars", "kind": "Bars" },
            { "id": 20, "name": "PGM", "kind": "Mix", "mixSource": "MuProgram", "mixTargetId": 131073 }
          ],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }],
          "multiviews": [{ "id": 1, "name": "MV 1" }]
        }"#;
        let doc = parse(src).unwrap();
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("unit"), "{}", err.message);
    }

    #[test]
    fn process_loopback_requires_exe_or_aumid() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "App", "kind": "Audio", "audioCaptureMode": "ProcessLoopback" }],
          "scenes": [{ "id": 1, "name": "Scene 1" }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let err = validate(&parse(src).unwrap()).unwrap_err();
        assert!(err.message.contains("process"), "{}", err.message);

        let ok = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "App", "kind": "Audio", "audioCaptureMode": "ProcessLoopback", "audioProcessExe": "Spotify.exe" }],
          "scenes": [{ "id": 1, "name": "Scene 1" }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        validate(&parse(ok).unwrap()).unwrap();
    }

    #[test]
    fn asio_accepts_independent_lr_maps() {
        let src = br#"{
          "version": 2,
          "inputs": [
            { "id": 2, "name": "Mono", "kind": "Audio", "audioDeviceKind": "Asio", "audioDeviceId": "{453661B3-88C3-45C4-8877-4C03B6490C33}", "audioMapLeft": 0, "audioMapRight": 0 },
            { "id": 3, "name": "Cross", "kind": "Audio", "audioDeviceKind": "Asio", "audioDeviceId": "{453661B3-88C3-45C4-8877-4C03B6490C33}", "audioMapLeft": 1, "audioMapRight": 2 }
          ],
          "scenes": [{ "id": 1, "name": "Scene 1" }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        validate(&parse(src).unwrap()).unwrap();
    }

    #[test]
    fn asio_rejects_process_loopback() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "ASIO", "kind": "Audio", "audioDeviceKind": "Asio", "audioCaptureMode": "ProcessLoopback", "audioProcessExe": "app.exe" }],
          "scenes": [{ "id": 1, "name": "Scene 1" }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let err = validate(&parse(src).unwrap()).unwrap_err();
        assert!(err.message.contains("ASIO"), "{}", err.message);
    }

    #[test]
    fn scene_rejects_audio_only_layer() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Mic", "kind": "Audio" }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let err = validate(&parse(src).unwrap()).unwrap_err();
        assert!(err.message.contains("audio-only"), "{}", err.message);
    }

    #[test]
    fn rejects_invalid_master_fps() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Bars", "kind": "Bars" }],
          "scenes": [{ "id": 1, "name": "Scene 1" }],
          "units": [{ "id": 1, "name": "MU 1" }],
          "settings": { "masterFpsNum": 999999, "masterFpsDen": 1 }
        }"#;
        let err = validate(&parse(src).unwrap()).unwrap_err();
        assert!(err.message.contains("master fps"), "{}", err.message);
    }

    fn session_with_camera(camera: &str, state_camera: &str) -> String {
        format!(
            r#"{{
              "version": 2,
              "inputs": [{{ "id": 2, "name": "Bars", "kind": "Bars" }}],
              "scenes": [{{ "id": 1, "name": "Scene 1", "camera": {camera},
                "layers": [{{ "inputId": 2, "layerId": 1, "width": 1, "height": 1 }}],
                "states": [{{ "id": 1, "name": "Close", "camera": {state_camera},
                  "layers": [{{ "layerId": 1, "geom": {{ "width": 1, "height": 1 }} }}] }}] }}],
              "units": [{{ "id": 1, "name": "MU 1" }}]
            }}"#
        )
    }

    #[test]
    fn camera_out_of_range_is_rejected() {
        let doc =
            parse(session_with_camera(r#"{"zoom": 0.5}"#, r#"{"zoom": 2}"#).as_bytes()).unwrap();
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("zoom"), "{}", err.message);

        let doc =
            parse(session_with_camera(r#"{"zoom": 2}"#, r#"{"x": 0.0, "zoom": 4}"#).as_bytes())
                .unwrap();
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("center"), "{}", err.message);
    }

    #[test]
    fn replay_requires_one_storage_path_when_enabled() {
        let mut doc = parse(
            br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Bars", "kind": "Bars" }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#,
        )
        .unwrap();
        // Let the document be created the way a host creates it: the channel input
        // seeds the format from the first Mixing Unit.
        crate::session::replay::add_channel_input(&mut doc).unwrap();
        doc.replay.as_mut().unwrap().enabled = true;
        assert!(validate(&doc).is_err());
        doc.replay
            .as_mut()
            .unwrap()
            .storage
            .push(crate::session::replay::ReplayStorageDto {
                path: "D:/replay".into(),
                angle_ids: vec![1],
            });
        doc.replay
            .as_mut()
            .unwrap()
            .angles
            .push(crate::session::replay::AngleDto {
                id: 1,
                name: "A".into(),
                source: crate::session::replay::AngleSourceKind::Scene,
                input_id: 0,
                scene_id: 1,
                audio: crate::session::replay::AngleAudio::Silent,
                audio_unit_id: 0,
                encoder: crate::session::replay::AngleEncoder::Gpu,
            });
        validate(&doc).unwrap();
        let bytes = crate::session::file::encode_file(&doc).unwrap();
        let back = crate::session::file::decode_file(&bytes).unwrap();
        let replay = back.replay.unwrap();
        assert!(replay.enabled);
        assert_eq!(replay.storage[0].path, "D:/replay");
        assert_eq!(replay.angles[0].scene_id, 1);
    }

    #[test]
    fn camera_round_trips_through_the_session_file() {
        let doc = parse(
            session_with_camera(r#"{"x": 0.6, "y": 0.4, "zoom": 2}"#, r#"{"zoom": 4}"#).as_bytes(),
        )
        .unwrap();
        validate(&doc).unwrap();
        let bytes = crate::session::file::encode_file(&doc).unwrap();
        let back = crate::session::file::decode_file(&bytes).unwrap();
        assert_eq!(back.scenes[0].camera.zoom, 2.0);
        assert_eq!(back.scenes[0].camera.x, 0.6);
        let state = &back.scenes[0].states[0];
        assert_eq!(state.camera.map(|camera| camera.zoom), Some(4.0));
    }

    fn session_with_replay() -> Document {
        use crate::session::replay::*;
        let mut doc = parse(
            br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Bars", "kind": "Bars" }],
          "scenes": [
            { "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] },
            { "id": 2, "name": "Scene 2", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }
          ],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#,
        )
        .unwrap();
        doc.replay = Some(ReplayDto::default());
        assert_eq!(add_channel_input(&mut doc).unwrap(), 0);
        // The fixture records like a host would: a document without a format could
        // never record, so it carries the one seeded from the Mixing Unit.
        doc.replay.as_mut().unwrap().format = seed_format(&doc).unwrap();
        let replay = doc.replay.as_mut().unwrap();
        replay.angles.push(AngleDto {
            id: 1,
            name: "A".into(),
            source: crate::session::replay::AngleSourceKind::Scene,
            input_id: 0,
            scene_id: 1,
            audio: AngleAudio::Silent,
            audio_unit_id: 0,
            encoder: crate::session::replay::AngleEncoder::Gpu,
        });
        replay.active_angle_id = Some(1);
        doc
    }

    fn replay_event(id: u64) -> crate::session::replay::ReplayEventDto {
        crate::session::replay::ReplayEventDto {
            id,
            recording_uuid: "rec".into(),
            in_frame: 10,
            out_frame: 20,
            name: format!("Replay {id}"),
            tags: Vec::new(),
            protected: false,
        }
    }

    /// A document that is ready to record: one scene angle on one drive, with the
    /// format a host seeds from its first Mixing Unit.
    fn recordable_replay() -> Document {
        use crate::session::replay::*;
        let mut doc = session_with_replay();
        {
            let replay = doc.replay.as_mut().unwrap();
            replay.enabled = true;
            replay.storage = vec![ReplayStorageDto {
                path: "D:/replay".into(),
                angle_ids: vec![1],
            }];
        }
        doc
    }

    /// Recording needs a resolution and a frame rate. A document without them used
    /// to reach the mixer, which could only answer with a bare code.
    #[test]
    fn replay_format_is_required_while_enabled() {
        use crate::session::replay::*;
        let mut doc = recordable_replay();
        doc.replay.as_mut().unwrap().format = ReplayFormatDto::default();
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("format"), "{}", err.message);
    }

    /// The segment length is the operator's setting; the recorder no longer picks
    /// one when the document says zero.
    #[test]
    fn replay_segment_frames_must_be_in_range() {
        let mut doc = recordable_replay();
        doc.replay.as_mut().unwrap().format.segment_frames = 0;
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("segment frames"), "{}", err.message);
    }

    /// Recording never guesses a drive: an angle without one is a setup error.
    #[test]
    fn every_angle_needs_a_drive() {
        let mut doc = recordable_replay();
        doc.replay.as_mut().unwrap().storage[0].angle_ids.clear();
        let err = validate(&doc).unwrap_err();
        assert!(
            err.message.contains("no storage assigned"),
            "{}",
            err.message
        );
    }

    /// A drive and an angle are a recording setup, so the flag has to say so.
    #[test]
    fn a_drive_and_an_angle_require_enabled() {
        let mut doc = recordable_replay();
        doc.replay.as_mut().unwrap().enabled = false;
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("must be enabled"), "{}", err.message);
    }

    /// An input angle records the input's own picture. A Replay channel recording
    /// itself is a loop, and an audio-only input has no picture to record.
    #[test]
    fn input_angles_reject_a_replay_channel_and_audio_only_inputs() {
        use crate::session::replay::*;
        let mut doc = recordable_replay();
        doc.inputs.push(
            serde_json::from_value(
                serde_json::json!({ "id": 10, "name": "Audio", "kind": "Audio" }),
            )
            .unwrap(),
        );
        {
            let replay = doc.replay.as_mut().unwrap();
            replay.angles[0] = AngleDto {
                id: 1,
                name: "A".into(),
                source: AngleSourceKind::Input,
                scene_id: 0,
                input_id: 524289,
                audio: AngleAudio::Silent,
                audio_unit_id: 0,
                encoder: crate::session::replay::AngleEncoder::Gpu,
            };
        }
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("Replay channel"), "{}", err.message);

        doc.replay.as_mut().unwrap().angles[0].input_id = 10;
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("no picture"), "{}", err.message);
    }

    /// Two angles recording one source would record the same picture twice and
    /// silently double the encode load.
    #[test]
    fn two_angles_cannot_record_one_source() {
        use crate::session::replay::*;
        let mut doc = recordable_replay();
        {
            let replay = doc.replay.as_mut().unwrap();
            replay.angles.push(AngleDto {
                id: 2,
                name: "B".into(),
                source: AngleSourceKind::Scene,
                scene_id: 1,
                input_id: 0,
                audio: AngleAudio::Silent,
                audio_unit_id: 0,
                encoder: crate::session::replay::AngleEncoder::Gpu,
            });
            replay.storage[0].angle_ids = vec![1, 2];
        }
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("already records"), "{}", err.message);
    }

    #[test]
    fn replay_document_with_one_channel_validates() {
        validate(&session_with_replay()).unwrap();
    }

    #[test]
    fn orphan_replay_input_is_rejected() {
        let mut doc = session_with_replay();
        let mut orphan = doc
            .inputs
            .iter()
            .find(|item| item.kind == InputKind::Replay)
            .unwrap()
            .clone();
        orphan.id = crate::ids::REPLAY_CHANNEL_BASE + 2;
        orphan.name = "Replay B".into();
        doc.inputs.push(orphan);
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("not assigned"), "{}", err.message);
    }

    #[test]
    fn replay_event_ids_must_be_unique() {
        let mut doc = session_with_replay();
        let replay = doc.replay.as_mut().unwrap();
        replay.events.push(replay_event(1));
        replay.events.push(replay_event(1));
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("event id"), "{}", err.message);
    }

    #[test]
    fn replay_active_angle_must_exist() {
        let mut doc = session_with_replay();
        doc.replay.as_mut().unwrap().active_angle_id = Some(5);
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("active angle"), "{}", err.message);
    }

    #[test]
    fn replay_clip_references_are_checked() {
        use crate::session::replay::*;
        let mut doc = session_with_replay();
        let replay = doc.replay.as_mut().unwrap();
        replay.events.push(replay_event(1));
        replay.playlists.push(PlaylistDto {
            id: 1,
            name: "Replay".into(),
            revision: 1,
            clips: vec![ClipDto {
                uid: "clip-1".into(),
                source: ClipSourceKind::Angle,
                angle_id: 1,
                ..ClipDto::default()
            }],
            default_transition: ClipTransitionDto::default(),
        });
        validate(&doc).unwrap();

        doc.replay.as_mut().unwrap().playlists[0].clips[0].angle_id = 9;
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("angle 9"), "{}", err.message);

        let clip = &mut doc.replay.as_mut().unwrap().playlists[0].clips[0];
        clip.angle_id = 1;

        let replay = doc.replay.as_mut().unwrap();
        replay.playlists[0].clips.push(ClipDto {
            uid: "clip-1".into(),
            source: ClipSourceKind::Angle,
            angle_id: 1,
            ..ClipDto::default()
        });
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("uid"), "{}", err.message);
    }

    /// Playing the same event twice is a normal replay move: two clips may share a
    /// source event as long as each carries its own uid, which is the handle a
    /// channel plays by.
    #[test]
    fn replay_clips_may_repeat_an_event_under_their_own_uids() {
        use crate::session::replay::*;
        let mut doc = session_with_replay();
        let replay = doc.replay.as_mut().unwrap();
        replay.events.push(replay_event(1));
        replay.playlists.push(PlaylistDto {
            id: 1,
            name: "Replay".into(),
            revision: 1,
            clips: vec![
                ClipDto {
                    uid: "clip-1".into(),
                    source: ClipSourceKind::Angle,
                    angle_id: 1,
                    ..ClipDto::default()
                },
                ClipDto {
                    uid: "clip-1-2".into(),
                    source: ClipSourceKind::Angle,
                    angle_id: 1,
                    ..ClipDto::default()
                },
            ],
            default_transition: ClipTransitionDto::default(),
        });
        validate(&doc).unwrap();
    }

    #[test]
    fn replay_channel_playlist_must_exist() {
        let mut doc = session_with_replay();
        doc.replay.as_mut().unwrap().channels[0].playlist_id = Some(7);
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("playlist 7"), "{}", err.message);
    }

    #[test]
    fn replay_routes_angles_across_several_storages() {
        use crate::session::replay::*;
        let mut doc = session_with_replay();
        {
            let replay = doc.replay.as_mut().unwrap();
            replay.angles.push(AngleDto {
                id: 2,
                name: "B".into(),
                source: crate::session::replay::AngleSourceKind::Scene,
                input_id: 0,
                scene_id: 2,
                audio: AngleAudio::Silent,
                audio_unit_id: 0,
                encoder: crate::session::replay::AngleEncoder::Gpu,
            });
            replay.enabled = true;
            replay.storage = vec![
                ReplayStorageDto {
                    path: "D:/replay-a".into(),
                    angle_ids: vec![1],
                },
                ReplayStorageDto {
                    path: "E:/replay-b".into(),
                    angle_ids: vec![2],
                },
            ];
        }
        validate(&doc).unwrap();

        // One angle cannot sit on two drives at once.
        doc.replay.as_mut().unwrap().storage[1].angle_ids = vec![1];
        let err = validate(&doc).unwrap_err();
        assert!(
            err.message.contains("more than one storage"),
            "{}",
            err.message
        );

        // Nor on a drive that does not know it.
        doc.replay.as_mut().unwrap().storage[1].angle_ids = vec![9];
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("does not exist"), "{}", err.message);

        // A drive without a path cannot be written to.
        doc.replay.as_mut().unwrap().storage[1].angle_ids = vec![2];
        doc.replay.as_mut().unwrap().storage[1].path = "  ".into();
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("needs a path"), "{}", err.message);
    }

    #[test]
    fn a_clip_cannot_borrow_audio_from_a_missing_angle() {
        use crate::session::replay::*;
        let mut doc = session_with_replay();
        {
            let replay = doc.replay.as_mut().unwrap();
            replay.playlists.push(PlaylistDto {
                id: 1,
                name: "Replay".into(),
                revision: 1,
                clips: vec![ClipDto {
                    uid: "clip-1".into(),
                    angle_id: 1,
                    audio: ClipAudio::Angle,
                    audio_angle_id: 1,
                    ..ClipDto::default()
                }],
                default_transition: ClipTransitionDto::default(),
            });
        }
        // Angle 1 exists, so borrowing its sound is fine.
        validate(&doc).unwrap();

        let replay = doc.replay.as_mut().unwrap();
        replay.playlists[0].clips[0].audio_angle_id = 7;
        let err = validate(&doc).unwrap_err();
        assert!(
            err.message.contains("takes audio from angle 7"),
            "{}",
            err.message
        );
    }

    #[test]
    fn an_animation_step_cannot_cue_a_channel_that_is_not_there() {
        use crate::session::replay::*;
        use crate::session::{SceneSequence, SequenceReplayAction, SequenceStep};
        let mut doc = session_with_replay();
        {
            let replay = doc.replay.as_mut().unwrap();
            replay.playlists.push(PlaylistDto {
                id: 1,
                name: "Replay".into(),
                revision: 1,
                clips: vec![ClipDto {
                    uid: "clip-1".into(),
                    angle_id: 1,
                    ..ClipDto::default()
                }],
                default_transition: ClipTransitionDto::default(),
            });
            replay.channels[0].playlist_id = Some(1);
            let scene = &mut doc.scenes[0];
            scene.states.push(crate::session::SceneState {
                id: 1,
                name: "Wide".into(),
                layers: Vec::new(),
                camera: None,
                enter: crate::session::Motion::default(),
            });
            scene.sequences.push(SceneSequence {
                id: 1,
                name: "Open".into(),
                steps: vec![
                    SequenceStep {
                        state_id: 1,
                        motion: None,
                        hold_frames: 10,
                        replay: None,
                    },
                    SequenceStep {
                        state_id: 1,
                        motion: None,
                        hold_frames: 10,
                        replay: Some(SequenceReplayAction {
                            channel: 0,
                            clip_uid: "clip-1".into(),
                            play: true,
                        }),
                    },
                ],
            });
        }
        // Cueing a clip that is on the channel is fine.
        validate(&doc).unwrap();

        // A clip the channel does not hold is refused.
        {
            let scene = &mut doc.scenes[0];
            scene.sequences[0].steps[1].replay = Some(SequenceReplayAction {
                channel: 0,
                clip_uid: "clip-9".into(),
                play: true,
            });
        }
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("clip-9"), "{}", err.message);

        // So is a channel that does not exist.
        {
            let scene = &mut doc.scenes[0];
            scene.sequences[0].steps[1].replay = Some(SequenceReplayAction {
                channel: 4,
                clip_uid: String::new(),
                play: false,
            });
        }
        let err = validate(&doc).unwrap_err();
        assert!(err.message.contains("channel 4"), "{}", err.message);

        // An empty clip id means "newest clip", which is allowed on a real channel.
        {
            let scene = &mut doc.scenes[0];
            scene.sequences[0].steps[1].replay = Some(SequenceReplayAction {
                channel: 0,
                clip_uid: String::new(),
                play: true,
            });
        }
        validate(&doc).unwrap();
    }
}
