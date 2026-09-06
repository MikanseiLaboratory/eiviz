use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::command::{Command, Incoming};
use crate::error::{ControlError, ControlResult};
use crate::event::{EnvelopeMeta, Event};
use crate::event_hub::EventHub;
use crate::ids::Resolver;
use crate::lifecycle::Lifecycle;
use crate::live::ResourceStatus;
use crate::port::{AutoApply, MixerPort};
use crate::query::{Capabilities, Query, Snapshot};
use crate::session::Document;
use crate::session::reconcile::plan;
use crate::session::store::CanonicalSessionStore;
use crate::video_trigger::{self, VideoAction, VideoRoles};

const DEDUPE_CAP: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestKey {
    pub client_instance_id: String,
    pub request_id: String,
}

#[derive(Debug, Clone)]
pub struct CommandOutcome {
    pub revision: u64,
    pub sequence: u64,
}

#[derive(Clone)]
struct CachedOutcome {
    outcome: Result<CommandOutcome, ControlError>,
    at: Instant,
}

pub struct ControlService {
    port: Box<dyn MixerPort>,
    store: CanonicalSessionStore,
    hub: EventHub,
    lifecycle: Lifecycle,
    resolver: Resolver,
    resources: Vec<ResourceStatus>,
    video_roles: HashMap<u64, VideoRoles>,
    dedupe: HashMap<RequestKey, CachedOutcome>,
    dedupe_order: VecDeque<RequestKey>,
}

impl ControlService {
    pub fn new(port: impl MixerPort + 'static) -> Self {
        Self {
            port: Box::new(port),
            store: CanonicalSessionStore::default(),
            hub: EventHub::default(),
            lifecycle: Lifecycle::Stopped,
            resolver: Resolver,
            resources: Vec::new(),
            video_roles: HashMap::new(),
            dedupe: HashMap::new(),
            dedupe_order: VecDeque::new(),
        }
    }

    pub fn lifecycle(&self) -> Lifecycle {
        self.lifecycle
    }

    pub fn revision(&self) -> u64 {
        self.store.revision()
    }

    pub fn document(&self) -> Option<&Document> {
        self.store.document()
    }

    pub fn port_mut(&mut self) -> &mut dyn MixerPort {
        &mut *self.port
    }

    pub fn events_after(&self, after: u64) -> Vec<Event> {
        self.hub.after(after)
    }

    #[inline(never)]
    pub fn create_runtime(&mut self, fps_num: u32, fps_den: u32) -> ControlResult<()> {
        self.lifecycle = Lifecycle::Starting;
        match self.port.create(fps_num, fps_den) {
            Ok(()) => {
                self.lifecycle = Lifecycle::Ready;
                let meta = self.meta("");
                self.hub.publish(Event::Ready { meta });
                Ok(())
            }
            Err(error) => {
                self.lifecycle = Lifecycle::Failed;
                Err(error)
            }
        }
    }

    pub fn destroy_runtime(&mut self) -> ControlResult<()> {
        self.lifecycle = Lifecycle::Stopping;
        let _ = self.port.destroy();
        self.abandon();
        let meta = self.meta("");
        self.hub.publish(Event::Shutdown { meta });
        Ok(())
    }

    /// Drop the canonical session without touching the GPU port. Used when the
    /// C ABI `mixer_destroy` path already owns teardown.
    pub fn abandon(&mut self) {
        self.store = CanonicalSessionStore::default();
        self.resources.clear();
        self.video_roles.clear();
        self.dedupe.clear();
        self.dedupe_order.clear();
        self.lifecycle = Lifecycle::Stopped;
    }

    pub fn query(&self, query: Query) -> ControlResult<Snapshot> {
        match query {
            Query::GetSnapshot
            | Query::GetCapabilities
            | Query::GetRevision
            | Query::GetLiveState => self.snapshot(),
        }
    }

