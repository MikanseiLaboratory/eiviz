//! Instant replay. Encode and disk live in the Pro module. This module owns
//! readback handoff, health, and channel playout.

mod audio;
mod channel;
mod health;
mod stretch;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use eiviz_control::error::ControlError;
use eiviz_control::session::replay::{
    AngleRecordMode, AngleSourceKind, ReplayDto, overflow_code, profile_code, replay_channel_gpu_id,
    retention_code,
};
use eiviz_pro_api::{
    EncodedFrame, Entitlements, LAYOUT_BGRA, PRO_BUSY, PRO_IO, PixelLayout, ProReplayAngle,
    ProReplayConfig, ProReplayDecoded, ProReplayEstimateAbi, ProReplayStatsAbi, ProReplayStorage,
    ProVideoView, ReplayAngleStats, ReplayStats, ReplayStore, ReplayStoreConfig, STATE_RECORDING,
    STATE_STOPPED, VideoFrame, WriteOutcome, estimate_rate_bps, free_bytes, remaining_secs,
};

use crate::abi::{
    ERR_INVALID_ARGUMENT, ERR_IO, ERR_NOT_SUPPORTED_PLAN, MixerReplayAngleStats,
    MixerReplayChannelState, MixerReplayEstimateOut, MixerReplayStats, REPLAY_ISSUE_CPU_ENCODE,
    REPLAY_ISSUE_FRAMES_DROPPED, REPLAY_ISSUE_GPU_READBACK, REPLAY_ISSUE_PRO_UNAVAILABLE,
    REPLAY_ISSUE_STORAGE_FULL, REPLAY_ISSUE_STORAGE_LOW, REPLAY_ISSUE_STORAGE_WRITE,
    REPLAY_RECORD_BASE, REPLAY_STATE_IDLE, REPLAY_STATE_RECORDING, REPLAY_STATE_STOPPED,
    REPLAY_STOP_FPS_CHANGED, REPLAY_STOP_NONE, REPLAY_STOP_USER,
};
use crate::upload::CpuFormat;

pub use health::{HealthMemory, HealthSample, step as health_step};

/// Segment length and buffering bounds. They live in the session model so the
/// document validation and the recorder refuse the same values; a store that
/// quietly clamped them could record something the operator never asked for.
pub use eiviz_control::session::replay::{
    ENCODE_QUEUE_MAX, ENCODE_QUEUE_MIN, SEGMENT_FRAMES_MAX, SEGMENT_FRAMES_MIN,
    WRITE_BUFFER_MAX_MB, WRITE_BUFFER_MIN_MB,
};

/// Why a Replay command was refused.
///
/// The code keeps the C ABI stable; the message is what the operator reads. A
/// refusal that collapses into "replay command was rejected" leaves the operator
/// with nothing to act on, so every failure carries the reason it was refused.
#[derive(Debug, Clone)]
pub struct ReplayFailure {
    pub code: i32,
    pub message: String,
}

impl ReplayFailure {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ERR_INVALID_ARGUMENT, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(ERR_IO, message)
    }

    pub fn plan(message: impl Into<String>) -> Self {
        Self::new(ERR_NOT_SUPPORTED_PLAN, message)
    }

    /// The control-layer error for this refusal, carrying the same reason.
    pub fn into_control(self) -> ControlError {
        control_error_with(self.code, &self.message)
    }
}

/// A plan quota as the operator should read it.
fn quota_text(quota: eiviz_pro_api::Quota<u32>) -> String {
    match quota.abi_count() {
        u32::MAX => "unlimited".to_string(),
        0 => "none".to_string(),
        count => count.to_string(),
    }
}

/// The audio graph runs at this rate, in this many channels. The waveform
/// buckets are sized from it.
const AUDIO_SAMPLE_RATE: u32 = crate::upload::AUDIO_RATE as u32;
const AUDIO_CHANNELS: usize = 2;

const ISSUE_CPU_ENCODE: u32 = REPLAY_ISSUE_CPU_ENCODE;
const ISSUE_STORAGE_WRITE: u32 = REPLAY_ISSUE_STORAGE_WRITE;
const ISSUE_GPU_READBACK: u32 = REPLAY_ISSUE_GPU_READBACK;
const ISSUE_STORAGE_LOW: u32 = REPLAY_ISSUE_STORAGE_LOW;
const ISSUE_STORAGE_FULL: u32 = REPLAY_ISSUE_STORAGE_FULL;
const ISSUE_FRAMES_DROPPED: u32 = REPLAY_ISSUE_FRAMES_DROPPED;
const ISSUE_STOPPED_BY_POLICY: u32 = crate::abi::REPLAY_ISSUE_STOPPED_BY_POLICY;
const ISSUE_PRO_UNAVAILABLE: u32 = REPLAY_ISSUE_PRO_UNAVAILABLE;
const SEVERITY_OK: u32 = crate::abi::REPLAY_SEVERITY_OK;
const SEVERITY_WARNING: u32 = crate::abi::REPLAY_SEVERITY_WARNING;
const SEVERITY_CRITICAL: u32 = crate::abi::REPLAY_SEVERITY_CRITICAL;
const STOP_BUFFER_FULL: u32 = crate::abi::REPLAY_STOP_BUFFER_FULL;
const STOP_STORAGE_FULL: u32 = crate::abi::REPLAY_STOP_STORAGE_FULL;
const STOP_IO: u32 = crate::abi::REPLAY_STOP_IO_ERROR;

struct RecordFrame {
    angle_id: u32,
    frame_index: u64,
    pts: i64,
    stride: usize,
    data: Vec<u8>,
}

struct Inbox {
    queues: Mutex<HashMap<u32, VecDeque<RecordFrame>>>,
    /// Frames that may wait per angle before the render thread drops one. Tied to
    /// the operator's queue setting, so a short encoder hiccup is absorbed instead
    /// of losing picture.
    cap: AtomicUsize,
    wake: Condvar,
}

/// One block of mixed audio waiting to be stored against the recording timeline.
struct AudioJob {
    angle_id: u32,
    frame_index: u64,
    sample_rate: u32,
    channels: u32,
    pcm: Vec<f32>,
}

/// Queue between the audio scheduler and the thread that writes PCM to disk.
struct AudioInbox {
    queue: Mutex<VecDeque<AudioJob>>,
    wake: Condvar,
}

impl AudioInbox {
    /// Blocks that may wait before the oldest is dropped. Audio is written in
    /// order, so a stalled disk loses the tail rather than corrupting the head.
    const CAPACITY: usize = 32;

    fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
        }
    }

    fn offer(&self, job: AudioJob) -> bool {
        let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        if queue.len() >= Self::CAPACITY {
            queue.pop_front();
            queue.push_back(job);
            self.wake.notify_one();
            return false;
        }
        queue.push_back(job);
        self.wake.notify_one();
        true
    }

    fn clear(&self) {
        self.queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }

    fn pop(&self, stop: &AtomicBool) -> Option<AudioJob> {
        let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        while queue.is_empty() && !stop.load(Ordering::Relaxed) {
            let (next, _) = self
                .wake
                .wait_timeout(queue, Duration::from_millis(50))
                .unwrap_or_else(|error| error.into_inner());
            queue = next;
        }
        queue.pop_front()
    }
}

impl Inbox {
    fn new() -> Self {
        Self {
            queues: Mutex::new(HashMap::new()),
            // Two frames is the old behaviour, used until a session configures a
            // real queue length.
            cap: AtomicUsize::new(2),
            wake: Condvar::new(),
        }
    }

    fn set_capacity(&self, cap: usize) {
        self.cap.store(cap.max(1), Ordering::Relaxed);
    }

    fn offer(&self, frame: RecordFrame) -> bool {
        let cap = self.cap.load(Ordering::Relaxed).max(1);
        let mut queues = self.queues.lock().unwrap_or_else(|error| error.into_inner());
        let queue = queues.entry(frame.angle_id).or_default();
        if queue.len() >= cap {
            return false;
        }
        queue.push_back(frame);
        self.wake.notify_one();
        true
    }

    fn pop(&self, stop: &AtomicBool) -> Option<RecordFrame> {
        let mut queues = self.queues.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if let Some(frame) = queues.values_mut().find_map(|queue| queue.pop_front()) {
                return Some(frame);
            }
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            let (guard, timeout) = self
                .wake
                .wait_timeout(queues, Duration::from_millis(50))
                .unwrap_or_else(|error| error.into_inner());
            queues = guard;
            if timeout.timed_out() && stop.load(Ordering::Relaxed) {
                return None;
            }
        }
    }
}

struct ChannelSlot {
    index: u32,
    source_id: u64,
    transport: channel::Transport,
    /// Playback audio cursor for this channel, kept outside the engine lock so a
    /// file read never blocks the render thread.
    audio: Arc<Mutex<audio::PlaybackAudio>>,
}

struct Inner {
    config: ReplayDto,
    /// Bumped whenever the playlist reaches the engine, so the playout loop can
    /// drop a stale snapshot instead of overwriting a newer one.
    playlist_revision: u64,
    recording: bool,
    uuid: String,
    mark_in: Option<u64>,
    has_frames: bool,
    last_frame: u64,
    first_frame: u64,
    dropped_readback: HashMap<u32, u64>,
    /// Frames the readback handed over while recording.
    offered: u64,
    /// Frames handed to the readback while recording.
    pushed: u64,
    recent_drops: u32,
    recent_at: Instant,
    epoch: Instant,
    write_drop_seen: u64,
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    stats: MixerReplayStats,
    angles: Vec<MixerReplayAngleStats>,
    channels: Vec<ChannelSlot>,
    /// Waveform buckets per angle, filled from the audio the scheduler mixes.
    peaks: HashMap<u32, eiviz_pro_api::PeakAccumulator>,
    health: HealthMemory,
    started_at: Option<Instant>,
}

pub struct ReplayEngine {
    inner: Mutex<Inner>,
    inbox: Arc<Inbox>,
    audio_inbox: Arc<AudioInbox>,
    audio_drops: AtomicU64,
    store: Mutex<Option<Arc<Mutex<Box<dyn ReplayStore>>>>>,
    stop_record: Arc<AtomicBool>,
    stop_workers: Arc<AtomicBool>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

/// Where a recording angle takes its picture from. A scene angle records the
/// scene's composited output; an input angle records the input's own picture
/// without composing a scene first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordSource {
    Scene(u64),
    Input(u64),
}

pub struct RecordTarget {
    pub source: RecordSource,
    pub readback_key: u64,
    pub width: u32,
    pub height: u32,
    pub angle_id: u32,
}

pub fn engine() -> &'static ReplayEngine {
    static ENGINE: OnceLock<ReplayEngine> = OnceLock::new();
    ENGINE.get_or_init(ReplayEngine::new)
}

impl ReplayEngine {
    fn new() -> Self {
        let engine = Self {
            inner: Mutex::new(Inner {
                config: ReplayDto::default(),
                playlist_revision: 1,
                recording: false,
                uuid: String::new(),
                mark_in: None,
                has_frames: false,
                last_frame: 0,
                first_frame: 0,
                dropped_readback: HashMap::new(),
                offered: 0,
                pushed: 0,
                recent_drops: 0,
                recent_at: Instant::now(),
                epoch: Instant::now(),
                write_drop_seen: 0,
                width: 1920,
                height: 1080,
                fps_num: 60,
                fps_den: 1,
                stats: MixerReplayStats::default(),
                angles: Vec::new(),
                channels: Vec::new(),
                peaks: HashMap::new(),
                health: HealthMemory::default(),
                started_at: None,
            }),
            inbox: Arc::new(Inbox::new()),
            audio_inbox: Arc::new(AudioInbox::new()),
            audio_drops: AtomicU64::new(0),
            store: Mutex::new(None),
            stop_record: Arc::new(AtomicBool::new(true)),
            stop_workers: Arc::new(AtomicBool::new(false)),
            workers: Mutex::new(Vec::new()),
        };
        engine.spawn_workers();
        engine
    }

    fn spawn_workers(&self) {
        let inbox = Arc::clone(&self.inbox);
        let stop = Arc::clone(&self.stop_record);
        let store_slot = Arc::new(());
        let _ = store_slot;
        // Workers are started on record/configure so the engine can be created
        // before the control service exists.
        let _ = (inbox, stop);
    }

    pub fn configure(&self, json: &str) -> Result<(), ReplayFailure> {
        let dto: Option<ReplayDto> = serde_json::from_str(json).map_err(|error| {
            ReplayFailure::invalid(format!("replay settings are not valid JSON: {error}"))
        })?;
        let Some(dto) = dto else {
            self.stop(REPLAY_STOP_USER);
            // Replay is gone from the project, so the recorded material goes with it.
            self.release_store();
            let mut inner = self.lock();
            inner.config = ReplayDto::default();
            inner.channels.clear();
            // Removing Replay leaves nothing to report, so the state goes back to
            // idle instead of holding the last recording's numbers.
            inner.stats.state = REPLAY_STATE_IDLE;
            inner.stats.stop_reason = REPLAY_STOP_NONE;
            return Ok(());
        };
        self.gate(&dto)?;
        {
            // The module was opened with a fixed angle set. A change mid-recording
            // cannot be honoured (the new angle has no earlier frames), so refuse
            // it explicitly instead of dropping frames for an unknown angle.
            let inner = self.lock();
            if inner.recording {
                let current: std::collections::BTreeSet<u32> =
                    inner.config.angles.iter().map(|angle| angle.id).collect();
                let next: std::collections::BTreeSet<u32> =
                    dto.angles.iter().map(|angle| angle.id).collect();
                if current != next {
                    return Err(ReplayFailure::invalid(
                        "replay angles cannot change while recording; stop the recording first",
                    ));
                }
            }
        }
        if !dto.enabled {
            self.stop(REPLAY_STOP_USER);
        }
        let mut inner = self.lock();
        let mut old = std::mem::take(&mut inner.channels);
        inner.config = dto;
        inner.playlist_revision = inner.playlist_revision.wrapping_add(1);
        let config = inner.config.clone();
        // The queue the operator asked for is the queue the encoder gets; the
        // records must wait the same number of frames in front of it.
        self.inbox.set_capacity(inbox_capacity(
            config.buffering.encode_queue_frames,
            config.format.width,
            config.format.height,
        ));
        inner.channels = config
            .channels
            .iter()
            .map(|channel| {
                let mut transport = old
                    .iter_mut()
                    .find(|slot| slot.index == channel.index)
                    .map(|slot| {
                        std::mem::replace(&mut slot.transport, channel::Transport::new())
                    })
                    .unwrap_or_else(channel::Transport::new);
                transport.set_playlist(playlist_clips(&config, channel.playlist_id));
                let audio = old
                    .iter()
                    .find(|slot| slot.index == channel.index)
                    .map(|slot| Arc::clone(&slot.audio))
                    .unwrap_or_else(|| Arc::new(Mutex::new(audio::PlaybackAudio::new(AUDIO_CHANNELS))));
                ChannelSlot {
                    index: channel.index,
                    source_id: if channel.input_id == 0 {
                        replay_channel_gpu_id(channel.index)
                    } else {
                        channel.input_id
                    },
                    transport,
                    audio,
                }
            })
            .collect();
        self.ensure_workers();
        Ok(())
    }

