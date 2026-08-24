use crate::audio::{AudioMatrix, AudioResamplingPolicy};
use crate::graph::MixingGraph;
use crate::ids::*;
use crate::input::{DeviceBinding, Input, InputSource, Playback};
use crate::mixing::MixingUnit;
use crate::output::{Multiview, MultiviewSource, Output, OutputVideoSource};
use crate::scene::Scene;
use crate::{DomainError, Result};
use eiviz_time::FrameRate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 6;

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

/// Persisted admission policy for auxiliary video work. Program is never a
/// target of this policy.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuxiliaryLoadSheddingPolicy {
    /// Render every admitted Preview and Multiview at the Program cadence and
    /// resolution. Timing pressure is diagnosed but never changes rendering.
    #[default]
    Disabled,
    /// Escalate through the explicitly configured auxiliary quality tiers.
    Thresholds(AuxiliaryLoadSheddingThresholds),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuxiliaryLoadSheddingThresholds {
    /// A frame at or below this deadline slack is overloaded.
    pub overload_slack_nanos: i64,
    /// A frame at or above this deadline slack is eligible for recovery.
    pub recover_slack_nanos: i64,
    /// Optional per-frame GPU time threshold. `None` disables GPU-time
    /// admission feedback without disabling deadline feedback.
    pub gpu_overload_nanos: Option<u64>,
    /// Healthy GPU time paired with `gpu_overload_nanos`.
    pub gpu_recover_nanos: Option<u64>,
    pub escalation_frames: u32,
    pub recovery_frames: u32,
    /// Ordered least-to-most restrictive. Nominal full quality is implicit.
    pub tiers: Vec<AuxiliaryQualityTier>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuxiliaryQualityTier {
    pub preview_cadence_divisor: u32,
    pub preview_resolution_divisor: u32,
    pub multiview_cadence_divisor: u32,
    pub multiview_resolution_divisor: u32,
}

impl AuxiliaryLoadSheddingPolicy {
    pub fn broadcast_default() -> Self {
        Self::Thresholds(AuxiliaryLoadSheddingThresholds {
            overload_slack_nanos: 1_000_000,
            recover_slack_nanos: 3_000_000,
            gpu_overload_nanos: Some(6_000_000),
            gpu_recover_nanos: Some(4_000_000),
            escalation_frames: 3,
            recovery_frames: 120,
            tiers: vec![
                AuxiliaryQualityTier {
                    preview_cadence_divisor: 2,
                    preview_resolution_divisor: 1,
                    multiview_cadence_divisor: 2,
                    multiview_resolution_divisor: 1,
                },
                AuxiliaryQualityTier {
                    preview_cadence_divisor: 2,
                    preview_resolution_divisor: 2,
                    multiview_cadence_divisor: 3,
                    multiview_resolution_divisor: 2,
                },
                AuxiliaryQualityTier {
                    preview_cadence_divisor: 4,
                    preview_resolution_divisor: 4,
                    multiview_cadence_divisor: 6,
                    multiview_resolution_divisor: 4,
                },
            ],
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoFormat {
    pub width: u32,
    pub height: u32,
    /// Logical output cadence. For interlaced formats this is the field
    /// cadence, so 1080i59.94 remains the exact `60000/1001` ratio.
    pub frame_rate: FrameRate,
    pub color: ColorSpace,
    pub interlaced: bool,
    #[serde(default)]
    pub field_order: Option<FieldOrder>,
    pub bit_depth: u8,
    #[serde(default)]
    pub color_conversion: ColorConversionPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorSpace {
    Bt709Sdr,
    Bt2020Pq,
    Bt2020Hlg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorMatrix {
    Bt601,
    Bt709,
    Bt2020NonConstantLuminance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorRange {
    Full,
    Limited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferFunction {
    Srgb,
    Bt709,
    Pq,
    Hlg,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorMetadata {
    pub matrix: ColorMatrix,
    pub range: ColorRange,
    pub transfer: TransferFunction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldOrder {
    TopFieldFirst,
    BottomFieldFirst,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldKind {
    Progressive,
    Top,
    Bottom,
}

/// A source color mismatch is rejected unless this policy explicitly
/// authorizes GPU conversion. The policy never changes the Program profile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorConversionPolicy {
    #[default]
    Exact,
    Gpu {
        tone_map: ToneMapPolicy,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToneMapPolicy {
    #[default]
    Disabled,
    /// Deterministic Reinhard HDR-to-SDR mapping. This is an
    /// operator-selected rendering policy, not a certification claim.
    HdrToSdr {
        source_peak_nits: u16,
        target_nits: u16,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
    #[serde(default)]
    pub resampling: AudioResamplingPolicy,
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
    #[serde(default)]
    pub auxiliary_load_shedding: AuxiliaryLoadSheddingPolicy,
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
            field_order: None,
            bit_depth: 8,
            color_conversion: ColorConversionPolicy::Exact,
        }
    }

    pub fn uhd_5994_sdr() -> Self {
        Self {
            width: 3840,
            height: 2160,
            ..Self::hd_5994()
        }
    }

    pub fn uhd_5994_hdr10_pq() -> Self {
        Self {
            width: 3840,
            height: 2160,
            color: ColorSpace::Bt2020Pq,
            bit_depth: 10,
            ..Self::hd_5994()
        }
    }

    pub fn uhd_5994_hlg() -> Self {
        Self {
            width: 3840,
            height: 2160,
            color: ColorSpace::Bt2020Hlg,
            bit_depth: 10,
            ..Self::hd_5994()
        }
    }

    pub fn hd_interlaced_5994(order: FieldOrder) -> Self {
        Self {
            interlaced: true,
            field_order: Some(order),
            ..Self::hd_5994()
        }
    }

    pub fn color_metadata(&self) -> ColorMetadata {
        self.color.metadata()
    }

    pub fn field_at(&self, boundary_index: u64) -> FieldKind {
        match self.field_order {
            None => FieldKind::Progressive,
            Some(FieldOrder::TopFieldFirst) if boundary_index.is_multiple_of(2) => FieldKind::Top,
            Some(FieldOrder::TopFieldFirst) => FieldKind::Bottom,
            Some(FieldOrder::BottomFieldFirst) if boundary_index.is_multiple_of(2) => {
                FieldKind::Bottom
            }
            Some(FieldOrder::BottomFieldFirst) => FieldKind::Top,
        }
    }

    pub fn is_baseline_1080p5994(&self) -> bool {
        self.width == 1920
            && self.height == 1080
            && self.frame_rate == eiviz_time::NTSC_5994
            && self.color == ColorSpace::Bt709Sdr
            && !self.interlaced
            && self.field_order.is_none()
            && self.bit_depth == 8
            && self.color_conversion == ColorConversionPolicy::Exact
    }

    pub fn is_cpu_reference_compatible(&self) -> bool {
        self.width <= 1920
            && self.height <= 1080
            && self.frame_rate == eiviz_time::NTSC_5994
            && self.color == ColorSpace::Bt709Sdr
            && !self.interlaced
            && self.field_order.is_none()
            && self.bit_depth == 8
            && self.color_conversion == ColorConversionPolicy::Exact
    }

    pub fn working_bytes_per_pixel(&self) -> u64 {
        if self.bit_depth > 8 { 8 } else { 4 }
    }
}

impl ColorSpace {
    pub const fn metadata(self) -> ColorMetadata {
        match self {
            Self::Bt709Sdr => ColorMetadata {
                matrix: ColorMatrix::Bt709,
                range: ColorRange::Limited,
                transfer: TransferFunction::Bt709,
            },
            Self::Bt2020Pq => ColorMetadata {
                matrix: ColorMatrix::Bt2020NonConstantLuminance,
                range: ColorRange::Limited,
                transfer: TransferFunction::Pq,
            },
            Self::Bt2020Hlg => ColorMetadata {
                matrix: ColorMatrix::Bt2020NonConstantLuminance,
                range: ColorRange::Limited,
                transfer: TransferFunction::Hlg,
            },
        }
    }
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
            resampling: AudioResamplingPolicy::ExactRate,
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
            auxiliary_load_shedding: AuxiliaryLoadSheddingPolicy::Disabled,
        };
        let unit = MixingUnit::new("Mix 1");
        let output = Output {
            id: OutputId::new(),
            name: "Program Window".into(),
            owner: unit.id,
            video_source: OutputVideoSource::Program,
            kind: crate::OutputKind::ProgramWindow,
            enabled: true,
            distribution: None,
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
        if self.audio.sample_rate == 0 || self.audio.channels == 0 {
            return Err(DomainError::msg(
                "audio sample rate and channel count must be non-zero",
            ));
        }
        self.validate_video_format()?;
        self.validate_auxiliary_load_shedding()?;
        if let AudioResamplingPolicy::Asrc { profile } = self.audio.resampling
            && (profile.target_latency_ms == 0
                || profile.max_buffer_ms <= profile.target_latency_ms
                || profile.max_drift_ppm == 0
                || profile.max_drift_ppm > 10_000)
        {
            return Err(DomainError::msg(
                "ASRC profile requires non-zero latency/drift, a larger buffer, and at most 10000 ppm correction",
            ));
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
            if let InputSource::Video { playback, .. } = &input.source {
                validate_playback(playback)?;
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
                validate_playback(&item.playback)?;
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
            if let OutputVideoSource::Multiview(view_id) = output.video_source {
                let view = self.multiviews.get(&view_id).ok_or_else(|| {
                    DomainError::InvalidRef(format!(
                        "output {} -> multiview {}",
                        output.id, view_id
                    ))
                })?;
                if view.owner != output.owner {
                    return Err(DomainError::InvalidRef(format!(
                        "output {} and multiview {} must share owner {}",
                        output.id, view_id, output.owner
                    )));
                }
            }
            if let crate::OutputKind::DeckLink { binding }
            | crate::OutputKind::AudioDevice { binding } = output.kind
                && !self.device_bindings.contains_key(&binding)
            {
                return Err(DomainError::InvalidRef(format!(
                    "output {} -> device binding {}",
                    output.id, binding
                )));
            }
            validate_distribution_output(output, &self.audio)?;
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

    fn validate_auxiliary_load_shedding(&self) -> Result<()> {
        let AuxiliaryLoadSheddingPolicy::Thresholds(policy) = &self.auxiliary_load_shedding else {
            return Ok(());
        };
        if policy.overload_slack_nanos >= policy.recover_slack_nanos {
            return Err(DomainError::msg(
                "auxiliary shedding recovery slack must exceed overload slack",
            ));
        }
        if policy.escalation_frames == 0 || policy.recovery_frames == 0 {
            return Err(DomainError::msg(
                "auxiliary shedding hysteresis frame counts must be non-zero",
            ));
        }
        match (policy.gpu_overload_nanos, policy.gpu_recover_nanos) {
            (None, None) => {}
            (Some(overload), Some(recover)) if overload > recover => {}
            _ => {
                return Err(DomainError::msg(
                    "auxiliary shedding GPU thresholds must be paired and overload must exceed recovery",
                ));
            }
        }
        if policy.tiers.is_empty() {
            return Err(DomainError::msg(
                "auxiliary shedding requires at least one quality tier",
            ));
        }
        let mut previous = AuxiliaryQualityTier {
            preview_cadence_divisor: 1,
            preview_resolution_divisor: 1,
            multiview_cadence_divisor: 1,
            multiview_resolution_divisor: 1,
        };
        for tier in &policy.tiers {
            let values = [
                tier.preview_cadence_divisor,
                tier.preview_resolution_divisor,
                tier.multiview_cadence_divisor,
                tier.multiview_resolution_divisor,
            ];
            if values.contains(&0) {
                return Err(DomainError::msg(
                    "auxiliary shedding divisors must be non-zero",
                ));
            }
            let monotonic = tier.preview_cadence_divisor >= previous.preview_cadence_divisor
                && tier.preview_resolution_divisor >= previous.preview_resolution_divisor
                && tier.multiview_cadence_divisor >= previous.multiview_cadence_divisor
                && tier.multiview_resolution_divisor >= previous.multiview_resolution_divisor;
            if !monotonic || *tier == previous {
                return Err(DomainError::msg(
                    "auxiliary shedding tiers must become strictly more restrictive",
                ));
            }
            if self.video.width / tier.preview_resolution_divisor == 0
                || self.video.height / tier.preview_resolution_divisor == 0
                || self.video.width / tier.multiview_resolution_divisor == 0
                || self.video.height / tier.multiview_resolution_divisor == 0
            {
                return Err(DomainError::msg(
                    "auxiliary shedding resolution divisor exceeds the video format",
                ));
            }
            previous = *tier;
        }
        Ok(())
    }

    fn validate_video_format(&self) -> Result<()> {
        if self.video.width == 0 || self.video.height == 0 {
            return Err(DomainError::msg("video dimensions must be non-zero"));
        }
        if !matches!(self.video.bit_depth, 8 | 10) {
            return Err(DomainError::msg("video bit depth must be 8 or 10"));
        }
        if self.video.interlaced != self.video.field_order.is_some() {
            return Err(DomainError::msg(
                "interlaced video requires an explicit field order and progressive video must not have one",
            ));
        }
        if matches!(
            self.video.color,
            ColorSpace::Bt2020Pq | ColorSpace::Bt2020Hlg
        ) && self.video.bit_depth != 10
        {
            return Err(DomainError::msg(
                "PQ and HLG project profiles require 10-bit video",
            ));
        }
        if let ColorConversionPolicy::Gpu {
            tone_map:
                ToneMapPolicy::HdrToSdr {
                    source_peak_nits,
                    target_nits,
                },
        } = self.video.color_conversion
            && (source_peak_nits == 0 || target_nits == 0 || target_nits >= source_peak_nits)
        {
            return Err(DomainError::msg(
                "HDR-to-SDR tone mapping requires non-zero target below source peak",
            ));
        }
        if self.compositor == CompositorBackend::CpuReference
            && !self.video.is_cpu_reference_compatible()
        {
            return Err(DomainError::msg(
                "CpuReference accepts only reduced-raster or full 1080p59.94 SDR 8-bit reference profiles; extended video profiles require explicit Wgpu",
            ));
        }
        Ok(())
    }
}

fn validate_distribution_output(output: &Output, audio: &AudioFormat) -> Result<()> {
    use crate::{AacEncoderProfile, H264EncoderProfile, OutputKind, TransportProfile};

    let expected_transport = match &output.kind {
        OutputKind::Rtmp { url } => {
            validate_endpoint(url, "rtmp://", "RTMP")?;
            Some("rtmp")
        }
        OutputKind::Srt { url } => {
            validate_endpoint(url, "srt://", "SRT")?;
            Some("srt")
        }
        OutputKind::Mp4 { path } => {
            if path.trim().is_empty() {
                return Err(DomainError::msg("MP4 output path must not be empty"));
            }
            Some("mp4")
        }
        _ => None,
    };

    match (expected_transport, &output.distribution) {
        (None, None) => return Ok(()),
        (None, Some(_)) => {
            return Err(DomainError::msg(format!(
                "non-distribution output {} must not have a distribution profile",
                output.id
            )));
        }
        (Some(kind), None) => {
            return Err(DomainError::msg(format!(
                "{kind} output {} requires an explicit codec and transport profile",
                output.id
            )));
        }
        (Some(_), Some(_)) => {}
    }

    let profile = output.distribution.as_ref().expect("checked above");
    if profile.queue_capacity == 0 {
        return Err(DomainError::msg(
            "distribution queue capacity must be non-zero",
        ));
    }
    if profile.reconnect.initial_delay_ms == 0
        || profile.reconnect.max_delay_ms < profile.reconnect.initial_delay_ms
    {
        return Err(DomainError::msg(
            "invalid reconnect delay range for distribution output",
        ));
    }
    let (video_bitrate, keyframe_interval, adapter) = match &profile.video {
        H264EncoderProfile::CiscoOpenH26426 {
            bitrate_bps,
            keyframe_interval_frames,
            ..
        } => (*bitrate_bps, *keyframe_interval_frames, None),
        H264EncoderProfile::ExternalAnnexB {
            adapter,
            bitrate_bps,
            keyframe_interval_frames,
        } => (
            *bitrate_bps,
            *keyframe_interval_frames,
            Some(adapter.as_str()),
        ),
    };
    if video_bitrate == 0 || keyframe_interval == 0 {
        return Err(DomainError::msg(
            "distribution H.264 bitrate and keyframe interval must be non-zero",
        ));
    }
    if adapter.is_some_and(|adapter| adapter.trim().is_empty()) {
        return Err(DomainError::msg(
            "external H.264 encoder adapter name must not be empty",
        ));
    }
    let (audio_bitrate, sample_rate, channels) = match &profile.audio {
        AacEncoderProfile::FdkAacLc {
            bitrate_bps,
            sample_rate,
            channels,
            ..
        }
        | AacEncoderProfile::ExternalRawAacLc {
            bitrate_bps,
            sample_rate,
            channels,
            ..
        } => (*bitrate_bps, *sample_rate, *channels),
    };
    if let AacEncoderProfile::ExternalRawAacLc { adapter, .. } = &profile.audio
        && adapter.trim().is_empty()
    {
        return Err(DomainError::msg(
            "external AAC encoder adapter name must not be empty",
        ));
    }
    if audio_bitrate == 0 {
        return Err(DomainError::msg(
            "distribution AAC bitrate must be non-zero",
        ));
    }
    if sample_rate != audio.sample_rate || channels != audio.channels {
        return Err(DomainError::msg(format!(
            "distribution AAC profile {sample_rate} Hz/{channels} ch does not match project {} Hz/{} ch",
            audio.sample_rate, audio.channels
        )));
    }
    let transport_matches = matches!(
        (&output.kind, &profile.transport),
        (
            OutputKind::Rtmp { .. },
            TransportProfile::RtmpPublish { .. }
        ) | (
            OutputKind::Srt { .. },
            TransportProfile::SrtCallerMpegTs { .. }
        ) | (
            OutputKind::Mp4 { .. },
            TransportProfile::FragmentedMp4 { .. }
        )
    );
    if !transport_matches {
        return Err(DomainError::msg(format!(
            "output {} transport profile does not match its output kind",
            output.id
        )));
    }
    match &profile.transport {
        TransportProfile::RtmpPublish {
            chunk_size,
            connect_timeout_ms,
        } if !(128..=65_536).contains(chunk_size) || *connect_timeout_ms == 0 => {
            return Err(DomainError::msg(
                "RTMP chunk size must be 128..=65536 and timeout must be non-zero",
            ));
        }
        TransportProfile::SrtCallerMpegTs {
            latency_ms,
            connect_timeout_ms,
            ..
        } if *latency_ms == 0 || *connect_timeout_ms == 0 => {
            return Err(DomainError::msg(
                "SRT latency and connect timeout must be non-zero",
            ));
        }
        _ => {}
    }
    Ok(())
}

fn validate_endpoint(value: &str, scheme: &str, label: &str) -> Result<()> {
    if !value.starts_with(scheme) || value.len() == scheme.len() {
        return Err(DomainError::msg(format!(
            "{label} endpoint must use explicit {scheme} URL"
        )));
    }
    Ok(())
}

fn validate_playback(playback: &Playback) -> Result<()> {
    if !playback.speed.is_finite() || playback.speed <= 0.0 {
        return Err(DomainError::msg(
            "playback speed must be finite and greater than zero",
        ));
    }
    if playback
        .out_us
        .is_some_and(|out_us| out_us <= playback.in_us)
    {
        return Err(DomainError::msg(
            "playback out point must be after in point",
        ));
    }
    Ok(())
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
    fn extended_profiles_preserve_baseline_and_exact_color_metadata() {
        let baseline = VideoFormat::hd_5994();
        assert!(baseline.is_baseline_1080p5994());
        assert_eq!(
            baseline.color_metadata(),
            ColorMetadata {
                matrix: ColorMatrix::Bt709,
                range: ColorRange::Limited,
                transfer: TransferFunction::Bt709,
            }
        );
        let pq = VideoFormat::uhd_5994_hdr10_pq();
        assert_eq!(pq.frame_rate, eiviz_time::NTSC_5994);
        assert_eq!(pq.bit_depth, 10);
        assert_eq!(pq.color_metadata().transfer, TransferFunction::Pq);
        assert!(!pq.is_baseline_1080p5994());
    }

    #[test]
    fn interlaced_field_order_is_deterministic_at_rational_cadence() {
        let top = VideoFormat::hd_interlaced_5994(FieldOrder::TopFieldFirst);
        assert_eq!(top.frame_rate, eiviz_time::NTSC_5994);
        assert_eq!(top.field_at(0), FieldKind::Top);
        assert_eq!(top.field_at(1), FieldKind::Bottom);
        assert_eq!(top.field_at(1_000_000), FieldKind::Top);
        let bottom = VideoFormat::hd_interlaced_5994(FieldOrder::BottomFieldFirst);
        assert_eq!(bottom.field_at(0), FieldKind::Bottom);
        assert_eq!(bottom.field_at(1_000_001), FieldKind::Top);
        let time = eiviz_time::MediaTime::from_frame_index(1_000_000, top.frame_rate).unwrap();
        assert_eq!(
            time,
            eiviz_time::MediaTime::new(
                1_001_000_000,
                eiviz_time::Rational::new(1, 60_000).unwrap(),
            )
        );
    }

    #[test]
    fn invalid_hdr_and_field_profiles_are_rejected() {
        let mut project = Project::new("invalid video");
        project.compositor = CompositorBackend::Wgpu;
        project.video = VideoFormat::uhd_5994_hdr10_pq();
        project.video.bit_depth = 8;
        assert!(
            project
                .validate()
                .unwrap_err()
                .to_string()
                .contains("10-bit")
        );
        project.video = VideoFormat::hd_5994();
        project.video.interlaced = true;
        assert!(
            project
                .validate()
                .unwrap_err()
                .to_string()
                .contains("field order")
        );
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
    fn new_project_uses_explicit_exact_rate_audio_policy() {
        let project = Project::new("demo");
        assert_eq!(project.audio.resampling, AudioResamplingPolicy::ExactRate);
    }

    #[test]
    fn compositor_missing_media_audio_and_shedding_default_on_old_json() {
        let p = Project::new("demo");
        let mut v = serde_json::to_value(&p).unwrap();
        v.as_object_mut().unwrap().remove("compositor");
        v.as_object_mut().unwrap().remove("missing_media");
        v.as_object_mut().unwrap().remove("auxiliary_load_shedding");
        v["audio"].as_object_mut().unwrap().remove("resampling");
        for output in v["outputs"].as_object_mut().unwrap().values_mut() {
            output.as_object_mut().unwrap().remove("video_source");
        }
        let loaded: Project = serde_json::from_value(v).unwrap();
        assert_eq!(loaded.compositor, CompositorBackend::CpuReference);
        assert_eq!(loaded.missing_media, MissingMediaPolicy::Slate);
        assert_eq!(
            loaded.auxiliary_load_shedding,
            AuxiliaryLoadSheddingPolicy::Disabled
        );
        assert_eq!(loaded.audio.resampling, AudioResamplingPolicy::ExactRate);
        assert!(
            loaded
                .outputs
                .values()
                .all(|output| output.video_source == OutputVideoSource::Program)
        );
    }

    #[test]
    fn auxiliary_shedding_policy_validates_hysteresis_and_ordered_tiers() {
        let mut project = Project::new("shedding");
        project.auxiliary_load_shedding = AuxiliaryLoadSheddingPolicy::broadcast_default();
        project.validate().unwrap();

        let AuxiliaryLoadSheddingPolicy::Thresholds(policy) = &mut project.auxiliary_load_shedding
        else {
            unreachable!();
        };
        policy.recover_slack_nanos = policy.overload_slack_nanos;
        assert!(project.validate().is_err());
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

    #[test]
    fn distribution_outputs_require_matching_explicit_profiles() {
        let mut project = Project::new("distribution");
        let owner = *project.mixing_units.keys().next().unwrap();
        let output = Output {
            id: OutputId::new(),
            name: "RTMP".into(),
            owner,
            video_source: OutputVideoSource::Program,
            kind: crate::OutputKind::Rtmp {
                url: "rtmp://127.0.0.1/live/key".into(),
            },
            enabled: false,
            distribution: None,
        };
        project.outputs.insert(output.id, output.clone());
        project
            .mixing_units
            .get_mut(&owner)
            .unwrap()
            .outputs
            .push(output.id);
        assert!(
            project
                .validate()
                .unwrap_err()
                .to_string()
                .contains("explicit")
        );

        project.outputs.get_mut(&output.id).unwrap().distribution =
            Some(crate::DistributionProfile {
                video: crate::H264EncoderProfile::ExternalAnnexB {
                    adapter: "certified-h264".into(),
                    bitrate_bps: 8_000_000,
                    keyframe_interval_frames: 120,
                },
                audio: crate::AacEncoderProfile::ExternalRawAacLc {
                    adapter: "certified-aac".into(),
                    bitrate_bps: 192_000,
                    sample_rate: 48_000,
                    channels: 2,
                },
                transport: crate::TransportProfile::SrtCallerMpegTs {
                    latency_ms: 120,
                    stream_id: None,
                    connect_timeout_ms: 5_000,
                },
                queue_capacity: 128,
                reconnect: crate::ReconnectProfile {
                    initial_delay_ms: 100,
                    max_delay_ms: 5_000,
                    max_attempts: 0,
                },
            });
        assert!(
            project
                .validate()
                .unwrap_err()
                .to_string()
                .contains("does not match")
        );
        project
            .outputs
            .get_mut(&output.id)
            .unwrap()
            .distribution
            .as_mut()
            .unwrap()
            .transport = crate::TransportProfile::RtmpPublish {
            chunk_size: 4096,
            connect_timeout_ms: 5_000,
        };
        project.validate().unwrap();
    }
}
