use crate::ids::{InputId, SceneId, SceneItemId};
use crate::input::Playback;
use crate::transform::Transform2D;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub id: SceneId,
    pub name: String,
    pub items: Vec<SceneItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneItem {
    pub id: SceneItemId,
    pub input: InputId,
    pub transform: Transform2D,
    pub z_order: i32,
    pub playback: Playback,
}

impl Scene {
    pub fn sorted_items(&self) -> Vec<&SceneItem> {
        let mut items: Vec<&SceneItem> = self.items.iter().collect();
        items.sort_by_key(|i| i.z_order);
        items
    }

    pub fn hit_test(&self, nx: f32, ny: f32) -> Option<SceneItemId> {
        self.sorted_items()
            .into_iter()
            .rev()
            .find(|item| item.transform.visible() && item.transform.contains_norm(nx, ny))
            .map(|item| item.id)
    }
}