    fn gate(&self, dto: &ReplayDto) -> Result<(), ReplayFailure> {
        let ent = crate::pro::entitlements();
        // Counts are checked even while recording is stopped. A document that
        // exceeds the plan can never run, and accepting it hides the problem
        // until the operator presses record.
        if !dto.channels.is_empty()
            && !Entitlements::admits_count(ent.replay_channels, dto.channels.len() - 1, false)
        {
            return Err(ReplayFailure::plan(format!(
                "this plan allows {} Replay channels, the project has {}",
                quota_text(ent.replay_channels),
                dto.channels.len()
            )));
        }
        if !dto.angles.is_empty()
            && !Entitlements::admits_count(ent.replay_angles, dto.angles.len() - 1, false)
        {
            return Err(ReplayFailure::plan(format!(
                "this plan allows {} Replay angles, the project has {}",
                quota_text(ent.replay_angles),
                dto.angles.len()
            )));
        }
        if !dto.enabled {
            return Ok(());
        }
        if !Entitlements::admits_count(ent.replay_angles, 0, true) {
            return Err(ReplayFailure::plan(format!(
                "this plan allows {} Replay angles",
                quota_text(ent.replay_angles)
            )));
        }
        if crate::pro::module().replay().is_none() {
            return Err(ReplayFailure::io("replay requires the Pro module"));
        }
        Ok(())
    }

    pub fn record(&self, on: bool) -> Result<(), ReplayFailure> {
        if !on {
            self.stop(REPLAY_STOP_USER);
            return Ok(());
        }
        let (paths, width, height, fps_num, fps_den, profile, angles, reserve, queue, write_mb, overflow, retention, segment_frames, protect_from) = {
            let inner = self.lock();
            if !inner.config.enabled {
                return Err(ReplayFailure::invalid(
                    "replay is not enabled; apply Replay Setup with at least one drive and one angle",
                ));
            }
            if inner.config.storage.is_empty() {
                return Err(ReplayFailure::invalid(
                    "replay has no storage drive; add one in Replay Setup",
                ));
            }
            let mut paths = Vec::with_capacity(inner.config.storage.len());
            for (index, storage) in inner.config.storage.iter().enumerate() {
                if storage.path.trim().is_empty() {
                    return Err(ReplayFailure::invalid(format!(
                        "replay storage {index} has an empty path"
                    )));
                }
                paths.push(storage.path.clone());
            }
            // Anything a clip or a protected event still points at must survive an
            // overwrite, so the module learns the earliest such frame.
            let mut guard = u64::MAX;
            for event in &inner.config.events {
                if event.protected {
                    guard = guard.min(event.in_frame);
                }
            }
            for list in &inner.config.playlists {
                for clip in &list.clips {
                    guard = guard.min(clip.in_frame);
                }
            }
            (
                paths,
                inner.config.format.width,
                inner.config.format.height,
                inner.config.format.fps_num,
                inner.config.format.fps_den,
                profile_code(inner.config.format.profile),
                inner.config.angles.clone(),
                u64::from(inner.config.buffering.reserve_mb) * 1024 * 1024,
                inner.config.buffering.encode_queue_frames,
                inner.config.buffering.write_buffer_mb,
                overflow_code(inner.config.buffering.overflow_policy),
                retention_code(inner.config.retention),
                // The segment length is the operator's setting; the module must
                // not quietly substitute its own.
                inner.config.format.segment_frames,
                if guard == u64::MAX { 0 } else { guard },
            )
        };
        let (_, scene_ids, _) = crate::replay_record_context()
            .map_err(|_| ReplayFailure::io("the mixer has no recording context yet"))?;
        let angles = angles;
        if angles.is_empty() {
            return Err(ReplayFailure::invalid(
                "replay has no angles; add one in Replay Setup",
            ));
        }
        if width == 0 || height == 0 || fps_num == 0 || fps_den == 0 {
            return Err(ReplayFailure::invalid(
                "replay format needs a resolution and a frame rate; apply Replay Setup",
            ));
        }
        if !(SEGMENT_FRAMES_MIN..=SEGMENT_FRAMES_MAX).contains(&segment_frames) {
            return Err(ReplayFailure::invalid(format!(
                "replay segment frames must be between {SEGMENT_FRAMES_MIN} and {SEGMENT_FRAMES_MAX}, got {segment_frames}"
            )));
        }
        if !(ENCODE_QUEUE_MIN..=ENCODE_QUEUE_MAX).contains(&queue) {
            return Err(ReplayFailure::invalid(format!(
                "replay encode queue must be between {ENCODE_QUEUE_MIN} and {ENCODE_QUEUE_MAX} frames, got {queue}"
            )));
        }
        if !(WRITE_BUFFER_MIN_MB..=WRITE_BUFFER_MAX_MB).contains(&write_mb) {
            return Err(ReplayFailure::invalid(format!(
                "replay write buffer must be between {WRITE_BUFFER_MIN_MB} and {WRITE_BUFFER_MAX_MB} MiB, got {write_mb}"
            )));
        }
        for angle in &angles {
            match angle.source {
                AngleSourceKind::Scene => {
                    let gpu = eiviz_control::ids::scene_gpu_id(angle.scene_id);
                    if !scene_ids.contains(&gpu) && !scene_ids.contains(&angle.scene_id) {
                        return Err(ReplayFailure::io(format!(
                            "replay angle {} scene {} is not defined in the mixer",
                            angle.id, angle.scene_id
                        )));
                    }
                }
                AngleSourceKind::Input => {
                    // The session already refused an angle whose input does not
                    // exist. A source the renderer has no texture for is counted as
                    // a replaced frame rather than invented here.
                    if angle.input_id == 0 {
                        return Err(ReplayFailure::invalid(format!(
                            "replay angle {} records an input but names none",
                            angle.id
                        )));
                    }
                }
            }
        }
        if angles
            .iter()
            .any(|angle| angle.record_mode == AngleRecordMode::Passthrough)
        {
            // A passthrough angle stores the frames its OMT input already sends, so
            // it needs one. The session refuses the other combinations; saying it here
            // too keeps the mixer from recording an angle that can never fill.
            let missing = angles.iter().find(|angle| {
                angle.record_mode == AngleRecordMode::Passthrough
                    && (angle.source != AngleSourceKind::Input || angle.input_id == 0)
            });
            if let Some(angle) = missing {
                return Err(ReplayFailure::invalid(format!(
                    "replay angle {} passes frames through, so it must record an input",
                    angle.id
                )));
            }
        }
        // Free space has to be known before a recording starts: an unreadable
        // drive is not a full drive, and treating it as zero hid the real reason.
        let mut free: u64 = 0;
        for path in &paths {
            match free_bytes(path) {
                Some(bytes) => free = free.saturating_add(bytes),
                None => {
                    return Err(ReplayFailure::io(format!(
                        "cannot query free space at {path}"
                    )));
                }
            }
        }
        if free <= reserve {
            return Err(ReplayFailure::io(format!(
                "not enough free space: {free} bytes free, {reserve} bytes reserved"
            )));
        }
        let uuid = format!("rec-{}", uuid_stamp());
        let remembered = angles.clone();
        let backend = crate::pro::module()
            .replay()
            .ok_or_else(|| ReplayFailure::io("replay requires the Pro module"))?;
        // An angle records to the storage that names it. The mapping is explicit in
        // the session; guessing a drive would record to a disk the operator did not
        // choose, so an unassigned angle is a configuration error.
        let angle_targets = {
            let inner = self.lock();
            let mut targets = Vec::with_capacity(angles.len());
            for angle in &angles {
                let index = inner
                    .config
                    .storage
                    .iter()
                    .position(|storage| storage.angle_ids.contains(&angle.id))
                    .ok_or_else(|| {
                        ReplayFailure::invalid(format!(
                            "replay angle {} has no storage assigned",
                            angle.id
                        ))
                    })?;
                targets.push(eiviz_pro_api::ReplayAngleTarget {
                    angle_id: angle.id,
                    storage_index: index as u32,
                });
            }
            targets
        };
        let config = ReplayStoreConfig {
            recording_uuid: uuid.clone(),
            storages: paths
                .iter()
                .map(|path| eiviz_pro_api::ReplayStorageTarget { path: path.clone() })
                .collect(),
            angles: angle_targets,
            width,
            height,
            fps_num,
            fps_den,
            profile,
            segment_frames,
            encode_queue_frames: queue,
            write_buffer_bytes: u64::from(write_mb) * 1024 * 1024,
            overflow_policy: overflow,
            reserve_bytes: reserve,
            retention,
            protect_from_frame: protect_from,
        };
        let store = backend.open(&config).map_err(|error| {
            ReplayFailure::io(format!("the Pro module refused the recording: {error}"))
        })?;
        self.stop_record.store(false, Ordering::Relaxed);
        *self.store.lock().unwrap_or_else(|error| error.into_inner()) = Some(Arc::new(Mutex::new(store)));
        self.ensure_workers();
        let mut inner = self.lock();
        inner.recording = true;
        inner.uuid = uuid;
        inner.mark_in = None;
        // A new recording starts a new waveform and a new audio tail.
        inner.peaks.clear();
        self.audio_inbox.clear();
        inner.width = width;
        inner.height = height;
        inner.fps_num = fps_num;
        inner.fps_den = fps_den;
        inner.started_at = Some(Instant::now());
        inner.first_frame = 0;
        inner.last_frame = 0;
        inner.has_frames = false;
        inner.stats.state = REPLAY_STATE_RECORDING;
        inner.stats.stop_reason = REPLAY_STOP_NONE;
        inner.stats.remaining_is_estimate = 1;
        inner.config.angles = remembered;
        Ok(())
    }

    pub fn stop(&self, reason: u32) {
        self.stop_record.store(true, Ordering::Relaxed);
        self.inbox.wake.notify_all();
        // The recording stops, but what it recorded stays readable: marking and
        // playing a clip is a timeline operation, and an operator who stops the
        // recorder still cuts from the material that is already on disk. The store
        // is closed when Replay itself goes away (configure null / shutdown).
        let mut inner = self.lock();
        if inner.recording || inner.stats.state == REPLAY_STATE_RECORDING {
            inner.recording = false;
            inner.stats.state = REPLAY_STATE_STOPPED;
            inner.stats.stop_reason = reason;
        }
        // The waveform keeps what was recorded, including the last partial bucket.
        for accumulator in inner.peaks.values_mut() {
            accumulator.flush();
        }
    }

