//! Shared document editors used by SessionMutation and eivizctl.

use crate::error::{ControlError, ControlResult};
use crate::session::{
    Document, InputDto, MultiviewDto, SceneDto, SceneLayer, SceneLayoutPreset, UnitDto,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagCatalog {
    Input,
    Scene,
}

impl TagCatalog {
    pub fn parse(raw: &str) -> ControlResult<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "input" | "inputs" => Ok(Self::Input),
            "scene" | "scenes" => Ok(Self::Scene),
            _ => Err(ControlError::invalid("catalog must be input or scene")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Scene => "scene",
        }
    }
}

pub fn allocate_input(document: &mut Document, mut input: InputDto) -> ControlResult<u64> {
    if input.id == 0 {
        input.id = next_input_id(document);
    } else if document.inputs.iter().any(|item| item.id == input.id) {
        return Err(ControlError::conflict(format!("input {} exists", input.id)));
    }
    if input.guid.trim().is_empty() {
        input.guid = uuid::Uuid::new_v4().to_string();
    }
    if input.name.trim().is_empty() {
        input.name = format!("Input {}", input.id);
    }
    document.next_input_id = document.next_input_id.max(input.id.saturating_add(1));
    let id = input.id;
    document.inputs.push(input);
    Ok(id)
}

pub fn allocate_scene(document: &mut Document, mut scene: SceneDto) -> ControlResult<u64> {
    if scene.id == 0 {
        scene.id = next_scene_id(document);
    } else if document.scenes.iter().any(|item| item.id == scene.id) {
        return Err(ControlError::conflict(format!("scene {} exists", scene.id)));
    }
    if scene.guid.trim().is_empty() {
        scene.guid = uuid::Uuid::new_v4().to_string();
    }
    if scene.name.trim().is_empty() {
        scene.name = format!("Scene {}", scene.id);
    }
    document.next_scene_id = document.next_scene_id.max(scene.id.saturating_add(1));
    let id = scene.id;
    document.scenes.push(scene);
    Ok(id)
}

pub fn allocate_unit(document: &mut Document, mut unit: UnitDto) -> ControlResult<u64> {
    if unit.id == 0 {
        unit.id = next_unit_id(document);
    } else if document.units.iter().any(|item| item.id == unit.id) {
        return Err(ControlError::conflict(format!("unit {} exists", unit.id)));
    }
    if unit.name.trim().is_empty() {
        unit.name = format!("Mixing Unit {}", unit.id);
    }
    document.next_unit_id = document.next_unit_id.max(unit.id.saturating_add(1));
    let id = unit.id;
    document.units.push(unit);
    Ok(id)
}

pub fn allocate_multiview(document: &mut Document, mut layout: MultiviewDto) -> ControlResult<u64> {
    if layout.id == 0 {
        layout.id = next_multiview_id(document);
    } else if document.multiviews.iter().any(|item| item.id == layout.id) {
        return Err(ControlError::conflict(format!(
            "multiview {} exists",
            layout.id
        )));
    }
    if layout.name.trim().is_empty() {
        layout.name = format!("Multiview {}", layout.id);
    }
    document.next_multiview_id = document.next_multiview_id.max(layout.id.saturating_add(1));
    let id = layout.id;
    document.multiviews.push(layout);
    Ok(id)
}

pub fn add_scene_layer(
    document: &mut Document,
    scene_id: u64,
    layer: SceneLayer,
) -> ControlResult<()> {
    let scene = scene_mut(document, scene_id)?;
    scene.layers.push(layer);
    Ok(())
}

pub fn replace_scene_layer(
    document: &mut Document,
    scene_id: u64,
    index: usize,
    layer: SceneLayer,
) -> ControlResult<()> {
    let scene = scene_mut(document, scene_id)?;
    let slot = scene
        .layers
        .get_mut(index)
        .ok_or_else(|| ControlError::not_found(format!("layer {index}")))?;
    *slot = layer;
    Ok(())
}