    pub fn snapshot(&self) -> ControlResult<Snapshot> {
        if self.lifecycle != Lifecycle::Ready && self.lifecycle != Lifecycle::Starting {
            return Err(ControlError::unavailable("mixer is not ready"));
        }
        let document = self
            .store
            .document_cloned()
            .unwrap_or_else(|| serde_json::from_str("{}").unwrap_or_else(|_| unreachable_doc()));
        let live = self.port.live_state().unwrap_or_default();
        Ok(Snapshot {
            revision: self.store.revision(),
            sequence: self.hub.next_sequence(),
            document,
            live,
            resources: self.resources.clone(),
            capabilities: Capabilities::default(),
            lifecycle: self.lifecycle,
        })
    }

    pub fn execute(&mut self, key: RequestKey, command: Command) -> ControlResult<CommandOutcome> {
        if !key.request_id.is_empty() {
            if let Some(cached) = self.dedupe.get(&key) {
                return cached.outcome.clone();
            }
        }
        if self.lifecycle != Lifecycle::Ready && !matches!(command, Command::Shutdown) {
            if !self.port.is_ready() && !matches!(command, Command::ReplaceSession { .. }) {
                let err = ControlError::unavailable("mixer is not ready");
                self.remember(&key, Err(err.clone()));
                return Err(err);
            }
        }
        let result = match command {
            Command::ReplaceSession {
                document,
                expected_revision,
            } => self.replace_session(*document, expected_revision, &key.request_id),
            other => self.execute_live(&key.request_id, other),
        };
        self.remember(&key, result.clone());
        result
    }

    #[inline(never)]
    fn execute_live(
        &mut self,
        request_id: &str,
        command: Command,
    ) -> ControlResult<CommandOutcome> {
        match command {
            Command::Preview { unit_id, scene_id } => {
                let gpu = crate::ids::scene_gpu_id(scene_id);
                self.port.unit_set_preview(unit_id, gpu)?;
                self.after_live("Preview", request_id, Some(unit_id), false)
            }
            Command::Cut {
                unit_id,
                swap,
                incoming,
            } => {
                self.port.unit_cut(unit_id, swap, incoming.to_u64())?;
                self.after_live("Cut", request_id, Some(unit_id), false)
            }
            Command::Auto {
                unit_id,
                kind,
                duration_ms,
                swap,
                keep_preview,
                easing,
                direction,
                dip_r,
                dip_g,
                dip_b,
                dip_a,
                incoming,
                softness,
                param,
            } => {
                self.port.unit_auto(AutoApply {
                    unit_id,
                    kind,
                    duration_ms,
                    swap,
                    keep_preview,
                    easing,
                    direction,
                    dip_r,
                    dip_g,
                    dip_b,
                    dip_a,
                    incoming: incoming.to_u64(),
                    softness,
                    param,
                })?;
                self.after_live("Auto", request_id, Some(unit_id), true)
            }
            Command::SetMix { unit_id, value } => {
                self.port.unit_set_mix(unit_id, value)?;
                self.after_live("SetMix", request_id, Some(unit_id), false)
            }
            Command::VideoPlay { input_id, playing } => {
                self.port.video_set_playing(input_id, playing)?;
                self.after_live("VideoPlay", request_id, None, false)
            }
            Command::VideoLoop { input_id, looping } => {
                self.port.video_set_loop(input_id, looping)?;
                self.after_live("VideoLoop", request_id, None, false)
            }
            Command::VideoSeek {
                input_id,
                position_hns,
            } => {
                self.port.video_seek(input_id, position_hns)?;
                self.after_live("VideoSeek", request_id, None, false)
            }
            Command::AudioSetInput {
                input_id,
                bus_mask,
                gain,
                mute,
            } => {
                self.port.audio_set_input(input_id, bus_mask, gain, mute)?;
                self.after_live("AudioSetInput", request_id, None, false)
            }
            Command::AudioSetBus { bus_id, gain, mute } => {
                self.port.audio_set_bus_gain(bus_id, gain, mute)?;
                self.after_live("AudioSetBus", request_id, None, false)
            }
            Command::Snapshot {
                unit_id,
                kind,
                path,
            } => {
                let abi_kind = match kind {
                    crate::command::SnapshotKind::Program => 0,
                    crate::command::SnapshotKind::Preview => 1,
                    crate::command::SnapshotKind::Source(_) => 3,
                };
                self.port.snapshot(unit_id, abi_kind, &path)?;
                self.after_live("Snapshot", request_id, None, false)
            }
            Command::Discover { kind } => {
                let _ = match kind {
                    crate::command::DiscoverKind::Omt => self.port.discover_omt()?,
                    crate::command::DiscoverKind::Ndi => self.port.discover_ndi()?,
                    crate::command::DiscoverKind::Audio => String::new(),
                };
                self.after_live("Discover", request_id, None, false)
            }
            Command::OverlayAuto { .. } => self.after_live("OverlayAuto", request_id, None, false),
            Command::Shutdown => {
                self.destroy_runtime()?;
                Ok(CommandOutcome {
                    revision: self.store.revision(),
                    sequence: self.hub.next_sequence(),
                })
            }
            Command::ReplaceSession { .. } => {
                Err(ControlError::internal("replace routed incorrectly"))
            }
        }
    }

