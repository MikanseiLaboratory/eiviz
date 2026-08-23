use crate::DomainError;
use crate::ids::{InputId, MixingUnitId};
use crate::input::{InputSource, MixTap};
use crate::project::Project;
use std::collections::{HashMap, HashSet};

pub struct MixingGraph;

impl MixingGraph {
    pub fn edges(project: &Project) -> Vec<(MixingUnitId, MixingUnitId)> {
        let mut edges = Vec::new();
        for input in project.inputs.values() {
            if let InputSource::MixFeed {
                unit: src,
                tap: MixTap::Program | MixTap::Preview,
            } = input.source
            {
                for scene in project.scenes.values() {
                    if scene.items.iter().any(|it| it.input == input.id) {
                        for unit in project.mixing_units.values() {
                            if unit.program.scene == Some(scene.id)
                                || unit.preview.scene == Some(scene.id)
                                || unit.overlays.iter().any(|o| o.scene == Some(scene.id))
                            {
                                edges.push((src, unit.id));
                            }
                        }
                    }
                }
            }
        }
        edges
    }

    pub fn assert_acyclic(project: &Project) -> Result<(), DomainError> {
        let edges = Self::edges(project);
        let mut adj: HashMap<MixingUnitId, Vec<MixingUnitId>> = HashMap::new();
        for (src, dst) in edges {
            adj.entry(src).or_default().push(dst);
        }
        let mut visiting = HashSet::new();
        let mut visited = HashSet::new();
        for id in project.mixing_units.keys().copied() {
            if dfs(id, &adj, &mut visiting, &mut visited) {
                return Err(DomainError::Cycle);
            }
        }
        Ok(())
    }

    pub fn input_visible_on_program(project: &Project, unit: MixingUnitId, input: InputId) -> bool {
        let Some(u) = project.mixing_units.get(&unit) else {
            return false;
        };
        scene_uses(project, u.program.scene, input)
            || u.overlays
                .iter()
                .filter(|o| o.enabled)
                .any(|o| scene_uses(project, o.scene, input))
    }

    pub fn input_visible_on_preview(project: &Project, unit: MixingUnitId, input: InputId) -> bool {
        let Some(u) = project.mixing_units.get(&unit) else {
            return false;
        };
        scene_uses(project, u.preview.scene, input)
    }
}

fn scene_uses(project: &Project, scene: Option<crate::SceneId>, input: InputId) -> bool {
    scene
        .and_then(|id| project.scenes.get(&id))
        .map(|s| {
            s.items
                .iter()
                .any(|it| it.input == input && it.transform.visible())
        })
        .unwrap_or(false)
}

fn dfs(
    id: MixingUnitId,
    adj: &HashMap<MixingUnitId, Vec<MixingUnitId>>,
    visiting: &mut HashSet<MixingUnitId>,
    visited: &mut HashSet<MixingUnitId>,
) -> bool {
    if visited.contains(&id) {
        return false;
    }
    if !visiting.insert(id) {
        return true;
    }
    if let Some(next) = adj.get(&id) {
        for n in next {
            if dfs(*n, adj, visiting, visited) {
                return true;
            }
        }
    }
    visiting.remove(&id);
    visited.insert(id);
    false
}
