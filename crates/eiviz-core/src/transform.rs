use serde::{Deserialize, Serialize};

/// Normalized rectangle in parent space. Origin is top-left, x/y/w/h in 0..1
/// relative to the mixing unit canvas unless `pixel_space` is set.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform2D {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub rotation_deg: f32,
    pub opacity: f32,
    pub crop: Crop,
    pub pixel_space: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Crop {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Default for Crop {
    fn default() -> Self {
        Self {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        }
    }
}

impl Default for Transform2D {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            rotation_deg: 0.0,
            opacity: 1.0,
            crop: Crop::default(),
            pixel_space: false,
        }
    }
}

impl Transform2D {
    pub fn fullscreen() -> Self {
        Self::default()
    }

    pub fn visible(self) -> bool {
        self.opacity > 0.0 && self.width > 0.0 && self.height > 0.0
    }

    /// Axis-aligned hit test in normalized canvas coordinates.
    pub fn contains_norm(self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px <= self.x + self.width && py <= self.y + self.height
    }
}
