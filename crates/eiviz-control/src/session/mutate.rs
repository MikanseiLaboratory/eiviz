//! Typed document mutations. Callers stage a clone, canonicalize, then replace.

use crate::command::SessionMutation;
use crate::error::{ControlError, ControlResult};
use crate::session::{Document, InputDto, InputKind, MultiviewDto, SceneDto, UnitDto};

pub fn apply(document: &mut Document, mutation: SessionMutation) -> ControlResult<()> {
    match mutation {
        SessionMutation::UpsertInput { input } => upsert_input(document, *input),
        SessionMutation::DeleteInput { id } => {
            if !document.inputs.iter().any(|item| item.id == id) {
                return Err(ControlError::not_found(format!("input {id}")));
            }
            document.inputs.retain(|item| item.id != id);
            for scene in &mut document.scenes {
                scene.layers.retain(|layer| layer.input_id != id);
            }
            Ok(())
        }
        SessionMutation::UpsertScene { scene } => upsert_scene(document, *scene),
        SessionMutation::DeleteScene { id } => {
            if !document.scenes.iter().any(|item| item.id == id) {
                return Err(ControlError::not_found(format!("scene {id}")));
            }
            document.scenes.retain(|item| item.id != id);
            Ok(())
        }
        SessionMutation::SetSceneLayers { scene_id, layers } => {
            let scene = document
                .scenes
                .iter_mut()
                .find(|item| item.id == scene_id)
                .ok_or_else(|| ControlError::not_found(format!("scene {scene_id}")))?;
            scene.layers = layers;
            Ok(())
        }
        SessionMutation::UpsertUnit { unit } => upsert_unit(document, *unit),
        SessionMutation::DeleteUnit { id } => {
            if document.units.len() <= 1 {
                return Err(ControlError::invalid(
                    "refusing to delete the last mixing unit",
                ));
            }
            if !document.units.iter().any(|item| item.id == id) {
                return Err(ControlError::not_found(format!("unit {id}")));
            }
            document.units.retain(|item| item.id != id);
            if document.selected_unit_id == id {
                document.selected_unit_id = document.units.first().map(|item| item.id).unwrap_or(1);
            }
            Ok(())
        }
        SessionMutation::SetOverlaySlot {
            unit_id,
            index,
            slot,
        } => {
            let unit = document
                .units
                .iter_mut()
                .find(|item| item.id == unit_id)
                .ok_or_else(|| ControlError::not_found(format!("unit {unit_id}")))?;
            let index = index as usize;
            if index >= unit.overlays.len() {
                unit.overlays.resize(
                    index + 1,
                    crate::session::OverlaySlot {
                        scene_gpu_id: 0,
                        enabled: true,
                        ..crate::session::OverlaySlot::default()
                    },
                );
            }
            unit.overlays[index] = *slot;
            Ok(())
        }
        SessionMutation::AddMediaInput {
            name,
            media_kind: kind,
            host_path,
            video_loop,
            tags,
        } => {
            if !matches!(kind, InputKind::Still | InputKind::Video) {
                return Err(ControlError::invalid(
                    "uploaded media must be Still or Video",
                ));
            }
            if host_path.is_empty() {
                return Err(ControlError::invalid("host path required"));
            }
            let id = next_input_id(document);
            document.inputs.push(InputDto {
                id,
                guid: uuid::Uuid::new_v4().to_string(),
                name: if name.trim().is_empty() {
                    default_media_name(kind, &host_path)
                } else {
                    name
                },
                kind,
                path_or_address: Some(host_path),
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
                bandwidth_save: crate::session::BandwidthSave::NotOnPreviewOrProgram,
                keep_full_on_multiview: false,
                omt_quality: crate::session::OmtQuality::Default,
                ndi_bandwidth: crate::session::NdiBandwidth::Highest,
                video_loop,
                video_play_when: crate::session::VideoPlayWhen::Never,
                video_restart_when: crate::session::VideoTriggerWhen::Never,
                video_pause_when: crate::session::VideoTriggerWhen::Never,
                capture_width: 0,
                capture_height: 0,
                capture_fps_num: 0,
                capture_fps_den: 0,
                tags,
                mix_source: crate::session::MixSource::MuProgram,
                mix_target_id: 0,
                mix_audio_bus_id: 0,
                audio_capture_mode: crate::session::AudioCaptureMode::Mic,
                audio_device_kind: crate::session::AudioDeviceKind::None,
                audio_device_id: String::new(),
                audio_map_left: 0,
                audio_map_right: 1,
                audio_process_exe: String::new(),
                audio_process_aumid: String::new(),
            });
            document.next_input_id = id.saturating_add(1);
            Ok(())
        }
        SessionMutation::UpsertMultiview { layout } => upsert_multiview(document, *layout),
        SessionMutation::DeleteMultiview { id } => {
            if !document.multiviews.iter().any(|item| item.id == id) {
                return Err(ControlError::not_found(format!("multiview {id}")));
            }
            document.multiviews.retain(|item| item.id != id);
            Ok(())
        }
        SessionMutation::SetSettings {
            settings,
            outputs,
            buses,
            headphone_copy_master,
            next_output_id,
            next_bus_id,
        } => {
            let renderer = document.settings.renderer;
            let last_session_path = document.settings.last_session_path.clone();
            document.settings = *settings;
            document.settings.renderer = renderer;
            document.settings.last_session_path = last_session_path;
            document.outputs = outputs;
            document.buses = buses;
            if let Some(enabled) = headphone_copy_master {
                document.headphone_copy_master = enabled;
            }
            if next_output_id != 0 {
                document.next_output_id = next_output_id;
            }
            if next_bus_id != 0 {
                document.next_bus_id = next_bus_id;
            }
            Ok(())
        }
        SessionMutation::RelinkMedia { directories } => {
            if directories.iter().all(|dir| dir.trim().is_empty()) {
                return Err(ControlError::invalid("search directory required"));
            }
            crate::session::relink_missing_media(document, &directories);
            Ok(())
        }
        SessionMutation::CreateInput { input } => {
            crate::session::edit::allocate_input(document, *input).map(|_| ())
        }
        SessionMutation::CreateScene { scene } => {
            crate::session::edit::allocate_scene(document, *scene).map(|_| ())
        }
        SessionMutation::CreateUnit { unit } => {
            crate::session::edit::allocate_unit(document, *unit).map(|_| ())
        }
        SessionMutation::CreateMultiview { layout } => {
            crate::session::edit::allocate_multiview(document, *layout).map(|_| ())
        }
        SessionMutation::AddCatalogTag { catalog, tag } => {
            let catalog = crate::session::edit::TagCatalog::parse(&catalog)?;
            crate::session::edit::add_catalog_tag(document, catalog, tag)
        }
        SessionMutation::RenameCatalogTag { catalog, from, to } => {
            let catalog = crate::session::edit::TagCatalog::parse(&catalog)?;
            crate::session::edit::rename_catalog_tag(document, catalog, &from, &to)
        }
        SessionMutation::DeleteCatalogTag { catalog, tag } => {
            let catalog = crate::session::edit::TagCatalog::parse(&catalog)?;
            crate::session::edit::delete_catalog_tag(document, catalog, &tag)
        }
        SessionMutation::UpsertScenePreset { preset } => {
            crate::session::edit::upsert_scene_preset(document, *preset)
        }
        SessionMutation::DeleteScenePreset { name } => {
            crate::session::edit::delete_scene_preset(document, &name)
        }
    }
}

