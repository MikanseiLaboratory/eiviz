use crate::ids::{AudioBusId, InputId, MixingUnitId};
use serde::{Deserialize, Serialize};

/// Project-wide policy for crossing audio sample-clock domains.
///
/// A rate mismatch is never converted unless `Asrc` is persisted in the
/// project. `ExactRate` is also the deserialization default for legacy files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AudioResamplingPolicy {
    #[default]
    ExactRate,
    Asrc {
        profile: AsrcProfile,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsrcProfile {
    pub quality: AsrcQuality,
    /// Queue target used by the clock-drift servo.
    pub target_latency_ms: u32,
    /// Hard per-input bound. Old samples are discarded on overflow.
    pub max_buffer_ms: u32,
    /// Absolute correction limit for measured drift and queue steering.
    pub max_drift_ppm: u32,
}

impl Default for AsrcProfile {
    fn default() -> Self {
        Self::broadcast()
    }
}

impl AsrcProfile {
    pub const fn broadcast() -> Self {
        Self {
            quality: AsrcQuality::Broadcast,
            target_latency_ms: 40,
            max_buffer_ms: 250,
            max_drift_ppm: 2_000,
        }
    }

    pub const fn mastering() -> Self {
        Self {
            quality: AsrcQuality::Mastering,
            target_latency_ms: 80,
            max_buffer_ms: 500,
            max_drift_ppm: 1_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrcQuality {
    /// 32-tap Blackman-windowed sinc; bounded latency for live production.
    Broadcast,
    /// 64-tap Blackman-windowed sinc; higher stop-band rejection.
    Mastering,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioMatrix {
    pub buses: Vec<AudioBus>,
    pub routes: Vec<AudioRoute>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioBus {
    pub id: AudioBusId,
    pub name: String,
    pub linked_unit: Option<MixingUnitId>,
    pub gain_db: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioRoute {
    pub input: InputId,
    pub bus: AudioBusId,
    pub mode: RouteMode,
    pub gain_db: f32,
    pub muted: bool,
    pub solo: bool,
    pub delay_ms: f32,
    /// -1.0 = full left, 0.0 = center, 1.0 = full right.
    #[serde(default)]
    pub pan: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteMode {
    Manual,
    Follow { unit: MixingUnitId },
}

impl Default for AudioMatrix {
    fn default() -> Self {
        let master = AudioBus {
            id: AudioBusId::new(),
            name: "Master".into(),
            linked_unit: None,
            gain_db: 0.0,
        };
        Self {
            buses: vec![master],
            routes: Vec::new(),
        }
    }
}

impl AudioMatrix {
    /// Effective linear gain after mute/solo/follow. `follow_active` is computed
    /// by the runtime from the mixing unit snapshot.
    pub fn effective_linear_gain(&self, route: &AudioRoute, follow_active: bool) -> f32 {
        if route.muted {
            return 0.0;
        }
        let any_solo = self.routes.iter().any(|r| r.solo);
        if any_solo && !route.solo {
            return 0.0;
        }
        match route.mode {
            RouteMode::Follow { .. } if !follow_active => return 0.0,
            _ => {}
        }
        db_to_lin(route.gain_db)
    }
}

pub fn db_to_lin(db: f32) -> f32 {
    if db <= -120.0 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}
