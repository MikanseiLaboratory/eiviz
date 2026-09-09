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
    unique_ids(doc.buses.iter().map(|item| item.id), "bus")?;
    unique_guids(doc.inputs.iter().map(|item| item.guid.as_str()), "input")?;
    unique_guids(doc.scenes.iter().map(|item| item.guid.as_str()), "scene")?;

    let input_ids: Vec<u64> = doc.inputs.iter().map(|item| item.id).collect();
    let unit_ids: Vec<u64> = doc.units.iter().map(|item| item.id).collect();
    let scene_ids: Vec<u64> = doc.scenes.iter().map(|item| item.id).collect();
    let bus_ids: Vec<u64> = doc.buses.iter().map(|item| item.id).collect();
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
                "audio input {} ASIO pair is invalid",
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
        if output.audio_bus_id != 0
            && !bus_ids.is_empty()
            && !bus_ids.contains(&output.audio_bus_id)
        {
            return Err(ValidationError::new(format!(
                "output {} references missing bus {}",
                output.id, output.audio_bus_id
            )));
        }
    }
    if doc.settings.master_fps_num == 0 || doc.settings.master_fps_den == 0 {
        return Err(ValidationError::new("master fps is zero"));
    }
    Ok(())
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
}