fn upsert_input(document: &mut Document, input: InputDto) -> ControlResult<()> {
    if input.id == 0 {
        return Err(ControlError::invalid("input id required"));
    }
    if let Some(existing) = document.inputs.iter_mut().find(|item| item.id == input.id) {
        *existing = input;
    } else {
        document.next_input_id = document.next_input_id.max(input.id.saturating_add(1));
        document.inputs.push(input);
    }
    Ok(())
}

fn upsert_scene(document: &mut Document, scene: SceneDto) -> ControlResult<()> {
    if scene.id == 0 {
        return Err(ControlError::invalid("scene id required"));
    }
    if let Some(existing) = document.scenes.iter_mut().find(|item| item.id == scene.id) {
        *existing = scene;
    } else {
        document.next_scene_id = document.next_scene_id.max(scene.id.saturating_add(1));
        document.scenes.push(scene);
    }
    Ok(())
}

fn upsert_unit(document: &mut Document, unit: UnitDto) -> ControlResult<()> {
    if unit.id == 0 {
        return Err(ControlError::invalid("unit id required"));
    }
    if let Some(existing) = document.units.iter_mut().find(|item| item.id == unit.id) {
        *existing = unit;
    } else {
        document.next_unit_id = document.next_unit_id.max(unit.id.saturating_add(1));
        document.units.push(unit);
    }
    Ok(())
}