    /// Releases the recorded material. Only when Replay is removed or the mixer
    /// shuts down: a stopped recorder keeps its frames.
    fn release_store(&self) {
        if let Some(store) = self.store.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            if let Ok(mut store) = store.lock() {
                let _ = store.close();
            }
        }
    }

    pub fn shutdown(&self) {
        self.stop(REPLAY_STOP_USER);
        self.release_store();
        self.stop_workers.store(true, Ordering::Relaxed);
        self.inbox.wake.notify_all();
        let mut workers = self.workers.lock().unwrap_or_else(|error| error.into_inner());
        for worker in workers.drain(..) {
            let _ = worker.join();
        }
        self.stop_workers.store(false, Ordering::Relaxed);
    }

    pub fn note_fps(&self, fps_num: u32, fps_den: u32) {
        let mut inner = self.lock();
        if inner.recording && (inner.fps_num != fps_num || inner.fps_den != fps_den) {
            drop(inner);
            self.stop(REPLAY_STOP_FPS_CHANGED);
            return;
        }
        inner.fps_num = fps_num;
        inner.fps_den = fps_den;
    }

    pub fn record_targets(&self) -> Vec<RecordTarget> {
        let inner = self.lock();
        if !inner.recording {
            return Vec::new();
        }
        inner
            .config
            .angles
            .iter()
            // A passthrough angle arrives already encoded; it never goes through the
            // readback path.
            .filter(|angle| angle.record_mode != AngleRecordMode::Passthrough)
            .map(|angle| RecordTarget {
                source: match angle.source {
                    AngleSourceKind::Input => RecordSource::Input(angle.input_id),
                    AngleSourceKind::Scene => RecordSource::Scene(eiviz_control::ids::scene_gpu_id(
                        angle.scene_id,
                    )),
                },
                readback_key: REPLAY_RECORD_BASE | u64::from(angle.id),
                width: inner.width,
                height: inner.height,
                angle_id: angle.id,
            })
            .collect()
    }

    /// Scenes the recorder reads. Only scene angles need them drawn.
    pub fn replay_scene_ids(&self) -> Vec<u64> {
        self.record_targets()
            .into_iter()
            .filter_map(|target| match target.source {
                RecordSource::Scene(gpu_id) => Some(gpu_id),
                RecordSource::Input(_) => None,
            })
            .collect()
    }

    /// Angle scenes that must stay rendered even while recording is stopped, so
    /// the operator can watch the cameras in the Replay window before pressing
    /// record. Full-quality receive stays limited to `record_targets`.
    pub fn live_angle_scene_ids(&self) -> Vec<u64> {
        let inner = self.lock();
        let mut ids: Vec<u64> = inner
            .config
            .angles
            .iter()
            .filter_map(|angle| match angle.source {
                AngleSourceKind::Scene => {
                    Some(eiviz_control::ids::scene_gpu_id(angle.scene_id))
                }
                AngleSourceKind::Input => None,
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Inputs the recorder reads directly. Their receivers stay at full quality
    /// while recording, exactly as an angle scene does.
    pub fn live_angle_input_ids(&self) -> Vec<u64> {
        let inner = self.lock();
        let mut ids: Vec<u64> = inner
            .config
            .angles
            .iter()
            .filter_map(|angle| match angle.source {
                AngleSourceKind::Input => Some(angle.input_id),
                AngleSourceKind::Scene => None,
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// The passthrough angle recording this input while a recording is running, if
    /// any. A receiver asks for the bitstream only while one is.
    pub fn passthrough_angle_for(&self, input_id: u64) -> Option<u32> {
        let inner = self.lock();
        if !inner.recording {
            return None;
        }
        inner
            .config
            .angles
            .iter()
            .find(|angle| {
                angle.source == AngleSourceKind::Input
                    && angle.record_mode == AngleRecordMode::Passthrough
                    && angle.input_id == input_id
            })
            .map(|angle| angle.id)
    }

    /// Stores a frame a passthrough angle received, exactly as the sender encoded
    /// it. The store keeps the bitstream, so the clip plays back what the camera
    /// sent rather than a re-encode of it.
    ///
    /// Returns whether the store took it. A refusal is counted against the angle, so
    /// a module that cannot keep a bitstream shows up as losses instead of as a
    /// recording that quietly holds nothing.
    pub fn write_encoded_frame(&self, angle_id: u32, frame: &EncodedFrame) -> bool {
        let store = self
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let Some(store) = store else {
            return false;
        };
        let frame_index = {
            let mut inner = self.lock();
            if !inner.recording {
                return false;
            }
            // The passthrough angle keeps the timeline moving even though the frames
            // arrive from the sender: marks and clips are cut on that timeline.
            let index = inner.last_frame.saturating_add(1);
            if !inner.has_frames {
                inner.first_frame = index;
                inner.has_frames = true;
            }
            inner.last_frame = index;
            inner.stats.recording_frame = index;
            inner.stats.last_frame = index;
            index
        };
        let outcome = store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .write_encoded(angle_id, frame_index, frame);
        match outcome {
            Ok(WriteOutcome::Accepted) => {
                let mut inner = self.lock();
                inner.stats.encoded_frames = inner.stats.encoded_frames.saturating_add(1);
                true
            }
            Ok(WriteOutcome::Dropped) => {
                let mut inner = self.lock();
                inner.stats.encoded_dropped = inner.stats.encoded_dropped.saturating_add(1);
                false
            }
            Err(error) => {
                let mut inner = self.lock();
                inner.stats.encoded_dropped = inner.stats.encoded_dropped.saturating_add(1);
                inner.stats.issues |= eiviz_pro_api::ISSUE_STORAGE_WRITE;
                crate::diag::error(&format!("replay passthrough write failed: {error}"));
                false
            }
        }
    }

    pub fn offer_frame(&self, angle_id: u32, frame_index: u64, pts: i64, data: Vec<u8>, stride: usize) {
        let accepted = self.inbox.offer(RecordFrame {
            angle_id,
            frame_index,
            pts,
            stride,
            data,
        });
        if !accepted {
            let mut inner = self.lock();
            *inner.dropped_readback.entry(angle_id).or_default() += 1;
            inner.recent_drops = inner.recent_drops.saturating_add(1);
        } else {
            let mut inner = self.lock();
            if !inner.has_frames {
                inner.first_frame = frame_index;
                inner.has_frames = true;
            }
            inner.last_frame = inner.last_frame.max(frame_index);
            inner.stats.recording_frame = inner.last_frame;
            inner.stats.last_frame = inner.last_frame;
            // Offered counts what the readback delivered; the encoder's own count
            // comes from the module, so the difference says where frames are lost.
            inner.offered += 1;
        }
    }

    /// Counts a frame handed to the readback. `frames_offered` should match it.
    pub fn note_pushed(&self) {
        let mut inner = self.lock();
        inner.pushed += 1;
    }

    /// Records readback frames that finished but were replaced before the disk
    /// writer could take them.
    pub fn note_replaced(&self, angle_id: u32, count: u64) {
        if count == 0 {
            return;
        }
        let mut inner = self.lock();
        *inner.dropped_readback.entry(angle_id).or_default() += count;
        let capped = count.min(u64::from(u32::MAX)) as u32;
        inner.recent_drops = inner.recent_drops.saturating_add(capped);
    }

    /// Feeds the audio the scheduler just mixed into the waveform buckets of every
    /// angle that records from a mixing unit.
    pub fn offer_audio(&self, by_unit: &HashMap<u64, Vec<f32>>) {
        let mut jobs: Vec<AudioJob> = Vec::new();
        {
            let mut inner = self.lock();
            if !inner.recording {
                return;
            }
            let targets: Vec<(u32, u64)> = inner
                .config
                .angles
                .iter()
                .filter(|angle| {
                    angle.audio == eiviz_control::session::replay::AngleAudio::Unit
                        && angle.audio_unit_id != 0
                })
                .map(|angle| (angle.id, angle.audio_unit_id))
                .collect();
            let frame_index = inner.last_frame;
            for (angle_id, unit_id) in targets {
                let Some(pcm) = by_unit.get(&unit_id) else {
                    continue;
                };
                inner
                    .peaks
                    .entry(angle_id)
                    .or_insert_with(|| eiviz_pro_api::PeakAccumulator::new(AUDIO_SAMPLE_RATE))
                    .push(pcm, AUDIO_CHANNELS);
                // The block is stored against the frame it was heard on, so audio
                // and video share one timeline.
                jobs.push(AudioJob {
                    angle_id,
                    frame_index,
                    sample_rate: AUDIO_SAMPLE_RATE,
                    channels: AUDIO_CHANNELS as u32,
                    pcm: pcm.clone(),
                });
            }
        }
        for job in jobs {
            if !self.audio_inbox.offer(job) {
                self.audio_drops.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Blocks dropped because the audio writer could not keep up.
    pub fn audio_drops(&self) -> u64 {
        self.audio_drops.load(Ordering::Relaxed)
    }

    pub fn peak_count(&self, angle_id: u32) -> u32 {
        let inner = self.lock();
        inner
            .peaks
            .get(&angle_id)
            .map(eiviz_pro_api::PeakAccumulator::bucket_count)
            .unwrap_or(0)
    }

    /// Buckets evicted from the front of an angle's ring.
    pub fn peak_dropped(&self, angle_id: u32) -> u32 {
        let inner = self.lock();
        inner
            .peaks
            .get(&angle_id)
            .map(eiviz_pro_api::PeakAccumulator::dropped_buckets)
            .unwrap_or(0)
    }

    /// Copies an angle's waveform. Returns how many buckets were written.
    pub fn copy_peaks(&self, angle_id: u32, from: u32, min_out: &mut [f32], max_out: &mut [f32]) -> u32 {
        let live = {
            let inner = self.lock();
            inner
                .peaks
                .get(&angle_id)
                .map(|accumulator| accumulator.copy(from, min_out, max_out) as u32)
                .unwrap_or(0)
        };
        if live > 0 {
            return live;
        }
        let Some(store) = self
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
        else {
            return 0;
        };
        store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .copy_peaks(angle_id, from, min_out, max_out)
            .unwrap_or(0)
    }

    /// Marks the in point on the recording timeline. Marking is a timeline
    /// operation, not a recording one: an operator marks material that is already
    /// recorded, so a stopped recorder can still be marked. Only an empty timeline
    /// has nothing to mark.
    pub fn mark_in(&self) -> Result<u64, ReplayFailure> {
        let mut inner = self.lock();
        if !inner.has_frames {
            return Err(ReplayFailure::io(
                "there is nothing to mark yet: the recording has no frames",
            ));
        }
        let frame = inner.last_frame;
        inner.mark_in = Some(frame);
        inner.stats.mark_in_frame = frame;
        inner.stats.mark_in_valid = 1;
        Ok(frame)
    }

    /// Closes the mark. A range with no frames in it is refused: it would become a
    /// clip that can never show anything, which is worse than being told to record
    /// something first. The mark stays open so the operator can close it once the
    /// recording has run again.
    pub fn close_mark(&self) -> Result<(String, u64, u64), ReplayFailure> {
        let mut inner = self.lock();
        let in_frame = inner
            .mark_in
            .ok_or_else(|| ReplayFailure::invalid("no Mark In is open"))?;
        let out_frame = inner.last_frame.max(in_frame);
        if out_frame <= in_frame {
            return Err(ReplayFailure::invalid(
                "the mark is empty: nothing was recorded after Mark In",
            ));
        }
        inner.mark_in = None;
        inner.stats.mark_in_valid = 0;
        Ok((inner.uuid.clone(), in_frame, out_frame))
    }

    pub fn cue(
        &self,
        channel: u32,
        angle_id: u32,
        in_frame: u64,
        out_frame: u64,
        recording_uuid: &str,
    ) -> Result<(), i32> {
        let mut inner = self.lock();
        let slot = inner
            .channels
            .iter_mut()
            .find(|slot| slot.index == channel)
            .ok_or(ERR_INVALID_ARGUMENT)?;
        slot.transport.cue(channel::Cue {
            recording_uuid: recording_uuid.to_string(),
            angle_id,
            in_frame,
            out_frame,
        });
        Ok(())
    }

    pub fn transport(
        &self,
        channel: u32,
        playing: bool,
        speed_milli: i32,
        seek_frame: i64,
    ) -> Result<(), i32> {
        let mut inner = self.lock();
        let slot = inner
            .channels
            .iter_mut()
            .find(|slot| slot.index == channel)
            .ok_or(ERR_INVALID_ARGUMENT)?;
        slot.transport.set_playing(playing);
        if speed_milli != 0 {
            slot.transport.set_rate(f64::from(speed_milli) / 1000.0);
        }
        if seek_frame >= 0 {
            slot.transport.seek_source(seek_frame as u64);
        }
        Ok(())
    }

    pub fn jump_to_now(&self, channel: u32) -> Result<(), i32> {
        let mut inner = self.lock();
        let frame = inner.last_frame.saturating_sub(3);
        let angle = inner
            .config
            .angles
            .first()
            .map(|angle| angle.id)
            .unwrap_or(0);
        let slot = inner
            .channels
            .iter_mut()
            .find(|slot| slot.index == channel)
            .ok_or(ERR_INVALID_ARGUMENT)?;
        // Live follows the recording edge; a rate below 1 drifts into slow
        // motion without leaving the live feed.
        slot.transport.jump_to_now(frame, angle);
        slot.transport.set_playing(true);
        Ok(())
    }

    pub fn copy_stats(&self, out: &mut MixerReplayStats, angles: &mut [MixerReplayAngleStats]) -> u32 {
        self.refresh_stats();
        let inner = self.lock();
        *out = inner.stats;
        let n = inner.angles.len().min(angles.len());
        angles[..n].copy_from_slice(&inner.angles[..n]);
        n as u32
    }

    pub fn copy_channels(&self, out: &mut [MixerReplayChannelState]) -> u32 {
        let inner = self.lock();
        let n = inner.channels.len().min(out.len());
        for (dst, src) in out.iter_mut().zip(inner.channels.iter()) {
            let clip = src.transport.clip();
            *dst = MixerReplayChannelState {
                channel_index: src.index,
                source_id: src.source_id,
                playing: u32::from(src.transport.playing),
                speed_milli: (src.transport.rate() * 1000.0) as i32,
                position_frame: src.transport.source_frame(),
                in_frame: clip.map(|clip| clip.in_frame).unwrap_or(0),
                out_frame: clip.map(|clip| clip.out_frame).unwrap_or(0),
                angle_id: clip.map(|clip| clip.angle_id).unwrap_or(0),
                clip_uid_hash: clip.map(|clip| clip_uid_hash(&clip.uid)).unwrap_or(0),
                substituted_frames: src.transport.substituted,
                decode_ms_avg_milli: if src.transport.decode_samples == 0 {
                    0
                } else {
                    (src.transport.decode_us_sum / src.transport.decode_samples / 1000) as u32
                },
                decode_misses: src.transport.substituted,
            };
        }
        n as u32
    }

    pub fn recording_uuid(&self) -> String {
        self.lock().uuid.clone()
    }

    /// Recorded frame range of one angle, for the export UI.
    pub fn angle_range(&self, angle_id: u32) -> Option<(u64, u64)> {
        let store = self
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()?;
        store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .range(angle_id)
            .ok()
            .flatten()
    }

    /// Installs a channel's clips without going through the session gate. Test
    /// support, so the cue resolution can be checked on its own.
    #[cfg(test)]
    pub fn test_set_playlist(&self, channel: u32, source_id: u64, clips: Vec<channel::Clip>) {
        let mut inner = self.lock();
        inner.channels.retain(|slot| slot.index != channel);
        let mut transport = channel::Transport::new();
        transport.set_playlist(clips);
        inner.channels.push(ChannelSlot {
            index: channel,
            source_id,
            transport,
            audio: Arc::new(Mutex::new(audio::PlaybackAudio::new(AUDIO_CHANNELS))),
        });
    }

    /// Clip and playing state of a channel. Test support for the animation hook.
    #[cfg(test)]
    pub fn animation_channel_clip(&self, channel: u32) -> Option<(String, bool)> {
        let inner = self.lock();
        inner
            .channels
            .iter()
            .find(|slot| slot.index == channel)
            .and_then(|slot| {
                slot.transport
                    .clip()
                    .map(|clip| (clip.uid.clone(), slot.transport.playing))
            })
    }

    /// Cues a channel because an animation step asked for it. The clip is named by
    /// the session, so the clip is resolved against the channel's playlist here.
    pub fn fire_animation_action(
        &self,
        channel: u32,
        clip_uid: &str,
        play: bool,
    ) -> Result<(), i32> {
        let mut inner = self.lock();
        let Some(slot) = inner.channels.iter_mut().find(|slot| slot.index == channel) else {
            return Err(ERR_INVALID_ARGUMENT);
        };
        let index = if clip_uid.is_empty() {
            slot.transport.clips().len().checked_sub(1)
        } else {
            slot.transport
                .clips()
                .iter()
                .position(|clip| clip.uid == clip_uid)
        };
        let Some(index) = index else {
            return Err(ERR_INVALID_ARGUMENT);
        };
        let clip = slot.transport.clips()[index].clone();
        slot.transport.set_clip(clip, index);
        slot.transport.set_playing(play);
        Ok(())
    }

    /// Starts an MP4 export of the requested segments.
    pub fn export_start(&self, spec: eiviz_pro_api::ReplayExportSpec) -> Result<(), i32> {
        let store = self
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or(ERR_IO)?;
        store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .export_start(spec)
            .map_err(|_| ERR_IO)
    }

    /// Progress of the running export, or an idle snapshot when none is running.
    pub fn export_progress(&self) -> eiviz_pro_api::ReplayExportProgress {
        let Some(store) = self
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
        else {
            return eiviz_pro_api::ReplayExportProgress::default();
        };
        store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .export_progress()
            .unwrap_or_default()
    }

    pub fn export_cancel(&self) -> Result<(), i32> {
        let store = self
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or(ERR_IO)?;
        store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .export_cancel()
            .map_err(|_| ERR_IO)
    }

    /// Live Replay state as JSON. `GetReplayState` needs this because the session
    /// document alone cannot tell a client what is playing.
    pub fn state_json(&self) -> String {
        let mut stats = MixerReplayStats::default();
        let mut angle_stats = [MixerReplayAngleStats::default(); 32];
        let angle_count = self.copy_stats(&mut stats, &mut angle_stats) as usize;
        let mut channel_states = [MixerReplayChannelState::default(); 26];
        let channel_count = self.copy_channels(&mut channel_states) as usize;
        let channels: Vec<serde_json::Value> = channel_states[..channel_count]
            .iter()
            .map(|channel| {
                serde_json::json!({
                    "index": channel.channel_index,
                    "sourceId": channel.source_id,
                    "playing": channel.playing != 0,
                    "speedMilli": channel.speed_milli,
                    "positionFrame": channel.position_frame,
                    "inFrame": channel.in_frame,
                    "outFrame": channel.out_frame,
                    "angleId": channel.angle_id,
                    "substitutedFrames": channel.substituted_frames,
                    "decodeMsAvgMilli": channel.decode_ms_avg_milli,
                })
            })
            .collect();
        let angles: Vec<serde_json::Value> = angle_stats[..angle_count]
            .iter()
            .map(|angle| {
                serde_json::json!({
                    "angleId": angle.angle_id,
                    "sceneGpuId": angle.scene_gpu_id,
                    "encodeMsAvgMilli": angle.encode_ms_avg_milli,
                    "encodeQueueLen": angle.encode_queue_len,
                    "writeLatencyP95Ms": angle.write_latency_p95_ms,
                    "framesWritten": angle.frames_written,
                    "droppedReadback": angle.dropped_readback,
                    "droppedEncode": angle.dropped_encode,
                    "droppedWrite": angle.dropped_write,
                    "bytesWritten": angle.bytes_written,
                    "segments": angle.segments,
                })
            })
            .collect();
        serde_json::json!({
            "state": stats.state,
            "stopReason": stats.stop_reason,
            "severity": stats.severity,
            "issues": stats.issues,
            "recordingFrame": stats.recording_frame,
            "firstFrame": stats.first_frame,
            "lastFrame": stats.last_frame,
            "remainingSecs": stats.remaining_secs,
            "remainingIsEstimate": stats.remaining_is_estimate != 0,
            "framesWritten": stats.frames_written,
            "framesDropped": stats.frames_dropped,
            // Passthrough angles: stored as the sender encoded them, never read back
            // or encoded again. `encodedDropped` is a module that could not keep them.
            "encodedFrames": stats.encoded_frames,
            "encodedDropped": stats.encoded_dropped,
            "markInFrame": stats.mark_in_frame,
            "markInValid": stats.mark_in_valid != 0,
            "recordingUuid": self.recording_uuid(),
            "audioBlocksDropped": self.audio_drops(),
            "angles": angles,
            "channels": channels,
        })
        .to_string()
    }

    pub fn estimate(
        path: &str,
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
        profile: u32,
        angles: u32,
        reserve_bytes: u64,
    ) -> MixerReplayEstimateOut {
        if let Some(backend) = crate::pro::module().replay()
            && let Ok(estimate) = backend.estimate(
                path,
                width,
                height,
                fps_num,
                fps_den,
                profile,
                angles,
                reserve_bytes,
            )
        {
            return MixerReplayEstimateOut {
                free_bytes: estimate.free_bytes,
                estimate_rate_bps: estimate.estimate_rate_bps,
                remaining_secs: estimate.remaining_secs,
                path_writable: u32::from(estimate.path_writable),
            };
        }
        let _ = (path, width, height, fps_num, fps_den, profile, angles, reserve_bytes);
        MixerReplayEstimateOut::default()
    }

    fn ensure_workers(&self) {
        let mut workers = self.workers.lock().unwrap_or_else(|error| error.into_inner());
        if !workers.is_empty() {
            return;
        }
        let inbox = Arc::clone(&self.inbox);
        let stop_record = Arc::clone(&self.stop_record);
        let stop_workers = Arc::clone(&self.stop_workers);
        workers.push(
            thread::Builder::new()
                .name("eiviz-replay-rec".into())
                .spawn(move || record_loop(inbox, stop_record, stop_workers))
                .expect("replay record thread"),
        );
        let stop_workers = Arc::clone(&self.stop_workers);
        workers.push(
            thread::Builder::new()
                .name("eiviz-replay-health".into())
                .spawn({
                    let stop_workers = Arc::clone(&stop_workers);
                    move || health_loop(stop_workers)
                })
                .expect("replay health thread"),
        );
        let audio_inbox = Arc::clone(&self.audio_inbox);
        let audio_stop = Arc::clone(&stop_workers);
        workers.push(
            thread::Builder::new()
                .name("eiviz-replay-audio".into())
                .spawn(move || audio_loop(audio_inbox, audio_stop))
                .expect("replay audio thread"),
        );
        workers.push(
            thread::Builder::new()
                .name("eiviz-replay-ch-A".into())
                .spawn(move || channel_loop(stop_workers))
                .expect("replay channel thread"),
        );
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn refresh_stats(&self) {
        let store = self
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let Some(store) = store else {
            return;
        };
        let snapshot = store.lock().unwrap_or_else(|error| error.into_inner()).stats();
        let mut inner = self.lock();
        apply_pro_stats(&mut inner, &snapshot);
    }
}

fn apply_pro_stats(inner: &mut Inner, snapshot: &ReplayStats) {
    inner.stats.state = if inner.recording {
        STATE_RECORDING
    } else if snapshot.state == STATE_STOPPED {
        REPLAY_STATE_STOPPED
    } else {
        inner.stats.state
    };
    if snapshot.stop_reason != 0 {
        inner.stats.stop_reason = snapshot.stop_reason;
    }
    inner.stats.free_bytes = snapshot.free_bytes;
    inner.stats.reserve_bytes = snapshot.reserve_bytes;
    inner.stats.write_rate_bps = snapshot.write_rate_bps;
    inner.stats.write_rate_instant_bps = snapshot.write_rate_instant_bps;
    inner.stats.frames_written = snapshot.frames_written;
    inner.stats.frames_offered = inner.offered;
    inner.stats.frames_pushed = inner.pushed;
    inner.stats.frames_dropped = snapshot.frames_dropped;
    inner.stats.frames_dropped_recent = snapshot.frames_dropped_recent;
    inner.stats.first_frame = snapshot.first_frame;
    inner.stats.last_frame = snapshot.last_frame.max(inner.last_frame);
    inner.stats.channel_count = inner.channels.len() as u32;
    let rate = estimate_rate_bps(
        profile_code(inner.config.format.profile),
        inner.width,
        inner.height,
        inner.fps_num,
        inner.fps_den,
        inner.config.angles.len().max(1) as u32,
    );
    inner.stats.estimate_rate_bps = rate;
    let measured = inner.started_at.is_some_and(|start| start.elapsed() >= Duration::from_secs(5));
    let pace = if measured && snapshot.write_rate_bps > 0 {
        snapshot.write_rate_bps.max(rate / 2)
    } else {
        rate
    };
    inner.stats.remaining_is_estimate = u32::from(!measured || snapshot.write_rate_bps == 0);
    // A store that has not measured the drive reports zero free bytes, and zero seconds
    // left would read as "the disk is full" while nothing is known. A disk that really
    // is full says so through its own issue flag, so unknown is reported as unknown and
    // the operator is not sent to clear space that is already there.
    inner.stats.remaining_secs = if snapshot.free_bytes == 0 {
        u32::MAX
    } else {
        remaining_secs(
            snapshot.free_bytes.saturating_sub(snapshot.reserve_bytes),
            pace,
        )
    };
    // One entry per configured angle, so the operator sees the readback drops and
    // the queue even when the module has not reported that angle yet.
    let configured: Vec<(u32, u64)> = inner
        .config
        .angles
        .iter()
        .map(|angle| (angle.id, eiviz_control::ids::scene_gpu_id(angle.scene_id)))
        .collect();
    let mut angles = Vec::with_capacity(configured.len());
    for (angle_id, scene_gpu_id) in configured {
        let empty = ReplayAngleStats {
            angle_id,
            ..ReplayAngleStats::default()
        };
        let reported = snapshot
            .angles
            .iter()
            .find(|angle| angle.angle_id == angle_id)
            .unwrap_or(&empty);
        angles.push(angle_stats(reported, &inner, scene_gpu_id));
    }
    inner.angles = angles;
    // What the copy actually reports, not what the module happened to send.
    inner.stats.angle_count = inner.angles.len() as u32;
}

fn angle_stats(
    angle: &ReplayAngleStats,
    inner: &Inner,
    scene_gpu_id: u64,
) -> MixerReplayAngleStats {
    MixerReplayAngleStats {
        angle_id: angle.angle_id,
        scene_gpu_id,
        encode_ms_avg_milli: angle.encode_ms_avg_milli,
        encode_ms_max_milli: angle.encode_ms_max_milli,
        encode_queue_len: angle.encode_queue_len,
        encode_queue_cap: angle.encode_queue_cap,
        write_buffer_bytes: angle.write_buffer_bytes,
        write_buffer_cap: angle.write_buffer_cap,
        write_latency_p95_ms: angle.write_latency_p95_ms,
        frames_written: angle.frames_written,
        dropped_readback: inner.dropped_readback.get(&angle.angle_id).copied().unwrap_or(0),
        dropped_encode: angle.dropped_encode,
        dropped_write: angle.dropped_write,
        bytes_written: angle.bytes_written,
        segments: angle.segments,
    }
}

fn record_loop(inbox: Arc<Inbox>, stop_record: Arc<AtomicBool>, stop_workers: Arc<AtomicBool>) {
    let engine = engine();
    // `stop_record` is true until the user starts, and again after Stop.
    // Exiting on that flag used to kill the thread during session load, and
    // `ensure_workers` never starts a second one.
    while let Some(frame) = inbox.pop(&stop_workers) {
        if stop_record.load(Ordering::Relaxed) {
            continue;
        }
        let store = engine
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let Some(store) = store else {
            continue;
        };
        // One guard. A second `lock()` in the same statement deadlocks this
        // thread while it still holds the first guard, and the render thread
        // then blocks in `offer_frame`.
        let (width, height, fps_num, fps_den) = {
            let inner = engine.lock();
            (inner.width, inner.height, inner.fps_num, inner.fps_den)
        };
        let view = VideoFrame {
            width,
            height,
            stride: frame.stride as u32,
            layout: PixelLayout::Bgra,
            pts_100ns: frame.pts,
            fps_num,
            fps_den,
            data: frame.data.into(),
        };
        let outcome = store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .write_video(frame.angle_id, frame.frame_index, &view);
        if matches!(outcome, Err(_)) {
            engine.stop(crate::abi::REPLAY_STOP_IO_ERROR);
            break;
        }
        let _ = outcome;
    }
}

fn audio_loop(inbox: Arc<AudioInbox>, stop_workers: Arc<AtomicBool>) {
    let engine = engine();
    while !stop_workers.load(Ordering::Relaxed) {
        let Some(job) = inbox.pop(&stop_workers) else {
            continue;
        };
        if engine.stop_record.load(Ordering::Relaxed) {
            continue;
        }
        let store = engine
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let Some(store) = store else {
            continue;
        };
        let written = store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .write_audio(
                job.angle_id,
                job.frame_index,
                job.sample_rate,
                job.channels,
                &job.pcm,
            );
        if written.is_err() {
            engine.audio_drops.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn health_loop(stop: Arc<AtomicBool>) {
    let engine = engine();
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(250));
        engine.refresh_stats();
        let (sample, now_ms) = {
            let inner = engine.lock();
            let ratio = inner
                .angles
                .iter()
                .map(|angle| {
                    if angle.encode_queue_cap == 0 {
                        0.0
                    } else {
                        angle.encode_queue_len as f32 / angle.encode_queue_cap as f32
                    }
                })
                .fold(0.0f32, f32::max);
            let write_ratio = inner
                .angles
                .iter()
                .map(|angle| {
                    if angle.write_buffer_cap == 0 {
                        0.0
                    } else {
                        angle.write_buffer_bytes as f32 / angle.write_buffer_cap as f32
                    }
                })
                .fold(0.0f32, f32::max);
            let period = if inner.fps_num == 0 {
                16.7
            } else {
                1000.0 * inner.fps_den as f32 / inner.fps_num as f32
            };
            let write_drops: u64 = inner.angles.iter().map(|angle| angle.dropped_write).sum();
            let dropped_write_recent = write_drops.saturating_sub(inner.write_drop_seen) as u32;
            (
                HealthSample {
                    encode_queue_ratio: ratio,
                    encode_ms_avg: inner
                        .angles
                        .iter()
                        .map(|angle| angle.encode_ms_avg_milli)
                        .max()
                        .unwrap_or(0) as f32,
                    frame_period_ms: period,
                    dropped_encode_recent: inner.stats.frames_dropped_recent,
                    write_buffer_ratio: write_ratio,
                    write_latency_p95_ms: inner
                        .angles
                        .iter()
                        .map(|angle| angle.write_latency_p95_ms)
                        .max()
                        .unwrap_or(0),
                    dropped_write_recent,
                    dropped_readback_recent: inner.recent_drops,
                    remaining_secs: inner.stats.remaining_secs,
                    warn_secs: inner.config.warnings.remaining_warn_secs,
                    alert_secs: inner.config.warnings.remaining_alert_secs,
                    critical_secs: inner.config.warnings.remaining_critical_secs,
                    storage_full: inner.stats.free_bytes > 0
                        && inner.stats.free_bytes <= inner.stats.reserve_bytes,
                    stop_reason: inner.stats.stop_reason,
                    pro_unavailable: crate::pro::module().replay().is_none() && inner.config.enabled,
                    // Only a measured free-space figure can be compared with the
                    // thresholds. A store that has not reported one yet (or cannot
                    // read the drive) reports zero, which is not "no room left".
                    storage_tracked: inner.stats.free_bytes > 0,
                },
                inner.epoch.elapsed().as_millis() as u64,
            )
        };
        let view = {
            let mut inner = engine.lock();
            let write_drops: u64 = inner.angles.iter().map(|angle| angle.dropped_write).sum();
            inner.write_drop_seen = write_drops;
            if inner.recent_at.elapsed() >= Duration::from_secs(5) {
                inner.recent_drops = 0;
                inner.recent_at = Instant::now();
            }
            health_step(&mut inner.health, &sample, now_ms)
        };
        if view.changed {
            let message = health_message(view.issues, view.severity);
            crate::diag::warn_throttled("replay.health", &message);
            if let Ok(mut svc) = crate::runtime::control().try_lock() {
                let (stop_reason, remaining_secs) = {
                    let inner = engine.lock();
                    (inner.stats.stop_reason, inner.stats.remaining_secs)
                };
                svc.publish_replay_health(
                    view.severity,
                    view.issues,
                    stop_reason,
                    remaining_secs,
                    message,
                );
            }
            let mut inner = engine.lock();
            inner.stats.issues = view.issues;
            inner.stats.severity = view.severity;
        }
    }
}

fn channel_loop(stop: Arc<AtomicBool>) {
    let engine = engine();
    while !stop.load(Ordering::Relaxed) {
        let (slots, period, last_frame, fps) = {
            let inner = engine.lock();
            let period = Duration::from_secs_f64(
                f64::from(inner.fps_den.max(1)) / f64::from(inner.fps_num.max(1)),
            );
            let revision = inner.playlist_revision;
            let slots = inner
                .channels
                .iter()
                .map(|slot| {
                    (
                        slot.index,
                        slot.source_id,
                        slot.transport.clone(),
                        Arc::clone(&slot.audio),
                        revision,
                    )
                })
                .collect::<Vec<_>>();
            (slots, period, inner.last_frame, (inner.fps_num, inner.fps_den))
        };
        let store = engine
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let (width, height) = {
            let inner = engine.lock();
            (inner.width, inner.height)
        };
        for (index, source_id, mut transport, audio, revision) in slots {
            transport.note_recorded_frame(last_frame);
            let Some(plan) = transport.step(1.0) else {
                // Holding the last picture is the correct answer for pause, jog
                // and a repeated frame. Only clear when there is nothing at all.
                if transport.clip().is_none() && !transport.is_live() {
                    paint_black(source_id);
                }
                continue;
            };
            let Some(store) = store.as_ref() else {
                continue;
            };
            let started = Instant::now();
            let decoded = render_plan(store, &plan, width, height);
            let elapsed = started.elapsed().as_micros() as u64;
            if let Some((pixels, frame_width, frame_height, stride, pts, substituted)) = decoded {
                paint(source_id, &pixels, frame_width, frame_height, stride, pts);
                transport.substituted = transport.substituted.saturating_add(u64::from(substituted));
            } else {
                // A missing or corrupt frame must not pin the channel: keep the
                // previous picture, advance, and surface the miss.
                transport.substituted = transport.substituted.saturating_add(1);
            }
            transport.decode_samples = transport.decode_samples.saturating_add(1);
            transport.decode_us_sum = transport.decode_us_sum.saturating_add(elapsed);
            // Sound follows picture: one frame of audio per frame of video.
            push_channel_audio(source_id, &transport, store, &audio, fps);
            let mut inner = engine.lock();
            // A configure that landed while this frame was in flight wins; the
            // stale snapshot must not overwrite its playlist.
            if inner.playlist_revision == revision {
                if let Some(slot) = inner.channels.iter_mut().find(|slot| slot.index == index) {
                    slot.transport = transport;
                }
            }
        }
        thread::sleep(period);
    }
}

/// How many frames may wait per angle before the render thread drops one.
///
/// The operator's queue setting is the number the encoder keeps, so the same
/// number waits in front of it; a short encoder hiccup must not lose picture. A
/// frame is a full packed copy, so a memory ceiling still applies: past it the
/// queue is cut regardless of the setting.
fn inbox_capacity(queue_frames: u32, width: u32, height: u32) -> usize {
    const CEILING_BYTES: usize = 64 * 1024 * 1024;
    let frame_bytes = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(2)
        .max(1);
    let by_memory = (CEILING_BYTES / frame_bytes).max(2);
    (queue_frames.max(2) as usize).min(by_memory).min(64)
}

/// Clip gain in dB to a linear multiplier. A boost is capped and a mistake is
/// clamped, so a mistyped value cannot wreck the mix.
fn gain_from_db(db: f32) -> f32 {
    if !db.is_finite() {
        return 1.0;
    }
    10f32.powf(db.clamp(-60.0, 12.0) / 20.0)
}

/// Parses the export request the host sends as JSON.
pub fn parse_export_spec(json: &str) -> Result<eiviz_pro_api::ReplayExportSpec, i32> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|_| ERR_INVALID_ARGUMENT)?;
    let path = value
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    if path.trim().is_empty() {
        return Err(ERR_INVALID_ARGUMENT);
    }
    let number = |key: &str| -> Option<u64> { value.get(key).and_then(serde_json::Value::as_u64) };
    let mut segments = Vec::new();
    if let Some(items) = value.get("segments").and_then(serde_json::Value::as_array) {
        for item in items {
            let angle_id = item
                .get("angleId")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as u32;
            let from_frame = item
                .get("fromFrame")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let to_frame = item
                .get("toFrame")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            if angle_id == 0 || to_frame < from_frame {
                return Err(ERR_INVALID_ARGUMENT);
            }
            segments.push(eiviz_pro_api::ReplayExportSegment {
                angle_id,
                from_frame,
                to_frame,
            });
        }
    }
    if segments.is_empty() {
        return Err(ERR_INVALID_ARGUMENT);
    }
    Ok(eiviz_pro_api::ReplayExportSpec {
        path,
        width: number("width").unwrap_or(1920) as u32,
        height: number("height").unwrap_or(1080) as u32,
        fps_num: number("fpsNum").unwrap_or(60) as u32,
        fps_den: number("fpsDen").unwrap_or(1) as u32,
        bitrate: number("bitrate").unwrap_or(20_000_000) as u32,
        sample_rate: number("sampleRate").unwrap_or(48_000) as u32,
        channels: number("channels").unwrap_or(2) as u32,
        with_audio: value
            .get("withAudio")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        segments,
    })
}

/// Progress as JSON, so the host can show it without its own struct.
pub fn export_progress_json(progress: &eiviz_pro_api::ReplayExportProgress) -> String {
    serde_json::json!({
        "state": progress.state,
        "done": progress.done,
        "total": progress.total,
        "skipped": progress.skipped,
        "bytes": progress.bytes,
        "message": progress.message,
    })
    .to_string()
}

/// Hands one frame of replay audio to the mixer as if it came from a live input.
fn push_audio(id: u64, frames: u32, pts: i64, planar: &[f32]) {
    let _ = crate::with_uploads(|uploads| {
        uploads.push_audio(
            id,
            AUDIO_SAMPLE_RATE as i32,
            AUDIO_CHANNELS as i32,
            frames,
            pts,
            planar,
        );
    });
}

/// Produces the audio for whatever the channel is showing. A paused or stopped
/// channel publishes silence, so the graph never repeats the last tail.
fn push_channel_audio(
    source_id: u64,
    transport: &channel::Transport,
    store: &Arc<Mutex<Box<dyn ReplayStore>>>,
    audio: &Arc<Mutex<audio::PlaybackAudio>>,
    fps: (u32, u32),
) {
    let frames = (f64::from(AUDIO_SAMPLE_RATE) * f64::from(fps.1.max(1))
        / f64::from(fps.0.max(1)))
    .round() as usize;
    if frames == 0 {
        return;
    }
    let planar = channel_audio_block(transport, store, audio, frames);
    let seconds = transport.source_position() * f64::from(fps.1.max(1)) / f64::from(fps.0.max(1));
    push_audio(
        source_id,
        frames as u32,
        (seconds * 10_000_000.0) as i64,
        &planar,
    );
}

/// The audio block for the frame the channel is showing, in the planar layout the
/// mixer's inputs use. A paused channel, a clip with no audio of its own, and a
/// clip that has run out all produce silence rather than a repeated tail.
fn channel_audio_block(
    transport: &channel::Transport,
    store: &Arc<Mutex<Box<dyn ReplayStore>>>,
    audio: &Arc<Mutex<audio::PlaybackAudio>>,
    frames: usize,
) -> Vec<f32> {
    let mut planar = vec![0.0f32; frames * AUDIO_CHANNELS];
    if !transport.is_playing() {
        return planar;
    }
    let Some(clip) = transport.clip() else {
        return planar;
    };
    // A clip may take its sound from another angle, or from nowhere.
    let Some(angle_id) = clip.audio_angle() else {
        return planar;
    };
    let speed = transport.playback_speed();
    let start = transport.source_position();
    let mut reader = |from: u64, want: usize, out: &mut [f32]| -> usize {
        store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .read_audio(angle_id, from, want, out)
            .unwrap_or(0)
    };
    let mut cursor = audio.lock().unwrap_or_else(|error| error.into_inner());
    // The transport owns the position, so audio cannot drift from picture.
    if transport.is_holding() {
        // An inserted transition holds the picture past the clip's end. Re-anchoring
        // the sound to that held frame would repeat one block for the whole
        // transition, so it runs on instead and becomes silence at the clip's end.
        cursor.fill_planar(frames, speed, &mut reader, &mut planar);
    } else if transport.preserves_pitch() {
        cursor.fill_planar_pitched(frames, speed, start, &mut reader, &mut planar);
    } else {
        cursor.reset(start);
        cursor.fill_planar(frames, speed, &mut reader, &mut planar);
    }
    drop(cursor);
    // Clip gain, so a quiet camera can be lifted or a hot one tamed without
    // touching the mixing unit.
    if (clip.gain - 1.0).abs() > 1e-6 {
        for sample in planar.iter_mut() {
            *sample *= clip.gain;
        }
    }
    planar
}

/// Decodes the frames a plan asks for and blends them when a transition is in
/// progress. Returns the UYVY pixels plus the layout to paint.
fn render_plan(
    store: &Mutex<Box<dyn ReplayStore>>,
    plan: &channel::FramePlan,
    width: u32,
    height: u32,
) -> Option<(Vec<u8>, u32, u32, usize, i64, bool)> {
    let (angle, frame) = plan.primary?;
    if width == 0 || height == 0 {
        return None;
    }
    let bytes = (width as usize).saturating_mul(height as usize).saturating_mul(2);
    let mut primary = vec![0u8; bytes];
    let mut secondary = vec![0u8; bytes];
    let mut guard = store.lock().unwrap_or_else(|error| error.into_inner());
    let first = guard.decode(angle, frame, &mut primary).ok()?;
    let stride = first.stride as usize;
    let len = stride
        .saturating_mul(first.height as usize)
        .min(primary.len());
    primary.truncate(len);
    let mut substituted = first.substituted;
    let mut pts = first.pts_100ns;
    if let Some((angle, frame)) = plan.secondary {
        if plan.mix > 0.0 && plan.mix < 1.0 {
            if let Ok(second) = guard.decode(angle, frame, &mut secondary) {
                substituted |= second.substituted;
                let second_len = (second.stride as usize)
                    .saturating_mul(second.height as usize)
                    .min(secondary.len());
                let common = len.min(second_len);
                combine(
                    plan.kind,
                    plan.direction,
                    plan.mix,
                    first.width as usize,
                    first.height as usize,
                    stride,
                    &mut primary[..common],
                    &secondary[..common],
                );
                // A combined frame is a picture of its own, so it carries its own
                // timestamp. Without this the compositor would see the same
                // timestamp for every frame of an inserted transition and never
                // upload the dissolve: the picture would hold and then cut.
                pts = blend_pts(first.pts_100ns, second.pts_100ns, plan.mix);
            }
        }
    }
    drop(guard);
    Some((
        primary,
        first.width,
        first.height,
        stride,
        pts,
        substituted,
    ))
}

/// Timestamp of a frame blended from two recordings: between the outgoing and the
/// incoming frame, weighted the same way the picture is. Always ahead of the
/// outgoing frame, so a transition that holds both edges still produces a new
/// picture on every output frame.
fn blend_pts(outgoing: i64, incoming: i64, mix: f64) -> i64 {
    let weight = mix.clamp(0.0, 1.0);
    let delta = incoming.saturating_sub(outgoing) as f64 * weight;
    let step = delta.round() as i64;
    outgoing.saturating_add(step.max(1))
}

/// Combines two decoded UYVY frames into one picture. Two bytes per pixel means luma
/// and chroma are handled by the same walk, and a hard edge is a span copy.
///
/// Horizontal offsets are rounded to whole pixel pairs: UYVY shares one chroma pair
/// between two pixels, so an odd offset would take the colour of a neighbouring pixel
/// with it. A transition that moves sideways therefore moves in steps of two pixels,
/// which is below what an eye reads as motion.
#[allow(clippy::too_many_arguments)]
fn combine(
    kind: channel::TransitionKind,
    direction: channel::TransitionDirection,
    mix: f64,
    width: usize,
    height: usize,
    stride: usize,
    current: &mut [u8],
    next: &[u8],
) {
    use channel::TransitionKind;
    let mix = mix.clamp(0.0, 1.0);
    match kind {
        TransitionKind::Cut => {
            if mix >= 1.0 {
                current.copy_from_slice(next);
            }
        }
        TransitionKind::Dissolve => blend_uyvy(current, next, mix),
        TransitionKind::Dip => {
            // Out through black over the first half, in from black over the second: the
            // chroma walks to neutral as the luma falls, so the dip is black and not a
            // dark version of the picture.
            if mix < 0.5 {
                dip_to_black(current, mix * 2.0);
            } else {
                dip_from_black(current, next, (mix - 0.5) * 2.0);
            }
        }
        TransitionKind::Wipe => wipe_uyvy(current, next, mix, direction, width, height, stride),
        TransitionKind::Slide => {
            slide_uyvy(current, next, mix, direction, width, height, stride, false)
        }
        TransitionKind::Push => {
            slide_uyvy(current, next, mix, direction, width, height, stride, true)
        }
    }
}

/// Cross-dissolve between two UYVY frames. `mix` is the weight of the incoming frame.
fn blend_uyvy(current: &mut [u8], next: &[u8], mix: f64) {
    let weight = (mix.clamp(0.0, 1.0) * 256.0).round() as u32;
    let keep = 256 - weight;
    for (dst, src) in current.iter_mut().zip(next.iter()) {
        *dst = ((u32::from(*dst) * keep + u32::from(*src) * weight) >> 8) as u8;
    }
}

/// Fades a frame to black. `mix` 0 leaves it alone, 1 makes it black.
fn dip_to_black(current: &mut [u8], mix: f64) {
    let weight = (mix.clamp(0.0, 1.0) * 256.0).round() as u32;
    let keep = 256 - weight;
    for pair in current.chunks_exact_mut(4) {
        pair[0] = ((u32::from(pair[0]) * keep + 128 * weight) >> 8) as u8;
        pair[1] = ((u32::from(pair[1]) * keep) >> 8) as u8;
        pair[2] = ((u32::from(pair[2]) * keep + 128 * weight) >> 8) as u8;
        pair[3] = ((u32::from(pair[3]) * keep) >> 8) as u8;
    }
}

/// Brings a frame up from black. `mix` 0 leaves it black, 1 shows it whole.
fn dip_from_black(current: &mut [u8], next: &[u8], mix: f64) {
    let weight = (mix.clamp(0.0, 1.0) * 256.0).round() as u32;
    let keep = 256 - weight;
    for (dst, src) in current.chunks_exact_mut(4).zip(next.chunks_exact(4)) {
        dst[0] = ((u32::from(src[0]) * weight + 128 * keep) >> 8) as u8;
        dst[1] = ((u32::from(src[1]) * weight) >> 8) as u8;
        dst[2] = ((u32::from(src[2]) * weight + 128 * keep) >> 8) as u8;
        dst[3] = ((u32::from(src[3]) * weight) >> 8) as u8;
    }
}

/// A hard edge sweeping across the picture. `mix` is how much of the incoming frame
/// has arrived.
#[allow(clippy::too_many_arguments)]
fn wipe_uyvy(
    current: &mut [u8],
    next: &[u8],
    mix: f64,
    direction: channel::TransitionDirection,
    width: usize,
    height: usize,
    stride: usize,
) {
    use channel::TransitionDirection;
    if width == 0 || height == 0 || stride < width * 2 {
        return;
    }
    let reached = ((mix * 256.0).round() as usize).min(256);
    match direction {
        TransitionDirection::Left | TransitionDirection::Right => {
            // Columns, in pairs: the edge lands between chroma pairs.
            let edge = ((width / 2) * reached / 256) * 2;
            let (from, to) = match direction {
                TransitionDirection::Left => (width - edge, width),
                _ => (0, edge),
            };
            for row in 0..height {
                let base = row * stride;
                if base + width * 2 > current.len() || base + width * 2 > next.len() {
                    break;
                }
                let start = base + from * 2;
                let end = base + to * 2;
                current[start..end].copy_from_slice(&next[start..end]);
            }
        }
        TransitionDirection::Up | TransitionDirection::Down => {
            let edge = height * reached / 256;
            let (from, to) = match direction {
                TransitionDirection::Up => (height - edge, height),
                _ => (0, edge),
            };
            for row in from..to {
                let base = row * stride;
                let end = base + width * 2;
                if end > current.len() || end > next.len() {
                    break;
                }
                current[base..end].copy_from_slice(&next[base..end]);
            }
        }
    }
}

/// The incoming clip moving in. With `push` the outgoing picture moves out by the same
/// amount, so the two travel together; without it the outgoing stays put and is covered.
#[allow(clippy::too_many_arguments)]
fn slide_uyvy(
    current: &mut [u8],
    next: &[u8],
    mix: f64,
    direction: channel::TransitionDirection,
    width: usize,
    height: usize,
    stride: usize,
    push: bool,
) {
    use channel::TransitionDirection;
    if width == 0 || height == 0 || stride < width * 2 {
        return;
    }
    let row_bytes = width * 2;
    let reached = ((mix * 256.0).round() as usize).min(256);
    // A push moves the outgoing picture as well, and the two overlap, so the outgoing
    // is read from a copy: an in-place walk would read pixels it has already written.
    let scratch = if push { Some(current.to_vec()) } else { None };
    match direction {
        TransitionDirection::Left | TransitionDirection::Right => {
            // Whole pixel pairs: UYVY shares one chroma pair between two pixels.
            let shift = ((width / 2) * reached / 256) * 2;
            for row in 0..height {
                let base = row * stride;
                let end = base + row_bytes;
                if end > current.len() || end > next.len() {
                    break;
                }
                let source = scratch.as_deref().unwrap_or(&*current);
                let mut out = vec![0u8; row_bytes];
                match direction {
                    TransitionDirection::Left => {
                        // The incoming enters from the right edge, showing its left part.
                        out[..row_bytes - shift].copy_from_slice(&next[shift..row_bytes]);
                        if push {
                            out[row_bytes - shift..].copy_from_slice(&source[base..base + shift]);
                        } else {
                            out[row_bytes - shift..].copy_from_slice(&next[row_bytes - shift..]);
                        }
                    }
                    _ => {
                        out[shift..].copy_from_slice(&next[..row_bytes - shift]);
                        if push {
                            out[..shift].copy_from_slice(&source[base + row_bytes - shift..end]);
                        } else {
                            out[..shift].copy_from_slice(&next[..shift]);
                        }
                    }
                }
                current[base..end].copy_from_slice(&out);
            }
        }
        TransitionDirection::Up | TransitionDirection::Down => {
            let shift = height * reached / 256;
            for row in 0..height {
                let dst = row * stride;
                let end = dst + row_bytes;
                if end > current.len() || end > next.len() {
                    break;
                }
                let source = scratch.as_deref().unwrap_or(&*current);
                let taken = match direction {
                    // The incoming enters from below, showing its top part.
                    TransitionDirection::Up if row + shift < height => (next, (row + shift) * stride),
                    TransitionDirection::Up if push => (source, (row - (height - shift)) * stride),
                    // The incoming enters from above, showing its bottom part.
                    TransitionDirection::Down if row >= shift => (next, (row - shift) * stride),
                    TransitionDirection::Down if push => (source, (row + height - shift) * stride),
                    _ => continue,
                };
                if taken.1 + row_bytes > taken.0.len() {
                    break;
                }
                let pixels = taken.0[taken.1..taken.1 + row_bytes].to_vec();
                current[dst..end].copy_from_slice(&pixels);
            }
        }
    }
}

/// Stable handle for the clip a channel is playing.
fn clip_uid_hash(uid: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in uid.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Clips of the playlist a channel selects, in play order.
fn playlist_clips(config: &ReplayDto, playlist_id: Option<u64>) -> Vec<channel::Clip> {
    use eiviz_control::session::replay::{
        ReplayTransitionKind, ReplayTransitionTiming,
    };
    let Some(list) = config
        .playlists
        .iter()
        .find(|list| Some(list.id) == playlist_id)
    else {
        return Vec::new();
    };
    list.clips
        .iter()
        .map(|clip| {
            let transition = clip.transition;
            channel::Clip {
                uid: clip.uid.clone(),
                recording_uuid: clip.recording_uuid.clone(),
                angle_id: clip.angle_id,
                in_frame: clip.in_frame,
                out_frame: clip.out_frame,
                speed: f64::from(clip.speed),
                preserve_pitch: clip.preserve_pitch,
                gain: gain_from_db(clip.gain_db),
                audio: match clip.audio {
                    eiviz_control::session::replay::ClipAudio::Silent => {
                        channel::ClipAudioSource::Silent
                    }
                    eiviz_control::session::replay::ClipAudio::Angle => {
                        channel::ClipAudioSource::Angle(clip.audio_angle_id)
                    }
                    eiviz_control::session::replay::ClipAudio::FollowAngle => {
                        channel::ClipAudioSource::Follow
                    }
                },
                transition: channel::ClipTransition {
                    kind: match transition.kind {
                        ReplayTransitionKind::Cut => channel::TransitionKind::Cut,
                        ReplayTransitionKind::Dissolve => channel::TransitionKind::Dissolve,
                        ReplayTransitionKind::Dip => channel::TransitionKind::Dip,
                        ReplayTransitionKind::Wipe => channel::TransitionKind::Wipe,
                        ReplayTransitionKind::Slide => channel::TransitionKind::Slide,
                        ReplayTransitionKind::Push => channel::TransitionKind::Push,
                    },
                    direction: match transition.direction {
                        eiviz_control::session::replay::ReplayTransitionDirection::Left => {
                            channel::TransitionDirection::Left
                        }
                        eiviz_control::session::replay::ReplayTransitionDirection::Right => {
                            channel::TransitionDirection::Right
                        }
                        eiviz_control::session::replay::ReplayTransitionDirection::Up => {
                            channel::TransitionDirection::Up
                        }
                        eiviz_control::session::replay::ReplayTransitionDirection::Down => {
                            channel::TransitionDirection::Down
                        }
                    },
                    timing: match transition.timing {
                        ReplayTransitionTiming::Additive => channel::TransitionTiming::Additive,
                        ReplayTransitionTiming::Overwrap => channel::TransitionTiming::Overwrap,
                    },
                    frames: transition.duration_frames,
                },
            }
        })
        .collect()
}

fn paint(id: u64, pixels: &[u8], width: u32, height: u32, stride: usize, pts: i64) {
    if width == 0 || height == 0 || stride < width as usize * 2 {
        return;
    }
    let _ = crate::with_uploads(|uploads| {
        if let Some((mut buf, _format, _w, _h)) =
            uploads.take_playout_buf(id, width, height, CpuFormat::Uyvy, 2)
        {
            if buf.len() < pixels.len() {
                buf.resize(pixels.len(), 0);
            }
            let n = pixels.len().min(buf.len());
            buf[..n].copy_from_slice(&pixels[..n]);
            uploads.finish_playout_cpu(id, buf, pts);
        }
    });
}

/// Studio-range black in UYVY. Two luma samples share one chroma pair, and an
/// all-zero frame is a dark green rather than black, so the bytes matter.
fn fill_black_uyvy(buf: &mut [u8]) {
    const BLACK: [u8; 4] = [128, 16, 128, 16];
    for group in buf.chunks_exact_mut(4) {
        group.copy_from_slice(&BLACK);
    }
}

/// Paints the channel black while it holds nothing. A solid 16x16 frame is enough
/// for a placeholder and costs nothing to upload every frame.
fn paint_black(id: u64) {
    let _ = crate::with_uploads(|uploads| {
        if let Some((mut buf, _format, _width, _height)) =
            uploads.take_playout_buf(id, 16, 16, CpuFormat::Uyvy, 2)
        {
            fill_black_uyvy(&mut buf);
            uploads.finish_playout_cpu(id, buf, 0);
        }
    });
}

fn health_message(issues: u32, severity: u32) -> String {
    format!("replay health issues={issues:#x} severity={severity}")
}

fn uuid_stamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Control-layer error for a Replay refusal. The message is the reason the mixer
/// gave; a client that reads it can tell "no storage" from "no Pro module".
pub fn control_error_with(code: i32, message: &str) -> ControlError {
    if code == ERR_NOT_SUPPORTED_PLAN {
        ControlError::not_supported_plan(message)
    } else if code == ERR_IO {
        ControlError::io(message)
    } else {
        ControlError::invalid(message)
    }
}

pub fn control_error(code: i32) -> ControlError {
    if code == ERR_NOT_SUPPORTED_PLAN {
        ControlError::not_supported_plan("replay is not included in this plan")
    } else if code == ERR_IO {
        ControlError::io("replay storage or pro module is unavailable")
    } else {
        ControlError::invalid("replay command was rejected")
    }
}

/// Adapter used when the loaded module speaks the C ABI. The in-process
/// community module does not implement replay.
pub struct FfiReplayStore {
    api: eiviz_pro_api::ProApi,
    handle: *mut std::ffi::c_void,
}

unsafe impl Send for FfiReplayStore {}

impl Drop for FfiReplayStore {
    fn drop(&mut self) {
        if self.handle.is_null() {
            return;
        }
        if let Some(close) = self.api.replay_close {
            unsafe { close(self.handle) };
        }
        self.handle = std::ptr::null_mut();
    }
}

impl ReplayStore for FfiReplayStore {
    fn write_video(
        &mut self,
        angle_id: u32,
        frame_index: u64,
        frame: &VideoFrame,
    ) -> eiviz_pro_api::ProResult<WriteOutcome> {
        let Some(write) = self.api.replay_write_video else {
            return Err(eiviz_pro_api::ProError::unavailable("replay write is missing"));
        };
        let view = ProVideoView {
            width: frame.width,
            height: frame.height,
            stride: frame.stride,
            layout: match frame.layout {
                PixelLayout::Bgra => LAYOUT_BGRA,
                PixelLayout::Uyvy => eiviz_pro_api::LAYOUT_UYVY,
            },
            pts_100ns: frame.pts_100ns,
            fps_num: frame.fps_num,
            fps_den: frame.fps_den,
            data: frame.data.as_ref().as_ptr(),
            data_len: frame.data.len(),
        };
        let status = unsafe { write(self.handle, angle_id, frame_index, &view) };
        if status.code == 0 {
            Ok(WriteOutcome::Accepted)
        } else if status.code == PRO_BUSY || status.code == eiviz_pro_api::PRO_INVALID {
            // An angle the module never opened cannot be stored, but that must not
            // end the recording.
            Ok(WriteOutcome::Dropped)
        } else if status.code == PRO_IO {
            Err(eiviz_pro_api::ProError::io("replay write rejected"))
        } else {
            Err(eiviz_pro_api::ProError::io("replay write failed"))
        }
    }

    fn decode(
        &mut self,
        angle_id: u32,
        frame_index: u64,
        dst: &mut [u8],
    ) -> eiviz_pro_api::ProResult<eiviz_pro_api::DecodedFrame> {
        let Some(decode) = self.api.replay_decode else {
            return Err(eiviz_pro_api::ProError::unavailable("replay decode is missing"));
        };
        let mut out = ProReplayDecoded {
            struct_size: std::mem::size_of::<ProReplayDecoded>() as u32,
            ..ProReplayDecoded::default()
        };
        let status = unsafe {
            decode(
                self.handle,
                angle_id,
                frame_index,
                dst.as_mut_ptr(),
                dst.len(),
                &mut out,
            )
        };
        if status.code != 0 {
            return Err(eiviz_pro_api::ProError::unavailable("replay frame is not recorded"));
        }
        Ok(eiviz_pro_api::DecodedFrame {
            actual_frame_index: out.actual_frame_index,
            width: out.width,
            height: out.height,
            stride: out.stride,
            pts_100ns: out.pts_100ns,
            substituted: out.substituted != 0,
        })
    }

    fn range(&self, angle_id: u32) -> eiviz_pro_api::ProResult<Option<(u64, u64)>> {
        let Some(range) = self.api.replay_range else {
            return Ok(None);
        };
        let mut first = 0u64;
        let mut last = 0u64;
        let status = unsafe { range(self.handle, angle_id, &mut first, &mut last) };
        if status.code == 0 {
            Ok(Some((first, last)))
        } else {
            Ok(None)
        }
    }

    fn write_audio(
        &mut self,
        angle_id: u32,
        frame_index: u64,
        sample_rate: u32,
        channels: u32,
        pcm: &[f32],
    ) -> eiviz_pro_api::ProResult<WriteOutcome> {
        let Some(write) = self.api.replay_write_audio else {
            return Err(eiviz_pro_api::ProError::unavailable(
                "replay audio is missing",
            ));
        };
        let channels = channels.max(1);
        let view = eiviz_pro_api::ProReplayAudioView {
            struct_size: std::mem::size_of::<eiviz_pro_api::ProReplayAudioView>() as u32,
            sample_rate,
            channels,
            frames: (pcm.len() / channels as usize) as u32,
            data: pcm.as_ptr(),
            data_len: std::mem::size_of_val(pcm),
        };
        let status = unsafe { write(self.handle, angle_id, frame_index, &view) };
        if status.code == 0 {
            Ok(WriteOutcome::Accepted)
        } else if status.code == PRO_BUSY || status.code == eiviz_pro_api::PRO_INVALID {
            Ok(WriteOutcome::Dropped)
        } else {
            Err(eiviz_pro_api::ProError::io("replay audio write failed"))
        }
    }

    /// Stores a frame the sender already encoded. A module that cannot keep a
    /// bitstream says so, and the recorder reports it rather than quietly
    /// re-encoding the picture.
    fn write_encoded(
        &mut self,
        angle_id: u32,
        frame_index: u64,
        frame: &EncodedFrame,
    ) -> eiviz_pro_api::ProResult<WriteOutcome> {
        let Some(write) = self.api.replay_write_encoded else {
            return Err(eiviz_pro_api::ProError::unavailable(
                "this Pro module cannot record an already-encoded frame",
            ));
        };
        let view = eiviz_pro_api::ProReplayEncodedView {
            struct_size: std::mem::size_of::<eiviz_pro_api::ProReplayEncodedView>() as u32,
            width: frame.width,
            height: frame.height,
            pts_100ns: frame.pts_100ns,
            fps_num: frame.fps_num,
            fps_den: frame.fps_den,
            dropped_before: frame.dropped_before,
            data: frame.payload.as_ptr(),
            data_len: frame.payload.len(),
        };
        let status = unsafe { write(self.handle, angle_id, frame_index, &view) };
        if status.code == 0 {
            Ok(WriteOutcome::Accepted)
        } else if status.code == PRO_BUSY || status.code == eiviz_pro_api::PRO_INVALID {
            Ok(WriteOutcome::Dropped)
        } else {
            Err(eiviz_pro_api::ProError::io(
                "recording an already-encoded frame failed",
            ))
        }
    }

    fn read_audio(
        &mut self,
        angle_id: u32,
        from_frame: u64,
        frames: usize,
        out: &mut [f32],
    ) -> eiviz_pro_api::ProResult<usize> {
        let Some(read) = self.api.replay_read_audio else {
            return Err(eiviz_pro_api::ProError::unavailable(
                "replay audio is missing",
            ));
        };
        let mut written = 0u32;
        let status = unsafe {
            read(
                self.handle,
                angle_id,
                from_frame,
                frames,
                out.as_mut_ptr(),
                &mut written,
            )
        };
        if status.code == 0 {
            Ok(written as usize)
        } else {
            Err(eiviz_pro_api::ProError::io("replay audio read failed"))
        }
    }

    fn peak_count(&mut self, angle_id: u32) -> eiviz_pro_api::ProResult<(u32, u32)> {
        let Some(copy) = self.api.replay_copy_peaks else {
            return Ok((0, 0));
        };
        let mut copied = 0u32;
        let mut dropped = 0u32;
        // A zero capacity asks the module for the counts only.
        let status = unsafe {
            copy(
                self.handle,
                angle_id,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                &mut copied,
                &mut dropped,
            )
        };
        if status.code == 0 {
            Ok((copied, dropped))
        } else {
            Err(eiviz_pro_api::ProError::io("replay peaks are unavailable"))
        }
    }

    fn copy_peaks(
        &mut self,
        angle_id: u32,
        from: u32,
        min_out: &mut [f32],
        max_out: &mut [f32],
    ) -> eiviz_pro_api::ProResult<u32> {
        let Some(copy) = self.api.replay_copy_peaks else {
            return Ok(0);
        };
        let cap = min_out.len().min(max_out.len()) as u32;
        let mut copied = 0u32;
        let mut dropped = 0u32;
        let status = unsafe {
            copy(
                self.handle,
                angle_id,
                from,
                min_out.as_mut_ptr(),
                max_out.as_mut_ptr(),
                cap,
                &mut copied,
                &mut dropped,
            )
        };
        if status.code == 0 {
            Ok(copied)
        } else {
            Err(eiviz_pro_api::ProError::io("replay peaks are unavailable"))
        }
    }

    fn export_start(&mut self, spec: eiviz_pro_api::ReplayExportSpec) -> eiviz_pro_api::ProResult<()> {
        let Some(start) = self.api.replay_export_start else {
            return Err(eiviz_pro_api::ProError::unavailable(
                "replay export is missing",
            ));
        };
        let path = spec.path.as_bytes();
        let segments: Vec<eiviz_pro_api::ProReplayExportSegment> = spec
            .segments
            .iter()
            .map(|segment| eiviz_pro_api::ProReplayExportSegment {
                angle_id: segment.angle_id,
                from_frame: segment.from_frame,
                to_frame: segment.to_frame,
            })
            .collect();
        let request = eiviz_pro_api::ProReplayExportRequest {
            struct_size: std::mem::size_of::<eiviz_pro_api::ProReplayExportRequest>() as u32,
            path: path.as_ptr(),
            path_len: path.len(),
            width: spec.width,
            height: spec.height,
            fps_num: spec.fps_num,
            fps_den: spec.fps_den,
            bitrate: spec.bitrate,
            sample_rate: spec.sample_rate,
            channels: spec.channels,
            with_audio: u32::from(spec.with_audio),
            segments: segments.as_ptr(),
            segment_count: segments.len() as u32,
        };
        let status = unsafe { start(self.handle, &request) };
        if status.code == 0 {
            Ok(())
        } else {
            Err(eiviz_pro_api::ProError::io("replay export did not start"))
        }
    }

    fn export_progress(&mut self) -> eiviz_pro_api::ProResult<eiviz_pro_api::ReplayExportProgress> {
        let Some(progress) = self.api.replay_export_progress else {
            return Ok(eiviz_pro_api::ReplayExportProgress::default());
        };
        let mut out = eiviz_pro_api::ProReplayExportProgress::default();
        let mut message = vec![0u8; 512];
        let mut length = 0usize;
        let status = unsafe {
            progress(
                self.handle,
                &mut out,
                message.as_mut_ptr(),
                message.len(),
                &mut length,
            )
        };
        if status.code != 0 {
            return Err(eiviz_pro_api::ProError::io("replay export progress failed"));
        }
        message.truncate(length.min(message.len()));
        Ok(eiviz_pro_api::ReplayExportProgress {
            state: out.state,
            done: out.done,
            total: out.total,
            skipped: out.skipped,
            bytes: out.bytes,
            message: String::from_utf8_lossy(&message).into_owned(),
        })
    }

    fn export_cancel(&mut self) -> eiviz_pro_api::ProResult<()> {
        let Some(cancel) = self.api.replay_export_cancel else {
            return Err(eiviz_pro_api::ProError::unavailable(
                "replay export is missing",
            ));
        };
        let status = unsafe { cancel(self.handle) };
        if status.code == 0 {
            Ok(())
        } else {
            Err(eiviz_pro_api::ProError::io("replay export cancel failed"))
        }
    }

    fn stats(&self) -> ReplayStats {
        let Some(stats_fn) = self.api.replay_stats else {
            return ReplayStats::default();
        };
        let mut stats = ProReplayStatsAbi {
            struct_size: std::mem::size_of::<ProReplayStatsAbi>() as u32,
            ..ProReplayStatsAbi::default()
        };
        let mut angles = [eiviz_pro_api::ProReplayAngleStatsAbi::default(); 8];
        let mut written = 0u32;
        let status = unsafe {
            stats_fn(
                self.handle,
                &mut stats,
                angles.as_mut_ptr(),
                angles.len() as u32,
                &mut written,
            )
        };
        if status.code != 0 {
            return ReplayStats::default();
        }
        ReplayStats {
            state: stats.state,
            stop_reason: stats.stop_reason,
            issues: stats.issues,
            first_frame: stats.first_frame,
            last_frame: stats.last_frame,
            free_bytes: stats.free_bytes,
            reserve_bytes: stats.reserve_bytes,
            write_rate_bps: stats.write_rate_bps,
            write_rate_instant_bps: stats.write_rate_instant_bps,
            frames_written: stats.frames_written,
            frames_dropped: stats.frames_dropped,
            frames_dropped_recent: stats.frames_dropped_recent,
            encoded_frames: stats.encoded_frames,
            angles: angles
                .iter()
                .take(written as usize)
                .map(|angle| ReplayAngleStats {
                    angle_id: angle.angle_id,
                    encode_ms_avg_milli: angle.encode_ms_avg_milli,
                    encode_ms_max_milli: angle.encode_ms_max_milli,
                    encode_queue_len: angle.encode_queue_len,
                    encode_queue_cap: angle.encode_queue_cap,
                    write_buffer_bytes: angle.write_buffer_bytes,
                    write_buffer_cap: angle.write_buffer_cap,
                    write_latency_p95_ms: angle.write_latency_p95_ms,
                    frames_written: angle.frames_written,
                    dropped_encode: angle.dropped_encode,
                    dropped_write: angle.dropped_write,
                    bytes_written: angle.bytes_written,
                    segments: angle.segments,
                })
                .collect(),
        }
    }

    fn close(&mut self) -> eiviz_pro_api::ProResult<()> {
        // Stopping recording must flush and join the encoder and writer threads
        // inside the module. Leaving it to Drop means the segment index is never
        // written and stop appears to hang until the next recording.
        if self.handle.is_null() {
            return Ok(());
        }
        let Some(close) = self.api.replay_close else {
            return Ok(());
        };
        let handle = std::mem::replace(&mut self.handle, std::ptr::null_mut());
        let status = unsafe { close(handle) };
        if status.code == 0 {
            Ok(())
        } else {
            Err(eiviz_pro_api::ProError::io("replay close failed"))
        }
    }
}

pub fn open_ffi_store(config: &ReplayStoreConfig) -> eiviz_pro_api::ProResult<Box<dyn ReplayStore>> {
    let loaded = crate::pro_load::loaded_api().ok_or_else(|| {
        eiviz_pro_api::ProError::unavailable("replay module is not loaded")
    })?;
    let (api, module) = loaded;
    let Some(open) = api.replay_open else {
        return Err(eiviz_pro_api::ProError::unavailable("replay open is missing"));
    };
    let uuid = config.recording_uuid.as_bytes();
    let storages: Vec<ProReplayStorage> = config
        .storages
        .iter()
        .map(|item| ProReplayStorage {
            path: item.path.as_ptr(),
            path_len: item.path.len(),
        })
        .collect();
    let angles: Vec<ProReplayAngle> = config
        .angles
        .iter()
        .map(|item| ProReplayAngle {
            angle_id: item.angle_id,
            storage_index: item.storage_index,
        })
        .collect();
    let raw = ProReplayConfig {
        struct_size: std::mem::size_of::<ProReplayConfig>() as u32,
        recording_uuid: uuid.as_ptr(),
        recording_uuid_len: uuid.len(),
        storages: storages.as_ptr(),
        storage_count: storages.len() as u32,
        angles: angles.as_ptr(),
        angle_count: angles.len() as u32,
        width: config.width,
        height: config.height,
        fps_num: config.fps_num,
        fps_den: config.fps_den,
        profile: config.profile,
        segment_frames: config.segment_frames,
        encode_queue_frames: config.encode_queue_frames,
        write_buffer_bytes: config.write_buffer_bytes,
        overflow_policy: config.overflow_policy,
        reserve_bytes: config.reserve_bytes,
        retention: config.retention,
        protect_from_frame: config.protect_from_frame,
    };
    let mut handle = std::ptr::null_mut();
    let status = unsafe { open(module, &raw, &mut handle) };
    if status.code != 0 || handle.is_null() {
        return Err(eiviz_pro_api::ProError::io("replay open failed"));
    }
    Ok(Box::new(FfiReplayStore { api, handle }))
}

pub fn estimate_ffi(
    path: &str,
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    profile: u32,
    angles: u32,
    reserve_bytes: u64,
) -> eiviz_pro_api::ProResult<eiviz_pro_api::ReplayEstimate> {
    let (api, module) = crate::pro_load::loaded_api().ok_or_else(|| {
        eiviz_pro_api::ProError::unavailable("replay module is not loaded")
    })?;
    let Some(estimate) = api.replay_estimate else {
        return Err(eiviz_pro_api::ProError::unavailable("replay estimate is missing"));
    };
    let mut out = ProReplayEstimateAbi {
        struct_size: std::mem::size_of::<ProReplayEstimateAbi>() as u32,
        ..ProReplayEstimateAbi::default()
    };
    let status = unsafe {
        estimate(
            module,
            path.as_ptr(),
            path.len(),
            width,
            height,
            fps_num,
            fps_den,
            profile,
            angles,
            reserve_bytes,
            &mut out,
        )
    };
    if status.code != 0 {
        return Err(eiviz_pro_api::ProError::io("replay estimate failed"));
    }
    Ok(eiviz_pro_api::ReplayEstimate {
        free_bytes: out.free_bytes,
        estimate_rate_bps: out.estimate_rate_bps,
        remaining_secs: out.remaining_secs,
        path_writable: out.path_writable != 0,
    })
}
#[cfg(test)]
mod export_tests {
    use super::*;

    #[test]
    fn an_export_request_needs_a_path_and_segments() {
        assert!(parse_export_spec("{\"segments\":[]}").is_err());
        assert!(parse_export_spec("{\"path\":\"out.mp4\"}").is_err());
        // A segment that ends before it starts cannot be exported.
        assert!(
            parse_export_spec(
                "{\"path\":\"out.mp4\",\"segments\":[{\"angleId\":1,\"fromFrame\":9,\"toFrame\":2}]}"
            )
            .is_err()
        );
        // Angle 0 is not an angle.
        assert!(
            parse_export_spec(
                "{\"path\":\"out.mp4\",\"segments\":[{\"angleId\":0,\"fromFrame\":0,\"toFrame\":1}]}"
            )
            .is_err()
        );
        assert!(parse_export_spec("not json").is_err());
    }

    #[test]
    fn an_export_request_carries_its_segments_and_defaults() {
        let spec = parse_export_spec(
            "{\"path\":\"out.mp4\",\"width\":1280,\"height\":720,\"fpsNum\":50,\"fpsDen\":1,\"segments\":[{\"angleId\":2,\"fromFrame\":10,\"toFrame\":20},{\"angleId\":3,\"fromFrame\":0,\"toFrame\":5}]}",
        )
        .unwrap();
        assert_eq!(spec.path, "out.mp4");
        assert_eq!(spec.width, 1280);
        assert_eq!(spec.height, 720);
        assert_eq!(spec.fps_num, 50);
        assert_eq!(spec.segments.len(), 2);
        assert_eq!(spec.segments[0].angle_id, 2);
        assert_eq!(spec.segments[0].from_frame, 10);
        assert_eq!(spec.segments[1].to_frame, 5);
        // Audio and a sane bitrate are on unless the host says otherwise.
        assert!(spec.with_audio);
        assert_eq!(spec.sample_rate, 48_000);
    }

    #[test]
    fn export_progress_is_json_the_host_can_read() {
        let progress = eiviz_pro_api::ReplayExportProgress {
            state: eiviz_pro_api::EXPORT_DONE,
            done: 12,
            total: 12,
            skipped: 1,
            bytes: 4096,
            message: "boom".into(),
        };
        let json = export_progress_json(&progress);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["state"], eiviz_pro_api::EXPORT_DONE);
        assert_eq!(value["done"], 12);
        assert_eq!(value["skipped"], 1);
        assert_eq!(value["message"], "boom");
    }
}
#[cfg(test)]
mod queue_tests {
    use super::*;

    fn frame(index: u64) -> RecordFrame {
        RecordFrame {
            angle_id: 1,
            frame_index: index,
            pts: 0,
            stride: 4,
            data: vec![0u8; 4],
        }
    }

    #[test]
    fn an_idle_channel_is_black_not_green() {
        // Filling a UYVY frame with zeros leaves U and V at zero, which decodes to
        // a dark green. Studio black is Y=16 with neutral chroma.
        let mut buf = vec![0u8; 16];
        fill_black_uyvy(&mut buf);
        assert_eq!(
            buf,
            [128, 16, 128, 16, 128, 16, 128, 16, 128, 16, 128, 16, 128, 16, 128, 16]
        );
    }

    #[test]
    fn the_queue_setting_reaches_the_inbox() {
        // A 1080p frame is about 4 MiB, so six of them fit under the ceiling.
        assert_eq!(inbox_capacity(6, 1920, 1080), 6);
        assert_eq!(inbox_capacity(2, 1920, 1080), 2);
        // A large setting is still bounded by memory, not by hope.
        assert_eq!(inbox_capacity(60, 1920, 1080), 16);
        // 4K frames are four times the size, so fewer may wait.
        assert_eq!(inbox_capacity(60, 3840, 2160), 4);
        // Tiny frames stop at the hard limit.
        assert_eq!(inbox_capacity(1000, 16, 16), 64);
    }

    #[test]
    fn the_inbox_holds_the_configured_number_of_frames() {
        let inbox = Inbox::new();
        inbox.set_capacity(4);
        for index in 0..4 {
            assert!(inbox.offer(frame(index)), "frame {index} should fit");
        }
        assert!(!inbox.offer(frame(4)), "the fifth frame does not fit");
        // The worker drains, and room appears again.
        let stop = AtomicBool::new(false);
        assert!(inbox.pop(&stop).is_some());
        assert!(inbox.offer(frame(4)));
    }
}

#[cfg(test)]
mod animation_action_tests {
    use super::*;

    fn clip(uid: &str, in_frame: u64, out_frame: u64) -> channel::Clip {
        channel::Clip {
            uid: uid.into(),
            recording_uuid: "rec".into(),
            angle_id: 1,
            in_frame,
            out_frame,
            speed: 1.0,
            preserve_pitch: false,
            gain: 1.0,
            audio: channel::ClipAudioSource::Follow,
            transition: channel::ClipTransition::CUT,
        }
    }

    /// Cueing from an animation resolves the clip on the channel's playlist, so a
    /// session only has to name it.
    #[test]
    fn an_animation_cue_resolves_its_clip() {
        let engine = engine();
        engine.test_set_playlist(
            0,
            replay_channel_gpu_id(0),
            vec![clip("clip-a", 0, 30), clip("clip-b", 40, 70)],
        );

        // Naming the clip cues that clip and leaves it paused unless asked.
        engine.fire_animation_action(0, "clip-b", false).unwrap();
        let state = engine.animation_channel_clip(0).unwrap();
        assert_eq!(state.0, "clip-b");
        assert!(!state.1);

        // An empty id means the newest clip, which is the last one.
        engine.fire_animation_action(0, "", true).unwrap();
        let state = engine.animation_channel_clip(0).unwrap();
        assert_eq!(state.0, "clip-b");
        assert!(state.1);

        engine.fire_animation_action(0, "clip-a", true).unwrap();
        assert_eq!(engine.animation_channel_clip(0).unwrap().0, "clip-a");

        // A clip that is not on the channel, and a channel that does not exist,
        // are both errors rather than silent no-ops.
        assert!(engine.fire_animation_action(0, "clip-z", false).is_err());
        assert!(engine.fire_animation_action(9, "", false).is_err());
        engine.test_set_playlist(0, 0, Vec::new());
        // With no clips there is nothing to cue.
        assert!(engine.fire_animation_action(0, "", true).is_err());
        engine.test_set_playlist(0, 0, Vec::new());
        let mut inner = engine.lock();
        inner.channels.clear();
    }
}
#[cfg(test)]
mod channel_audio_tests {
    use super::*;
    use crate::replay::channel::{Clip, ClipAudioSource, ClipTransition, Transport};

    /// A store that hands out a constant tone and remembers which angle and
    /// position it was asked for, so the test can tell silence from sound and a
    /// moving read from a repeated one, without a real recording.
    struct ToneStore {
        level: f32,
        reads: Arc<Mutex<Vec<(u32, u64)>>>,
    }

    impl ToneStore {
        /// The store, plus the log of angles and positions it was asked for.
        fn new(level: f32) -> (Arc<Mutex<Box<dyn ReplayStore>>>, Arc<Mutex<Vec<(u32, u64)>>>) {
            let reads = Arc::new(Mutex::new(Vec::new()));
            let store: Box<dyn ReplayStore> = Box::new(Self {
                level,
                reads: Arc::clone(&reads),
            });
            (Arc::new(Mutex::new(store)), reads)
        }
    }

    impl ReplayStore for ToneStore {
        fn write_video(
            &mut self,
            _angle_id: u32,
            _frame_index: u64,
            _frame: &VideoFrame,
        ) -> Result<WriteOutcome, eiviz_pro_api::ProError> {
            Err(eiviz_pro_api::ProError::unavailable("test store"))
        }

        fn decode(
            &mut self,
            _angle_id: u32,
            _frame_index: u64,
            _dst: &mut [u8],
        ) -> Result<eiviz_pro_api::DecodedFrame, eiviz_pro_api::ProError> {
            Err(eiviz_pro_api::ProError::unavailable("test store"))
        }

        fn range(&self, _angle_id: u32) -> Result<Option<(u64, u64)>, eiviz_pro_api::ProError> {
            Ok(None)
        }

        fn stats(&self) -> ReplayStats {
            ReplayStats::default()
        }

        fn close(&mut self) -> Result<(), eiviz_pro_api::ProError> {
            Ok(())
        }

        fn read_audio(
            &mut self,
            angle_id: u32,
            from_frame: u64,
            frames: usize,
            out: &mut [f32],
        ) -> Result<usize, eiviz_pro_api::ProError> {
            self.reads
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push((angle_id, from_frame));
            let count = frames.min(out.len() / AUDIO_CHANNELS);
            for sample in out.iter_mut().take(count * AUDIO_CHANNELS) {
                *sample = self.level;
            }
            Ok(count)
        }
    }

    fn clip(audio: ClipAudioSource, gain: f32) -> Clip {
        Clip {
            uid: "clip-1".into(),
            recording_uuid: "rec".into(),
            angle_id: 1,
            in_frame: 0,
            out_frame: 600,
            speed: 1.0,
            preserve_pitch: false,
            gain,
            audio,
            transition: ClipTransition::CUT,
        }
    }

    fn transport(clip: Clip, playing: bool) -> Transport {
        let mut transport = Transport::new();
        transport.set_clip(clip.clone(), 0);
        transport.set_playlist(vec![clip]);
        transport.set_playing(playing);
        transport
    }

    fn block(transport: &Transport, store: &Arc<Mutex<Box<dyn ReplayStore>>>) -> Vec<f32> {
        let audio = Arc::new(Mutex::new(audio::PlaybackAudio::new(AUDIO_CHANNELS)));
        channel_audio_block(transport, store, &audio, 1024)
    }

    fn loud(samples: &[f32], level: f32) -> usize {
        samples
            .iter()
            .filter(|sample| (**sample - level).abs() < 0.02)
            .count()
    }

    /// A playing channel reads the recording's sound; a paused one publishes
    /// silence, so the graph never repeats the last tail.
    #[test]
    fn a_playing_channel_publishes_its_clip_audio_and_a_paused_one_silence() {
        let (store, _) = ToneStore::new(0.5);
        let playing = block(&transport(clip(ClipAudioSource::Follow, 1.0), true), &store);
        assert!(
            loud(&playing, 0.5) > playing.len() / 2,
            "the tone must fill the block: {} loud samples of {}",
            loud(&playing, 0.5),
            playing.len()
        );

        let paused = block(&transport(clip(ClipAudioSource::Follow, 1.0), false), &store);
        assert!(
            paused.iter().all(|sample| *sample == 0.0),
            "a paused channel is silent"
        );
    }

    /// A clip that takes no sound, or takes it from somewhere else, is honoured.
    #[test]
    fn a_clip_decides_which_angle_its_audio_comes_from() {
        let (store, reads) = ToneStore::new(0.5);
        let silent = block(&transport(clip(ClipAudioSource::Silent, 1.0), true), &store);
        assert!(
            silent.iter().all(|sample| *sample == 0.0),
            "a silent clip publishes silence even though the store has sound"
        );

        let other = block(&transport(clip(ClipAudioSource::Angle(3), 1.0), true), &store);
        assert!(loud(&other, 0.5) > other.len() / 2);
        let asked: Vec<u32> = reads
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .map(|(angle, _)| *angle)
            .collect();
        assert!(
            asked.contains(&3),
            "the clip's own angle must be the one read: {asked:?}"
        );
        assert!(!asked.contains(&1), "the picture angle is not read: {asked:?}");
    }

    /// An inserted (additive) transition holds the picture past the clip's end.
    /// Re-anchoring the sound to that held frame would replay one block for the
    /// whole transition, which is heard as a buzz, so the read must move on.
    #[test]
    fn a_held_transition_does_not_repeat_one_audio_block() {
        let (store, reads) = ToneStore::new(0.5);
        let audio = Arc::new(Mutex::new(audio::PlaybackAudio::new(AUDIO_CHANNELS)));
        // Normal playback first, so the cursor sits where the picture is.
        let playing = transport(clip(ClipAudioSource::Follow, 1.0), true);
        channel_audio_block(&playing, &store, &audio, 1024);

        // out_frame is 600, so the clip's length is 601 frames: seeking to 601
        // holds the last frame, which is what the insertion does.
        let mut holding = transport(clip(ClipAudioSource::Follow, 1.0), true);
        holding.seek_source(601);
        assert!(holding.is_holding(), "the picture must be held");
        channel_audio_block(&holding, &store, &audio, 1024);
        channel_audio_block(&holding, &store, &audio, 1024);

        let positions: Vec<u64> = reads
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .map(|(_, from)| *from)
            .collect();
        assert!(positions.len() >= 3, "{positions:?}");
        assert_ne!(
            positions[1], positions[2],
            "the held sound must move on instead of repeating: {positions:?}"
        );
    }

    /// Clip gain scales the block, which is what the operator's dB field means.
    #[test]
    fn clip_gain_scales_the_published_audio() {
        let (store, _) = ToneStore::new(0.5);
        let quiet = block(&transport(clip(ClipAudioSource::Follow, 0.5), true), &store);
        assert!(
            loud(&quiet, 0.25) > quiet.len() / 2,
            "half gain halves the level"
        );
    }
}
#[cfg(test)]
mod gain_tests {
    use super::*;

    #[test]
    fn clip_gain_is_linear_and_bounded() {
        assert!((gain_from_db(0.0) - 1.0).abs() < 1e-6);
        // -6 dB is about half, which is what an operator expects.
        assert!((gain_from_db(-6.0206) - 0.5).abs() < 1e-3);
        assert!((gain_from_db(6.0206) - 2.0).abs() < 1e-2);
        // Silence stays silent, and a wild value is clamped instead of trusted.
        assert_eq!(gain_from_db(-120.0), gain_from_db(-60.0));
        assert!(gain_from_db(60.0) <= gain_from_db(12.0) + 1e-6);
        assert_eq!(gain_from_db(f32::NAN), 1.0);
        assert_eq!(gain_from_db(f32::INFINITY), 1.0);
    }
}

#[cfg(test)]
mod blend_pts_tests {
    use super::*;

    /// An inserted transition holds both edges, so the frames it is made of do not
    /// change. The timestamp has to change anyway, or the compositor treats the
    /// whole insertion as one frame and the dissolve never reaches the output.
    #[test]
    fn a_blended_frame_moves_ahead_of_the_outgoing_one() {
        let outgoing = 1_000_000;
        let incoming = 2_000_000;
        let first = blend_pts(outgoing, incoming, 0.1);
        let middle = blend_pts(outgoing, incoming, 0.5);
        let last = blend_pts(outgoing, incoming, 0.9);
        assert!(first > outgoing, "{first} must move past {outgoing}");
        assert!(first < middle && middle < last);
        assert!(last < incoming);
        // The ends of the ramp keep the timestamps of the frames they show.
        assert!(blend_pts(outgoing, incoming, 1.0) <= incoming);
        assert!(blend_pts(outgoing, incoming, 0.0) > outgoing);
    }

    /// A recording that runs backwards, or two clips from the same instant, must
    /// still produce a new timestamp: a frozen picture is worse than a wrong one.
    #[test]
    fn a_blended_frame_always_moves_forward() {
        let outgoing = 5_000_000;
        assert!(blend_pts(outgoing, outgoing, 0.5) > outgoing);
        assert!(blend_pts(outgoing, outgoing - 1_000_000, 0.5) > outgoing);
        assert!(blend_pts(i64::MAX, i64::MAX, 0.5) > outgoing);
    }
}


