use crate::session::{Document, InputDto, InputKind, OverlaySlot, SceneDto, SceneLayer, UnitDto};
use serde::{Deserialize, Serialize};

/// Client-originated mutation. Live ops change Mix Effect state. Session ops
/// edit the canonical document. Ops never carry GPU handles or native surfaces.
#[derive(Debug, Clone)]
pub enum Command {
    Preview {
        unit_id: u64,
        scene_id: u64,
    },
    Cut {
        unit_id: u64,
        swap: bool,
        incoming: Incoming,
    },
    Auto {
        unit_id: u64,
        kind: u32,
        duration_ms: u32,
        swap: bool,
        keep_preview: bool,
        easing: u32,
        direction: u32,
        dip_r: f32,
        dip_g: f32,
        dip_b: f32,
        dip_a: f32,
        incoming: Incoming,
        softness: f32,
        param: f32,
    },
    SetMix {
        unit_id: u64,
        value: f32,
    },
    OverlayAuto {
        unit_id: u64,
        index: u32,
        duration_ms: u32,
        to_on: bool,
    },
    VideoPlay {
        input_id: u64,
        playing: bool,
    },
    VideoLoop {
        input_id: u64,
        looping: bool,
    },
    VideoSeek {
        input_id: u64,
        position_hns: i64,
    },
    AudioSetInput {
        input_id: u64,
        bus_mask: u32,
        gain: f32,
        mute: bool,
    },
    AudioSetBus {
        bus_id: u64,
        gain: f32,
        mute: bool,
    },
    ReplaceSession {
        document: Box<Document>,
        expected_revision: Option<u64>,
    },
    MutateSession {
        mutation: Box<SessionMutation>,
        expected_revision: Option<u64>,
    },
    Snapshot {
        unit_id: u64,
        kind: SnapshotKind,
        path: String,
    },
    Discover {
        kind: DiscoverKind,
        query: String,
    },
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Incoming {
    Preview,
    Program,
    Source(u64),
}

impl Incoming {
    pub fn from_u64(value: u64) -> Self {
        match value {
            0 => Self::Preview,
            u64::MAX => Self::Program,
            other => Self::Source(other),
        }
    }