pub fn delete_scene_layer(
    document: &mut Document,
    scene_id: u64,
    index: usize,
) -> ControlResult<()> {
    let scene = scene_mut(document, scene_id)?;
    if index >= scene.layers.len() {
        return Err(ControlError::not_found(format!("layer {index}")));
    }
    scene.layers.remove(index);
    Ok(())
}

pub fn move_scene_layer(
    document: &mut Document,
    scene_id: u64,
    from: usize,
    to: usize,
) -> ControlResult<()> {
    let scene = scene_mut(document, scene_id)?;
    if from >= scene.layers.len() || to >= scene.layers.len() {
        return Err(ControlError::invalid("layer index out of range"));
    }
    let layer = scene.layers.remove(from);
    scene.layers.insert(to, layer);
    Ok(())
}

pub fn add_catalog_tag(
    document: &mut Document,
    catalog: TagCatalog,
    tag: String,
) -> ControlResult<()> {
    let tag = tag.trim().to_string();
    if tag.is_empty() {
        return Err(ControlError::invalid("tag required"));
    }
    let tags = catalog_mut(document, catalog);
    if tags.iter().any(|item| item == &tag) {
        return Ok(());
    }
    tags.push(tag);
    Ok(())
}

pub fn rename_catalog_tag(
    document: &mut Document,
    catalog: TagCatalog,
    from: &str,
    to: &str,
) -> ControlResult<()> {
    let from = from.trim();
    let to = to.trim();
    if from.is_empty() || to.is_empty() {
        return Err(ControlError::invalid("tag name required"));
    }
    let tags = catalog_mut(document, catalog);
    let Some(index) = tags.iter().position(|item| item == from) else {
        return Err(ControlError::not_found(format!("tag {from}")));
    };
    if tags.iter().any(|item| item == to) && from != to {
        return Err(ControlError::conflict(format!("tag {to} exists")));
    }
    tags[index] = to.to_string();
    match catalog {
        TagCatalog::Input => {
            for input in &mut document.inputs {
                for tag in &mut input.tags {
                    if tag == from {
                        *tag = to.to_string();
                    }
                }
            }
        }
        TagCatalog::Scene => {
            for scene in &mut document.scenes {
                for tag in &mut scene.tags {
                    if tag == from {
                        *tag = to.to_string();
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn delete_catalog_tag(
    document: &mut Document,
    catalog: TagCatalog,
    tag: &str,
) -> ControlResult<()> {
    let tag = tag.trim();
    if tag.is_empty() {
        return Err(ControlError::invalid("tag required"));
    }
    let tags = catalog_mut(document, catalog);
    let before = tags.len();
    tags.retain(|item| item != tag);
    if tags.len() == before {
        return Err(ControlError::not_found(format!("tag {tag}")));
    }
    match catalog {
        TagCatalog::Input => {
            for input in &mut document.inputs {
                input.tags.retain(|item| item != tag);
            }
        }
        TagCatalog::Scene => {
            for scene in &mut document.scenes {
                scene.tags.retain(|item| item != tag);
            }
        }
    }
    Ok(())
}

pub fn upsert_scene_preset(
    document: &mut Document,
    preset: SceneLayoutPreset,
) -> ControlResult<()> {
    if preset.name.trim().is_empty() {
        return Err(ControlError::invalid("preset name required"));
    }
    if let Some(existing) = document
        .scene_presets
        .iter_mut()
        .find(|item| item.name == preset.name)
    {
        *existing = preset;
    } else {
        document.scene_presets.push(preset);
    }
    Ok(())
}

pub fn delete_scene_preset(document: &mut Document, name: &str) -> ControlResult<()> {
    let before = document.scene_presets.len();
    document.scene_presets.retain(|item| item.name != name);
    if document.scene_presets.len() == before {
        return Err(ControlError::not_found(format!("preset {name}")));
    }
    Ok(())
}

fn scene_mut(document: &mut Document, scene_id: u64) -> ControlResult<&mut SceneDto> {
    document
        .scenes
        .iter_mut()
        .find(|item| item.id == scene_id)
        .ok_or_else(|| ControlError::not_found(format!("scene {scene_id}")))
}

fn catalog_mut(document: &mut Document, catalog: TagCatalog) -> &mut Vec<String> {
    match catalog {
        TagCatalog::Input => &mut document.input_tags,
        TagCatalog::Scene => &mut document.scene_tags,
    }
}

fn next_input_id(document: &Document) -> u64 {
    let max = document
        .inputs
        .iter()
        .map(|item| item.id)
        .max()
        .unwrap_or(9);
    document.next_input_id.max(max.saturating_add(1)).max(10)
}

fn next_scene_id(document: &Document) -> u64 {
    let max = document
        .scenes
        .iter()
        .map(|item| item.id)
        .max()
        .unwrap_or(0);
    document.next_scene_id.max(max.saturating_add(1)).max(1)
}

fn next_unit_id(document: &Document) -> u64 {
    let max = document.units.iter().map(|item| item.id).max().unwrap_or(0);
    document.next_unit_id.max(max.saturating_add(1)).max(1)
}

fn next_multiview_id(document: &Document) -> u64 {
    let max = document
        .multiviews
        .iter()
        .map(|item| item.id)
        .max()
        .unwrap_or(0);
    document.next_multiview_id.max(max.saturating_add(1)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::parse;

    fn bars() -> Document {
        parse(
            br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Bars", "kind": "Bars" }],
          "scenes": [
            { "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }
          ],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#,
        )
        .unwrap()
    }

    fn sample_input(name: &str) -> InputDto {
        serde_json::from_value(serde_json::json!({
            "id": 0,
            "name": name,
            "kind": "ndi"
        }))
        .unwrap()
    }

    fn sample_layer(x: f32) -> SceneLayer {
        serde_json::from_value(serde_json::json!({
            "inputId": 2,
            "x": x,
            "width": 0.5,
            "height": 1
        }))
        .unwrap()
    }

    #[test]
    fn allocate_input_assigns_id_and_guid() {
        let mut doc = bars();
        let id = allocate_input(&mut doc, sample_input("Cam")).unwrap();
        assert!(id >= 10);
        let added = doc.inputs.iter().find(|item| item.id == id).unwrap();
        assert!(!added.guid.is_empty());
        assert_eq!(added.name, "Cam");
    }

    #[test]
    fn layer_move_and_tag_rename() {
        let mut doc = bars();
        add_scene_layer(&mut doc, 1, sample_layer(0.5)).unwrap();
        move_scene_layer(&mut doc, 1, 1, 0).unwrap();
        assert!((doc.scenes[0].layers[0].x - 0.5).abs() < f32::EPSILON);
        add_catalog_tag(&mut doc, TagCatalog::Input, "Cameras".into()).unwrap();
        doc.inputs[0].tags.push("Cameras".into());
        rename_catalog_tag(&mut doc, TagCatalog::Input, "Cameras", "Cams").unwrap();
        assert_eq!(doc.input_tags, ["Cams"]);
        assert_eq!(doc.inputs[0].tags, ["Cams"]);
        delete_catalog_tag(&mut doc, TagCatalog::Input, "Cams").unwrap();
        assert!(doc.input_tags.is_empty());
        assert!(doc.inputs[0].tags.is_empty());
    }

    #[test]
    fn upsert_clone_keeps_unspecified_fields() {
        let mut doc = bars();
        let original = doc.inputs[0].clone();
        let mut edited = original.clone();
        edited.name = "Bars A".into();
        assert_eq!(edited.kind, original.kind);
        assert_eq!(edited.path_or_address, original.path_or_address);
        assert_eq!(edited.bus_mask, original.bus_mask);
        crate::session::mutate::apply(
            &mut doc,
            crate::SessionMutation::UpsertInput {
                input: Box::new(edited),
            },
        )
        .unwrap();
        assert_eq!(doc.inputs[0].name, "Bars A");
        assert_eq!(doc.inputs[0].kind, original.kind);
    }
}