fn upsert_multiview(document: &mut Document, layout: MultiviewDto) -> ControlResult<()> {
    if layout.id == 0 {
        return Err(ControlError::invalid("multiview id required"));
    }
    if let Some(existing) = document
        .multiviews
        .iter_mut()
        .find(|item| item.id == layout.id)
    {
        *existing = layout;
    } else {
        document.next_multiview_id = document.next_multiview_id.max(layout.id.saturating_add(1));
        document.multiviews.push(layout);
    }
    Ok(())
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

fn default_media_name(kind: InputKind, path: &str) -> String {
    let file = std::path::Path::new(path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Media");
    if matches!(kind, InputKind::Video) {
        format!("Video {file}")
    } else {
        format!("Still {file}")
    }
}

impl Document {
    pub fn published_video_outputs(&self) -> Vec<&crate::session::OutputDto> {
        self.outputs
            .iter()
            .filter(|output| {
                output.enabled
                    && matches!(
                        output.transport,
                        crate::session::OutputTransport::Omt | crate::session::OutputTransport::Ndi
                    )
                    && matches!(
                        output.source_kind,
                        crate::session::OutputSourceKind::MuPreview
                            | crate::session::OutputSourceKind::MuProgram
                            | crate::session::OutputSourceKind::Multiview
                    )
            })
            .collect()
    }
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

    #[test]
    fn add_media_input_appends_still() {
        let mut doc = bars();
        apply(
            &mut doc,
            SessionMutation::AddMediaInput {
                name: "Logo".into(),
                media_kind: InputKind::Still,
                host_path: "/media/logo.png".into(),
                video_loop: true,
                tags: vec![],
            },
        )
        .unwrap();
        let added = doc.inputs.iter().find(|item| item.name == "Logo").unwrap();
        assert_eq!(added.kind, InputKind::Still);
        assert_eq!(added.path_or_address.as_deref(), Some("/media/logo.png"));
        assert!(!doc.outputs.iter().any(|_| false));
    }

    #[test]
    fn upsert_unit_from_remote_camel_case_json() {
        let mut doc = bars();
        let mutation: SessionMutation = serde_json::from_str(
            r#"{
                "kind": "upsertUnit",
                "unit": {
                    "id": 2,
                    "name": "Mixing Unit 2",
                    "audioLink": "follow",
                    "switcherSceneFilter": "exclude",
                    "switcherSceneIds": [1]
                }
            }"#,
        )
        .expect("Remote upsertUnit");
        apply(&mut doc, mutation).unwrap();
        let added = doc.units.iter().find(|item| item.id == 2).unwrap();
        assert_eq!(added.audio_link, crate::session::AudioLinkMode::Follow);
        assert_eq!(
            added.switcher_scene_filter,
            crate::session::SwitcherSceneFilter::Exclude
        );
        assert_eq!(added.switcher_scene_ids, vec![1]);
        assert_eq!(doc.next_unit_id, 3);
    }

    #[test]
    fn delete_missing_input_is_not_found() {
        let mut doc = bars();
        let err = apply(&mut doc, SessionMutation::DeleteInput { id: 99 }).unwrap_err();
        assert!(matches!(err, ControlError::NotFound { .. }));
    }

    #[test]
    fn upsert_multiview_appends_and_delete_removes() {
        let mut doc = bars();
        apply(
            &mut doc,
            SessionMutation::UpsertMultiview {
                layout: Box::new(crate::session::MultiviewDto {
                    id: 1,
                    name: "MV 1".into(),
                    preview_unit_id: 1,
                    program_unit_id: 1,
                    present_interval: 3,
                    tiles: vec![],
                    template: crate::session::MultiviewTemplate::PreviewProgram8,
                    preview_label_follow: true,
                    preview_label: String::new(),
                    program_label_follow: true,
                    program_label: String::new(),
                    label_anchor: None,
                    label_size: None,
                    label_unit: None,
                    always_on_top: true,
                }),
            },
        )
        .unwrap();
        assert_eq!(doc.multiviews.len(), 1);
        assert_eq!(doc.multiviews[0].name, "MV 1");
        apply(&mut doc, SessionMutation::DeleteMultiview { id: 1 }).unwrap();
        assert!(doc.multiviews.is_empty());
    }

    #[test]
    fn published_outputs_skip_input_and_disabled() {
        let mut doc = bars();
        doc.outputs.push(crate::session::OutputDto {
            id: 100,
            name: "PGM".into(),
            transport: crate::session::OutputTransport::Omt,
            source_kind: crate::session::OutputSourceKind::MuProgram,
            source_id: 0,
            unit_id: 1,
            use_gpu: true,
            enabled: true,
            audio_bus_id: 1,
            skip_encode_when_no_receivers: true,
            width: 0,
            height: 0,
            fps_num: 0,
            fps_den: 0,
        });
        doc.outputs.push(crate::session::OutputDto {
            id: 101,
            name: "In".into(),
            transport: crate::session::OutputTransport::Ndi,
            source_kind: crate::session::OutputSourceKind::Input,
            source_id: 2,
            unit_id: 1,
            use_gpu: false,
            enabled: true,
            audio_bus_id: 1,
            skip_encode_when_no_receivers: true,
            width: 0,
            height: 0,
            fps_num: 0,
            fps_den: 0,
        });
        let published = doc.published_video_outputs();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].name, "PGM");
    }

    #[test]
    fn set_settings_replaces_settings_and_outputs() {
        let mut doc = bars();
        doc.settings.renderer = crate::session::Renderer::Vulkan;
        doc.settings.last_session_path = Some("show.json".into());
        let mut settings = doc.settings.clone();
        settings.vmix_api_port = 9099;
        settings.renderer = crate::session::Renderer::Auto;
        settings.last_session_path = None;
        apply(
            &mut doc,
            SessionMutation::SetSettings {
                settings: Box::new(settings),
                outputs: vec![crate::session::OutputDto {
                    id: 100,
                    name: "PGM".into(),
                    transport: crate::session::OutputTransport::Omt,
                    source_kind: crate::session::OutputSourceKind::MuProgram,
                    source_id: 0,
                    unit_id: 1,
                    use_gpu: true,
                    enabled: true,
                    audio_bus_id: 1,
                    skip_encode_when_no_receivers: true,
                    width: 0,
                    height: 0,
                    fps_num: 0,
                    fps_den: 0,
                }],
                buses: vec![],
                headphone_copy_master: Some(true),
                next_output_id: 101,
                next_bus_id: 3,
            },
        )
        .unwrap();
        assert_eq!(doc.settings.vmix_api_port, 9099);
        assert_eq!(doc.settings.renderer, crate::session::Renderer::Vulkan);
        assert_eq!(doc.settings.last_session_path.as_deref(), Some("show.json"));
        assert_eq!(doc.outputs.len(), 1);
        assert!(doc.headphone_copy_master);
        assert_eq!(doc.next_output_id, 101);
    }

    #[test]
    fn set_settings_without_headphone_keeps_existing() {
        let mut doc = bars();
        doc.headphone_copy_master = true;
        let settings = doc.settings.clone();
        apply(
            &mut doc,
            SessionMutation::SetSettings {
                settings: Box::new(settings),
                outputs: vec![],
                buses: vec![],
                headphone_copy_master: None,
                next_output_id: 0,
                next_bus_id: 0,
            },
        )
        .unwrap();
        assert!(doc.headphone_copy_master);
    }

    #[test]
    fn relink_media_updates_unique_filename() {
        let root = std::env::temp_dir().join(format!("eiviz-mutate-relink-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("card.png");
        std::fs::write(&file, b"png").unwrap();
        let missing = root.join("gone").join("card.png");
        let json = format!(
            r#"{{
          "version": 2,
          "inputs": [{{ "id": 2, "name": "Card", "kind": "Still", "pathOrAddress": {} }}],
          "scenes": [{{ "id": 1, "name": "Scene 1", "layers": [{{ "inputId": 2, "width": 1, "height": 1 }}] }}],
          "units": [{{ "id": 1, "name": "MU 1" }}]
        }}"#,
            serde_json::to_string(&missing.to_string_lossy().into_owned()).unwrap()
        );
        let mut doc = parse(json.as_bytes()).unwrap();
        apply(
            &mut doc,
            SessionMutation::RelinkMedia {
                directories: vec![root.to_string_lossy().into_owned()],
            },
        )
        .unwrap();
        assert_eq!(
            doc.inputs[0].path_or_address.as_deref(),
            Some(file.to_string_lossy().as_ref())
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
