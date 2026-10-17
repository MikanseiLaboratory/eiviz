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
            let is_replay = document
                .inputs
                .iter()
                .any(|item| item.id == id && item.kind == InputKind::Replay);
            if is_replay {
                let only_channel = document
                    .replay
                    .as_ref()
                    .is_some_and(|replay| replay.channels.len() == 1);
                if only_channel {
                    return Err(ControlError::invalid(
                        "removing the last Replay input requires DeleteReplay",
                    ));
                }
                // The cue goes with the channel it names; a step that cues a
                // channel nobody owns would make the session unloadable.
                let index = document.replay.as_ref().and_then(|replay| {
                    replay
                        .channels
                        .iter()
                        .find(|channel| channel.input_id == id)
                        .map(|channel| channel.index)
                });
                crate::session::replay::remove_channel_input(document, id);
                if let Some(index) = index {
                    crate::session::replay::clear_cues_for_channel(document, index);
                }
            }
            remove_input(document, id);
            Ok(())
        }
        SessionMutation::UpsertScene { scene } => upsert_scene(document, *scene),
        SessionMutation::DeleteScene { id } => {
            if !document.scenes.iter().any(|item| item.id == id) {
                return Err(ControlError::not_found(format!("scene {id}")));
            }
            // An angle records a scene's composited output, so the scene is not the
            // operator's to delete while Replay depends on it.
            if let Some(angle) = document
                .replay
                .as_ref()
                .and_then(|replay| replay.angles.iter().find(|angle| angle.scene_id == id))
            {
                return Err(ControlError::invalid(format!(
                    "scene {id} is recorded as Replay angle {}; remove the angle first",
                    angle.id
                )));
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
            // An angle may record a unit's program mix, so the unit is not the
            // operator's to delete while Replay listens to it.
            if let Some(angle) = document.replay.as_ref().and_then(|replay| {
                replay.angles.iter().find(|angle| {
                    angle.audio == crate::session::replay::AngleAudio::Unit
                        && angle.audio_unit_id == id
                })
            }) {
                return Err(ControlError::invalid(format!(
                    "mixing unit {id} records Replay angle {}; set the angle to another unit or silence first",
                    angle.id
                )));
            }
            document.units.retain(|item| item.id != id);
            if document.selected_unit_id == id {
                document.selected_unit_id = document.units.first().map(|item| item.id).unwrap_or(1);
            }
            Ok(())
        }
        SessionMutation::UpsertOverlay { slot } => upsert_overlay(document, *slot),
        SessionMutation::DeleteOverlay { id } => {
            if !document.overlays.iter().any(|item| item.id == id) {
                return Err(ControlError::not_found(format!("overlay {id}")));
            }
            document.overlays.retain(|item| item.id != id);
            for unit in &mut document.units {
                unit.overlays_on_air.retain(|item| *item != id);
            }
            Ok(())
        }
        SessionMutation::SetTransitions { presets } => {
            document.transitions = presets;
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
                audio_units: vec![document.units.first().map(|unit| unit.id).unwrap_or(1)],
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
                audio_capture_mode: crate::session::AudioCaptureMode::Mic,
                audio_device_kind: crate::session::AudioDeviceKind::None,
                audio_device_id: String::new(),
                audio_map_left: 0,
                audio_map_right: 1,
                audio_process_exe: String::new(),
                audio_process_aumid: String::new(),
                decklink_mode: String::new(),
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
            headphone,
            headphone_copy_monitor,
            next_output_id,
            stream_encoders,
            next_stream_encoder_id,
        } => {
            let renderer = document.settings.renderer;
            let last_session_path = document.settings.last_session_path.clone();
            document.settings = *settings;
            document.settings.renderer = renderer;
            document.settings.last_session_path = last_session_path;
            document.outputs = outputs;
            if let Some(headphone) = headphone {
                document.headphone = headphone;
            }
            if let Some(enabled) = headphone_copy_monitor {
                document.headphone_copy_monitor = enabled;
            }
            if next_output_id != 0 {
                document.next_output_id = next_output_id;
            }
            if let Some(encoders) = stream_encoders {
                document.stream_encoders = encoders;
            }
            if next_stream_encoder_id != 0 {
                document.next_stream_encoder_id = next_stream_encoder_id;
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
        SessionMutation::UpsertReplaySettings { replay } => {
            // Replay is created only by adding a Replay input. Settings may be
            // edited afterwards, never invented.
            if document.replay.is_none() {
                return Err(ControlError::invalid(
                    "add a Replay input before editing Replay settings",
                ));
            }
            if replay.channels.is_empty() {
                return Err(ControlError::invalid(
                    "Replay settings require an explicitly added Replay input",
                ));
            }
            document.replay = Some(*replay);
            Ok(())
        }
        SessionMutation::AddReplayChannel => {
            crate::session::replay::add_channel_input(document).map_err(ControlError::invalid)?;
            Ok(())
        }
        SessionMutation::DeleteReplay => {
            let Some(replay) = document.replay.take() else {
                return Err(ControlError::not_found("replay"));
            };
            for channel in replay.channels {
                remove_input(document, channel.input_id);
            }
            // Animation steps that cue a channel go with Replay: a cue to a channel
            // that no longer exists would make the session unloadable.
            crate::session::replay::clear_all_cues(document);
            Ok(())
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
    let kind = input.kind;
    let id = input.id;
    let was_replay = document
        .inputs
        .iter()
        .any(|item| item.id == id && item.kind == InputKind::Replay);
    if was_replay && kind != InputKind::Replay {
        // The channel and the replay document belong to a Replay input, so its
        // kind is not editable: a channel whose input changed kind is a session
        // nobody can load. Removing the input is the explicit way out.
        return Err(ControlError::invalid(
            "a Replay input keeps its kind; remove the input to free the channel",
        ));
    }
    // A Replay input always owns a channel, because Replay exists only while one
    // does. A client that adds the input itself gets the document here rather than
    // an input the Replay settings would refuse to edit. The channel is claimed
    // before the input list changes, so a refused id adds nothing.
    if kind == InputKind::Replay {
        crate::session::replay::adopt_channel_input(document, id).map_err(ControlError::invalid)?;
    }
    if let Some(existing) = document.inputs.iter_mut().find(|item| item.id == id) {
        *existing = input;
    } else {
        document.next_input_id = document.next_input_id.max(id.saturating_add(1));
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

fn upsert_overlay(
    document: &mut Document,
    mut slot: crate::session::OverlaySlot,
) -> ControlResult<()> {
    if slot.id == 0 {
        slot.id = document.next_overlay_id.max(1);
        document.next_overlay_id = slot.id.saturating_add(1);
    } else {
        document.next_overlay_id = document.next_overlay_id.max(slot.id.saturating_add(1));
    }
    if let Some(existing) = document.overlays.iter_mut().find(|item| item.id == slot.id) {
        *existing = slot;
    } else {
        document.overlays.push(slot);
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

/// Drops an input and every scene layer that referenced it.
fn remove_input(document: &mut Document, id: u64) {
    document.inputs.retain(|item| item.id != id);
    for scene in &mut document.scenes {
        scene.layers.retain(|layer| layer.input_id != id);
    }
}

fn next_input_id(document: &Document) -> u64 {
    let max = document
        .inputs
        .iter()
        .map(|item| item.id)
        .max()
        .unwrap_or(9);
    crate::ids::skip_reserved_bands(document.next_input_id.max(max.saturating_add(1)).max(10))
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
            audio_unit_id: 1,
            skip_encode_when_no_receivers: true,
            width: 0,
            height: 0,
            fps_num: 0,
            fps_den: 0,
            decklink_device: String::new(),
            decklink_mode: String::new(),
            decklink_external_key: false,
            rtmp_url: String::new(),
            rtmp_secret_ref: String::new(),
            stream_encoder_id: 0,
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
            audio_unit_id: 1,
            skip_encode_when_no_receivers: true,
            width: 0,
            height: 0,
            fps_num: 0,
            fps_den: 0,
            decklink_device: String::new(),
            decklink_mode: String::new(),
            decklink_external_key: false,
            rtmp_url: String::new(),
            rtmp_secret_ref: String::new(),
            stream_encoder_id: 0,
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
                    audio_unit_id: 1,
                    skip_encode_when_no_receivers: true,
                    width: 0,
                    height: 0,
                    fps_num: 0,
                    fps_den: 0,
                    decklink_device: String::new(),
                    decklink_mode: String::new(),
                    decklink_external_key: false,
                    rtmp_url: String::new(),
                    rtmp_secret_ref: String::new(),
                    stream_encoder_id: 0,
                }],
                headphone: None,
                headphone_copy_monitor: Some(true),
                stream_encoders: None,
                next_stream_encoder_id: 0,
                next_output_id: 101,
            },
        )
        .unwrap();
        assert_eq!(doc.settings.vmix_api_port, 9099);
        assert_eq!(doc.settings.renderer, crate::session::Renderer::Vulkan);
        assert_eq!(doc.settings.last_session_path.as_deref(), Some("show.json"));
        assert_eq!(doc.outputs.len(), 1);
        assert!(doc.headphone_copy_monitor);
        assert_eq!(doc.next_output_id, 101);
    }

    #[test]
    fn set_settings_without_headphone_keeps_existing() {
        let mut doc = bars();
        doc.headphone_copy_monitor = true;
        let settings = doc.settings.clone();
        apply(
            &mut doc,
            SessionMutation::SetSettings {
                settings: Box::new(settings),
                outputs: vec![],
                headphone: None,
                headphone_copy_monitor: None,
                next_output_id: 0,
                stream_encoders: None,
                next_stream_encoder_id: 0,
            },
        )
        .unwrap();
        assert!(doc.headphone_copy_monitor);
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

    fn bars_with_replay() -> Document {
        let mut doc = bars();
        crate::session::replay::add_channel_input(&mut doc).unwrap();
        doc
    }

    #[test]
    fn delete_replay_removes_the_document_and_every_input() {
        let mut doc = bars_with_replay();
        crate::session::replay::add_channel_input(&mut doc).unwrap();
        assert_eq!(doc.inputs.len(), 3);
        apply(&mut doc, SessionMutation::DeleteReplay).unwrap();
        assert!(doc.replay.is_none());
        assert!(!doc.inputs.iter().any(|item| item.kind == InputKind::Replay));
    }

    #[test]
    fn delete_replay_without_replay_is_not_found() {
        let mut doc = bars();
        let err = apply(&mut doc, SessionMutation::DeleteReplay).unwrap_err();
        assert!(matches!(err, ControlError::NotFound { .. }));
    }

    #[test]
    fn deleting_the_last_channel_points_at_delete_replay() {
        let mut doc = bars_with_replay();
        let id = doc.inputs.last().unwrap().id;
        let err = apply(&mut doc, SessionMutation::DeleteInput { id }).unwrap_err();
        assert!(err.message().contains("DeleteReplay"), "{}", err.message());
    }

    #[test]
    fn replay_settings_cannot_create_the_document() {
        let mut doc = bars();
        let err = apply(
            &mut doc,
            SessionMutation::UpsertReplaySettings {
                replay: Box::new(crate::session::replay::ReplayDto::default()),
            },
        )
        .unwrap_err();
        assert!(
            err.message().contains("add a Replay input"),
            "{}",
            err.message()
        );
        assert!(doc.replay.is_none());
    }

    #[test]
    fn allocated_input_ids_never_enter_the_replay_bands() {
        let mut doc = bars_with_replay();
        // Pretend a host bumped the counter into the Replay channel band.
        doc.next_input_id = crate::ids::REPLAY_CHANNEL_BASE;
        let id = next_input_id(&doc);
        assert!(id >= crate::ids::REPLAY_RECORD_BASE + 0x1_0000, "{id:#x}");
    }

    /// A client that adds the input itself - which is what a remote host sends -
    /// must end up with the channel and the document, or the Replay settings would
    /// refuse to open on it.
    #[test]
    fn upserting_a_replay_input_creates_its_channel_and_document() {
        let mut doc = bars();
        let id = crate::session::replay::REPLAY_CHANNEL_BASE | 1;
        let mutation: SessionMutation = serde_json::from_str(&format!(
            r#"{{ "kind": "upsertInput", "input": {{ "id": {id}, "name": "Replay A", "kind": "Replay" }} }}"#
        ))
        .expect("Remote upsertInput");
        apply(&mut doc, mutation).unwrap();

        let replay = doc.replay.as_ref().expect("the document follows the input");
        assert_eq!(replay.channels.len(), 1);
        assert_eq!(replay.channels[0].index, 0);
        assert_eq!(replay.channels[0].input_id, id);
        assert!(!replay.enabled, "adding an input never starts recording");

        // The settings the operator edits next are accepted now.
        let replay = replay.clone();
        apply(
            &mut doc,
            SessionMutation::UpsertReplaySettings {
                replay: Box::new(replay),
            },
        )
        .unwrap();
    }

    /// Two channels are two inputs, and each keeps its own letter.
    #[test]
    fn a_second_replay_input_takes_the_next_channel() {
        let mut doc = bars();
        let first = crate::session::replay::REPLAY_CHANNEL_BASE | 1;
        let second = crate::session::replay::REPLAY_CHANNEL_BASE | 2;
        for (id, name) in [(first, "Replay A"), (second, "Replay B")] {
            let mutation: SessionMutation = serde_json::from_str(&format!(
                r#"{{ "kind": "upsertInput", "input": {{ "id": {id}, "name": "{name}", "kind": "Replay" }} }}"#
            ))
            .unwrap();
            apply(&mut doc, mutation).unwrap();
        }
        let replay = doc.replay.as_ref().unwrap();
        assert_eq!(replay.channels.len(), 2);
        assert_eq!(replay.channels[0].input_id, first);
        assert_eq!(replay.channels[1].input_id, second);
        // Re-upserting an input that already owns its channel changes nothing.
        let mutation: SessionMutation = serde_json::from_str(&format!(
            r#"{{ "kind": "upsertInput", "input": {{ "id": {first}, "name": "Replay A", "kind": "Replay" }} }}"#
        ))
        .unwrap();
        apply(&mut doc, mutation).unwrap();
        assert_eq!(doc.replay.as_ref().unwrap().channels.len(), 2);
    }

    /// An input that names an id outside the reserved band is not a Replay
    /// channel, and is refused before anything is added.
    #[test]
    fn a_replay_input_outside_the_band_adds_nothing() {
        let mut doc = bars();
        let mutation: SessionMutation = serde_json::from_str(
            r#"{ "kind": "upsertInput", "input": { "id": 99, "name": "Replay", "kind": "Replay" } }"#,
        )
        .unwrap();
        let err = apply(&mut doc, mutation).unwrap_err();
        assert!(
            err.message().contains("Replay channel id"),
            "{}",
            err.message()
        );
        assert!(doc.replay.is_none(), "no document was invented");
        assert!(
            !doc.inputs.iter().any(|item| item.id == 99),
            "no input landed"
        );
    }

    /// Changing a Replay input into another kind would leave its channel behind,
    /// which no session can load. The operator removes the input instead.    #[test]
    fn a_replay_input_cannot_change_its_kind() {
        let mut doc = bars();
        let id = crate::session::replay::REPLAY_CHANNEL_BASE | 1;
        let add: SessionMutation = serde_json::from_str(&format!(
            r#"{{ "kind": "upsertInput", "input": {{ "id": {id}, "name": "Replay A", "kind": "Replay" }} }}"#
        ))
        .unwrap();
        apply(&mut doc, add).unwrap();

        let change: SessionMutation = serde_json::from_str(&format!(
            r#"{{ "kind": "upsertInput", "input": {{ "id": {id}, "name": "Camera", "kind": "Color" }} }}"#
        ))
        .unwrap();
        let err = apply(&mut doc, change).unwrap_err();
        assert!(
            err.message().contains("keeps its kind"),
            "{}",
            err.message()
        );
        // The input and its channel are untouched.
        assert!(
            doc.inputs
                .iter()
                .any(|item| item.id == id && item.kind == InputKind::Replay)
        );
        assert_eq!(doc.replay.as_ref().unwrap().channels.len(), 1);
        crate::session::validate_for_apply(&doc).unwrap();
    }

    /// A scene sequence may cue a Replay channel. Removing what it names has to
    /// take the cue with it, or the session stops loading.
    #[test]
    fn deleting_replay_clears_the_animation_cues_that_named_it() {
        let mut doc = bars_with_replay();
        cue_channel(&mut doc, 0);
        apply(&mut doc, SessionMutation::DeleteReplay).unwrap();
        assert!(doc.replay.is_none());
        assert_eq!(doc.scenes[0].sequences[0].steps[0].replay, None);
        crate::session::validate_for_apply(&doc).unwrap();
    }

    /// Removing one channel clears only the cues that named that channel.
    #[test]
    fn removing_a_channel_clears_only_its_own_cues() {
        let mut doc = bars_with_replay();
        crate::session::replay::add_channel_input(&mut doc).unwrap();
        cue_channel(&mut doc, 0);
        cue_channel(&mut doc, 1);
        let second = doc.inputs.last().unwrap().id;
        apply(&mut doc, SessionMutation::DeleteInput { id: second }).unwrap();
        assert_eq!(doc.replay.as_ref().unwrap().channels.len(), 1);
        let steps = &doc.scenes[0].sequences[0].steps;
        assert_eq!(
            steps[0].replay.as_ref().map(|cue| cue.channel),
            Some(0),
            "the other channel's cue stays"
        );
        assert_eq!(
            steps[1].replay, None,
            "the removed channel's cue goes with it"
        );
        crate::session::validate_for_apply(&doc).unwrap();
    }

    /// An angle records a scene, so the scene is not free to delete while Replay
    /// uses it. The refusal names the angle instead of failing validation later.
    #[test]
    fn a_scene_that_replay_records_cannot_be_deleted() {
        use crate::session::replay::*;
        let mut doc = bars_with_replay();
        doc.replay.as_mut().unwrap().angles.push(AngleDto {
            id: 1,
            name: "Cam 1".into(),
            scene_id: 1,
            audio: AngleAudio::Silent,
            audio_unit_id: 0,
            record_mode: AngleRecordMode::Composite,
        });
        crate::session::validate_for_apply(&doc).unwrap();

        let err = apply(&mut doc, SessionMutation::DeleteScene { id: 1 }).unwrap_err();
        assert!(
            err.message().contains("Replay angle 1"),
            "{}",
            err.message()
        );
        assert!(
            doc.scenes.iter().any(|scene| scene.id == 1),
            "the scene stays"
        );
    }

    /// An angle may record a unit's program mix, so the unit is protected too.
    #[test]
    fn a_unit_that_replay_records_cannot_be_deleted() {
        use crate::session::replay::*;
        let mut doc = bars_with_replay();
        doc.units.push(UnitDto {
            id: 2,
            ..doc.units[0].clone()
        });
        doc.replay.as_mut().unwrap().angles.push(AngleDto {
            id: 1,
            name: "Cam 1".into(),
            scene_id: 1,
            audio: AngleAudio::Unit,
            audio_unit_id: 2,
            record_mode: AngleRecordMode::Composite,
        });
        crate::session::validate_for_apply(&doc).unwrap();

        let err = apply(&mut doc, SessionMutation::DeleteUnit { id: 2 }).unwrap_err();
        assert!(
            err.message().contains("Replay angle 1"),
            "{}",
            err.message()
        );
        assert!(doc.units.iter().any(|unit| unit.id == 2), "the unit stays");
    }

    /// One step that cues a channel, which is how the Scene editor drives Replay.
    /// Called twice, it appends a second step.
    fn cue_channel(doc: &mut Document, channel: u32) {
        use crate::session::{SceneSequence, SequenceReplayAction, SequenceStep};
        let scene = &mut doc.scenes[0];
        if scene.states.is_empty() {
            scene.states.push(crate::session::SceneState {
                id: 1,
                name: "Wide".into(),
                layers: Vec::new(),
                camera: None,
                enter: crate::session::Motion::default(),
            });
        }
        let step = SequenceStep {
            state_id: 1,
            motion: None,
            hold_frames: 10,
            replay: Some(SequenceReplayAction {
                channel,
                clip_uid: String::new(),
                play: true,
            }),
        };
        match scene.sequences.first_mut() {
            Some(sequence) => sequence.steps.push(step),
            None => scene.sequences.push(SceneSequence {
                id: 1,
                name: "Open".into(),
                steps: vec![step],
            }),
        }
    }
}