    pub fn to_u64(self) -> u64 {
        match self {
            Self::Preview => 0,
            Self::Program => u64::MAX,
            Self::Source(id) => id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotKind {
    Program,
    Preview,
    Source(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoverKind {
    Omt,
    Ndi,
    Audio,
    Uvc,
    UvcModes,
}

impl DiscoverKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Omt => "omt",
            Self::Ndi => "ndi",
            Self::Audio => "audio",
            Self::Uvc => "uvc",
            Self::UvcModes => "uvcModes",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "omt" => Some(Self::Omt),
            "ndi" => Some(Self::Ndi),
            "audio" => Some(Self::Audio),
            "uvc" => Some(Self::Uvc),
            "uvcModes" | "uvc-modes" | "uvc_modes" => Some(Self::UvcModes),
            _ => None,
        }
    }
}

impl Command {
    pub fn is_live(&self) -> bool {
        matches!(
            self,
            Self::Preview { .. }
                | Self::Cut { .. }
                | Self::Auto { .. }
                | Self::SetMix { .. }
                | Self::OverlayAuto { .. }
                | Self::VideoPlay { .. }
                | Self::VideoLoop { .. }
                | Self::VideoSeek { .. }
                | Self::AudioSetInput { .. }
                | Self::AudioSetBus { .. }
        )
    }

    pub fn is_session(&self) -> bool {
        matches!(
            self,
            Self::ReplaceSession { .. } | Self::MutateSession { .. }
        )
    }

    pub fn requires_idempotency(&self) -> bool {
        matches!(
            self,
            Self::Cut { .. } | Self::ReplaceSession { .. } | Self::Shutdown
        )
    }

    pub fn expected_input_kind_for_path(kind: InputKind) -> bool {
        matches!(kind, InputKind::Still | InputKind::Video)
    }
}

/// Collaborative document edit. Applied against a staged clone, then replaced
/// through the same reconcile path as `ReplaceSession`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SessionMutation {
    UpsertInput {
        input: Box<InputDto>,
    },
    DeleteInput {
        id: u64,
    },
    UpsertScene {
        scene: Box<SceneDto>,
    },
    DeleteScene {
        id: u64,
    },
    SetSceneLayers {
        #[serde(alias = "scene_id")]
        scene_id: u64,
        layers: Vec<SceneLayer>,
    },
    UpsertUnit {
        unit: Box<UnitDto>,
    },
    DeleteUnit {
        id: u64,
    },
    SetOverlaySlot {
        #[serde(alias = "unit_id")]
        unit_id: u64,
        index: u32,
        slot: Box<OverlaySlot>,
    },
    AddMediaInput {
        name: String,
        #[serde(alias = "media_kind")]
        media_kind: InputKind,
        #[serde(alias = "host_path")]
        host_path: String,
        #[serde(alias = "video_loop")]
        video_loop: bool,
        tags: Vec<String>,
    },
    UpsertMultiview {
        layout: Box<crate::session::MultiviewDto>,
    },
    DeleteMultiview {
        id: u64,
    },
    SetSettings {
        settings: Box<crate::session::SessionSettings>,
        outputs: Vec<crate::session::OutputDto>,
        buses: Vec<crate::session::BusDto>,
        #[serde(default, alias = "headphone_copy_master")]
        headphone_copy_master: Option<bool>,
        #[serde(default, alias = "next_output_id")]
        next_output_id: u64,
        #[serde(default, alias = "next_bus_id")]
        next_bus_id: u64,
    },
}

impl SessionMutation {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::UpsertInput { .. } => "UpsertInput",
            Self::DeleteInput { .. } => "DeleteInput",
            Self::UpsertScene { .. } => "UpsertScene",
            Self::DeleteScene { .. } => "DeleteScene",
            Self::SetSceneLayers { .. } => "SetSceneLayers",
            Self::UpsertUnit { .. } => "UpsertUnit",
            Self::DeleteUnit { .. } => "DeleteUnit",
            Self::SetOverlaySlot { .. } => "SetOverlaySlot",
            Self::AddMediaInput { .. } => "AddMediaInput",
            Self::UpsertMultiview { .. } => "UpsertMultiview",
            Self::DeleteMultiview { .. } => "DeleteMultiview",
            Self::SetSettings { .. } => "SetSettings",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DiscoverKind, SessionMutation};
    use crate::session::{OutputSourceKind, OutputTransport};

