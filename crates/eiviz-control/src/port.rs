use crate::error::ControlResult;
use crate::live::{LiveState, UnitLiveState};

/// GPU/OS mixer surface owned by `eiviz_mixer`. Control never talks to wgpu.
pub trait MixerPort: Send {
    fn ping(&self) -> u32 {
        0x4549_5649
    }

    fn create(
        &mut self,
        fps_num: u32,
        fps_den: u32,
        renderer: crate::session::Renderer,
    ) -> ControlResult<()>;
    fn destroy(&mut self) -> ControlResult<()>;
    fn is_ready(&self) -> bool;

    fn create_unit(&mut self, id: u64, width: u32, height: u32) -> ControlResult<()>;
    fn destroy_unit(&mut self, id: u64) -> ControlResult<()>;
    fn configure_unit(
        &mut self,
        id: u64,
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
    ) -> ControlResult<()>;

    fn define_scene(&mut self, spec: SceneApply) -> ControlResult<()>;
    fn destroy_scene(&mut self, id: u64) -> ControlResult<()>;

    fn define_generator(&mut self, spec: GeneratorApply) -> ControlResult<()>;
    fn define_mix_input(&mut self, spec: MixInputApply) -> ControlResult<()>;
    fn load_still(&mut self, id: u64, path: &str) -> ControlResult<()>;
    fn video_start(&mut self, spec: VideoStartApply) -> ControlResult<()>;
    fn omt_connect(&mut self, spec: LiveConnectApply) -> ControlResult<()>;
    fn ndi_connect(&mut self, spec: LiveConnectApply) -> ControlResult<()>;
    fn destroy_source(&mut self, id: u64) -> ControlResult<()>;
    fn set_live_save(&mut self, id: u64, mode: u32, flags: u32) -> ControlResult<()>;
    fn set_omt_quality(&mut self, id: u64, quality: u32) -> ControlResult<()>;

    fn output_add(&mut self, spec: OutputApply) -> ControlResult<()>;
    fn output_remove(&mut self, id: u64) -> ControlResult<()>;

    fn audio_bus_upsert(&mut self, spec: BusApply) -> ControlResult<()>;
    fn audio_bus_remove(&mut self, id: u64) -> ControlResult<()>;
    fn audio_set_input(
        &mut self,
        id: u64,
        bus_mask: u32,
        gain: f32,
        mute: bool,
    ) -> ControlResult<()>;
    fn audio_set_bus_gain(&mut self, id: u64, gain: f32, mute: bool) -> ControlResult<()>;
    fn audio_set_unit_link(&mut self, unit_id: u64, bus_id: u64, mode: u32) -> ControlResult<()>;
    fn audio_set_headphone_copy_master(&mut self, enabled: bool) -> ControlResult<()>;

    fn set_frame_buffer(&mut self, frames: u32) -> ControlResult<()>;
    fn set_rebar_optimization(&mut self, enabled: bool) -> ControlResult<()>;
    fn set_ndi_gpu_upload(&mut self, enabled: bool) -> ControlResult<()>;
    fn set_bus_colors(
        &mut self,
        preview: [u8; 3],
        program: [u8; 3],
        inactive: [u8; 3],
    ) -> ControlResult<()>;
    fn set_mv_label(
        &mut self,
        scene_id: u64,
        size: f32,
        percent: bool,
        top: bool,
    ) -> ControlResult<()>;
    fn bind_multiview(
        &mut self,
        scene_id: u64,
        preview_unit: u64,
        program_unit: u64,
    ) -> ControlResult<()>;

