use crate::session::{Document, InputKind, MixSource, OutputSourceKind, OutputTransport};

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

    for scene in &doc.scenes {
        for layer in &scene.layers {
            if !input_ids.contains(&layer.input_id) {
                return Err(ValidationError::new(format!(
                    "scene {} references missing input {}",
                    scene.id, layer.input_id
                )));
            }
        }
    }
    for input in &doc.inputs {
        if input.kind == InputKind::Mix {
            if input.mix_target_id == 0 || !unit_ids.contains(&input.mix_target_id) {
                return Err(ValidationError::new(format!(
                    "mix input {} references missing unit {}",
                    input.id, input.mix_target_id
                )));
            }
            if input.mix_source == MixSource::SessionMultiview
                && !mv_ids.contains(&input.mix_target_id)
            {
                // mix_target_id is the MU for preview/program; SessionMultiview uses mix_target as layout id
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
    validate(doc)?;
    for input in &doc.inputs {
        match input.kind {
            InputKind::Still | InputKind::Video => {
                let Some(path) = input.path_or_address.as_deref() else {
                    return Err(ValidationError::new(format!(
                        "input {} is missing a file path",
                        input.id
                    )));
                };
                if path.trim().is_empty() {
                    return Err(ValidationError::new(format!(
                        "input {} is missing a file path",
                        input.id
                    )));
                }
                if !std::path::Path::new(path).is_file() {
                    return Err(ValidationError::new(format!(
                        "input {} file does not exist: {path}",
                        input.id
                    )));
                }
            }
            _ => {}
        }
    }
    Ok(())
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
        if input.kind == InputKind::Mix && input.mix_target_id != 0 {
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
}