    #[test]
    fn host_set_settings_json_uses_camel_case_fields() {
        let json = r#"{
            "kind": "setSettings",
            "settings": {
                "masterFpsNum": 60000,
                "masterFpsDen": 1001,
                "internalColorFormat": "Uyvy",
                "rebarOptimization": false,
                "vmixApiEnabledValue": true,
                "rebarOptimizationEnabled": false
            },
            "outputs": [{
                "id": 100,
                "name": "eiviz-pgm",
                "transport": "Omt",
                "sourceKind": "MuProgram",
                "sourceId": 0,
                "unitId": 1,
                "useGpu": true,
                "enabled": true,
                "audioBusId": 1,
                "skipEncodeWhenNoReceivers": true
            }, {
                "id": 101,
                "name": "eiviz-prv",
                "transport": "Omt",
                "sourceKind": "MuPreview",
                "unitId": 1,
                "useGpu": true,
                "enabled": true,
                "audioBusId": 1
            }],
            "buses": [{
                "id": 1,
                "name": "Master",
                "role": "Master",
                "deviceKind": "None",
                "deviceId": "",
                "mapLeft": 0,
                "mapRight": 1,
                "gain": 1,
                "mute": false
            }],
            "headphoneCopyMaster": false,
            "nextOutputId": 102,
            "nextBusId": 3
        }"#;
        let parsed: SessionMutation = serde_json::from_str(json).expect("host setSettings JSON");
        let SessionMutation::SetSettings {
            outputs,
            headphone_copy_master,
            next_output_id,
            next_bus_id,
            ..
        } = parsed
        else {
            panic!("expected setSettings");
        };
        assert!(!headphone_copy_master.unwrap());
        assert_eq!(next_output_id, 102);
        assert_eq!(next_bus_id, 3);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].transport, OutputTransport::Omt);
        assert_eq!(outputs[0].source_kind, OutputSourceKind::MuProgram);
        assert_eq!(outputs[0].unit_id, 1);
        assert_eq!(outputs[1].source_kind, OutputSourceKind::MuPreview);
        assert_eq!(outputs[1].id, 101);
    }

    #[test]
    fn set_settings_json_may_omit_headphone_copy_master() {
        let json = r#"{
            "kind": "setSettings",
            "settings": { "masterFpsNum": 60000, "masterFpsDen": 1001 },
            "outputs": [],
            "buses": []
        }"#;
        let parsed: SessionMutation =
            serde_json::from_str(json).expect("setSettings without headphone");
        let SessionMutation::SetSettings {
            headphone_copy_master,
            next_output_id,
            next_bus_id,
            ..
        } = parsed
        else {
            panic!("expected setSettings");
        };
        assert_eq!(headphone_copy_master, None);
        assert_eq!(next_output_id, 0);
        assert_eq!(next_bus_id, 0);
    }

    #[test]
    fn set_overlay_slot_json_uses_camel_case_unit_id() {
        let json = r#"{
            "kind": "setOverlaySlot",
            "unitId": 1,
            "index": 0,
            "slot": {
                "sceneGpuId": 1,
                "x": 0.62,
                "y": 0.08,
                "width": 0.32,
                "height": 0.32,
                "opacity": 1,
                "z": 0,
                "enabled": true
            }
        }"#;
        let parsed: SessionMutation = serde_json::from_str(json).expect("host overlay JSON");
        let SessionMutation::SetOverlaySlot { unit_id, index, .. } = parsed else {
            panic!("expected setOverlaySlot");
        };
        assert_eq!(unit_id, 1);
        assert_eq!(index, 0);
    }

    #[test]
    fn remote_upsert_unit_accepts_camel_case_enums() {
        let json = r#"{
            "kind": "upsertUnit",
            "unit": {
                "id": 2,
                "name": "Mixing Unit 2",
                "width": 1920,
                "height": 1080,
                "fpsNum": 60000,
                "fpsDen": 1001,
                "audioBusId": 1,
                "audioLink": "follow",
                "switcherSceneFilter": "all",
                "switcherSceneIds": []
            }
        }"#;
        let parsed: SessionMutation = serde_json::from_str(json).expect("Remote upsertUnit JSON");
        let SessionMutation::UpsertUnit { unit } = parsed else {
            panic!("expected upsertUnit");
        };
        assert_eq!(unit.id, 2);
        assert_eq!(unit.audio_link, crate::session::AudioLinkMode::Follow);
        assert_eq!(
            unit.switcher_scene_filter,
            crate::session::SwitcherSceneFilter::All
        );
    }

    #[test]
    fn remote_upsert_unit_accepts_independent_and_include() {
        let json = r#"{
            "kind": "upsertUnit",
            "unit": {
                "id": 3,
                "name": "Mixing Unit 3",
                "audioLink": "independent",
                "switcherSceneFilter": "include",
                "switcherSceneIds": [1, 4]
            }
        }"#;
        let parsed: SessionMutation =
            serde_json::from_str(json).expect("Remote independent unit JSON");
        let SessionMutation::UpsertUnit { unit } = parsed else {
            panic!("expected upsertUnit");
        };
        assert_eq!(unit.audio_link, crate::session::AudioLinkMode::Independent);
        assert_eq!(
            unit.switcher_scene_filter,
            crate::session::SwitcherSceneFilter::Include
        );
        assert_eq!(unit.switcher_scene_ids, vec![1, 4]);
    }

    #[test]
    fn remote_set_settings_accepts_camel_case_enums() {
        let json = r#"{
            "kind": "setSettings",
            "settings": {
                "renderer": "dx12",
                "internalColorFormat": "bgra",
                "multiviewLabelUnit": "percent",
                "multiviewLabelAnchor": "top"
            },
            "outputs": [{
                "id": 100,
                "name": "eiviz-pgm",
                "transport": "omt",
                "sourceKind": "muProgram",
                "useGpu": true,
                "enabled": true
            }],
            "buses": [{
                "id": 1,
                "name": "Master",
                "role": "master",
                "deviceKind": "wasapi"
            }]
        }"#;
        let parsed: SessionMutation = serde_json::from_str(json).expect("Remote setSettings JSON");
        let SessionMutation::SetSettings {
            settings,
            outputs,
            buses,
            ..
        } = parsed
        else {
            panic!("expected setSettings");
        };
        assert_eq!(settings.renderer, crate::session::Renderer::Dx12);
        assert_eq!(
            settings.internal_color_format,
            crate::session::InternalColorFormat::Bgra
        );
        assert_eq!(
            settings.multiview_label_unit,
            crate::session::MvLabelUnit::Percent
        );
        assert_eq!(
            settings.multiview_label_anchor,
            crate::session::MvLabelAnchor::Top
        );
        assert_eq!(outputs[0].transport, crate::session::OutputTransport::Omt);
        assert_eq!(
            outputs[0].source_kind,
            crate::session::OutputSourceKind::MuProgram
        );
        assert_eq!(buses[0].role, crate::session::AudioBusRole::Master);
        assert_eq!(
            buses[0].device_kind,
            crate::session::AudioDeviceKind::Wasapi
        );
    }

    #[test]
    fn remote_upsert_input_accepts_camel_case_enums() {
        let json = r#"{
            "kind": "upsertInput",
            "input": {
                "id": 12,
                "name": "Cam",
                "kind": "ndi",
                "bandwidthSave": "notOnPreviewOrProgram",
                "omtQuality": "high",
                "ndiBandwidth": "lowest",
                "videoPlayWhen": "onActive",
                "videoRestartWhen": "onPreview",
                "videoPauseWhen": "onDeactivated",
                "mixSource": "muPreview"
            }
        }"#;
        let parsed: SessionMutation = serde_json::from_str(json).expect("Remote upsertInput JSON");
        let SessionMutation::UpsertInput { input } = parsed else {
            panic!("expected upsertInput");
        };
        assert_eq!(input.kind, crate::session::InputKind::NDI);
        assert_eq!(
            input.bandwidth_save,
            crate::session::BandwidthSave::NotOnPreviewOrProgram
        );
        assert_eq!(input.omt_quality, crate::session::OmtQuality::High);
        assert_eq!(input.ndi_bandwidth, crate::session::NdiBandwidth::Lowest);
        assert_eq!(
            input.video_play_when,
            crate::session::VideoPlayWhen::OnActive
        );
        assert_eq!(
            input.video_restart_when,
            crate::session::VideoTriggerWhen::OnPreview
        );
        assert_eq!(
            input.video_pause_when,
            crate::session::VideoTriggerWhen::OnDeactivated
        );
        assert_eq!(input.mix_source, crate::session::MixSource::MuPreview);
    }

    #[test]
    fn discover_kind_parses_host_device_kinds() {
        assert_eq!(DiscoverKind::parse("omt"), Some(DiscoverKind::Omt));
        assert_eq!(DiscoverKind::parse("ndi"), Some(DiscoverKind::Ndi));
        assert_eq!(DiscoverKind::parse("uvc"), Some(DiscoverKind::Uvc));
        assert_eq!(
            DiscoverKind::parse("uvcModes"),
            Some(DiscoverKind::UvcModes)
        );
        assert_eq!(
            DiscoverKind::parse("uvc-modes"),
            Some(DiscoverKind::UvcModes)
        );
        assert_eq!(DiscoverKind::parse("audio"), Some(DiscoverKind::Audio));
        assert_eq!(DiscoverKind::parse("unknown"), None);
    }
}
