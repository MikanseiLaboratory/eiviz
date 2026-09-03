//! Flatten eiviz session + live buses into a vMix `Vmix` document.

use std::collections::HashMap;

use vmix_core::{
    Audio, AudioBus, Dynamic, Input, InputOverlay, Inputs, Mix, Overlays, OverlaysOverlay,
    Position, State, Transition, Transitions, Vmix,
};

use crate::abi::{DURATION_FRAMES, DURATION_MS, SCENE_BASE, TRANSITION_FADE};
use crate::session::{Document, InputKind, TransitionPreset, UnitDto};

const VERSION: &str = "0.2.0";
const EDITION: &str = "eiviz";

#[derive(Debug, Clone)]
pub struct UnitLive {
    pub program_source: u64,
    pub preview_source: u64,
    pub overlay_sources: Vec<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct LiveSnapshot {
    pub units: HashMap<u64, UnitLive>,
}

#[derive(Debug, Clone)]
pub struct FlatInput {
    pub number: u32,
    pub key: String,
    pub title: String,
    pub input_type: String,
    pub source_id: u64,
    pub overlays: Vec<InputOverlay>,
}

#[derive(Debug, Clone)]
pub struct FlatMap {
    pub inputs: Vec<FlatInput>,
}

impl FlatMap {
    pub fn build(doc: &Document) -> Self {
        let mut inputs = Vec::new();
        let mut number = 1u32;
        for input in &doc.inputs {
            inputs.push(FlatInput {
                number,
                key: guid_or_fallback(&input.guid, input.id, false),
                title: title_or_fallback(&input.name, input.id, false),
                input_type: input_type(input.kind),
                source_id: input.id,
                overlays: Vec::new(),
            });
            number += 1;
        }
        let key_by_input_id: HashMap<u64, String> = doc
            .inputs
            .iter()
            .map(|input| (input.id, guid_or_fallback(&input.guid, input.id, false)))
            .collect();
        for scene in &doc.scenes {
            let mut overlays = Vec::new();
            for (index, layer) in scene.layers.iter().enumerate() {
                let Some(key) = key_by_input_id.get(&layer.input_id) else {
                    continue;
                };
                overlays.push(InputOverlay {
                    index: index.to_string(),
                    key: key.clone(),
                    position: Some(Position {
                        pan_x: None,
                        pan_y: None,
                        zoom_x: None,
                        zoom_y: None,
                        x: Some(layer.x.to_string()),
                        y: Some(layer.y.to_string()),
                        width: Some(layer.width.to_string()),
                        height: Some(layer.height.to_string()),
                    }),
                });
            }
            inputs.push(FlatInput {
                number,
                key: guid_or_fallback(&scene.guid, scene.id, true),
                title: title_or_fallback(&scene.name, scene.id, true),
                input_type: "Blank".into(),
                source_id: SCENE_BASE | scene.id,
                overlays,
            });
            number += 1;
        }
        Self { inputs }
    }

    pub fn by_number(&self, number: u32) -> Option<&FlatInput> {
        self.inputs.iter().find(|input| input.number == number)
    }

    pub fn by_source(&self, source_id: u64) -> Option<&FlatInput> {
        self.inputs
            .iter()
            .find(|input| input.source_id == source_id)
    }