    fn unit_cut(&mut self, unit_id: u64, swap: bool, incoming: u64) -> ControlResult<()>;
    fn unit_auto(&mut self, spec: AutoApply) -> ControlResult<()>;
    fn unit_set_preview(&mut self, unit_id: u64, scene_gpu_id: u64) -> ControlResult<()>;
    fn unit_set_mix(&mut self, unit_id: u64, mix: f32) -> ControlResult<()>;
    fn unit_set_state(
        &mut self,
        unit_id: u64,
        program: u64,
        preview: u64,
        mix: f32,
    ) -> ControlResult<()>;
    fn overlay_auto(&mut self, spec: OverlayAutoApply) -> ControlResult<()> {
        let _ = spec;
        Ok(())
    }
    fn unit_live(&self, unit_id: u64) -> ControlResult<UnitLiveState>;
    fn live_state(&self) -> ControlResult<LiveState>;

    fn video_set_playing(&mut self, id: u64, playing: bool) -> ControlResult<()>;
    fn video_set_loop(&mut self, id: u64, looping: bool) -> ControlResult<()>;
    fn video_seek(&mut self, id: u64, position_hns: i64) -> ControlResult<()>;

    fn snapshot(&mut self, unit_id: u64, kind: u32, path: &str) -> ControlResult<()>;
    fn configure_vmix_api(
        &mut self,
        http_enabled: bool,
        tcp_enabled: bool,
        port: u32,
        user: &str,
        pass: &str,
    ) -> ControlResult<()>;
    fn configure_native_api(&mut self, enabled: bool, port: u32) -> ControlResult<()>;
    fn discover_omt(&self) -> ControlResult<String> {
        Ok(String::new())
    }
    fn discover_ndi(&self) -> ControlResult<String> {
        Ok(String::new())
    }

