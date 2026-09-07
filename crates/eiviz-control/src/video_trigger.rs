use std::collections::{HashMap, HashSet};

use crate::live::LiveState;
use crate::session::{Document, InputKind, VideoPlayWhen, VideoTriggerWhen};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VideoRoles {
    pub on_program: bool,
    pub on_preview: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoAction {
    SeekZero,
    Pause,
    Play,
}

/// Mixer-owned video transport. GUI input-preview windows are not Preview.
pub fn collect_roles(doc: &Document, live: &LiveState) -> HashMap<u64, VideoRoles> {
    let mut roles = HashMap::new();
    for state in live.units.values() {
        mark(
            doc,
            &mut roles,
            &mut HashSet::new(),
            state.program_source,
            true,
            false,
        );
        mark(
            doc,
            &mut roles,
            &mut HashSet::new(),
            state.preview_source,
            false,
            true,
        );
        if state.mix > 0.001 {
            let incoming = if state.incoming_source != 0 {
                state.incoming_source
            } else {
                state.preview_source
            };
            mark(doc, &mut roles, &mut HashSet::new(), incoming, true, false);
        }
        for source in &state.overlay_sources {
            mark(doc, &mut roles, &mut HashSet::new(), *source, true, false);
        }
    }
    roles
}

pub fn tick(
    doc: &Document,
    live: &LiveState,
    previous: &mut HashMap<u64, VideoRoles>,
) -> Vec<(u64, VideoAction)> {
    let now_map = collect_roles(doc, live);
    let mut actions = Vec::new();
    for input in &doc.inputs {
        if input.kind != InputKind::Video {
            continue;
        }
        let now = now_map.get(&input.id).copied().unwrap_or_default();
        let prev = previous.get(&input.id).copied().unwrap_or_default();
        let rose_program = now.on_program && !prev.on_program;
        let fell_program = !now.on_program && prev.on_program;
        let rose_preview = now.on_preview && !prev.on_preview;
        let paused = matches_trigger(
            input.video_pause_when,
            rose_program,
            fell_program,
            rose_preview,
        );
        let restarted = matches_trigger(
            input.video_restart_when,
            rose_program,
            fell_program,
            rose_preview,
        );
        let play =
            restarted || should_play(input.video_play_when, rose_program, rose_preview, now, prev);
        if restarted {
            actions.push((input.id, VideoAction::SeekZero));
        }
        // Play/Restart on the same edge wins over Pause. Active + To Active Pause
        // used to freeze the clip the instant it went to Program.
        if play {
            actions.push((input.id, VideoAction::Play));
        } else if paused {
            actions.push((input.id, VideoAction::Pause));
        }
        previous.insert(input.id, now);
    }
    actions
}

fn should_play(
    when: VideoPlayWhen,
    rose_program: bool,
    rose_preview: bool,
    now: VideoRoles,
    prev: VideoRoles,
) -> bool {
    match when {
        VideoPlayWhen::OnActive => rose_program,
        VideoPlayWhen::OnPreview => rose_preview,
        VideoPlayWhen::Always => {
            (now.on_program || now.on_preview) && !(prev.on_program || prev.on_preview)
        }
        VideoPlayWhen::Never => false,
    }
}

fn matches_trigger(
    when: VideoTriggerWhen,
    rose_program: bool,
    fell_program: bool,
    rose_preview: bool,
) -> bool {
    match when {
        VideoTriggerWhen::OnActive => rose_program,
        VideoTriggerWhen::OnDeactivated => fell_program,
        VideoTriggerWhen::OnPreview => rose_preview,
        VideoTriggerWhen::Never => false,
    }
}

fn mark(
    doc: &Document,
    roles: &mut HashMap<u64, VideoRoles>,
    visited: &mut HashSet<u64>,
    source: u64,
    program: bool,
    preview: bool,
) {
    if source == 0 || !visited.insert(source) {
        return;
    }
    if source & crate::ids::SCENE_BASE == crate::ids::SCENE_BASE
        && let Some(scene) = doc
            .scenes
            .iter()
            .find(|scene| crate::ids::scene_gpu_id(scene.id) == source)
    {
        for layer in &scene.layers {
            mark(doc, roles, visited, layer.input_id, program, preview);
        }
        return;
    }
    let entry = roles.entry(source).or_default();
    entry.on_program |= program;
    entry.on_preview |= preview;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::UnitLiveState;
    use crate::session::parse;

    #[test]
    fn on_active_plays_when_program_rises() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Clip", "kind": "Video", "pathOrAddress": "clip.mp4", "videoPlayWhen": "OnActive" }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let doc = parse(src).unwrap();
        let mut live = LiveState::default();
        live.units.insert(
            1,
            UnitLiveState {
                program_source: crate::ids::scene_gpu_id(1),
                preview_source: 0,
                ..UnitLiveState::default()
            },
        );
        let mut prev = HashMap::new();
        let actions = tick(&doc, &live, &mut prev);
        assert!(actions.contains(&(2, VideoAction::Play)));
        let again = tick(&doc, &live, &mut prev);
        assert!(!again.contains(&(2, VideoAction::Play)));
    }

    #[test]
    fn never_restart_does_not_seek_when_program_rises() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Clip", "kind": "Video", "pathOrAddress": "clip.mp4", "videoPlayWhen": "Never", "videoRestartWhen": "Never" }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let doc = parse(src).unwrap();
        let mut live = LiveState::default();
        live.units.insert(
            1,
            UnitLiveState {
                program_source: crate::ids::scene_gpu_id(1),
                preview_source: 0,
                ..UnitLiveState::default()
            },
        );
        let mut prev = HashMap::new();
        let actions = tick(&doc, &live, &mut prev);
        assert!(!actions.contains(&(2, VideoAction::SeekZero)));
        assert!(!actions.contains(&(2, VideoAction::Play)));
        let again = tick(&doc, &live, &mut prev);
        assert!(again.is_empty());
    }

    #[test]
    fn on_active_play_wins_over_on_active_pause() {
        let src = br#"{
          "version": 2,
          "inputs": [{
            "id": 2,
            "name": "Clip",
            "kind": "Video",
            "pathOrAddress": "clip.mp4",
            "videoPlayWhen": "OnActive",
            "videoRestartWhen": "OnActive",
            "videoPauseWhen": "OnActive"
          }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let doc = parse(src).unwrap();
        let mut live = LiveState::default();
        live.units.insert(
            1,
            UnitLiveState {
                program_source: crate::ids::scene_gpu_id(1),
                preview_source: 0,
                ..UnitLiveState::default()
            },
        );
        let mut prev = HashMap::new();
        let actions = tick(&doc, &live, &mut prev);
        assert!(actions.contains(&(2, VideoAction::SeekZero)));
        assert!(actions.contains(&(2, VideoAction::Play)));
        assert!(!actions.contains(&(2, VideoAction::Pause)));
    }

    #[test]
    fn pause_on_active_still_freezes_when_play_is_preview() {
        let src = br#"{
          "version": 2,
          "inputs": [{
            "id": 2,
            "name": "Clip",
            "kind": "Video",
            "pathOrAddress": "clip.mp4",
            "videoPlayWhen": "OnPreview",
            "videoPauseWhen": "OnActive"
          }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let doc = parse(src).unwrap();
        let mut live = LiveState::default();
        live.units.insert(
            1,
            UnitLiveState {
                program_source: crate::ids::scene_gpu_id(1),
                preview_source: 0,
                ..UnitLiveState::default()
            },
        );
        let mut prev = HashMap::new();
        let actions = tick(&doc, &live, &mut prev);
        assert!(actions.contains(&(2, VideoAction::Pause)));
        assert!(!actions.contains(&(2, VideoAction::Play)));
    }

    #[test]
    fn always_play_is_edge_triggered() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Clip", "kind": "Video", "pathOrAddress": "clip.mp4", "videoPlayWhen": "Always" }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let doc = parse(src).unwrap();
        let mut live = LiveState::default();
        live.units.insert(
            1,
            UnitLiveState {
                program_source: crate::ids::scene_gpu_id(1),
                preview_source: 0,
                ..UnitLiveState::default()
            },
        );
        let mut prev = HashMap::new();
        let actions = tick(&doc, &live, &mut prev);
        assert!(actions.contains(&(2, VideoAction::Play)));
        let again = tick(&doc, &live, &mut prev);
        assert!(!again.contains(&(2, VideoAction::Play)));
    }

    #[test]
    fn colliding_scene_and_input_ids_do_not_recurse() {
        let src = br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Bars", "kind": "Bars" }],
          "scenes": [
            { "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] },
            { "id": 2, "name": "Scene 2", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }
          ],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#;
        let doc = parse(src).unwrap();
        let mut live = LiveState::default();
        live.units.insert(
            1,
            UnitLiveState {
                program_source: crate::ids::scene_gpu_id(2),
                preview_source: crate::ids::scene_gpu_id(1),
                ..UnitLiveState::default()
            },
        );
        let roles = collect_roles(&doc, &live);
        assert_eq!(roles.get(&2).copied().unwrap_or_default().on_program, true);
    }
}