    pub fn resolve_input(&self, raw: &str) -> Result<Option<&FlatInput>, String> {
        if raw.is_empty() {
            return Ok(None);
        }
        if raw == "0" || raw == "-1" {
            return Ok(None);
        }
        if let Ok(number) = raw.parse::<u32>()
            && number > 0
        {
            return self
                .by_number(number)
                .map(Some)
                .ok_or_else(|| format!("unknown Input {raw}"));
        }
        if let Some(found) = self.inputs.iter().find(|input| input.key == raw) {
            return Ok(Some(found));
        }
        if let Some(found) = self.inputs.iter().find(|input| input.title == raw) {
            return Ok(Some(found));
        }
        Err(format!("unknown Input {raw}"))
    }
}

pub fn resolve_mix(doc: &Document, raw: Option<&str>) -> Result<u64, String> {
    let Some(raw) = raw.filter(|value| !value.is_empty()) else {
        return default_unit_id(doc).ok_or_else(|| "no Mixing Unit".into());
    };
    if raw == "0" {
        return default_unit_id(doc).ok_or_else(|| "no Mixing Unit".into());
    }
    let index = raw
        .parse::<usize>()
        .map_err(|_| format!("invalid Mix {raw}"))?;
    if index == 0 {
        return default_unit_id(doc).ok_or_else(|| "no Mixing Unit".into());
    }
    doc.units
        .get(index - 1)
        .map(|unit| unit.id)
        .ok_or_else(|| format!("unknown Mix {raw}"))
}

fn default_unit_id(doc: &Document) -> Option<u64> {
    if doc.selected_unit_id != 0 && doc.units.iter().any(|unit| unit.id == doc.selected_unit_id) {
        return Some(doc.selected_unit_id);
    }
    doc.units.first().map(|unit| unit.id)
}

pub fn fade_duration_ms(unit: Option<&UnitDto>, fps_num: u32, fps_den: u32) -> u32 {
    let Some(unit) = unit else {
        return 1000;
    };
    let preset = unit
        .transitions
        .iter()
        .find(|item| item.kind == TRANSITION_FADE)
        .or_else(|| unit.transitions.first());
    match preset {
        Some(preset) => preset_duration_ms(preset, fps_num, fps_den),
        None => 1000,
    }
}

pub fn preset_duration_ms(preset: &TransitionPreset, fps_num: u32, fps_den: u32) -> u32 {
    if preset.duration_unit == DURATION_MS {
        return preset.duration_value.max(1);
    }
    if preset.duration_unit == DURATION_FRAMES || preset.duration_unit == 0 {
        let fps = if fps_den == 0 {
            60.0
        } else {
            fps_num as f32 / fps_den as f32
        };
        let fps = if fps <= 0.0 { 60.0 } else { fps };
        return ((preset.duration_value.max(1) as f32) * 1000.0 / fps).round() as u32;
    }
    preset.duration_value.max(1)
}

pub fn render_xml(doc: &Document, live: &LiveSnapshot) -> Result<String, String> {
    let vmix = build_vmix(doc, live);
    let xml = vmix_core::to_string(&vmix).map_err(|error| error.to_string())?;
    Ok(sanitize_vmix_xml(
        xml.replace("<Vmix>", "<vmix>")
            .replace("</Vmix>", "</vmix>"),
    ))
}

/// `vmix-core::to_string` writes empty attributes for `None`. `from_str` rejects those.
fn sanitize_vmix_xml(xml: String) -> String {
    let mut out = String::with_capacity(xml.len());
    let chars: Vec<char> = xml.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if i + 2 < chars.len() && chars[i] == '=' && chars[i + 1] == '"' && chars[i + 2] == '"' {
            while matches!(out.chars().last(), Some(c) if c != ' ' && c != '<') {
                out.pop();
            }
            if out.ends_with(' ') {
                out.pop();
            }
            i += 3;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    for tag in [
        "preset", "list", "replay", "crop", "outputs", "busA", "busB", "busC", "busD", "busE",
        "busF", "busG",
    ] {
        out = out.replace(&format!("<{tag}/>"), "");
    }
    out.replace("<position/>", "")
}

pub fn build_vmix(doc: &Document, live: &LiveSnapshot) -> Vmix {
    let flat = FlatMap::build(doc);
    let selected = resolve_mix(doc, None).ok();
    let selected_live = selected.and_then(|id| live.units.get(&id));
    let preview = selected_live
        .and_then(|unit| flat.by_source(unit.preview_source))
        .map(|input| input.number.to_string())
        .unwrap_or_else(|| "0".into());
    let active = selected_live
        .and_then(|unit| flat.by_source(unit.program_source))
        .map(|input| input.number.to_string())
        .unwrap_or_else(|| "0".into());

    let mixes = doc
        .units
        .iter()
        .enumerate()
        .skip(1)
        .map(|(index, unit)| {
            let live_unit = live.units.get(&unit.id);
            Mix {
                number: (index + 1).to_string(),
                preview: live_unit
                    .and_then(|item| flat.by_source(item.preview_source))
                    .map(|input| input.number.to_string())
                    .unwrap_or_else(|| "0".into()),
                active: live_unit
                    .and_then(|item| flat.by_source(item.program_source))
                    .map(|input| input.number.to_string())
                    .unwrap_or_else(|| "0".into()),
            }
        })
        .collect();

    let overlay_sources = selected_live
        .map(|unit| unit.overlay_sources.as_slice())
        .unwrap_or(&[]);
    let overlays = Overlays {
        overlay: (1..=8)
            .map(|slot| {
                let source = overlay_sources.get(slot - 1).copied().unwrap_or(0);
                OverlaysOverlay {
                    number: slot.to_string(),
                    input: flat
                        .by_source(source)
                        .map(|input| input.number.to_string())
                        .filter(|_| source != 0),
                }
            })
            .collect(),
    };

    let unit = selected.and_then(|id| doc.units.iter().find(|item| item.id == id));
    let transitions = Transitions {
        transition: unit
            .map(|item| item.transitions.as_slice())
            .unwrap_or(&[])
            .iter()
            .take(4)
            .enumerate()
            .map(|(index, preset)| Transition {
                number: (index + 1).to_string(),
                effect: crate::session::transition_label(preset.kind).to_string(),
                duration: preset_duration_ms(
                    preset,
                    unit.map(|item| item.fps_num).unwrap_or(60_000),
                    unit.map(|item| item.fps_den).unwrap_or(1_001),
                )
                .to_string(),
            })
            .collect(),
    };

    Vmix {
        version: VERSION.into(),
        edition: EDITION.into(),
        preset: None,
        inputs: Inputs {
            input: flat.inputs.iter().map(to_vmix_input).collect(),
        },
        outputs: None,
        overlays,
        preview,
        active,
        fade_to_black: false,
        transitions,
        recording: false,
        external: false,
        streaming: false,
        play_list: false,
        multi_corder: false,
        fullscreen: false,
        mix: mixes,
        audio: dummy_audio(),
        dynamic: dummy_dynamic(),
    }
}

fn to_vmix_input(flat: &FlatInput) -> Input {
    Input {
        key: flat.key.clone(),
        number: flat.number.to_string(),
        input_type: flat.input_type.clone(),
        title: flat.title.clone(),
        short_title: flat.title.clone(),
        state: State::Running,
        position: "0".into(),
        duration: "0".into(),
        input_loop: false,
        muted: None,
        volume: None,
        balance: None,
        solo: None,
        solo_pfl: None,
        audiobusses: None,
        meter_f1: None,
        meter_f2: None,
        gain_db: None,
        selected_index: None,
        preset: None,
        list: None,
        image: Vec::new(),
        replay: None,
        overlay: flat.overlays.clone(),
        crop: None,
        input_position: None,
    }
}

fn dummy_audio() -> Audio {
    Audio {
        master: AudioBus {
            volume: 100.0,
            muted: false,
            meter_f1: 0.0,
            meter_f2: 0.0,
            headphones_volume: None,
            solo: None,
            send_to_master: None,
        },
        bus_a: None,
        bus_b: None,
        bus_c: None,
        bus_d: None,
        bus_e: None,
        bus_f: None,
        bus_g: None,
    }
}

fn dummy_dynamic() -> Dynamic {
    Dynamic {
        input1: String::new(),
        input2: String::new(),
        input3: String::new(),
        input4: String::new(),
        value1: String::new(),
        value2: String::new(),
        value3: String::new(),
        value4: String::new(),
    }
}

fn input_type(kind: InputKind) -> String {
    match kind {
        InputKind::Color | InputKind::Black => "Colour",
        InputKind::Bars => "Colour",
        InputKind::Still => "Image",
        InputKind::Video => "Video",
        InputKind::Omt => "OMT",
        InputKind::Ndi => "NDI",
        InputKind::Uvc => "Capture",
        InputKind::Mix => "Mix",
    }
    .into()
}

fn guid_or_fallback(guid: &str, id: u64, scene: bool) -> String {
    if guid.is_empty() {
        if scene {
            format!("scene-{id}")
        } else {
            format!("input-{id}")
        }
    } else {
        guid.to_string()
    }
}

fn title_or_fallback(name: &str, id: u64, scene: bool) -> String {
    if name.is_empty() {
        if scene {
            format!("Scene {id}")
        } else {
            format!("Input {id}")
        }
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_doc() -> Document {
        crate::session::parse(
            br#"{
            "version": 2,
            "selectedUnitId": 1,
            "inputs": [
                {"id": 1, "guid": "aaa", "name": "Color", "kind": "Color"},
                {"id": 2, "guid": "bbb", "name": "Bars", "kind": "Bars"}
            ],
            "scenes": [{
                "id": 1,
                "guid": "scene-1",
                "name": "Scene One",
                "layers": [{"inputId": 2}]
            }],
            "units": [{
                "id": 1,
                "name": "MU1",
                "transitions": [{"kind": 1, "durationValue": 500, "durationUnit": 1}]
            }]
        }"#,
        )
        .expect("sample session")
    }

    #[test]
    fn flatten_numbers_inputs_then_scenes() {
        let doc = sample_doc();
        let flat = FlatMap::build(&doc);
        assert_eq!(flat.inputs.len(), 3);
        assert_eq!(flat.inputs[0].number, 1);
        assert_eq!(flat.inputs[0].source_id, 1);
        assert_eq!(flat.inputs[1].number, 2);
        assert_eq!(flat.inputs[2].number, 3);
        assert_eq!(flat.inputs[2].input_type, "Blank");
        assert_eq!(flat.inputs[2].source_id, SCENE_BASE | 1);
        assert_eq!(flat.inputs[2].overlays.len(), 1);
        assert_eq!(flat.inputs[2].overlays[0].key, "bbb");
    }

    #[test]
    fn xml_round_trips_through_vmix_core() {
        let doc = sample_doc();
        let mut live = LiveSnapshot::default();
        live.units.insert(
            1,
            UnitLive {
                program_source: SCENE_BASE | 1,
                preview_source: 1,
                overlay_sources: Vec::new(),
            },
        );
        let xml = render_xml(&doc, &live).expect("xml");
        assert!(xml.contains("<vmix>"), "{xml}");
        let parsed =
            vmix_core::from_str(&xml).unwrap_or_else(|error| panic!("parse {error}: {xml}"));
        assert_eq!(parsed.preview, "1");
        assert_eq!(parsed.active, "3");
        assert_eq!(parsed.inputs.input.len(), 3);
        assert_eq!(parsed.inputs.input[2].input_type, "Blank");
        assert_eq!(parsed.inputs.input[2].overlay.len(), 1);
    }

    #[test]
    fn mix_and_input_resolve() {
        let doc = sample_doc();
        let flat = FlatMap::build(&doc);
        assert_eq!(resolve_mix(&doc, None).unwrap(), 1);
        assert_eq!(resolve_mix(&doc, Some("1")).unwrap(), 1);
        assert!(resolve_mix(&doc, Some("9")).is_err());
        assert_eq!(flat.resolve_input("2").unwrap().unwrap().title, "Bars");
        assert_eq!(flat.resolve_input("aaa").unwrap().unwrap().number, 1);
        assert!(flat.resolve_input("missing").is_err());
    }

    #[test]
    fn fade_ms_from_preset() {
        let doc = sample_doc();
        assert_eq!(fade_duration_ms(doc.units.first(), 60_000, 1_001), 500);
    }
}
