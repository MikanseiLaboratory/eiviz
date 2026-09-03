use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::abi::{
    OverlayDesc, SAVE_ALWAYS_FULL, SAVE_ALWAYS_LOW, SAVE_FLAG_MULTIVIEW,
    SAVE_NOT_ON_PREVIEW_OR_PROGRAM, SAVE_NOT_ON_PROGRAM, SRC_KIND_INPUT, SRC_KIND_MU_MULTIVIEW,
    SRC_KIND_SCENE, is_scene, mixing_unit_from_source,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceRoles {
    pub on_program: bool,
    pub on_preview: bool,
    pub on_multiview: bool,
}

impl SourceRoles {
    fn mark_program(&mut self) {
        self.on_program = true;
    }

    fn mark_preview(&mut self) {
        self.on_preview = true;
    }

    fn mark_multiview(&mut self) {
        self.on_multiview = true;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LiveSave {
    pub mode: u32,
    pub flags: u32,
}

impl Default for LiveSave {
    fn default() -> Self {
        Self {
            mode: SAVE_NOT_ON_PREVIEW_OR_PROGRAM,
            flags: 0,
        }
    }
}

/// Hold full quality this long after the source leaves Preview/Program so TAKE
/// and T-bar flicker do not thrash OMT metadata or recreate the NDI receiver.
pub const DROP_FULL_HOLD: Duration = Duration::from_millis(280);

/// Raise immediately; drop to low only after `DROP_FULL_HOLD` of continuous unused.
pub fn debounce_want_full(want: bool, drop_at: &mut Option<Instant>) -> bool {
    if want {
        *drop_at = None;
        return true;
    }
    let started = drop_at.get_or_insert_with(Instant::now);
    Instant::now().saturating_duration_since(*started) < DROP_FULL_HOLD
}

pub fn want_full(save: LiveSave, roles: SourceRoles) -> bool {
    let keep_mv = save.flags & SAVE_FLAG_MULTIVIEW != 0 && roles.on_multiview;
    match save.mode {
        SAVE_ALWAYS_LOW => false,
        SAVE_ALWAYS_FULL => true,
        SAVE_NOT_ON_PROGRAM => roles.on_program || keep_mv,
        SAVE_NOT_ON_PREVIEW_OR_PROGRAM => roles.on_program || roles.on_preview || keep_mv,
        _ => roles.on_program || roles.on_preview || keep_mv,
    }
}

pub fn collect_source_roles(
    scene_specs: &[(u64, u32, u32, Arc<[OverlayDesc]>, crate::MvLabelStyle)],
    snapshot: &[crate::abi::UnitSnap],
    monitor_sources: &[u64],
    outputs: &[(u32, u64)],
) -> HashMap<u64, SourceRoles> {
    let spec_map: HashMap<u64, &[OverlayDesc]> = scene_specs
        .iter()
        .map(|spec| (spec.0, spec.3.as_ref()))
        .collect();
    let mut roles = HashMap::<u64, SourceRoles>::new();
    for (_, _, _, _, _, state, _, _) in snapshot {
        add(state.program_source, Role::Program, &spec_map, &mut roles);
        add(state.preview_source, Role::Preview, &spec_map, &mut roles);
        if state.mix > 0.001 {
            add(state.mix_incoming(), Role::Program, &spec_map, &mut roles);
        }
        for overlay in state.overlays.iter().take(state.overlay_count as usize) {
            add(overlay.source_id, Role::Program, &spec_map, &mut roles);
        }
        for slot in state.mv_slots.iter().take(state.mv_slot_count as usize) {
            add(*slot, Role::Multiview, &spec_map, &mut roles);
        }
    }
    for id in monitor_sources {
        add(*id, Role::Preview, &spec_map, &mut roles);
    }
    for &(kind, source_id) in outputs {
        match kind {
            SRC_KIND_INPUT => add(source_id, Role::Program, &spec_map, &mut roles),
            SRC_KIND_SCENE | SRC_KIND_MU_MULTIVIEW => {
                add(source_id, Role::Multiview, &spec_map, &mut roles)
            }
            _ => {}
        }
    }
    roles
}

#[derive(Clone, Copy)]
enum Role {
    Program,
    Preview,
    Multiview,
}

fn add(
    id: u64,
    role: Role,
    spec_map: &HashMap<u64, &[OverlayDesc]>,
    roles: &mut HashMap<u64, SourceRoles>,
) {
    if crate::abi::is_multiview(id) {
        return;
    }
    if is_scene(id) {
        if let Some(layers) = spec_map.get(&id) {
            for layer in *layers {
                add(layer.source_id, role, spec_map, roles);
            }
        }
        return;
    }
    if mixing_unit_from_source(id).is_some() {
        return;
    }
    if id == 0 {
        return;
    }
    let entry = roles.entry(id).or_default();
    match role {
        Role::Program => entry.mark_program(),
        Role::Preview => entry.mark_preview(),
        Role::Multiview => entry.mark_multiview(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{SCENE_BASE, SRC_BLUE, SRC_COLOR, UnitState};

    fn scene(id: u64, layers: &[u64]) -> (u64, u32, u32, Arc<[OverlayDesc]>, crate::MvLabelStyle) {
        let overlays: Arc<[OverlayDesc]> = layers
            .iter()
            .map(|source_id| OverlayDesc {
                source_id: *source_id,
                opacity: 1.0,
                ..OverlayDesc::default()
            })
            .collect();
        (
            SCENE_BASE | id,
            1920,
            1080,
            overlays,
            crate::MvLabelStyle::default(),
        )
    }

    fn unit(program: u64, preview: u64, mix: f32) -> crate::abi::UnitSnap {
        (
            1,
            1920,
            1080,
            60_000,
            1_001,
            UnitState {
                program_source: program,
                preview_source: preview,
                mix,
                ..UnitState::default()
            },
            0,
            None,
        )
    }

    #[test]
    fn nested_scene_on_program_counts_inputs() {
        let specs = [scene(1, &[SRC_COLOR, 20])];
        let snapshot = [unit(SCENE_BASE | 1, SRC_BLUE, 0.0)];
        let roles = collect_source_roles(&specs, &snapshot, &[], &[]);
        assert!(roles.get(&20).is_some_and(|item| item.on_program));
        assert!(!roles.get(&20).is_some_and(|item| item.on_preview));
        assert!(roles.get(&SRC_BLUE).is_some_and(|item| item.on_preview));
        assert!(!roles.get(&SRC_BLUE).is_some_and(|item| item.on_program));
    }

    #[test]
    fn input_monitor_counts_as_preview() {
        let roles = collect_source_roles(&[], &[], &[20, SRC_COLOR], &[]);
        assert!(
            roles
                .get(&20)
                .is_some_and(|item| item.on_preview && !item.on_program)
        );
        assert!(roles.get(&SRC_COLOR).is_some_and(|item| item.on_preview));
    }

    #[test]
    fn tbar_mix_puts_preview_on_program() {
        let snapshot = [unit(SRC_COLOR, 20, 0.4)];
        let roles = collect_source_roles(&[], &snapshot, &[], &[]);
        assert!(
            roles
                .get(&20)
                .is_some_and(|item| item.on_program && item.on_preview)
        );
    }

    #[test]
    fn save_modes_match_issue_options() {
        let unused = SourceRoles::default();
        let pgm = SourceRoles {
            on_program: true,
            ..SourceRoles::default()
        };
        let prv = SourceRoles {
            on_preview: true,
            ..SourceRoles::default()
        };
        let mv = SourceRoles {
            on_multiview: true,
            ..SourceRoles::default()
        };
        let always_low = LiveSave {
            mode: SAVE_ALWAYS_LOW,
            flags: 0,
        };
        let not_pgm = LiveSave {
            mode: SAVE_NOT_ON_PROGRAM,
            flags: 0,
        };
        let not_pvw = LiveSave {
            mode: SAVE_NOT_ON_PREVIEW_OR_PROGRAM,
            flags: 0,
        };
        let always_full = LiveSave {
            mode: SAVE_ALWAYS_FULL,
            flags: 0,
        };
        let not_pvw_mv = LiveSave {
            mode: SAVE_NOT_ON_PREVIEW_OR_PROGRAM,
            flags: SAVE_FLAG_MULTIVIEW,
        };
        assert!(!want_full(always_low, pgm));
        assert!(want_full(always_full, unused));
        assert!(!want_full(not_pgm, unused));
        assert!(!want_full(not_pgm, prv));
        assert!(want_full(not_pgm, pgm));
        assert!(!want_full(not_pvw, unused));
        assert!(want_full(not_pvw, prv));
        assert!(!want_full(not_pvw, mv));
        assert!(want_full(not_pvw_mv, mv));
    }

    #[test]
    fn debounce_raises_immediately_and_holds_drop() {
        let mut drop_at = None;
        assert!(debounce_want_full(true, &mut drop_at));
        assert!(debounce_want_full(false, &mut drop_at));
        std::thread::sleep(DROP_FULL_HOLD + Duration::from_millis(40));
        assert!(!debounce_want_full(false, &mut drop_at));
        assert!(debounce_want_full(true, &mut drop_at));
        assert!(drop_at.is_none());
    }
}
