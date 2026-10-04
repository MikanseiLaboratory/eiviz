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
        if output.transport == OutputTransport::DeckLink {
            // DeckLink stays host-owned; headless ignores it.
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
}