    /// Concrete impls forward to `reconcile::apply_one` so the large match is
    /// monomorphized per port type instead of taking `&mut dyn MixerPort`.
    fn apply_reconcile(
        &mut self,
        next: &crate::session::Document,
        op: &crate::session::reconcile::ReconcileOp,
        statuses: &mut Vec<crate::live::ResourceStatus>,
    ) -> ControlResult<()>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayLayer {
    pub source_id: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub crop_x: f32,
    pub crop_y: f32,
    pub crop_width: f32,
    pub crop_height: f32,
    pub opacity: f32,
    pub z: i32,
    pub audio_follow: bool,
    pub hidden: bool,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneApply {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub layers: Vec<OverlayLayer>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeneratorApply {
    pub id: u64,
    pub bars: bool,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub scroll: bool,
    pub tone_hz: f32,
    pub tone_level_dbfs: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MixInputApply {
    pub id: u64,
    pub target_id: u64,
    pub source_kind: u32,
    pub delay: u32,
    pub audio_bus_id: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoStartApply {
    pub id: u64,
    pub path: String,
    pub capture: bool,
    pub loop_playback: bool,
    pub playing: bool,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub frame_buffer_frames: u32,
    pub position_hns: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiveConnectApply {
    pub id: u64,
    pub address: String,
    pub use_gpu: bool,
    pub frame_buffer_frames: u32,
    pub quality_or_bandwidth: u32,
    pub save_mode: u32,
    pub keep_full_on_multiview: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputApply {
    pub id: u64,
    pub transport: u32,
    pub name: String,
    pub source_kind: u32,
    pub source_id: u64,
    pub unit_id: u64,
    pub use_gpu: bool,
    pub audio_bus_id: u64,
    pub skip_encode_when_no_receivers: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BusApply {
    pub id: u64,
    pub name: String,
    pub role: u32,
    pub device_kind: u32,
    pub device_id: String,
    pub map_left: u32,
    pub map_right: u32,
    pub exclusive: bool,
    pub gain: f32,
    pub mute: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AutoApply {
    pub unit_id: u64,
    pub kind: u32,
    pub duration_ms: u32,
    pub swap: bool,
    pub keep_preview: bool,
    pub easing: u32,
    pub direction: u32,
    pub dip_r: f32,
    pub dip_g: f32,
    pub dip_b: f32,
    pub dip_a: f32,
    pub incoming: u64,
    pub softness: f32,
    pub param: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayAutoApply {
    pub unit_id: u64,
    pub to_on: bool,
    pub duration_ms: u32,
    pub source_id: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub opacity: f32,
    pub z: i32,
    pub audio_follow: bool,
    pub hidden: bool,
}

/// In-memory mixer used by API/contract tests. It does not touch GPU or I/O.
#[derive(Debug, Default)]
pub struct NullMixer {
    ready: bool,
    units: std::collections::HashMap<u64, UnitLiveState>,
}

impl MixerPort for NullMixer {
    fn create(
        &mut self,
        _fps_num: u32,
        _fps_den: u32,
        _renderer: crate::session::Renderer,
    ) -> ControlResult<()> {
        self.ready = true;
        Ok(())
    }
    fn destroy(&mut self) -> ControlResult<()> {
        self.ready = false;
        self.units.clear();
        Ok(())
    }
    fn is_ready(&self) -> bool {
        self.ready
    }
    fn create_unit(&mut self, id: u64, _width: u32, _height: u32) -> ControlResult<()> {
        self.units.entry(id).or_default();
        Ok(())
    }
    fn destroy_unit(&mut self, id: u64) -> ControlResult<()> {
        self.units.remove(&id);
        Ok(())
    }
    fn configure_unit(
        &mut self,
        _id: u64,
        _width: u32,
        _height: u32,
        _fps_num: u32,
        _fps_den: u32,
    ) -> ControlResult<()> {
        Ok(())
    }
    fn define_scene(&mut self, _spec: SceneApply) -> ControlResult<()> {
        Ok(())
    }
    fn destroy_scene(&mut self, _id: u64) -> ControlResult<()> {
        Ok(())
    }
    fn define_generator(&mut self, _spec: GeneratorApply) -> ControlResult<()> {
        Ok(())
    }
    fn define_mix_input(&mut self, _spec: MixInputApply) -> ControlResult<()> {
        Ok(())
    }
    fn load_still(&mut self, _id: u64, _path: &str) -> ControlResult<()> {
        Ok(())
    }
    fn video_start(&mut self, _spec: VideoStartApply) -> ControlResult<()> {
        Ok(())
    }
    fn omt_connect(&mut self, _spec: LiveConnectApply) -> ControlResult<()> {
        Ok(())
    }
    fn ndi_connect(&mut self, _spec: LiveConnectApply) -> ControlResult<()> {
        Ok(())
    }
    fn destroy_source(&mut self, _id: u64) -> ControlResult<()> {
        Ok(())
    }
    fn set_live_save(&mut self, _id: u64, _mode: u32, _flags: u32) -> ControlResult<()> {
        Ok(())
    }
    fn set_omt_quality(&mut self, _id: u64, _quality: u32) -> ControlResult<()> {
        Ok(())
    }
    fn output_add(&mut self, _spec: OutputApply) -> ControlResult<()> {
        Ok(())
    }
    fn output_remove(&mut self, _id: u64) -> ControlResult<()> {
        Ok(())
    }
    fn audio_bus_upsert(&mut self, _spec: BusApply) -> ControlResult<()> {
        Ok(())
    }
    fn audio_bus_remove(&mut self, _id: u64) -> ControlResult<()> {
        Ok(())
    }
    fn audio_set_input(
        &mut self,
        _id: u64,
        _bus_mask: u32,
        _gain: f32,
        _mute: bool,
    ) -> ControlResult<()> {
        Ok(())
    }
    fn audio_set_bus_gain(&mut self, _id: u64, _gain: f32, _mute: bool) -> ControlResult<()> {
        Ok(())
    }
    fn audio_set_unit_link(
        &mut self,
        _unit_id: u64,
        _bus_id: u64,
        _mode: u32,
    ) -> ControlResult<()> {
        Ok(())
    }
    fn audio_set_headphone_copy_master(&mut self, _enabled: bool) -> ControlResult<()> {
        Ok(())
    }
    fn set_frame_buffer(&mut self, _frames: u32) -> ControlResult<()> {
        Ok(())
    }
    fn set_rebar_optimization(&mut self, _enabled: bool) -> ControlResult<()> {
        Ok(())
    }
    fn set_ndi_gpu_upload(&mut self, _enabled: bool) -> ControlResult<()> {
        Ok(())
    }
    fn set_bus_colors(
        &mut self,
        _preview: [u8; 3],
        _program: [u8; 3],
        _inactive: [u8; 3],
    ) -> ControlResult<()> {
        Ok(())
    }
    fn set_mv_label(
        &mut self,
        _scene_id: u64,
        _size: f32,
        _percent: bool,
        _top: bool,
    ) -> ControlResult<()> {
        Ok(())
    }
    fn bind_multiview(
        &mut self,
        _scene_id: u64,
        _preview_unit: u64,
        _program_unit: u64,
    ) -> ControlResult<()> {
        Ok(())
    }
    fn unit_cut(&mut self, unit_id: u64, swap: bool, incoming: u64) -> ControlResult<()> {
        let unit = self.units.entry(unit_id).or_default();
        if swap {
            std::mem::swap(&mut unit.program_source, &mut unit.preview_source);
        } else if incoming != 0 {
            unit.program_source = incoming;
        }
        Ok(())
    }
    fn unit_auto(&mut self, spec: AutoApply) -> ControlResult<()> {
        self.unit_cut(spec.unit_id, spec.swap, spec.incoming)
    }
    fn unit_set_preview(&mut self, unit_id: u64, scene_gpu_id: u64) -> ControlResult<()> {
        self.units.entry(unit_id).or_default().preview_source = scene_gpu_id;
        Ok(())
    }
    fn unit_set_mix(&mut self, unit_id: u64, mix: f32) -> ControlResult<()> {
        self.units.entry(unit_id).or_default().mix = mix;
        Ok(())
    }
    fn unit_set_state(
        &mut self,
        unit_id: u64,
        program: u64,
        preview: u64,
        mix: f32,
    ) -> ControlResult<()> {
        self.units.insert(
            unit_id,
            UnitLiveState {
                program_source: program,
                preview_source: preview,
                mix,
                ..UnitLiveState::default()
            },
        );
        Ok(())
    }
    fn unit_live(&self, unit_id: u64) -> ControlResult<UnitLiveState> {
        self.units
            .get(&unit_id)
            .cloned()
            .ok_or_else(|| crate::error::ControlError::not_found("unit"))
    }
    fn live_state(&self) -> ControlResult<LiveState> {
        Ok(LiveState {
            units: self.units.clone(),
        })
    }
    fn video_set_playing(&mut self, _id: u64, _playing: bool) -> ControlResult<()> {
        Ok(())
    }
    fn video_set_loop(&mut self, _id: u64, _looping: bool) -> ControlResult<()> {
        Ok(())
    }
    fn video_seek(&mut self, _id: u64, _position_hns: i64) -> ControlResult<()> {
        Ok(())
    }
    fn snapshot(&mut self, _unit_id: u64, _kind: u32, _path: &str) -> ControlResult<()> {
        Ok(())
    }
    fn configure_vmix_api(
        &mut self,
        _http_enabled: bool,
        _tcp_enabled: bool,
        _port: u32,
        _user: &str,
        _pass: &str,
    ) -> ControlResult<()> {
        Ok(())
    }
    fn configure_native_api(&mut self, _enabled: bool, _port: u32) -> ControlResult<()> {
        Ok(())
    }
    fn apply_reconcile(
        &mut self,
        next: &crate::session::Document,
        op: &crate::session::reconcile::ReconcileOp,
        statuses: &mut Vec<crate::live::ResourceStatus>,
    ) -> ControlResult<()> {
        crate::session::reconcile::apply_one(self, next, op, statuses)
    }
}
