use crate::audio::AudioMatrix;
use crate::graph::MixingGraph;
use crate::ids::*;
use crate::input::{DeviceBinding, Input, InputSource};
use crate::mixing::MixingUnit;
use crate::output::{Multiview, MultiviewSource, Output};
use crate::scene::Scene;
use crate::{DomainError, Result};
use eiviz_time::FrameRate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;

/// Explicit compositor selection. Never switch at runtime without a command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompositorBackend {
    /// CI / golden-frame reference. Not a substitute for [`Self::Wgpu`].
    #[default]
    CpuReference,
    /// Production GPU path. Absence of a device is a hard error.
    Wgpu,
}

/// Configured response to a missing asset or device. Not an implicit source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissingMediaPolicy {
    #[default]
    Slate,
    LastGood,
    Fail,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoFormat {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub color: ColorSpace,
    pub interlaced: bool,
    pub bit_depth: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorSpace {
    Bt709Sdr,
    Bt2020Pq,
    Bt2020Hlg,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub id: ProjectId,
    pub name: String,
    pub video: VideoFormat,
    pub audio: AudioFormat,
    pub inputs: BTreeMap<InputId, Input>,
    pub scenes: BTreeMap<SceneId, Scene>,
    pub mixing_units: BTreeMap<MixingUnitId, MixingUnit>,
    pub outputs: BTreeMap<OutputId, Output>,
    pub multiviews: BTreeMap<MultiviewId, Multiview>,
    pub audio_matrix: AudioMatrix,
    pub device_bindings: BTreeMap<DeviceBindingId, DeviceBinding>,
    pub assets: BTreeMap<AssetId, AssetRef>,
    #[serde(default)]
    pub compositor: CompositorBackend,
    #[serde(default)]
    pub missing_media: MissingMediaPolicy,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssetRef {
    pub id: AssetId,
    pub original_name: String,
    pub sha256_hex: String,
    pub relative_path: String,
    pub missing: bool,
}

impl VideoFormat {
    pub fn hd_5994() -> Self {
        Self {
            width: 1920,
            height: 1080,
            frame_rate: eiviz_time::NTSC_5994,
            color: ColorSpace::Bt709Sdr,
            interlaced: false,
            bit_depth: 8,
        }
    }
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
        }
    }
}

impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        let mut project = Self {
            schema_version: SCHEMA_VERSION,
            id: ProjectId::new(),
            name: name.into(),
            video: VideoFormat::hd_5994(),
            audio: AudioFormat::default(),
            inputs: BTreeMap::new(),
            scenes: BTreeMap::new(),
            mixing_units: BTreeMap::new(),
            outputs: BTreeMap::new(),
            multiviews: BTreeMap::new(),
            audio_matrix: AudioMatrix::default(),
            device_bindings: BTreeMap::new(),
            assets: BTreeMap::new(),
            compositor: CompositorBackend::CpuReference,
            missing_media: MissingMediaPolicy::Slate,
        };
        let unit = MixingUnit::new("Mix 1");
        let output = Output {
            id: OutputId::new(),
            name: "Program Window".into(),
            owner: unit.id,
            kind: crate::OutputKind::ProgramWindow,
            enabled: true,
        };
        let mut unit = unit;
        unit.outputs.push(output.id);
        project.outputs.insert(output.id, output);
        project.mixing_units.insert(unit.id, unit);
        project
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version > SCHEMA_VERSION {
            return Err(DomainError::msg(format!(
                "unsupported schema {}",
                self.schema_version
            )));
        }
        for input in self.inputs.values() {
            match input.source {
                InputSource::Image { asset } | InputSource::Video { asset, .. } => {
                    if !self.assets.contains_key(&asset) {
                        return Err(DomainError::InvalidRef(format!(
                            "input {} -> asset {}",
                            input.id, asset
                        )));
                    }
                }
                InputSource::DeckLink { binding } | InputSource::AudioDevice { binding } => {
                    if !self.device_bindings.contains_key(&binding) {
                        return Err(DomainError::InvalidRef(format!(
                            "input {} -> device binding {}",
                            input.id, binding
                        )));
                    }
                }
                InputSource::MixFeed { unit, .. } => {
                    if !self.mixing_units.contains_key(&unit) {
                        return Err(DomainError::InvalidRef(format!(
                            "input {} -> mixing unit {}",
                            input.id, unit
                        )));
                    }
                }
                InputSource::ColorBars
                | InputSource::SolidColor { .. }
                | InputSource::Ndi { .. }
                | InputSource::Omt { .. } => {}
            }
        }
        for scene in self.scenes.values() {
            for item in &scene.items {
                if !self.inputs.contains_key(&item.input) {
                    return Err(DomainError::InvalidRef(format!(
                        "scene item {} -> input {}",
                        item.id, item.input
                    )));
                }
            }
        }
        for unit in self.mixing_units.values() {
            if let Some(s) = unit.program.scene
                && !self.scenes.contains_key(&s)
            {
                return Err(DomainError::InvalidRef("program scene".into()));
            }
            if let Some(s) = unit.preview.scene
                && !self.scenes.contains_key(&s)
            {
                return Err(DomainError::InvalidRef("preview scene".into()));
            }
            for overlay in &unit.overlays {
                if let Some(s) = overlay.scene
                    && !self.scenes.contains_key(&s)
                {
                    return Err(DomainError::InvalidRef("overlay scene".into()));
                }
            }
            for oid in &unit.outputs {
                match self.outputs.get(oid) {
                    None => return Err(DomainError::UnknownId(oid.to_string())),
                    Some(o) if o.owner != unit.id => {
                        return Err(DomainError::msg("output owner mismatch"));
                    }
                    _ => {}
                }
            }
            for multiview_id in &unit.multiviews {
                match self.multiviews.get(multiview_id) {
                    None => return Err(DomainError::UnknownId(multiview_id.to_string())),
                    Some(view) if view.owner != unit.id => {
                        return Err(DomainError::msg("multiview owner mismatch"));
                    }
                    _ => {}
                }
            }
        }
        for output in self.outputs.values() {
            let owner = self.mixing_units.get(&output.owner).ok_or_else(|| {
                DomainError::InvalidRef(format!("output {} owner {}", output.id, output.owner))
            })?;
            if !owner.outputs.contains(&output.id) {
                return Err(DomainError::InvalidRef(format!(
                    "output {} missing from owner {}",
                    output.id, output.owner
                )));
            }
        }
        for view in self.multiviews.values() {
            let owner = self.mixing_units.get(&view.owner).ok_or_else(|| {
                DomainError::InvalidRef(format!("multiview {} owner {}", view.id, view.owner))
            })?;
            if !owner.multiviews.contains(&view.id) {
                return Err(DomainError::InvalidRef(format!(
                    "multiview {} missing from owner {}",
                    view.id, view.owner
                )));
            }
            if view.columns == 0 || view.rows == 0 {
                return Err(DomainError::msg("multiview grid must be non-zero"));
            }
            let capacity = view
                .columns
                .checked_mul(view.rows)
                .ok_or_else(|| DomainError::Capacity("multiview grid overflow".into()))?;
            if view.tiles.len() > capacity as usize {
                return Err(DomainError::Capacity(format!(
                    "multiview {} has too many tiles",
                    view.id
                )));
            }
            let mut coordinates = std::collections::BTreeSet::new();
            for tile in &view.tiles {
                if tile.column >= view.columns || tile.row >= view.rows {
                    return Err(DomainError::InvalidRef(format!(
                        "multiview {} tile ({},{}) outside {}x{}",
                        view.id, tile.column, tile.row, view.columns, view.rows
                    )));
                }
                if !coordinates.insert((tile.column, tile.row)) {
                    return Err(DomainError::DuplicateId(format!(
                        "multiview {} tile ({},{})",
                        view.id, tile.column, tile.row
                    )));
                }
                match tile.source {
                    MultiviewSource::Black => {}
                    MultiviewSource::Input(input) if self.inputs.contains_key(&input) => {}
                    MultiviewSource::Preview(unit) | MultiviewSource::Program(unit)
                        if self.mixing_units.contains_key(&unit) => {}
                    MultiviewSource::Input(input) => {
                        return Err(DomainError::InvalidRef(format!(
                            "multiview {} input {}",
                            view.id, input
                        )));
                    }
                    MultiviewSource::Preview(unit) | MultiviewSource::Program(unit) => {
                        return Err(DomainError::InvalidRef(format!(
                            "multiview {} unit {}",
                            view.id, unit
                        )));
                    }
                }
            }
        }
        MixingGraph::assert_acyclic(self)?;
        Ok(())
    }

    pub fn insert_input(&mut self, input: Input) -> Result<()> {
        if self.inputs.contains_key(&input.id) {
            return Err(DomainError::DuplicateId(input.id.to_string()));
        }
        let id = input.id;
        self.inputs.insert(id, input);
        if let Err(error) = self.validate() {
            self.inputs.remove(&id);
            return Err(error);
        }
        Ok(())
    }

    pub fn insert_scene(&mut self, scene: Scene) -> Result<()> {
        if self.scenes.contains_key(&scene.id) {
            return Err(DomainError::DuplicateId(scene.id.to_string()));
        }
        let id = scene.id;
        self.scenes.insert(id, scene);
        if let Err(error) = self.validate() {
            self.scenes.remove(&id);
            return Err(error);
        }
        Ok(())
    }

    pub fn mixing_unit_mut(&mut self, id: MixingUnitId) -> Result<&mut MixingUnit> {
        self.mixing_units
            .get_mut(&id)
            .ok_or_else(|| DomainError::UnknownId(id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Transform2D;
    use crate::input::{Input, InputSource};
    use crate::scene::{Scene, SceneItem};

    #[test]
    fn default_project_validates() {
        let p = Project::new("demo");
        p.validate().unwrap();
        assert_eq!(p.video.frame_rate, eiviz_time::NTSC_5994);
    }

    #[test]
    fn rejects_unknown_scene_item_input() {
        let mut p = Project::new("demo");
        let scene = Scene {
            id: SceneId::new(),
            name: "s".into(),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: InputId::new(),
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        let id = scene.id;
        assert!(p.insert_scene(scene).is_err());
        assert!(!p.scenes.contains_key(&id));
    }

    #[test]
    fn cycle_is_rejected() {
        let mut p = Project::new("demo");
        let a_id = *p.mixing_units.keys().next().unwrap();
        let b = MixingUnit::new("Mix 2");
        let b_id = b.id;
        p.mixing_units.insert(b_id, b.clone());

        let feed_a = Input {
            id: InputId::new(),
            name: "feedA".into(),
            tags: vec![],
            groups: vec![],
            source: InputSource::MixFeed {
                unit: a_id,
                tap: crate::MixTap::Program,
            },
        };
        let feed_b = Input {
            id: InputId::new(),
            name: "feedB".into(),
            tags: vec![],
            groups: vec![],
            source: InputSource::MixFeed {
                unit: b_id,
                tap: crate::MixTap::Program,
            },
        };
        let scene_a = Scene {
            id: SceneId::new(),
            name: "sa".into(),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: feed_b.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        let scene_b = Scene {
            id: SceneId::new(),
            name: "sb".into(),
            items: vec![SceneItem {
                id: SceneItemId::new(),
                input: feed_a.id,
                transform: Transform2D::fullscreen(),
                z_order: 0,
                playback: Default::default(),
            }],
        };
        p.inputs.insert(feed_a.id, feed_a);
        p.inputs.insert(feed_b.id, feed_b);
        p.scenes.insert(scene_a.id, scene_a.clone());
        p.scenes.insert(scene_b.id, scene_b.clone());
        p.mixing_units.get_mut(&a_id).unwrap().program.scene = Some(scene_a.id);
        p.mixing_units.get_mut(&b_id).unwrap().program.scene = Some(scene_b.id);
        assert_eq!(p.validate(), Err(DomainError::Cycle));
        let _ = b;
    }

    #[test]
    fn compositor_and_missing_media_default_on_old_json() {
        let p = Project::new("demo");
        let mut v = serde_json::to_value(&p).unwrap();
        v.as_object_mut().unwrap().remove("compositor");
        v.as_object_mut().unwrap().remove("missing_media");
        let loaded: Project = serde_json::from_value(v).unwrap();
        assert_eq!(loaded.compositor, CompositorBackend::CpuReference);
        assert_eq!(loaded.missing_media, MissingMediaPolicy::Slate);
    }

    #[test]
    fn multiview_rejects_duplicate_and_out_of_grid_tiles() {
        let mut p = Project::new("multiview");
        let unit = *p.mixing_units.keys().next().unwrap();
        let view = Multiview {
            id: MultiviewId::new(),
            name: "bad".into(),
            owner: unit,
            columns: 2,
            rows: 1,
            tiles: vec![
                crate::MultiviewTile {
                    column: 0,
                    row: 0,
                    source: MultiviewSource::Black,
                },
                crate::MultiviewTile {
                    column: 0,
                    row: 0,
                    source: MultiviewSource::Black,
                },
            ],
        };
        p.multiviews.insert(view.id, view.clone());
        p.mixing_units
            .get_mut(&unit)
            .unwrap()
            .multiviews
            .push(view.id);
        assert!(matches!(p.validate(), Err(DomainError::DuplicateId(_))));

        p.multiviews.get_mut(&view.id).unwrap().tiles = vec![crate::MultiviewTile {
            column: 2,
            row: 0,
            source: MultiviewSource::Black,
        }];
        assert!(matches!(p.validate(), Err(DomainError::InvalidRef(_))));
    }
}
