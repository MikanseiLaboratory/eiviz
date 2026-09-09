use crate::error::{ControlError, ControlResult};
use crate::session::{Document, InputDto, SceneDto};
use serde::{Deserialize, Serialize};

pub const SCENE_BASE: u64 = 0x0001_0000;
pub const MULTIVIEW_BASE: u64 = 0x0002_0000;
pub const MU_SOURCE_FLAG: u64 = 0x8000_0000_0000_0000;
pub const MU_BUS_PREVIEW: u64 = 0x1000_0000_0000_0000;
pub const MU_ID_MASK: u64 = 0x0FFF_FFFF_FFFF_FFFF;

pub fn scene_gpu_id(scene_id: u64) -> u64 {
    SCENE_BASE | scene_id
}

pub fn multiview_gpu_id(layout_id: u64) -> u64 {
    MULTIVIEW_BASE | layout_id
}

pub fn mu_program(unit_id: u64) -> u64 {
    MU_SOURCE_FLAG | (unit_id & MU_ID_MASK)
}

pub fn mu_preview(unit_id: u64) -> u64 {
    MU_SOURCE_FLAG | MU_BUS_PREVIEW | (unit_id & MU_ID_MASK)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceKind {
    Input,
    Scene,
    Unit,
    Output,
    Multiview,
    Bus,
}

impl ResourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Scene => "scene",
            Self::Unit => "unit",
            Self::Output => "output",
            Self::Multiview => "multiview",
            Self::Bus => "bus",
        }
    }
}

/// Typed resource identity. Numeric id is the stable key. GUID and name are
/// accepted on the wire for Input/Scene compatibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRef {
    pub kind: ResourceKind,
    pub id: u64,
    pub guid: String,
    pub name: String,
}

impl ResourceRef {
    pub fn numeric(kind: ResourceKind, id: u64) -> Self {
        Self {
            kind,
            id,
            guid: String::new(),
            name: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Resolver;

impl Resolver {
    pub fn resolve_input<'a>(&self, doc: &'a Document, raw: &str) -> ControlResult<&'a InputDto> {
        resolve_named(
            raw,
            doc.inputs.iter(),
            |item| item.id,
            |item| item.guid.as_str(),
            |item| item.name.as_str(),
            ResourceKind::Input,
        )
    }

    pub fn resolve_scene<'a>(&self, doc: &'a Document, raw: &str) -> ControlResult<&'a SceneDto> {
        resolve_named(
            raw,
            doc.scenes.iter(),
            |item| item.id,
            |item| item.guid.as_str(),
            |item| item.name.as_str(),
            ResourceKind::Scene,
        )
    }

    pub fn resolve_unit(&self, doc: &Document, raw: &str) -> ControlResult<u64> {
        if raw.is_empty() || raw == "0" {
            if doc.selected_unit_id != 0 {
                return Ok(doc.selected_unit_id);
            }
            return doc
                .units
                .first()
                .map(|unit| unit.id)
                .ok_or_else(|| ControlError::not_found("no mixing unit"));
        }
        if let Ok(index) = raw.parse::<u32>() {
            if index >= 1 {
                if let Some(unit) = doc.units.get((index - 1) as usize) {
                    return Ok(unit.id);
                }
            }
        }
        if let Ok(id) = raw.parse::<u64>() {
            if doc.units.iter().any(|unit| unit.id == id) {
                return Ok(id);
            }
        }
        let matches: Vec<_> = doc
            .units
            .iter()
            .filter(|unit| unit.name.eq_ignore_ascii_case(raw))
            .collect();
        match matches.as_slice() {
            [unit] => Ok(unit.id),
            [] => Err(ControlError::not_found(format!("mixing unit {raw}"))),
            _ => Err(ControlError::ambiguous(format!("mixing unit {raw}"))),
        }
    }
}

fn resolve_named<'a, T>(
    raw: &str,
    items: impl Iterator<Item = &'a T>,
    id_of: impl Fn(&T) -> u64,
    guid_of: impl Fn(&T) -> &str,
    name_of: impl Fn(&T) -> &str,
    kind: ResourceKind,
) -> ControlResult<&'a T> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(ControlError::invalid(format!(
            "empty {} selector",
            kind.as_str()
        )));
    }
    let items: Vec<&'a T> = items.collect();
    if let Ok(id) = raw.parse::<u64>() {
        if let Some(item) = items.iter().copied().find(|item| id_of(item) == id) {
            return Ok(item);
        }
    }
    let guid_hits: Vec<_> = items
        .iter()
        .copied()
        .filter(|item| !guid_of(item).is_empty() && guid_of(item).eq_ignore_ascii_case(raw))
        .collect();
    match guid_hits.as_slice() {
        [item] => return Ok(*item),
        [] => {}
        _ => {
            return Err(ControlError::ambiguous(format!(
                "{} guid {raw}",
                kind.as_str()
            )));
        }
    }
    let name_hits: Vec<_> = items
        .iter()
        .copied()
        .filter(|item| name_of(item).eq_ignore_ascii_case(raw))
        .collect();
    match name_hits.as_slice() {
        [item] => Ok(*item),
        [] => Err(ControlError::not_found(format!("{} {raw}", kind.as_str()))),
        _ => Err(ControlError::ambiguous(format!(
            "{} name {raw}",
            kind.as_str()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{InputDto, InputKind};

    fn doc_with_inputs(inputs: Vec<InputDto>) -> Document {
        let mut doc: Document = serde_json::from_str("{}").unwrap();
        doc.inputs = inputs;
        doc.canonicalize()
    }

    fn input(id: u64, name: &str, guid: &str) -> InputDto {
        InputDto {
            id,
            guid: guid.into(),
            name: name.into(),
            kind: InputKind::Color,
            path_or_address: None,
            color_r: 0.0,
            color_g: 0.0,
            color_b: 0.0,
            scroll: false,
            tone_hz: 0.0,
            tone_level_dbfs: -20.0,
            bus_mask: 1,
            gain: 1.0,
            mute: false,
            use_gpu: false,
            frame_buffer_frames: 1,
            bandwidth_save: Default::default(),
            keep_full_on_multiview: false,
            omt_quality: Default::default(),
            ndi_bandwidth: Default::default(),
            video_loop: true,
            video_play_when: Default::default(),
            video_restart_when: Default::default(),
            video_pause_when: Default::default(),
            capture_width: 0,
            capture_height: 0,
            capture_fps_num: 0,
            capture_fps_den: 0,
            tags: Vec::new(),
            mix_source: Default::default(),
            mix_target_id: 0,
            mix_audio_bus_id: 0,
            audio_capture_mode: Default::default(),
            audio_device_kind: Default::default(),
            audio_device_id: String::new(),
            audio_map_left: 0,
            audio_map_right: 1,
            audio_process_exe: String::new(),
            audio_process_aumid: String::new(),
        }
    }

    #[test]
    fn name_collision_is_ambiguous() {
        let doc = doc_with_inputs(vec![
            input(2, "Cam", "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
            input(3, "Cam", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        ]);
        let err = Resolver.resolve_input(&doc, "Cam").unwrap_err();
        assert!(matches!(err, ControlError::Ambiguous { .. }));
    }

    #[test]
    fn guid_wins_over_name() {
        let doc = doc_with_inputs(vec![
            input(2, "Cam", "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
            input(3, "Other", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        ]);
        let hit = Resolver
            .resolve_input(&doc, "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
            .unwrap();
        assert_eq!(hit.id, 2);
    }
}