    #[inline(never)]
    pub fn replace_session(
        &mut self,
        document: Document,
        expected_revision: Option<u64>,
        request_id: &str,
    ) -> ControlResult<CommandOutcome> {
        if !self.port.is_ready() {
            self.create_runtime(
                document.settings.master_fps_num,
                document.settings.master_fps_den,
            )?;
        } else if self.lifecycle != Lifecycle::Ready {
            self.lifecycle = Lifecycle::Ready;
        }
        let previous_rev = self.store.revision();
        let previous_doc = self.store.document_cloned();
        let (committed, revision, previous) =
            self.store.replace(document, expected_revision, false)?;
        let ops = plan(previous.as_ref(), &committed);
        let mut statuses = Vec::new();
        for op in &ops {
            if let Err(error) = self.port.apply_reconcile(&committed, op, &mut statuses) {
                self.store.rollback(previous_doc.clone(), previous_rev);
                if let Some(prev) = previous_doc {
                    let rollback = plan(Some(&committed), &prev);
                    let mut ignored = Vec::new();
                    for op in &rollback {
                        let _ = self.port.apply_reconcile(&prev, op, &mut ignored);
                    }
                }
                return Err(error);
            }
        }
        let meta = self.meta(request_id);
        self.hub.publish(Event::SessionChanged {
            meta: meta.clone(),
            document: Box::new(committed.clone()),
        });
        self.hub.publish(Event::CommandApplied {
            meta: meta.clone(),
            command: "ReplaceSession".into(),
        });
        for status in statuses {
            let meta = self.meta(request_id);
            self.hub.publish(Event::Resource { meta, status });
        }
        self.tick_video(request_id);
        Ok(CommandOutcome {
            revision,
            sequence: self.hub.next_sequence(),
        })
    }

    pub fn vmix_cut(
        &mut self,
        unit_id: u64,
        input_raw: &str,
        named_input: bool,
    ) -> ControlResult<CommandOutcome> {
        let swap = !named_input;
        let incoming = if named_input {
            Incoming::Source(self.resolve_scene_gpu(input_raw)?)
        } else {
            Incoming::Preview
        };
        self.execute(
            RequestKey {
                client_instance_id: "vmix".into(),
                request_id: String::new(),
            },
            Command::Cut {
                unit_id,
                swap,
                incoming,
            },
        )
    }

    pub fn vmix_preview(&mut self, unit_id: u64, input_raw: &str) -> ControlResult<CommandOutcome> {
        let scene_id = self.resolve_scene_id(input_raw)?;
        self.execute(
            RequestKey {
                client_instance_id: "vmix".into(),
                request_id: String::new(),
            },
            Command::Preview { unit_id, scene_id },
        )
    }

    pub fn vmix_auto(
        &mut self,
        unit_id: u64,
        input_raw: &str,
        named_input: bool,
        duration_ms: u32,
    ) -> ControlResult<CommandOutcome> {
        let incoming = if named_input {
            Incoming::Source(self.resolve_scene_gpu(input_raw)?)
        } else {
            Incoming::Preview
        };
        self.execute(
            RequestKey {
                client_instance_id: "vmix".into(),
                request_id: String::new(),
            },
            Command::Auto {
                unit_id,
                kind: 1,
                duration_ms: duration_ms.max(1),
                swap: !named_input,
                keep_preview: true,
                easing: 0,
                direction: 0,
                dip_r: 0.0,
                dip_g: 0.0,
                dip_b: 0.0,
                dip_a: 1.0,
                incoming,
                softness: 0.02,
                param: 0.0,
            },
        )
    }

    pub fn resolve_scene_id(&self, raw: &str) -> ControlResult<u64> {
        let doc = self
            .store
            .document()
            .ok_or_else(|| ControlError::unavailable("session not published"))?;
        Ok(self.resolver.resolve_scene(doc, raw)?.id)
    }

    pub fn resolve_scene_gpu(&self, raw: &str) -> ControlResult<u64> {
        let doc = self
            .store
            .document()
            .ok_or_else(|| ControlError::unavailable("session not published"))?;
        if raw == "0" {
            return Ok(0);
        }
        let scene = self.resolver.resolve_scene(doc, raw)?;
        Ok(crate::ids::scene_gpu_id(scene.id))
    }

    fn after_live(
        &mut self,
        name: &str,
        request_id: &str,
        unit_id: Option<u64>,
        transition: bool,
    ) -> ControlResult<CommandOutcome> {
        let live = self.port.live_state().unwrap_or_default();
        let meta = self.meta(request_id);
        self.hub.publish(Event::CommandApplied {
            meta: meta.clone(),
            command: name.into(),
        });
        if transition {
            if let Some(unit_id) = unit_id {
                self.hub.publish(Event::TransitionStarted {
                    meta: meta.clone(),
                    unit_id,
                });
            }
        }
        self.hub.publish(Event::LiveChanged { meta, live });
        self.tick_video(request_id);
        Ok(CommandOutcome {
            revision: self.store.revision(),
            sequence: self.hub.next_sequence(),
        })
    }

    #[inline(never)]
    fn tick_video(&mut self, request_id: &str) {
        let Some(doc) = self.store.document_cloned() else {
            return;
        };
        let Ok(live) = self.port.live_state() else {
            return;
        };
        let actions = video_trigger::tick(&doc, &live, &mut self.video_roles);
        for (id, action) in actions {
            let _ = match action {
                VideoAction::SeekZero => self.port.video_seek(id, 0),
                VideoAction::Pause => self.port.video_set_playing(id, false),
                VideoAction::Play => self.port.video_set_playing(id, true),
            };
        }
        let _ = request_id;
    }

    fn meta(&self, request_id: &str) -> EnvelopeMeta {
        self.hub.meta(self.store.revision(), request_id)
    }

    fn remember(&mut self, key: &RequestKey, outcome: ControlResult<CommandOutcome>) {
        if key.request_id.is_empty() {
            return;
        }
        if self.dedupe_order.len() >= DEDUPE_CAP {
            if let Some(old) = self.dedupe_order.pop_front() {
                self.dedupe.remove(&old);
            }
        }
        self.dedupe.insert(
            key.clone(),
            CachedOutcome {
                outcome,
                at: Instant::now(),
            },
        );
        self.dedupe_order.push_back(key.clone());
        self.dedupe
            .retain(|_, item| item.at.elapsed() < Duration::from_secs(60));
    }
}

fn unreachable_doc() -> Document {
    serde_json::from_str("{\"version\":2}").expect("empty document")
}

pub trait ControlFacade: Send + Sync {
    fn execute(&self, key: RequestKey, command: Command) -> ControlResult<CommandOutcome>;
    fn snapshot(&self) -> ControlResult<Snapshot>;
    fn events_after(&self, after: u64) -> Vec<Event>;
    fn lifecycle(&self) -> Lifecycle;
}

impl ControlFacade for std::sync::Mutex<ControlService> {
    fn execute(&self, key: RequestKey, command: Command) -> ControlResult<CommandOutcome> {
        self.lock()
            .map_err(|_| ControlError::internal("control lock"))?
            .execute(key, command)
    }

    fn snapshot(&self) -> ControlResult<Snapshot> {
        self.lock()
            .map_err(|_| ControlError::internal("control lock"))?
            .snapshot()
    }

    fn events_after(&self, after: u64) -> Vec<Event> {
        self.lock()
            .map(|svc| svc.events_after(after))
            .unwrap_or_default()
    }

    fn lifecycle(&self) -> Lifecycle {
        self.lock()
            .map(|svc| svc.lifecycle())
            .unwrap_or(Lifecycle::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{LiveState, UnitLiveState};
    use crate::port::*;
    use crate::session::parse;

    #[derive(Clone, Default)]
    struct FakeMixer {
        ready: bool,
        units: std::sync::Arc<std::sync::Mutex<HashMap<u64, UnitLiveState>>>,
        scenes: usize,
        ops: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        fail_next: bool,
    }

    impl FakeMixer {
        fn ops(&self) -> Vec<String> {
            self.ops.lock().map(|slot| slot.clone()).unwrap_or_default()
        }
        fn units(&self) -> HashMap<u64, UnitLiveState> {
            self.units
                .lock()
                .map(|slot| slot.clone())
                .unwrap_or_default()
        }
        fn push_op(&self, op: String) {
            if let Ok(mut slot) = self.ops.lock() {
                slot.push(op);
            }
        }
    }

    impl MixerPort for FakeMixer {
        fn create(&mut self, _fps_num: u32, _fps_den: u32) -> ControlResult<()> {
            self.ready = true;
            self.push_op("create".into());
            Ok(())
        }
        fn destroy(&mut self) -> ControlResult<()> {
            self.ready = false;
            Ok(())
        }
        fn is_ready(&self) -> bool {
            self.ready
        }
        fn create_unit(&mut self, id: u64, _w: u32, _h: u32) -> ControlResult<()> {
            self.units
                .lock()
                .ok()
                .map(|mut slot| slot.insert(id, UnitLiveState::default()));
            self.push_op(format!("create_unit {id}"));
            Ok(())
        }
        fn destroy_unit(&mut self, id: u64) -> ControlResult<()> {
            self.units.lock().ok().map(|mut slot| slot.remove(&id));
            Ok(())
        }
        fn configure_unit(
            &mut self,
            id: u64,
            _w: u32,
            _h: u32,
            _n: u32,
            _d: u32,
        ) -> ControlResult<()> {
            self.push_op(format!("configure_unit {id}"));
            Ok(())
        }
        fn define_scene(&mut self, spec: SceneApply) -> ControlResult<()> {
            self.scenes += 1;
            self.push_op(format!("define_scene {}", spec.id));
            Ok(())
        }
        fn destroy_scene(&mut self, id: u64) -> ControlResult<()> {
            self.push_op(format!("destroy_scene {id}"));
            Ok(())
        }
        fn define_generator(&mut self, spec: GeneratorApply) -> ControlResult<()> {
            self.push_op(format!("generator {}", spec.id));
            Ok(())
        }
        fn define_mix_input(&mut self, spec: MixInputApply) -> ControlResult<()> {
            self.push_op(format!("mix {}", spec.id));
            Ok(())
        }
        fn load_still(&mut self, id: u64, _path: &str) -> ControlResult<()> {
            self.push_op(format!("still {id}"));
            Ok(())
        }
        fn video_start(&mut self, spec: VideoStartApply) -> ControlResult<()> {
            self.push_op(format!("video {}", spec.id));
            Ok(())
        }
        fn omt_connect(&mut self, spec: LiveConnectApply) -> ControlResult<()> {
            if self.fail_next {
                self.fail_next = false;
                return Err(ControlError::io("omt down"));
            }
            self.push_op(format!("omt {}", spec.id));
            Ok(())
        }
        fn ndi_connect(&mut self, spec: LiveConnectApply) -> ControlResult<()> {
            self.push_op(format!("ndi {}", spec.id));
            Ok(())
        }
        fn destroy_source(&mut self, id: u64) -> ControlResult<()> {
            self.push_op(format!("destroy_source {id}"));
            Ok(())
        }
        fn set_live_save(&mut self, _id: u64, _mode: u32, _flags: u32) -> ControlResult<()> {
            Ok(())
        }
        fn set_omt_quality(&mut self, _id: u64, _q: u32) -> ControlResult<()> {
            Ok(())
        }
        fn output_add(&mut self, spec: OutputApply) -> ControlResult<()> {
            self.push_op(format!("output {}", spec.id));
            Ok(())
        }
        fn output_remove(&mut self, id: u64) -> ControlResult<()> {
            self.push_op(format!("remove_output {id}"));
            Ok(())
        }
        fn audio_bus_upsert(&mut self, spec: BusApply) -> ControlResult<()> {
            self.push_op(format!("bus {}", spec.id));
            Ok(())
        }
        fn audio_bus_remove(&mut self, _id: u64) -> ControlResult<()> {
            Ok(())
        }
        fn audio_set_input(
            &mut self,
            _id: u64,
            _m: u32,
            _g: f32,
            _mute: bool,
        ) -> ControlResult<()> {
            Ok(())
        }
        fn audio_set_bus_gain(&mut self, _id: u64, _g: f32, _m: bool) -> ControlResult<()> {
            Ok(())
        }
        fn audio_set_unit_link(&mut self, _u: u64, _b: u64, _m: u32) -> ControlResult<()> {
            Ok(())
        }
        fn audio_set_headphone_copy_master(&mut self, _e: bool) -> ControlResult<()> {
            Ok(())
        }
        fn set_frame_buffer(&mut self, _f: u32) -> ControlResult<()> {
            Ok(())
        }
        fn set_rebar_optimization(&mut self, _e: bool) -> ControlResult<()> {
            Ok(())
        }
        fn set_ndi_gpu_upload(&mut self, _e: bool) -> ControlResult<()> {
            Ok(())
        }
        fn set_bus_colors(&mut self, _a: [u8; 3], _b: [u8; 3], _c: [u8; 3]) -> ControlResult<()> {
            Ok(())
        }
        fn set_mv_label(&mut self, _id: u64, _s: f32, _p: bool, _t: bool) -> ControlResult<()> {
            Ok(())
        }
        fn bind_multiview(&mut self, _s: u64, _p: u64, _g: u64) -> ControlResult<()> {
            Ok(())
        }
        fn unit_cut(&mut self, unit_id: u64, swap: bool, incoming: u64) -> ControlResult<()> {
            if let Ok(mut units) = self.units.lock() {
                let unit = units.entry(unit_id).or_default();
                if swap {
                    std::mem::swap(&mut unit.program_source, &mut unit.preview_source);
                } else if incoming != 0 {
                    unit.program_source = incoming;
                }
            }
            self.push_op(format!("cut {unit_id}"));
            Ok(())
        }
        fn unit_auto(&mut self, spec: AutoApply) -> ControlResult<()> {
            self.push_op(format!("auto {}", spec.unit_id));
            Ok(())
        }
        fn unit_set_preview(&mut self, unit_id: u64, scene_gpu_id: u64) -> ControlResult<()> {
            if let Ok(mut units) = self.units.lock() {
                units.entry(unit_id).or_default().preview_source = scene_gpu_id;
            }
            Ok(())
        }
        fn unit_set_mix(&mut self, unit_id: u64, mix: f32) -> ControlResult<()> {
            if let Ok(mut units) = self.units.lock() {
                units.entry(unit_id).or_default().mix = mix;
            }
            Ok(())
        }
        fn unit_set_state(
            &mut self,
            unit_id: u64,
            program: u64,
            preview: u64,
            mix: f32,
        ) -> ControlResult<()> {
            if let Ok(mut units) = self.units.lock() {
                units.insert(
                    unit_id,
                    UnitLiveState {
                        program_source: program,
                        preview_source: preview,
                        mix,
                        ..UnitLiveState::default()
                    },
                );
            }
            Ok(())
        }
        fn unit_live(&self, unit_id: u64) -> ControlResult<UnitLiveState> {
            self.units
                .lock()
                .ok()
                .and_then(|slot| slot.get(&unit_id).cloned())
                .ok_or_else(|| ControlError::not_found("unit"))
        }
        fn live_state(&self) -> ControlResult<LiveState> {
            Ok(LiveState {
                units: self
                    .units
                    .lock()
                    .map(|slot| slot.clone())
                    .unwrap_or_default(),
            })
        }
        fn video_set_playing(&mut self, id: u64, playing: bool) -> ControlResult<()> {
            self.push_op(format!("playing {id} {playing}"));
            Ok(())
        }
        fn video_set_loop(&mut self, _id: u64, _l: bool) -> ControlResult<()> {
            Ok(())
        }
        fn video_seek(&mut self, id: u64, _hns: i64) -> ControlResult<()> {
            self.push_op(format!("seek {id}"));
            Ok(())
        }
        fn snapshot(&mut self, _u: u64, _k: u32, _p: &str) -> ControlResult<()> {
            Ok(())
        }
        fn configure_vmix_api(
            &mut self,
            _e: bool,
            _p: u32,
            _u: &str,
            _pw: &str,
        ) -> ControlResult<()> {
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

    fn on_big_stack<F: FnOnce() + Send + 'static>(f: F) {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(f)
            .expect("spawn")
            .join()
            .expect("join");
    }

    fn bars_doc() -> Document {
        parse(
            br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Bars", "kind": "Bars" }],
          "scenes": [
            { "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] },
            { "id": 2, "name": "Scene 2", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }
          ],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#,
        )
        .unwrap()
    }

    #[test]
    fn replace_session_is_idempotent() {
        on_big_stack(|| {
            let fake = FakeMixer::default();
            let probe = fake.clone();
            let mut svc = ControlService::new(fake);
            svc.replace_session(bars_doc(), None, "1").unwrap();
            let first = probe.ops().len();
            svc.replace_session(bars_doc(), Some(1), "2").unwrap();
            let ops = probe.ops();
            assert!(ops.len() >= first);
            assert!(!ops[first..].iter().any(|op| op.starts_with("generator")));
        });
    }

    #[test]
    fn revision_conflict_is_rejected() {
        on_big_stack(|| {
            let mut svc = ControlService::new(FakeMixer::default());
            svc.replace_session(bars_doc(), None, "1").unwrap();
            let err = svc.replace_session(bars_doc(), Some(99), "2").unwrap_err();
            assert!(matches!(err, ControlError::Conflict { .. }));
        });
    }

    #[test]
    fn request_dedupe_does_not_double_cut() {
        on_big_stack(|| {
            let fake = FakeMixer::default();
            let probe = fake.clone();
            let mut svc = ControlService::new(fake);
            svc.replace_session(bars_doc(), None, "boot").unwrap();
            let key = RequestKey {
                client_instance_id: "t".into(),
                request_id: "cut-1".into(),
            };
            let cmd = Command::Cut {
                unit_id: 1,
                swap: true,
                incoming: Incoming::Preview,
            };
            svc.execute(key.clone(), cmd.clone()).unwrap();
            svc.execute(key, cmd).unwrap();
            assert_eq!(
                probe
                    .ops()
                    .iter()
                    .filter(|op| op.starts_with("cut"))
                    .count(),
                1
            );
        });
    }

    #[test]
    fn named_cut_does_not_swap_preview() {
        on_big_stack(|| {
            let fake = FakeMixer::default();
            let probe = fake.clone();
            let mut svc = ControlService::new(fake);
            svc.replace_session(bars_doc(), None, "boot").unwrap();
            let preview_before = probe.units()[&1].preview_source;
            svc.vmix_cut(1, "2", true).unwrap();
            let unit = probe.units()[&1].clone();
            assert_eq!(unit.preview_source, preview_before);
            assert_eq!(unit.program_source, crate::ids::scene_gpu_id(2));
        });
    }
}
