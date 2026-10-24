//! Instant replay. Encode and disk live in the Pro module. This module owns
//! readback handoff, health, and channel playout.

mod audio;
pub use eiviz_replay::channel;
pub mod gpu_encode;
mod health;
pub(crate) mod profile;
mod stretch;
pub mod transition_pass;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use eiviz_control::error::ControlError;
use eiviz_control::session::replay::{
    AngleSourceKind, ReplayDto, overflow_code, profile_code,
    replay_channel_gpu_id, retention_code,
};
use eiviz_pro_api::{
    EncodedFrame, Entitlements, LAYOUT_BGRA, PRO_BUSY, PRO_IO, PixelLayout, ProReplayAngle,
    ProReplayConfig, ProReplayDecoded, ProReplayEstimateAbi, ProReplayPacket, ProReplayStatsAbi,
    ProReplayStorage, ProVideoView, ReplayAngleStats, ReplayPacket, ReplayStats, ReplayStore,
    ReplayStoreConfig, STATE_RECORDING, STATE_STOPPED, VideoFrame, WriteOutcome, estimate_rate_bps,
    free_bytes, remaining_secs,
};

use self::transition_pass::TransitionPass;
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
    layout: PixelLayout,
    data: Vec<u8>,
}

struct Inbox {
    /// Angles kept in key order, so the recorder can serve them round-robin without
    /// rebuilding and sorting a key list on every frame. A recording runs this path
    /// sixty times a second, and the sort was pure overhead.
    queues: Mutex<BTreeMap<u32, VecDeque<RecordFrame>>>,
    /// Frames that may wait per angle before the render thread drops one. Tied to
    /// the operator's queue setting, so a short encoder hiccup is absorbed instead
    /// of losing picture.
    /// Which angle to serve next, so one angle cannot monopolise the recorder.
    cursor: Mutex<u32>,
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
            queues: Mutex::new(BTreeMap::new()),
            // Two frames is the old behaviour, used until a session configures a
            // real queue length.
            cursor: Mutex::new(0),
            cap: AtomicUsize::new(2),
            wake: Condvar::new(),
        }
    }

    fn set_capacity(&self, cap: usize) {
        self.cap.store(cap.max(1), Ordering::Relaxed);
    }

    fn offer(&self, frame: RecordFrame) -> bool {
        let cap = self.cap.load(Ordering::Relaxed).max(1);
        let mut queues = self
            .queues
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let queue = queues.entry(frame.angle_id).or_default();
        if queue.len() >= cap {
            return false;
        }
        queue.push_back(frame);
        self.wake.notify_one();
        true
    }

    fn pop(&self, stop: &AtomicBool) -> Option<RecordFrame> {
        let mut queues = self
            .queues
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // Serve the angle after the last one, not whichever the map happens to list first.
        // Taking from a HashMap order picked the same angle every time and starved the
        // others: measured with four CPU angles, one recorded 579 frames and another 1,
        // with 573 readback drops on the starved one.
        let mut cursor = self
            .cursor
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            // BTreeMap keys are already ordered, so the next angle is a range lookup
            // rather than a fresh Vec allocation and sort every frame.
            let start = *cursor;
            let next = queues
                .range(start..)
                .find(|(_, queue)| !queue.is_empty())
                .map(|(key, _)| *key)
                .or_else(|| {
                    queues
                        .range(..start)
                        .find(|(_, queue)| !queue.is_empty())
                        .map(|(key, _)| *key)
                });
            if let Some(key) = next {
                if let Some(frame) = queues.get_mut(&key).and_then(|queue| queue.pop_front()) {
                    *cursor = key.wrapping_add(1);
                    return Some(frame);
                }
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

/// What an angle's audio follows: one input, or the inputs of one scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioSourceSpec {
    pub angle_id: u32,
    /// The input the angle records, when it records an input.
    pub input_id: u64,
    /// The scene's gpu id, when the angle records a scene.
    pub scene_id: u64,
}
struct ChannelSlot {
    index: u32,
    source_id: u64,
    transport: channel::Transport,
    /// Bumped by every control change. The playout loop takes a snapshot to decode a
    /// frame and used to write the whole transport back afterwards, which discarded a
    /// cue or speed change that landed while the frame was in flight. The revision lets
    /// it keep the newer transport instead.
    transport_revision: u64,
    /// Playback audio cursor for this channel, kept outside the engine lock so a
    /// file read never blocks the render thread.
    audio: Arc<Mutex<audio::PlaybackAudio>>,
}

impl ChannelSlot {
    fn touch_transport(&mut self) {
        self.transport_revision = self.transport_revision.wrapping_add(1);
    }
}

struct Inner {
    config: ReplayDto,
    /// Whether playout decodes a frame or two ahead of the picture. The operator
    /// owns this from Replay Setup: it buys jitter tolerance with picture memory.
    decode_ahead: bool,
    /// Frames this engine accepted per angle. The store reports its own count, and a
    /// module that under-reports an angle past the eighth left the operator looking at
    /// zeros next to files that were plainly written; the larger of the two is shown,
    /// since both only grow.
    angle_written: std::collections::HashMap<u32, u64>,
    /// Frames a GPU angle lost because its encode ring had no free texture. Kept
    /// apart from readback drops because the two say different things: one is a
    /// shallow readback ring, the other an encoder that is behind.
    angle_gpu_dropped: std::collections::HashMap<u32, u64>,
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
    /// The recording targets the render loop asks for every frame. Rebuilding this
    /// Vec and re-locking the engine several times per frame was measurable, so it is
    /// built when the config or the recording state changes and cloned as an Arc.
    targets_cache: Option<Arc<[RecordTarget]>>,
    /// Waveform buckets per angle, filled from the audio the scheduler mixes.
    peaks: HashMap<u32, eiviz_pro_api::PeakAccumulator>,
    health: HealthMemory,
    started_at: Option<Instant>,
    /// Wall clock (milliseconds since the epoch) at the recording start: the time of
    /// frame 0, which is what the deck shows as the recording's timecode.
    started_unix_ms: u64,
    /// Why record and play are refused while the document stays loaded.
    /// Community and a missing Pro module set this. A plan that simply has too
    /// many channels refuses the document instead.
    operations_reason: Option<String>,
}

pub struct ReplayEngine {
    inner: Mutex<Inner>,
    inbox: Arc<Inbox>,
    audio_inbox: Arc<AudioInbox>,
    audio_drops: AtomicU64,
    /// The recording being written (or the last one recorded). Writes go here.
    record_store: Mutex<Option<Arc<Mutex<Box<dyn ReplayStore>>>>>,
    /// Recordings opened for playback, keyed by the recording id the clips carry.
    /// One entry per recording (not per channel), so two channels can play two
    /// different recordings at the same time.
    playback_stores: Mutex<HashMap<String, Arc<Mutex<Box<dyn ReplayStore>>>>>,
    /// Bumped when a recording is attached for playback, so a channel drops the
    /// frames it decoded ahead from the recording that is no longer its own.
    playback_generation: AtomicU64,
    stop_record: Arc<AtomicBool>,
    stop_workers: Arc<AtomicBool>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    /// The render device, shared with the playout decoder. Set once when the
    /// renderer starts; the channel worker can then decode packets on the GPU.
    gpu: OnceLock<(wgpu::Device, wgpu::Queue)>,
    /// Decoders whose GPU pipelines were built before playback needed them. The
    /// first vmx decode builds shader pipelines and can take a second on a debug
    /// build; doing that in front of the first clip would freeze playout.
    prepared: Mutex<HashMap<u32, PreparedDecoder>>,
    warming: AtomicBool,
}

struct PreparedDecoder {
    width: u32,
    height: u32,
    profile: u32,
    codec: vmx::Codec,
    /// The incoming side of a transition needs its own codec: one codec cannot
    /// have two decodes in flight because both use its coefficient buffer.
    secondary: Option<vmx::Codec>,
    /// The blend that joins the two textures. Built here so the first transition
    /// does not compile a shader in front of playout.
    transition: Option<TransitionPass>,
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
    /// Encode from the scene texture instead of reading it back first.
    pub gpu_encode: bool,
    /// What the CPU path reads back. A CPU encoder gets packed UYVY straight from
    /// the GPU pack pass; a GPU angle never reads back a picture at all.
    pub layout: PixelLayout,
    /// Replay format profile code, for the codec preset.
    pub profile: u32,
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
                decode_ahead: true,
                angle_written: std::collections::HashMap::new(),
                angle_gpu_dropped: std::collections::HashMap::new(),
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
                targets_cache: None,
                peaks: HashMap::new(),
                health: HealthMemory::default(),
                started_at: None,
            started_unix_ms: 0,
                operations_reason: None,
            }),
            inbox: Arc::new(Inbox::new()),
            audio_inbox: Arc::new(AudioInbox::new()),
            audio_drops: AtomicU64::new(0),
            record_store: Mutex::new(None),
            playback_stores: Mutex::new(HashMap::new()),
            playback_generation: AtomicU64::new(0),
            stop_record: Arc::new(AtomicBool::new(true)),
            stop_workers: Arc::new(AtomicBool::new(false)),
            workers: Mutex::new(Vec::new()),
            gpu: OnceLock::new(),
            prepared: Mutex::new(HashMap::new()),
            warming: AtomicBool::new(false),
        };
        // A recording that silently runs the scalar path on an AVX2 machine is hard
        // to tell from a slow disk. Name the path once, where the log starts.
        let simd = vmx::SimdCapabilities::detect().select_path(960);
        crate::diag::info(&format!(
            "replay simd path={simd} color={} profile={}",
            vmx::ColorSimdPath::detect(),
            profile::is_enabled(),
        ));
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

    /// Remembers the render device so a channel can decode a packet on the GPU.
    /// Called once, when the render loop starts.
    pub fn set_gpu(&self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let _ = self.gpu.set((device.clone(), queue.clone()));
        self.warm_decoders();
    }

    fn gpu_device(&self) -> Option<(wgpu::Device, wgpu::Queue)> {
        self.gpu.get().cloned()
    }

    /// Builds each channel's GPU decoder before playout needs it. The first vmx
    /// decode creates the compute pipelines, which is far too slow to do in front
    /// of the first frame of a clip.
    fn warm_decoders(&self) {
        if self.warming.swap(true, Ordering::AcqRel) {
            return;
        }
        let Some((device, queue)) = self.gpu_device() else {
            self.warming.store(false, Ordering::Release);
            return;
        };
        let (width, height, profile, channels, recording) = {
            let inner = self.lock();
            (
                inner.width,
                inner.height,
                profile_code(inner.config.format.profile),
                inner
                    .channels
                    .iter()
                    .map(|slot| slot.index)
                    .collect::<Vec<_>>(),
                inner.recording,
            )
        };
        if recording || channels.is_empty() {
            self.warming.store(false, Ordering::Release);
            return;
        }
        // A stop that follows another stop, or a re-apply of the same document,
        // must not rebuild pipelines that are already waiting.
        let already_ready = {
            let prepared = self
                .prepared
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            channels.iter().all(|index| {
                prepared.get(index).is_some_and(|entry| {
                    entry.width == width && entry.height == height && entry.profile == profile
                })
            })
        };
        if already_ready {
            self.warming.store(false, Ordering::Release);
            return;
        }
        let _ = thread::Builder::new()
            .name("eiviz-replay-warm".into())
            .spawn(move || {
                let engine = engine();
                for index in channels {
                    if !engine.warming.load(Ordering::Relaxed) {
                        break;
                    }
                    let profile_vmx = match profile {
                        0 => vmx::Profile::Hq,
                        2 => vmx::Profile::Lq,
                        _ => vmx::Profile::Sq,
                    };
                    let Ok(mut codec) = vmx::Codec::new(vmx::Config {
                        width: width as i32,
                        height: height as i32,
                        profile: profile_vmx,
                        color_space: vmx::ColorSpace::Bt709,
                    }) else {
                        break;
                    };
                    // The blank decode is only there to build the pipelines; its
                    // picture is discarded.
                    {
                        let _guard = crate::device::lock_gpu_queue();
                        let _ = codec.decode_to_texture(&device, &queue);
                    }
                    let secondary = vmx::Codec::new(vmx::Config {
                        width: width as i32,
                        height: height as i32,
                        profile: profile_vmx,
                        color_space: vmx::ColorSpace::Bt709,
                    })
                    .ok()
                    .map(|mut second| {
                        let _guard = crate::device::lock_gpu_queue();
                        let _ = second.decode_to_texture(&device, &queue);
                        second
                    });
                    // A transition between two clips blends their textures, so the
                    // pipeline is built here instead of on the channel worker.
                    let transition = Some(TransitionPass::new(
                        &device,
                        wgpu::TextureFormat::Bgra8Unorm,
                    ));
                    engine
                        .prepared
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .insert(
                            index,
                            PreparedDecoder {
                                width,
                                height,
                                profile,
                                codec,
                                secondary,
                                transition,
                            },
                        );
                }
                engine.warming.store(false, Ordering::Release);
                crate::diag::info("replay GPU decoders warmed");
            });
    }

    fn is_warming(&self) -> bool {
        self.warming.load(Ordering::Relaxed)
    }

    fn take_prepared(
        &self,
        index: u32,
        width: u32,
        height: u32,
        profile: u32,
    ) -> Option<PreparedDecoder> {
        let mut prepared = self
            .prepared
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // Taken rather than borrowed: the decoder owns it from here, and an entry
        // that does not match the current format would only go stale.
        let entry = prepared.remove(&index)?;
        if entry.width != width || entry.height != height || entry.profile != profile {
            return None;
        }
        Some(entry)
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
            inner.operations_reason = None;
            inner.channels.clear();
            // Removing Replay leaves nothing to report, so the state goes back to
            // idle instead of holding the last recording's numbers.
            inner.stats.state = REPLAY_STATE_IDLE;
            inner.stats.stop_reason = REPLAY_STOP_NONE;
            return Ok(());
        };
        let operations_reason = self.gate(&dto)?;
        // Both are the operator's to set now, so a configure is what turns them on.
        profile::set_enabled(dto.profiling);
        self.lock().decode_ahead = dto.decode_ahead;
        // Encoding needs a ring of full-size textures per angle. Refusing here, with the
        // numbers, beats allocating until the driver fails somewhere unrelated.
        {
            let ring = crate::replay::gpu_encode::ring_bytes(&dto);
            // Three gigabytes of ring: far more than any supported angle count needs at
            // 1080p, so hitting this means the configuration is out of scope, not unlucky.
            const BUDGET: u64 = 3 * 1024 * 1024 * 1024;
            if ring > BUDGET {
                return Err(ReplayFailure::invalid(format!(
                    "{} angles need {:.1} GB of encoder ring at {}x{}, which is above the {:.1} GB this recorder reserves; lower the angle count or the recording size",
                    dto.angles.len().max(1),
                    ring as f64 / (1024.0 * 1024.0 * 1024.0),
                    dto.format.width,
                    dto.format.height,
                    BUDGET as f64 / (1024.0 * 1024.0 * 1024.0),
                )));
            }
        }
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
        // The recordings this session uses are opened up front, from the session's
        // own list: a clip names a recording and the session says where it lives, so
        // playback never has to search a drive to find it. One store per recording,
        // shared by the channels, so two channels can show two recordings at once.
        // A recording in progress keeps its own store and is never replaced.
        {
            let referenced = referenced_recordings(&dto);
            let recording_in_progress = {
                let inner = self.lock();
                (inner.recording, inner.uuid.clone())
            };
            for (uuid, storage) in referenced {
                if recording_in_progress.0 && recording_in_progress.1 == uuid {
                    continue;
                }
                if self
                    .playback_stores
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .contains_key(&uuid)
                {
                    continue;
                }
                let directory = std::path::Path::new(&storage).join(&uuid);
                if !directory.is_dir() {
                    crate::diag::info(&format!(
                        "replay attach: recording {uuid} is not at {}",
                        directory.display()
                    ));
                    continue;
                }
                let path = directory.display().to_string();
                match crate::pro_load::loaded_api() {
                    Some((api, module)) => match attach_ffi(api, module, &path) {
                        Ok(store) => {
                            self.playback_stores
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .insert(
                                    uuid.clone(),
                                    std::sync::Arc::new(std::sync::Mutex::new(store)),
                                );
                            self.playback_generation.fetch_add(1, Ordering::Relaxed);
                            crate::diag::info(&format!("replay attach: attached {path}"));
                        }
                        Err(error) => crate::diag::error(&format!(
                            "replay attach: {path} could not be opened: {error}"
                        )),
                    },
                    None => crate::diag::error("replay attach: no module is loaded"),
                }
            }
        }        let mut inner = self.lock();
        let mut old = std::mem::take(&mut inner.channels);
        // The recording store is opened with the angle set the operator had at the time,
        // and it keeps that list: a later configure with more angles left the extra ones
        // without a store entry, so their statistics read zero even though the recorder
        // wrote them. A changed angle set while stopped reopens it.
        let previous_angles: Vec<u32> = inner.config.angles.iter().map(|angle| angle.id).collect();
        let next_angles: Vec<u32> = dto.angles.iter().map(|angle| angle.id).collect();
        let angles_changed = previous_angles != next_angles;
        let recording = inner.recording;
        drop(inner);
        // Only when the store actually holds a different angle set: closing one that
        // already matches would drop a recording the operator is still working with.
        let stale_store = angles_changed
            && !recording
            && self
                .record_store
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .is_some_and(|store| {
                    let reported = store
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .stats()
                        .angles
                        .len();
                    reported != next_angles.len()
                });
        if stale_store {
            self.release_store();
            *self.record_store.lock().unwrap_or_else(|error| error.into_inner()) = None;
        }
        let mut inner = self.lock();
        inner.operations_reason = operations_reason;
        inner.config = dto;
        inner.targets_cache = None;
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
                    .map(|slot| std::mem::replace(&mut slot.transport, channel::Transport::new()))
                    .unwrap_or_else(channel::Transport::new);
                // The operator's channel settings outlive a reconfiguration: editing the
                // playlist must not silently reset the speed or turn looping off.
                let keep_looping = transport.is_looping();
                let keep_rate = transport.rate();
                let clips = playlist_clips(&config, channel.playlist_id);
                // What the channel was actually given, transitions included: a playlist
                // that arrives without them plays as a row of cuts.
                crate::diag::warn_throttled(
                    "replay-configure-clips",
                    &format!(
                        "replay channel={} playlist={:?} clips={} transitions={:?}",
                        channel.index,
                        channel.playlist_id,
                        clips.len(),
                        clips
                            .iter()
                            .map(|clip| (clip.transition.kind, clip.transition.frames))
                            .collect::<Vec<_>>()
                    ),
                );
                transport.set_playlist(clips);
                transport.set_looping(keep_looping);
                transport.set_rate(keep_rate);
                let audio = old
                    .iter()
                    .find(|slot| slot.index == channel.index)
                    .map(|slot| Arc::clone(&slot.audio))
                    .unwrap_or_else(|| {
                        Arc::new(Mutex::new(audio::PlaybackAudio::new(AUDIO_CHANNELS)))
                    });
                ChannelSlot {
                    index: channel.index,
                    source_id: if channel.input_id == 0 {
                        replay_channel_gpu_id(channel.index)
                    } else {
                        channel.input_id
                    },
                    transport,
                    transport_revision: 0,
                    audio,
                }
            })
            .collect();
        drop(inner);
        self.prepared
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        self.warm_decoders();
        self.ensure_workers();
        Ok(())
    }

    /// `Ok(None)` means record and play may run. `Ok(Some)` keeps the document
    /// and refuses those operations. `Err` is a document the plan cannot hold,
    /// such as a fifth channel on Professional.
    fn gate(&self, dto: &ReplayDto) -> Result<Option<String>, ReplayFailure> {
        let ent = crate::pro::entitlements();
        let denied = matches!(ent.replay_channels, eiviz_pro_api::Quota::Denied)
            || matches!(ent.replay_angles, eiviz_pro_api::Quota::Denied);
        if denied {
            return Ok(Some(
                "Instant Replay requires a Professional license and the Pro module".into(),
            ));
        }
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
            return Ok(None);
        }
        if crate::pro::module().replay().is_none() {
            return Ok(Some("replay requires the Pro module".into()));
        }
        Ok(None)
    }

    pub fn record(&self, on: bool) -> Result<(), ReplayFailure> {
        if !on {
            self.stop(REPLAY_STOP_USER);
            return Ok(());
        }
        if let Some(reason) = self.lock().operations_reason.clone() {
            return Err(ReplayFailure::plan(reason));
        }
        let (
            paths,
            width,
            height,
            fps_num,
            fps_den,
            profile,
            angles,
            reserve,
            queue,
            write_mb,
            overflow,
            retention,
            segment_frames,
            protect_from,
        ) = {
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
        *self.record_store.lock().unwrap_or_else(|error| error.into_inner()) =
            Some(Arc::new(Mutex::new(store)));
        self.ensure_workers();
        let mut inner = self.lock();
        inner.recording = true;
        inner.uuid = uuid;
        inner.mark_in = None;
        // A new recording starts a new waveform and a new audio tail.
        inner.peaks.clear();
        inner.angle_gpu_dropped.clear();
        self.audio_inbox.clear();
        inner.width = width;
        inner.height = height;
        inner.fps_num = fps_num;
        inner.fps_den = fps_den;
        inner.started_at = Some(Instant::now());
        inner.started_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0);
        inner.first_frame = 0;
        inner.last_frame = 0;
        inner.has_frames = false;
        inner.stats.state = REPLAY_STATE_RECORDING;
        inner.stats.stop_reason = REPLAY_STOP_NONE;
        inner.stats.remaining_is_estimate = 1;
        inner.config.angles = remembered;
        inner.targets_cache = None;
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
            inner.targets_cache = None;
            inner.stats.state = REPLAY_STATE_STOPPED;
            inner.stats.stop_reason = reason;
        }
        // The waveform keeps what was recorded, including the last partial bucket.
        for accumulator in inner.peaks.values_mut() {
            accumulator.flush();
        }
        drop(inner);
        // Playback usually starts after the recorder stops. Warm the GPU decoders
        // now so the first clip does not pay for shader pipeline creation.
        if !self.stop_workers.load(Ordering::Relaxed) {
            self.warm_decoders();
        }
    }

    /// Releases the recorded material. Only when Replay is removed or the mixer
    /// shuts down: a stopped recorder keeps its frames.
    fn release_store(&self) {
        if let Some(store) = self.record_store.lock().unwrap_or_else(|e| e.into_inner()).clone() {
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
        let mut workers = self
            .workers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
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

    pub fn record_targets(&self) -> Arc<[RecordTarget]> {
        let mut inner = self.lock();
        if !inner.recording {
            return empty_targets();
        }
        if let Some(cache) = &inner.targets_cache {
            return Arc::clone(cache);
        }
        let targets: Vec<RecordTarget> = inner
            .config
            .angles
            .iter()
            .map(|angle| RecordTarget {
                source: match angle.source {
                    AngleSourceKind::Input => RecordSource::Input(angle.input_id),
                    AngleSourceKind::Scene => {
                        RecordSource::Scene(eiviz_control::ids::scene_gpu_id(angle.scene_id))
                    }
                },
                readback_key: REPLAY_RECORD_BASE | u64::from(angle.id),
                width: inner.width,
                height: inner.height,
                angle_id: angle.id,
                gpu_encode: angle.encoder == eiviz_control::session::replay::AngleEncoder::Gpu,
                layout: if angle.encoder == eiviz_control::session::replay::AngleEncoder::Gpu {
                    PixelLayout::Bgra
                } else {
                    // The CPU encoder needs 4:2:2. Packing on the GPU keeps the
                    // readback at half the bytes and skips the scalar conversion.
                    PixelLayout::Uyvy
                },
                profile: profile_code(inner.config.format.profile) as u32,
            })
            .collect();
        let cache: Arc<[RecordTarget]> = Arc::from(targets);
        inner.targets_cache = Some(Arc::clone(&cache));
        cache
    }

    /// Scenes the recorder reads. Only scene angles need them drawn.
    pub fn replay_scene_ids(&self) -> Vec<u64> {
        self.record_targets()
            .iter()
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
                AngleSourceKind::Scene => Some(eiviz_control::ids::scene_gpu_id(angle.scene_id)),
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

    /// Stores a frame this mixer encoded on the GPU from the angle's own texture.
    ///
    /// The frame index is the render frame the picture came from, so marks and clips
    /// land on the same timeline the recording uses.
    pub fn write_gpu_encoded(
        &self,
        angle_id: u32,
        frame_index: u64,
        pts: i64,
        payload: &[u8],
    ) -> bool {
        let (width, height, fps_num, fps_den) = {
            let inner = self.lock();
            if !inner.recording {
                return false;
            }
            (inner.width, inner.height, inner.fps_num, inner.fps_den)
        };
        let frame = EncodedFrame {
            payload: Arc::new(payload.to_vec()),
            width,
            height,
            pts_100ns: pts,
            fps_num,
            fps_den,
            dropped_before: 0,
        };
        let store = self
            .record_store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let Some(store) = store else {
            return false;
        };
        let outcome = store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .write_encoded(angle_id, frame_index, &frame);
        match outcome {
            Ok(WriteOutcome::Accepted) => {
                let mut inner = self.lock();
                *inner.angle_written.entry(angle_id).or_insert(0) += 1;
                inner.stats.frames_written = inner.stats.frames_written.saturating_add(1);
                if !inner.has_frames {
                    inner.first_frame = frame_index;
                    inner.has_frames = true;
                }
                inner.last_frame = inner.last_frame.max(frame_index);
                inner.stats.recording_frame = inner.last_frame;
                inner.stats.last_frame = inner.last_frame;
                true
            }
            Ok(WriteOutcome::Dropped) => {
                let mut inner = self.lock();
                inner.stats.frames_dropped = inner.stats.frames_dropped.saturating_add(1);
                false
            }
            Err(error) => {
                // Counted, not flagged. A single refused write used to light the storage
                // warning for the rest of the session; the health line already reports a
                // disk that is genuinely behind, and the drop counter says how many
                // frames were lost.
                let mut inner = self.lock();
                inner.stats.frames_dropped = inner.stats.frames_dropped.saturating_add(1);
                crate::diag::error(&format!("replay GPU encode write failed: {error}"));
                false
            }
        }
    }

    pub fn offer_frame(
        &self,
        angle_id: u32,
        frame_index: u64,
        pts: i64,
        data: Vec<u8>,
        stride: usize,
        layout: PixelLayout,
    ) {
        let accepted = self.inbox.offer(RecordFrame {
            angle_id,
            frame_index,
            pts,
            stride,
            layout,
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

    /// Records that a GPU angle could not encode, so the health line can say why a
    /// recording is short instead of leaving it to look like a slow disk.
    pub fn note_encoder_unavailable(&self) {
        let mut inner = self.lock();
        inner.stats.issues |= crate::abi::REPLAY_ISSUE_ENCODER_UNAVAILABLE;
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

    /// Records a GPU angle that had no free encode ring texture. This is not a
    /// readback drop: the picture never left the GPU, the encoder was still busy.
    pub fn note_encode_dropped(&self, angle_id: u32) {
        let mut inner = self.lock();
        *inner.angle_gpu_dropped.entry(angle_id).or_default() += 1;
    }

    /// Feeds the audio the scheduler just mixed into the waveform buckets of every
    /// angle that records from a mixing unit.
    /// The audio of what each angle records, for the angles whose audio follows
    /// their source: `(angle id, input id or 0, scene id or 0)`. Built once per
    /// configure and read by the audio graph every block.
    pub fn audio_sources(&self) -> Arc<[AudioSourceSpec]> {
        let inner = self.lock();
        inner
            .config
            .angles
            .iter()
            .filter(|angle| {
                angle.audio == eiviz_control::session::replay::AngleAudio::Source
            })
            .map(|angle| AudioSourceSpec {
                angle_id: angle.id,
                input_id: match angle.source {
                    AngleSourceKind::Input => angle.input_id,
                    AngleSourceKind::Scene => 0,
                },
                scene_id: match angle.source {
                    AngleSourceKind::Scene => eiviz_control::ids::scene_gpu_id(angle.scene_id),
                    AngleSourceKind::Input => 0,
                },
            })
            .collect()
    }

    pub fn offer_audio(&self, mixed: &crate::audio::MixedAudio) {
        let by_unit = &mixed.by_unit;
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
            // Angles that follow a mixing unit take that unit's program mix.
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
            // Angles that follow their source take its own sound: the input the angle
            // records, or the inputs in the scene it records.
            let sources: Vec<(u32, &Vec<f32>)> = inner
                .config
                .angles
                .iter()
                .filter(|angle| {
                    angle.audio == eiviz_control::session::replay::AngleAudio::Source
                })
                .filter_map(|angle| mixed.for_angle(angle.id).map(|pcm| (angle.id, pcm)))
                .collect();
            for (angle_id, pcm) in sources {
                inner
                    .peaks
                    .entry(angle_id)
                    .or_insert_with(|| eiviz_pro_api::PeakAccumulator::new(AUDIO_SAMPLE_RATE))
                    .push(pcm, AUDIO_CHANNELS);
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
    pub fn copy_peaks(
        &self,
        angle_id: u32,
        from: u32,
        min_out: &mut [f32],
        max_out: &mut [f32],
    ) -> u32 {
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
            .record_store
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

    /// Opens the recording a clip belongs to, so clips from a session that spans more
    /// than one recording still play. A recording in progress is never replaced: the
    /// store is then the file being written.
    ///
    /// Returns whether the recording is open after the call.
    fn attach_for_playback(&self, uuid: &str) -> Result<bool, ReplayFailure> {
        let uuid = uuid.trim();
        if uuid.is_empty() {
            return Ok(true);
        }
        if self
            .playback_stores
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(uuid)
        {
            return Ok(true);
        }
        // The recording being written right now is served by the record store.
        {
            let inner = self.lock();
            if inner.recording && inner.uuid == uuid {
                return Ok(true);
            }
        }
        // Where the session says this recording lives; a document written before the
        // list existed is looked up under its storage entries by id.
        let directory = {
            let inner = self.lock();
            let known = inner
                .config
                .recordings
                .iter()
                .find(|recording| recording.uuid.trim() == uuid)
                .map(|recording| {
                    let storage = if recording.storage.trim().is_empty() {
                        inner
                            .config
                            .storage
                            .first()
                            .map(|item| item.path.clone())
                            .unwrap_or_default()
                    } else {
                        recording.storage.clone()
                    };
                    std::path::Path::new(&storage).join(uuid)
                });
            // A recording listed on a drive that moved is looked for on the session's
            // other drives before it counts as missing. A direct path check only.
            known
                .filter(|candidate| candidate.is_dir())
                .or_else(|| {
                    inner
                        .config
                        .storage
                        .iter()
                        .map(|storage| std::path::Path::new(&storage.path).join(uuid))
                        .find(|candidate| candidate.is_dir())
                })
        };
        let Some(directory) = directory else {
            return Ok(false);
        };
        if !directory.is_dir() {
            return Ok(false);
        }
        let Some((api, module)) = crate::pro_load::loaded_api() else {
            return Err(ReplayFailure::io("replay requires the Pro module"));
        };
        let store = attach_ffi(api, module, &directory.display().to_string()).map_err(|error| {
            ReplayFailure::io(format!("recording {uuid} could not be opened: {error}"))
        })?;
        self.playback_stores
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(
                uuid.to_string(),
                std::sync::Arc::new(std::sync::Mutex::new(store)),
            );
        self.playback_generation.fetch_add(1, Ordering::Relaxed);
        crate::diag::info(&format!(
            "replay attach: attached {} for {uuid}",
            directory.display()
        ));
        Ok(true)
    }

    /// The store a channel plays a given recording from.
    fn playback_store(&self, uuid: &str) -> Option<Arc<Mutex<Box<dyn ReplayStore>>>> {
        let uuid = uuid.trim();
        if !uuid.is_empty() {
            if let Some(store) = self
                .playback_stores
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(uuid)
            {
                return Some(Arc::clone(store));
            }
        }
        // Live playout and a clip from the recording still being written use the
        // record store. A clip that names another recording it could not open gets
        // nothing: reading the wrong recording would show a picture that belongs to a
        // different moment, which is worse than saying it is missing.
        if !uuid.is_empty() {
            // The record store keeps the last recording open after Stop, so a clip from
            // it still plays without being attached a second time.
            let is_record_store = {
                let inner = self.lock();
                inner.uuid == uuid
            };
            if !is_record_store {
                return None;
            }
        }
        self.record_store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn cue(
        &self,
        channel: u32,
        angle_id: u32,
        in_frame: u64,
        out_frame: u64,
        recording_uuid: &str,
    ) -> Result<(), ReplayFailure> {
        let mut inner = self.lock();
        if let Some(reason) = inner.operations_reason.clone() {
            return Err(ReplayFailure::plan(reason));
        }
        drop(inner);
        // Bring the recording this clip belongs to into the store slot. A session that
        // spans several recordings (recorded, restarted, recorded again) needs this:
        // otherwise the older clips read frames from the newer recording.
        // Best effort: a recording that is not on a drive leaves the store as it is.
        // The cue then either uses the open recording or reports that none is open.
        let _ = self.attach_for_playback(recording_uuid)?;
        // Without an attached recording there is nothing to read: say so, instead of
        // cueing a clip that can only play black. The session keeps the timecodes.
        if self.playback_store(recording_uuid).is_none() {
            return Err(ReplayFailure::io(
                "the recording for this clip is not on any drive; the timecodes stay in the session",
            ));
        }
        if let Some((first, _)) = self.angle_range(angle_id)
            && eiviz_control::session::replay::clip_overwritten(out_frame, Some(first))
        {
            return Err(ReplayFailure::invalid(
                "this clip's frames were overwritten and can no longer be played",
            ));
        }
        let mut inner = self.lock();
        let slot = inner
            .channels
            .iter_mut()
            .find(|slot| slot.index == channel)
            .ok_or_else(|| ReplayFailure::invalid("replay channel does not exist"))?;
        slot.transport.cue(channel::Cue {
            recording_uuid: recording_uuid.to_string(),
            angle_id,
            in_frame,
            out_frame,
        });
        slot.touch_transport();
        Ok(())
    }

    pub fn transport(
        &self,
        channel: u32,
        playing: bool,
        speed_milli: i32,
        seek_frame: i64,
    ) -> Result<(), ReplayFailure> {
        let mut inner = self.lock();
        if playing && let Some(reason) = inner.operations_reason.clone() {
            return Err(ReplayFailure::plan(reason));
        }
        let slot = inner
            .channels
            .iter_mut()
            .find(|slot| slot.index == channel)
            .ok_or_else(|| ReplayFailure::invalid("replay channel does not exist"))?;
        let mut changed = false;
        if slot.transport.playing != playing {
            slot.transport.set_playing(playing);
            changed = true;
        }
        if speed_milli != 0 {
            let rate = f64::from(speed_milli) / 1000.0;
            if (slot.transport.rate() - rate).abs() > 1e-6 {
                slot.transport.set_rate(rate);
                changed = true;
            }
        }
        if seek_frame >= 0 {
            let target = seek_frame as u64;
            if slot.transport.source_frame() != target {
                slot.transport.seek_source(target);
                changed = true;
            }
        }
        if changed {
            slot.touch_transport();
        }
        Ok(())
    }

    /// Turns looping on or off for a channel's playlist.
    pub fn set_looping(&self, channel: u32, looping: bool) -> Result<(), i32> {
        let mut inner = self.lock();
        let slot = inner
            .channels
            .iter_mut()
            .find(|slot| slot.index == channel)
            .ok_or(ERR_INVALID_ARGUMENT)?;
        if slot.transport.is_looping() != looping {
            slot.transport.set_looping(looping);
            slot.touch_transport();
        }
        Ok(())
    }

    /// Moves a channel by `delta` frames inside the clip it is playing.
    ///
    /// Stepping belongs here: the transport owns the position, and the host has no
    /// business reconstructing it from a mark.
    pub fn step_channel(&self, channel: u32, delta: i32) -> Result<(), i32> {
        let mut inner = self.lock();
        let slot = inner
            .channels
            .iter_mut()
            .find(|slot| slot.index == channel)
            .ok_or(ERR_INVALID_ARGUMENT)?;
        let Some(clip) = slot.transport.clip() else {
            return Err(ERR_INVALID_ARGUMENT);
        };
        let current = slot.transport.source_frame() as i64;
        let target =
            (current + i64::from(delta)).clamp(clip.in_frame as i64, clip.out_frame as i64);
        slot.transport.seek_source(target as u64);
        slot.transport.set_playing(false);
        slot.touch_transport();
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
        slot.touch_transport();
        Ok(())
    }

    pub fn copy_stats(
        &self,
        out: &mut MixerReplayStats,
        angles: &mut [MixerReplayAngleStats],
    ) -> u32 {
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

    /// Wall clock of the recording's frame 0, in milliseconds since the epoch; 0 when
    /// nothing has been recorded since this session started.
    pub fn started_unix_ms(&self) -> u64 {
        self.lock().started_unix_ms
    }

    pub fn recording_uuid(&self) -> String {
        self.lock().uuid.clone()
    }

    /// Jog proxy for one frame. Missing proxies stay an error so the tile does
    /// not decode the full-size picture instead.
    pub fn copy_proxy(
        &self,
        angle_id: u32,
        frame: u64,
        dst: &mut [u8],
    ) -> Result<(u32, u32), ReplayFailure> {
        let store = self
            .record_store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or_else(|| ReplayFailure::invalid("proxy is missing"))?;
        let decoded = store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .decode_proxy(angle_id, frame, dst)
            .map_err(|_| ReplayFailure::invalid("proxy is missing"))?;
        Ok((decoded.width, decoded.height))
    }

    /// Recorded frame range of one angle, for the export UI.
    pub fn angle_range(&self, angle_id: u32) -> Option<(u64, u64)> {
        let store = self
            .record_store
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
            transport_revision: 0,
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
        slot.touch_transport();
        Ok(())
    }

    /// Starts an MP4 export of the requested segments.
    pub fn export_start(&self, spec: eiviz_pro_api::ReplayExportSpec) -> Result<(), i32> {
        let store = self
            .record_store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or(ERR_IO)?;
        let result = store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .export_start(spec);
        // The ABI only carries a code, so whatever the module says about a refusal
        // never reaches the operator. Keep it in the log, or a refused export is a dead
        // button with no explanation.
        match &result {
            Ok(()) => crate::diag::info("replay export start accepted"),
            Err(error) => crate::diag::error(&format!("replay export start refused: {error}")),
        }
        result.map_err(|_| ERR_IO)
    }

    /// Progress of the running export, or an idle snapshot when none is running.
    pub fn export_progress(&self) -> eiviz_pro_api::ReplayExportProgress {
        let Some(store) = self
            .record_store
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
            .record_store
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
        // The readback totals belong in the state report: whether the ring is deep enough
        // is a measured question, not one to guess at. They come from the counters the
        // readback path itself fills, so the two can never disagree.
        let readback_mapped = stats.frames_offered;
        let readback_exhausted: u64 = angle_stats[..angle_count]
            .iter()
            .map(|angle| angle.dropped_readback)
            .sum();
        let mut channel_states = [MixerReplayChannelState::default(); 26];
        let running_order: Vec<usize> = self
            .lock()
            .channels
            .iter()
            .map(|slot| slot.transport.list_len())
            .collect();
        let channel_count = self.copy_channels(&mut channel_states) as usize;
        let channels: Vec<serde_json::Value> = channel_states[..channel_count]
            .iter()
            .enumerate()
            .map(|(index, channel)| {
                serde_json::json!({
                    "index": channel.channel_index,
                    "sourceId": channel.source_id,
                    "playing": channel.playing != 0,
                    "speedMilli": channel.speed_milli,
                    "positionFrame": channel.position_frame,
                    "playlistLength": running_order.get(index).copied().unwrap_or(0),
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
                    // What the readback path delivered and refused, which is how the ring
                    // depth is judged rather than guessed at.
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
            "markInFrame": stats.mark_in_frame,
            "markInValid": stats.mark_in_valid != 0,
            "recordingUuid": self.recording_uuid(),
            "audioBlocksDropped": self.audio_drops(),
            // Readback totals across angles: frames the recorder received, and frames it
            // could not take (every ring slot busy, or replaced before the writer).
            "readbackMapped": readback_mapped,
            "readbackExhausted": readback_exhausted,
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
        // Worked out here rather than taken from the module. Free space is the OS's
        // answer for this path, and the rate is the same bitrate model the recorder
        // uses, so both halves of the division are ones this process can stand behind.
        // A module built against an older estimate struct would otherwise hand the
        // operator a confident wrong number: that is how a 124 GB drive came to read
        // "1 TiB".
        let free = eiviz_pro_api::free_bytes(path).unwrap_or(0);
        let rate = eiviz_pro_api::estimate_rate_bps(
            profile,
            width,
            height,
            fps_num,
            fps_den,
            angles.max(1),
        );
        MixerReplayEstimateOut {
            free_bytes: free,
            estimate_rate_bps: rate,
            remaining_secs: eiviz_pro_api::remaining_secs(free.saturating_sub(reserve_bytes), rate),
            path_writable: u32::from(std::fs::create_dir_all(path).is_ok()),
        }
    }

    fn ensure_workers(&self) {
        let mut workers = self
            .workers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
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

    /// The channels that should be playing, and the frame timing they share.
    ///
    /// Read together under one lock so a worker set is built from one configuration
    /// rather than from two halves of two.
    fn playout_setup(
        &self,
    ) -> (
        Vec<(u32, u64, Arc<Mutex<audio::PlaybackAudio>>)>,
        PlayoutContext,
    ) {
        let inner = self.lock();
        let period = Duration::from_secs_f64(
            f64::from(inner.fps_den.max(1)) / f64::from(inner.fps_num.max(1)),
        );
        let channels = inner
            .channels
            .iter()
            .map(|slot| (slot.index, slot.source_id, Arc::clone(&slot.audio)))
            .collect();
        let context = PlayoutContext {
            width: inner.width,
            height: inner.height,
            profile: profile_code(inner.config.format.profile),
            fps: (inner.fps_num, inner.fps_den),
            decode_ahead: inner.decode_ahead,
            // A zero frame rate would otherwise turn the sleeper into a busy loop.
            period: if period.is_zero() {
                Duration::from_millis(16)
            } else {
                period
            },
        };
        (channels, context)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn refresh_stats(&self) {
        let store = self
            .record_store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let Some(store) = store else {
            return;
        };
        let snapshot = store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stats();
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
    inner.stats.started_unix_ms = inner.started_unix_ms;
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
    let measured = inner
        .started_at
        .is_some_and(|start| start.elapsed() >= Duration::from_secs(5));
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
        // The store and the engine both count what was written. Whichever is larger is
        // the truth: a module that under-reports an angle must not hide frames that are
        // on disk, and a store that has flushed more than this engine saw is ahead.
        let ours = inner.angle_written.get(&angle_id).copied().unwrap_or(0);
        let mut stats = angle_stats(reported, &inner, scene_gpu_id);
        stats.frames_written = stats.frames_written.max(ours);
        angles.push(stats);
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
        dropped_readback: inner
            .dropped_readback
            .get(&angle.angle_id)
            .copied()
            .unwrap_or(0),
        dropped_encode: angle.dropped_encode.saturating_add(
            inner
                .angle_gpu_dropped
                .get(&angle.angle_id)
                .copied()
                .unwrap_or(0),
        ),
        dropped_write: angle.dropped_write,
        bytes_written: angle.bytes_written,
        segments: angle.segments,
    }
}

/// The empty target list. A stopped recorder is asked for its targets several times
/// per rendered frame, and allocating a fresh empty Arc each time was pointless.
/// The recordings this document uses, with the directory each lives in.
///
/// The session's own list is the authority: a recording is played from the place the
/// session says it was written. Only a document written before that list existed
/// falls back to looking under its storage entries for the id itself.
fn referenced_recordings(dto: &ReplayDto) -> Vec<(String, String)> {
    let mut ordered: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for recording in &dto.recordings {
        let uuid = recording.uuid.trim();
        if uuid.is_empty() || !seen.insert(uuid.to_string()) {
            continue;
        }
        let storage = if recording.storage.trim().is_empty() {
            dto.storage.first().map(|item| item.path.clone()).unwrap_or_default()
        } else {
            recording.storage.clone()
        };
        ordered.push((uuid.to_string(), storage));
    }
    // Older documents name the recording on the clip but have no list entry.
    let mut mentioned: Vec<&str> = dto
        .playlists
        .iter()
        .flat_map(|list| list.clips.iter())
        .map(|clip| clip.recording_uuid.trim())
        .chain(dto.events.iter().map(|event| event.recording_uuid.trim()))
        .filter(|uuid| !uuid.is_empty())
        .collect();
    mentioned.sort_unstable();
    mentioned.dedup();
    for uuid in mentioned {
        if !seen.insert(uuid.to_string()) {
            continue;
        }
        // The storage that actually holds it; the session never says for these.
        let storage = dto
            .storage
            .iter()
            .map(|item| item.path.clone())
            .find(|path| std::path::Path::new(path).join(uuid).is_dir())
            .or_else(|| dto.storage.first().map(|item| item.path.clone()))
            .unwrap_or_default();
        ordered.push((uuid.to_string(), storage));
    }
    ordered
}
fn empty_targets() -> Arc<[RecordTarget]> {
    static EMPTY: OnceLock<Arc<[RecordTarget]>> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::from(Vec::<RecordTarget>::new())))
}

struct StoreSink<'a> {
    store: &'a mut dyn ReplayStore,
    pts_100ns: i64,
    fps_num: u32,
    fps_den: u32,
}

impl eiviz_replay::FrameSink for StoreSink<'_> {
    fn write_picture(
        &mut self,
        angle_id: u32,
        frame: u64,
        picture: eiviz_replay::Picture<'_>,
    ) -> Result<(), String> {
        let layout = match picture.layout {
            1 => PixelLayout::Bgra,
            _ => PixelLayout::Uyvy,
        };
        let view = VideoFrame {
            width: picture.width,
            height: picture.height,
            stride: picture.stride as u32,
            layout,
            pts_100ns: self.pts_100ns,
            fps_num: self.fps_num,
            fps_den: self.fps_den,
            data: std::sync::Arc::new(picture.pixels.to_vec()),
        };
        self.store
            .write_video(angle_id, frame, &view)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

/// Says once per angle why the recorder found no picture, instead of counting
/// silently. A recording that does not fill then names the source that never
/// delivered, instead of looking like a slow readback.
pub fn log_missing_picture(angle_id: u32, reason: &str) {
    static SAID: Mutex<Option<std::collections::HashSet<u32>>> = Mutex::new(None);
    let mut guard = SAID.lock().unwrap_or_else(|error| error.into_inner());
    let said = guard.get_or_insert_with(Default::default);
    if said.insert(angle_id) {
        crate::diag::info(&format!("{reason} (angle {angle_id})"));
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
            .record_store
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
        let outcome = {
            let mut guard = store.lock().unwrap_or_else(|error| error.into_inner());
            let mut sink = StoreSink {
                store: &mut **guard,
                pts_100ns: frame.pts,
                fps_num,
                fps_den,
            };
            eiviz_replay::deliver_picture(
                &mut sink,
                frame.angle_id,
                frame.frame_index,
                eiviz_replay::Picture {
                    width,
                    height,
                    stride: frame.stride,
                    layout: match frame.layout {
                        PixelLayout::Uyvy => 0,
                        PixelLayout::Bgra => 1,
                    },
                    pixels: &frame.data,
                    pts_100ns: frame.pts,
                    fps_num,
                    fps_den,
                },
            )
        };
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
            .record_store
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
                    pro_unavailable: crate::pro::module().replay().is_none()
                        && inner.config.enabled,
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

/// One channel's GPU decoder. It keeps the codec and packet buffer alive across
/// frames, and remembers when the store cannot hand out packets so a test store
/// does not pay for a failed FFI call every frame.
struct ChannelDecoder {
    store_id: usize,
    width: u32,
    height: u32,
    profile: u32,
    packet_available: bool,
    codec: Option<vmx::Codec>,
    /// The incoming side of a transition. One codec cannot decode two packets in
    /// a row, because both decodes use its coefficient buffer.
    incoming_codec: Option<vmx::Codec>,
    packet: Vec<u8>,
    incoming_packet: Vec<u8>,
    /// Built on first use when the channel was not warmed; a channel that never
    /// runs a transition never builds one.
    transition: Option<TransitionPass>,
    first_decode: bool,
}

enum DecodedGpu {
    Frame(crate::upload::GpuVideoFrame, bool),
    /// The store cannot hand out packets. The caller keeps using CPU decode.
    Unsupported,
    /// A packet was available but decoding it failed. Hold the last picture.
    Failed,
}

impl ChannelDecoder {
    fn new(width: u32, height: u32, profile: u32) -> Self {
        Self {
            store_id: 0,
            width,
            height,
            profile,
            packet_available: true,
            codec: None,
            incoming_codec: None,
            packet: Vec::new(),
            incoming_packet: Vec::new(),
            transition: None,
            first_decode: true,
        }
    }

    fn has_codec(&self) -> bool {
        self.codec.is_some()
    }

    fn adopt(&mut self, prepared: PreparedDecoder) {
        self.codec = Some(prepared.codec);
        self.incoming_codec = prepared.secondary;
        // Only replaces a pass this decoder built itself, which cannot happen yet:
        // adoption is the first thing that runs for a channel.
        self.transition = prepared.transition;
        self.first_decode = false;
    }

    /// Decodes everything a plan asks for and returns a single picture.
    ///
    /// A transition needs two decodes and one blend. Both frames stay on the
    /// device: the outgoing and incoming textures go into the transition pass and
    /// the compositor receives the blended result, with no CPU frame in between.
    fn decode(
        &mut self,
        store: &Arc<Mutex<Box<dyn ReplayStore>>>,
        plan: &channel::FramePlan,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> DecodedGpu {
        let Some((angle, frame)) = plan.primary else {
            return DecodedGpu::Failed;
        };
        let (outgoing, outgoing_substituted) =
            match self.decode_one(store, angle, frame, device, queue, false) {
                DecodedGpu::Frame(frame, substituted) => (frame, substituted),
                other => return other,
            };
        let Some((second_angle, second_frame)) = plan.secondary else {
            return DecodedGpu::Frame(outgoing, outgoing_substituted);
        };
        if !wants_secondary(plan) {
            return DecodedGpu::Frame(outgoing, outgoing_substituted);
        }
        let (incoming, incoming_substituted) =
            match self.decode_one(store, second_angle, second_frame, device, queue, true) {
                DecodedGpu::Frame(frame, substituted) => (frame, substituted),
                // No packet source at all: the CPU path owns this channel.
                DecodedGpu::Unsupported => return DecodedGpu::Unsupported,
                // The incoming frame did not decode. Showing the outgoing one is
                // what the CPU path does, so a bad clip boundary is not black.
                DecodedGpu::Failed => return DecodedGpu::Frame(outgoing, outgoing_substituted),
            };
        let crate::upload::GpuVideoFrame {
            pts: outgoing_pts,
            texture: _outgoing_texture,
            view: outgoing_view,
            ..
        } = outgoing;
        let crate::upload::GpuVideoFrame {
            pts: incoming_pts,
            texture: _incoming_texture,
            view: incoming_view,
            ..
        } = incoming;
        let width = _outgoing_texture.width();
        let height = _outgoing_texture.height();
        // A fresh target per frame, like the decoders' own textures: the
        // compositor samples this one on a later frame, so a reused texture would
        // be overwritten while the previous picture is still on screen.
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("replay-transition-target"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let pass = self
            .transition
            .get_or_insert_with(|| TransitionPass::new(device, wgpu::TextureFormat::Bgra8Unorm));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("replay-transition"),
        });
        {
            let _guard = crate::device::lock_gpu_queue();
            pass.run(
                device,
                queue,
                &mut encoder,
                &outgoing_view,
                &incoming_view,
                &target_view,
                plan.kind,
                plan.direction,
                plan.mix as f32,
            );
            queue.submit(Some(encoder.finish()));
        }
        // A transition picture is its own picture, so it carries its own timestamp;
        // otherwise the compositor would see one timestamp for the whole blend.
        let pts = blend_pts(outgoing_pts, incoming_pts, plan.mix);
        DecodedGpu::Frame(
            crate::upload::GpuVideoFrame {
                pts,
                width,
                height,
                packed: false,
                bgra: true,
                texture: target,
                view: target_view,
                lease: None,
            },
            outgoing_substituted || incoming_substituted,
        )
    }

    /// Decodes one packet into its own texture. `incoming` selects the second codec
    /// and packet buffer that a transition's incoming frame needs.
    fn decode_one(
        &mut self,
        store: &Arc<Mutex<Box<dyn ReplayStore>>>,
        angle_id: u32,
        frame_index: u64,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        incoming: bool,
    ) -> DecodedGpu {
        let store_id = Arc::as_ptr(store) as usize;
        if store_id != self.store_id {
            // A new recording may be a different store (or a test double that has
            // no packet source), so the capability is re-checked once.
            self.store_id = store_id;
            self.packet_available = true;
            self.codec = None;
            self.incoming_codec = None;
            self.first_decode = true;
        }
        if !self.packet_available {
            return DecodedGpu::Unsupported;
        }
        let info = {
            let packet = if incoming {
                &mut self.incoming_packet
            } else {
                &mut self.packet
            };
            let mut guard = store.lock().unwrap_or_else(|error| error.into_inner());
            match guard.read_packet(angle_id, frame_index, packet) {
                Ok(info) => info,
                Err(error) => {
                    return if error.kind == eiviz_pro_api::ProErrorKind::Unavailable {
                        self.packet_available = false;
                        DecodedGpu::Unsupported
                    } else {
                        DecodedGpu::Failed
                    };
                }
            }
        };
        if info.codec != eiviz_pro_api::REPLAY_CODEC_VMX1 {
            self.packet_available = false;
            return DecodedGpu::Unsupported;
        }
        let codec_slot = if incoming {
            &mut self.incoming_codec
        } else {
            &mut self.codec
        };
        if codec_slot.is_none() {
            let profile = match self.profile {
                0 => vmx::Profile::Hq,
                2 => vmx::Profile::Lq,
                _ => vmx::Profile::Sq,
            };
            let started = Instant::now();
            match vmx::Codec::new(vmx::Config {
                width: self.width as i32,
                height: self.height as i32,
                profile,
                color_space: vmx::ColorSpace::Bt709,
            }) {
                Ok(codec) => {
                    *codec_slot = Some(codec);
                    crate::diag::info(&format!(
                        "replay GPU decoder built {}x{}{} in {}ms",
                        self.width,
                        self.height,
                        if incoming { " (incoming)" } else { "" },
                        started.elapsed().as_millis()
                    ));
                }
                Err(error) => {
                    crate::diag::error(&format!(
                        "replay GPU decode codec could not be built {}x{}: {error}",
                        self.width, self.height
                    ));
                    return DecodedGpu::Failed;
                }
            }
        }
        let Some(codec) = codec_slot.as_mut() else {
            return DecodedGpu::Failed;
        };
        let packet = if incoming {
            &self.incoming_packet
        } else {
            &self.packet
        };
        if let Err(error) = codec.load_from(packet) {
            crate::diag::error(&format!(
                "replay GPU packet load failed angle={angle_id} frame={frame_index} bytes={}: {error}",
                packet.len()
            ));
            return DecodedGpu::Failed;
        }
        // The surface configure may wait for the device; a submit during that
        // wait is a validation error. Share the renderer's queue lock.
        let first = self.first_decode && !incoming;
        let submit_started = Instant::now();
        let submitted = {
            let _guard = crate::device::lock_gpu_queue();
            codec.decode_to_texture(device, queue)
        };
        if first {
            self.first_decode = false;
            crate::diag::info(&format!(
                "replay GPU decode first submit in {}ms",
                submit_started.elapsed().as_millis()
            ));
        }
        match submitted {
            Ok(frame) => {
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                DecodedGpu::Frame(
                    crate::upload::GpuVideoFrame {
                        pts: info.pts_100ns,
                        width: frame.width,
                        height: frame.height,
                        packed: false,
                        bgra: true,
                        texture: frame.texture,
                        view,
                        lease: None,
                    },
                    info.substituted,
                )
            }
            Err(error) => {
                crate::diag::error(&format!("replay GPU decode failed: {error}"));
                DecodedGpu::Failed
            }
        }
    }
}

/// What a channel's playout needs that stays the same for the whole run. A configure
/// that changes any of it stops the workers and starts them again.
#[derive(Clone, PartialEq, Eq)]
struct PlayoutContext {
    width: u32,
    height: u32,
    profile: u32,
    fps: (u32, u32),
    period: Duration,
    /// Decode ahead of the picture (Replay Setup). Off means the synchronous path.
    decode_ahead: bool,
}

/// One decoded picture waiting to be shown, with the transport it was decoded at so
/// its block of sound is the one that belongs to that picture.
struct AheadFrame {
    frame: crate::upload::GpuVideoFrame,
    transport: channel::Transport,
}

/// How many decoded frames a channel keeps ready.
///
/// One frame is what takes the decode off the publish path: the picture for a tick is
/// already there when the tick starts, so a decode that runs long delays the next
/// picture instead of this one. Two leaves a frame of slack for a tick that overran.
/// The cost is picture memory (a 1080p BGRA frame is 8 MiB), so 4K keeps one.
///
/// A depth of zero is the operator turning the queue off in Replay Setup, which is
/// the first thing to try if a playback problem is ever blamed on it: the
/// synchronous path is then exactly the one that shipped before.
fn ahead_depth(width: u32, height: u32, enabled: bool) -> usize {
    const BUDGET_BYTES: u64 = 32 * 1024 * 1024;
    if !enabled {
        return 0;
    }
    let frame_bytes = u64::from(width.max(1)) * u64::from(height.max(1)) * 4;
    ((BUDGET_BYTES / frame_bytes.max(1)).clamp(1, 2)) as usize
}

/// Whether the queue may run ahead of the picture.
///
/// A decode has to fit in half a frame for that to pay off. On a software adapter or a
/// CPU that is already busy one decode takes most of the frame, and decoding one more
/// would cost more than the queue saves, so the channel stays on the synchronous path.
fn may_run_ahead(last_decode: Duration, period: Duration) -> bool {
    last_decode.max(Duration::from_micros(200)) <= period.mul_f64(0.5)
}

/// One channel's playout, owned by that channel's thread.
///
/// Each channel keeps its own transport, decoder and deadline, so a decode that runs
/// long falls behind on its own instead of moving every other channel's frame time.
/// The shared state is the engine's slots (short lock), the store (short lock) and
/// the GPU queue, which the renderer shares the same way.
struct ChannelPlayout {
    index: u32,
    source_id: u64,
    audio: Arc<Mutex<audio::PlaybackAudio>>,
    decoder: ChannelDecoder,
    next: Instant,
    /// Decoded pictures waiting their turn, oldest first.
    ahead: std::collections::VecDeque<AheadFrame>,
    /// Revisions the queued pictures belong to. A cue, a speed change or a new
    /// playlist makes them wrong, and showing one would put the old transport on air.
    seen: (u64, u64, u64),
    depth: usize,
    /// How long the last decode took. A machine where that is already a large part
    /// of a frame (a software adapter, a busy CPU) must not try to run ahead: the
    /// extra decode would cost more time than the queue saves.
    last_decode: Duration,
    /// Playhead at the previous tick. A timeline cue fires when this and the
    /// current frame sit on opposite sides of its boundary.
    last_source: Option<u64>,
}

impl ChannelPlayout {
    fn new(
        index: u32,
        source_id: u64,
        audio: Arc<Mutex<audio::PlaybackAudio>>,
        context: &PlayoutContext,
    ) -> Self {
        Self {
            index,
            source_id,
            audio,
            decoder: ChannelDecoder::new(context.width, context.height, context.profile),
            next: Instant::now(),
            ahead: std::collections::VecDeque::with_capacity(4),
            seen: (0, 0, 0),
            depth: ahead_depth(context.width, context.height, context.decode_ahead),
            last_decode: Duration::ZERO,
            last_source: None,
        }
    }

    /// Fires scene states the playhead crossed since the previous tick.
    ///
    /// The mixer lock is taken only after the replay lock is released. A jog
    /// that crosses the same boundary again fires once for that crossing.
    fn fire_crossed_cues(&mut self, engine: &ReplayEngine, current: u64, period: Duration) {
        let Some(previous) = self.last_source.replace(current) else {
            return;
        };
        if previous == current {
            return;
        }
        let due = {
            let inner = engine.lock();
            // Two seconds of frames is well above a late tick and well below a
            // loop, a cue or a jump to live, so only real playback fires states.
            let max_step = (2.0 / period.as_secs_f64().max(1e-6)) as u64;
            eiviz_control::session::replay::due_timeline_cues(
                previous,
                current,
                &inner.config.timeline_cues,
                &inner.config.playlists,
                max_step,
            )
        };
        for (scene_id, state_id) in due {
            let _ = unsafe { crate::mixer_scene_go_to(scene_id, state_id) };
        }
    }

    /// Runs this channel until `stop` is set, once per output frame.
    fn run(mut self, engine: &ReplayEngine, context: &PlayoutContext, stop: &AtomicBool) {
        while !stop.load(Ordering::Relaxed) {
            let due = self.next;
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                thread::sleep(wait);
            }
            let now = Instant::now();
            // A tick that overran covers more than one output frame. Advancing the
            // transport by only one frame turned playback into slow motion whenever a
            // decode took longer than the period. A long stall resyncs instead of
            // replaying every missed frame at once.
            let late = if now > due + context.period {
                let frames =
                    ((now - due).as_secs_f64() / context.period.as_secs_f64()).floor() as u32 + 1;
                frames.clamp(1, 6)
            } else {
                1
            };
            if late > 1 {
                profile::late(late);
            }
            self.tick(engine, context, late);
            self.next = due + context.period * late;
            if self.next <= Instant::now() {
                // Still behind after the catch-up cap. Resync instead of accumulating
                // lateness, which would turn every later tick into a catch-up tick.
                self.next = Instant::now() + context.period;
            }
            profile::log_if_due();
        }
    }

    /// Advances this channel by one output frame: one tick of the transport, one
    /// picture, and one block of sound.
    fn tick(&mut self, engine: &ReplayEngine, context: &PlayoutContext, late: u32) {
        let index = self.index;
        let source_id = self.source_id;
        let audio = Arc::clone(&self.audio);
        // A configure that dropped this channel stops the loop, but one stale tick can
        // still land before that thread sees the flag.
        let (mut transport, transport_revision, revision, last_frame, playback_generation) = {
            let inner = engine.lock();
            let Some(slot) = inner.channels.iter().find(|slot| slot.index == index) else {
                return;
            };
            (
                slot.transport.clone(),
                slot.transport_revision,
                inner.playlist_revision,
                inner.last_frame,
                engine.playback_generation.load(Ordering::Relaxed),
            )
        };
        let gpu = engine.gpu_device();
        // The recording the on-air clip belongs to. Two channels can hold clips from
        // two recordings at the same time; each reads its own.
        let store = engine.playback_store(transport.recording_uuid());
        let width = context.width;
        let height = context.height;
        let profile = context.profile;
        let fps = context.fps;
        // A cue, a speed change or a reconfigure makes every queued picture wrong, and
        // the ones already decoded belong to the transport that was on air before it.
        if self.seen != (transport_revision, revision, playback_generation) {
            self.ahead.clear();
            self.seen = (transport_revision, revision, playback_generation);
        }
        transport.note_recorded_frame(last_frame);
        self.fire_crossed_cues(engine, transport.source_frame(), context.period);
        // A picture decoded during an earlier tick's spare time is shown straight away:
        // that is what keeps it on time when a decode runs long.
        if let Some(pending) = self.ahead.pop_front() {
            profile::queued();
            let pushed = matches!(
                crate::with_uploads(|uploads| uploads.push_playout_gpu(source_id, pending.frame)),
                Ok(Ok(()))
            );
            if pushed {
                if let (Some(store), Some((device, queue))) = (store.as_ref(), gpu.as_ref()) {
                    // The sound belongs to the picture that was just shown, not to the
                    // decode cursor, which is already a frame or two further on.
                    push_channel_audio(source_id, &pending.transport, store, &audio, fps);
                    self.pump(engine, context, &mut transport, store, device, queue, late);
                }
            } else {
                transport.substituted = transport.substituted.saturating_add(1);
            }
            write_back_transport(engine, index, revision, transport_revision, transport);
            return;
        }
        // `step` advances even when it decides to hold the picture, so the transport
        // must be written back on every path. Stepping every missed frame and keeping
        // the last plan catches up without showing each intermediate picture.
        let mut plan = None;
        for _ in 0..late {
            if let Some(next_plan) = transport.step(1.0) {
                plan = Some(next_plan);
            }
        }
        let Some(plan) = plan else {
            // Holding the last picture is the correct answer for pause, jog and a
            // repeated frame. Only clear when there is nothing at all.
            if transport.clip().is_none() && !transport.is_live() {
                paint_black(source_id);
            } else if let Some(store) = store.as_ref() {
                // A held picture still needs its sound. Slow motion shows one source
                // frame for several output frames, so the audio has to be produced
                // every tick or the channel runs at the wrong rate; a pause publishes
                // silence instead of letting the last block repeat.
                push_channel_audio(source_id, &transport, store, &audio, fps);
            }
            write_back_transport(engine, index, revision, transport_revision, transport);
            return;
        };
        let Some(store) = store.as_ref() else {
            write_back_transport(engine, index, revision, transport_revision, transport);
            return;
        };
        // A plain frame and a transition are both decoded on the device. The
        // transition pass blends the two textures, where the CPU path would have
        // copied both frames back for a blend and an upload.
        let transitioning = wants_secondary(&plan);
        if let Some((device, queue)) = gpu.as_ref() {
            let decoder = &mut self.decoder;
            let ready = if decoder.has_codec() {
                true
            } else if let Some(prepared) = engine.take_prepared(index, width, height, profile) {
                decoder.adopt(prepared);
                true
            } else {
                // Pipelines are still being built in the background. The CPU path
                // keeps the first clip playing in the meantime.
                !engine.is_warming()
            };

            if ready {
                let started = Instant::now();
                match decoder.decode(store, &plan, device, queue) {
                    DecodedGpu::Frame(frame, substituted) => {
                        let spent = started.elapsed();
                        let elapsed = spent.as_micros() as u64;
                        profile::decode(spent);
                        self.last_decode = spent;
                        let pushed = matches!(
                            crate::with_uploads(|uploads| {
                                uploads.push_playout_gpu(source_id, frame)
                            }),
                            Ok(Ok(()))
                        );
                        if pushed {
                            transport.substituted =
                                transport.substituted.saturating_add(u64::from(substituted));
                            transport.decode_samples = transport.decode_samples.saturating_add(1);
                            transport.decode_us_sum =
                                transport.decode_us_sum.saturating_add(elapsed);
                            push_channel_audio(source_id, &transport, store, &audio, fps);
                        } else {
                            transport.substituted = transport.substituted.saturating_add(1);
                        }
                        // Fill the queue from the time left in this tick: those
                        // pictures then cost the next ticks nothing to show.
                        self.pump(engine, context, &mut transport, store, device, queue, late);
                        write_back_transport(
                            &engine,
                            index,
                            revision,
                            transport_revision,
                            transport,
                        );
                        return;
                    }
                    DecodedGpu::Unsupported => {
                        // The store has no packet source; use the CPU path below.
                    }
                    DecodedGpu::Failed => {
                        transport.substituted = transport.substituted.saturating_add(1);
                        transport.decode_samples = transport.decode_samples.saturating_add(1);
                        write_back_transport(
                            &engine,
                            index,
                            revision,
                            transport_revision,
                            transport,
                        );
                        return;
                    }
                }
            }
        }
        let Some(mut primary) = take_playout_buffer(source_id, width, height) else {
            transport.substituted = transport.substituted.saturating_add(1);
            write_back_transport(&engine, index, revision, transport_revision, transport);
            return;
        };
        let mut secondary = if transitioning {
            match take_playout_buffer(source_id, width, height) {
                Some(buffer) => Some(buffer),
                None => {
                    recycle_playout_buffer(source_id, primary);
                    transport.substituted = transport.substituted.saturating_add(1);
                    write_back_transport(&engine, index, revision, transport_revision, transport);
                    return;
                }
            }
        } else {
            None
        };
        let started = Instant::now();
        let decoded = render_plan_into(
            store,
            &plan,
            width,
            height,
            &mut primary,
            secondary.as_mut(),
        );
        let elapsed = started.elapsed().as_micros() as u64;
        profile::decode(started.elapsed());
        if let Some(meta) = decoded {
            let publish_started = Instant::now();
            match (meta.secondary, secondary, meta.transition) {
                (true, Some(incoming), Some((kind, direction, mix))) => {
                    let _ = crate::with_uploads(|uploads| {
                        uploads.finish_playout_transition(
                            source_id, primary, incoming, meta.pts, kind, direction, mix,
                        );
                    });
                }
                (_, Some(incoming), _) => {
                    // The incoming frame did not decode. Show the outgoing one
                    // rather than holding a picture older than either.
                    recycle_playout_buffer(source_id, incoming);
                    let _ = crate::with_uploads(|uploads| {
                        uploads.finish_playout_cpu(source_id, primary, meta.pts);
                    });
                }
                _ => {
                    let _ = crate::with_uploads(|uploads| {
                        uploads.finish_playout_cpu(source_id, primary, meta.pts);
                    });
                }
            }
            profile::publish(publish_started.elapsed());
            transport.substituted = transport
                .substituted
                .saturating_add(u64::from(meta.substituted));
        } else {
            // A missing or corrupt frame must not pin the channel: keep the
            // previous picture, advance, and surface the miss.
            recycle_playout_buffer(source_id, primary);
            if let Some(incoming) = secondary {
                recycle_playout_buffer(source_id, incoming);
            }
            transport.substituted = transport.substituted.saturating_add(1);
        }
        transport.decode_samples = transport.decode_samples.saturating_add(1);
        transport.decode_us_sum = transport.decode_us_sum.saturating_add(elapsed);
        // Sound follows picture: one frame of audio per frame of video.
        push_channel_audio(source_id, &transport, store, &audio, fps);
        write_back_transport(engine, index, revision, transport_revision, transport);
    }

    /// Decodes pictures into the queue while this tick has time to spare.
    ///
    /// The queue is what absorbs a decode that runs long: the tick that shows the
    /// picture never waits for it. The decode is time boxed so the queue can never
    /// become the reason a tick is late, and the newest pictures win when a tick
    /// overran, so the channel catches up instead of replaying stale frames.
    #[allow(clippy::too_many_arguments)]
    fn pump(
        &mut self,
        engine: &ReplayEngine,
        context: &PlayoutContext,
        transport: &mut channel::Transport,
        store: &Arc<Mutex<Box<dyn ReplayStore>>>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        late: u32,
    ) {
        if self.depth == 0 {
            return;
        }
        // A tick that is already late catches up first: queueing more pictures for
        // later would only move the lateness on to the next tick.
        if late > 1 {
            return;
        }
        // The queue only pays off when a decode fits inside the frame with room to
        // spare. A software adapter or a CPU that is already full spends more than
        // half the frame on one decode, and running ahead there loses frames instead
        // of saving them.
        if !may_run_ahead(self.last_decode, context.period) {
            return;
        }
        let estimate = self.last_decode.max(Duration::from_micros(200));
        let deadline = Instant::now() + context.period.mul_f64(0.7);
        let mut steps = late;
        while self.ahead.len() < self.depth && Instant::now() + estimate <= deadline {
            let mut plan = None;
            for _ in 0..steps {
                if let Some(next) = transport.step(1.0) {
                    plan = Some(next);
                }
            }
            steps = 1;
            let Some(plan) = plan else {
                return;
            };
            let decoder = &mut self.decoder;
            let ready = if decoder.has_codec() {
                true
            } else if let Some(prepared) =
                engine.take_prepared(self.index, context.width, context.height, context.profile)
            {
                decoder.adopt(prepared);
                true
            } else {
                // The pipelines are still being built in the background; the
                // synchronous path keeps the channel playing until they are ready.
                !engine.is_warming()
            };
            if !ready {
                return;
            }
            let started = Instant::now();
            let decoded = decoder.decode(store, &plan, device, queue);
            let spent = started.elapsed();
            // A decode that already costs a quarter of the frame is a machine that
            // cannot afford to run ahead: queue the one picture, then stop. The
            // queue then degrades to one frame on a busy machine instead of turning
            // the spare time it does not have into lateness.
            let affordable = spent < context.period.mul_f64(0.25);
            match decoded {
                DecodedGpu::Frame(frame, substituted) => {
                    profile::decode(spent);
                    transport.substituted =
                        transport.substituted.saturating_add(u64::from(substituted));
                    transport.decode_samples = transport.decode_samples.saturating_add(1);
                    transport.decode_us_sum = transport
                        .decode_us_sum
                        .saturating_add(spent.as_micros() as u64);
                    if self.ahead.is_empty() {
                        // Say it once: whether the queue is in use is otherwise
                        // invisible, and it is the first thing to check when a
                        // playback problem is reported.
                        static ANNOUNCED: AtomicBool = AtomicBool::new(false);
                        if !ANNOUNCED.swap(true, Ordering::Relaxed) {
                            crate::diag::info(&format!(
                                "replay decode-ahead active depth={}",
                                self.depth
                            ));
                        }
                    }
                    self.ahead.push_back(AheadFrame {
                        frame,
                        transport: transport.clone(),
                    });
                    self.last_decode = spent;
                    if !affordable {
                        return;
                    }
                }
                // The synchronous path reports what went wrong; queue nothing.
                DecodedGpu::Unsupported | DecodedGpu::Failed => return,
            }
        }
        while self.ahead.len() > self.depth {
            self.ahead.pop_front();
        }
    }
}

/// One playout thread per channel.
///
/// The channel set can change while the mixer runs: adding a second Replay input
/// reconfigures the engine with one more channel. The set is watched here once per
/// hundred milliseconds, and a change stops the current workers and starts a new
/// set, so a channel that was added starts playing without a restart. Transports
/// live on the engine and survive the swap, so a channel keeps its position and
/// speed across it.
fn channel_loop(stop: Arc<AtomicBool>) {
    let engine = engine();
    let mut workers: Vec<JoinHandle<()>> = Vec::new();
    let mut running: Option<(Vec<u32>, PlayoutContext)> = None;
    let mut generation_stop = Arc::new(AtomicBool::new(false));
    while !stop.load(Ordering::Relaxed) {
        let (channels, context) = engine.playout_setup();
        let mut indices = channels
            .iter()
            .map(|(index, _, _)| *index)
            .collect::<Vec<_>>();
        indices.sort_unstable();
        // A setting or a format change lands here too: the workers hold the context
        // they were started with, so they are replaced when it differs, not only when
        // the set of channels does.
        let current = (indices, context.clone());
        if running.as_ref() != Some(&current) {
            generation_stop.store(true, Ordering::Release);
            for worker in workers.drain(..) {
                let _ = worker.join();
            }
            generation_stop = Arc::new(AtomicBool::new(false));
            running = Some(current);
            if !channels.is_empty() {
                let context = Arc::new(context);
                for (index, source_id, audio) in channels {
                    let worker_stop = Arc::clone(&generation_stop);
                    let worker_context = Arc::clone(&context);
                    let handle = thread::Builder::new()
                        .name(format!("eiviz-replay-ch{index}"))
                        .spawn(move || {
                            ChannelPlayout::new(index, source_id, audio, &worker_context).run(
                                engine,
                                &worker_context,
                                &worker_stop,
                            )
                        })
                        .expect("replay channel thread");
                    workers.push(handle);
                }
            }
        }
        // Short enough that a channel added from the host starts at once, long enough
        // that watching for one costs nothing.
        thread::sleep(Duration::from_millis(100));
    }
    generation_stop.store(true, Ordering::Release);
    for worker in workers {
        let _ = worker.join();
    }
}

/// Writes a channel's advanced transport back unless a control change or configure
/// landed while the frame was in flight.
fn write_back_transport(
    engine: &ReplayEngine,
    index: u32,
    revision: u64,
    transport_revision: u64,
    transport: channel::Transport,
) {
    let mut inner = engine.lock();
    // A configure that landed while this frame was in flight wins; the stale
    // snapshot must not overwrite its playlist.
    if inner.playlist_revision != revision {
        return;
    }
    if let Some(slot) = inner.channels.iter_mut().find(|slot| slot.index == index) {
        // A cue or speed change that landed while this frame was in flight wins
        // over the snapshot the decoder advanced.
        if slot.transport_revision == transport_revision {
            slot.transport = transport;
        }
    }
}

/// Takes a playout buffer for a channel. The buffer belongs to the upload ring, so
/// the decoder can fill it and publish it without a second full-frame copy.
fn take_playout_buffer(id: u64, width: u32, height: u32) -> Option<Vec<u8>> {
    if width == 0 || height == 0 {
        return None;
    }
    crate::with_uploads(|uploads| {
        uploads
            .take_playout_buf(id, width, height, CpuFormat::Uyvy, 2)
            .map(|(buffer, ..)| buffer)
    })
    .ok()
    .flatten()
}

fn recycle_playout_buffer(id: u64, buffer: Vec<u8>) {
    let _ = crate::with_uploads(|uploads| uploads.recycle_playout_buf(id, buffer));
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
        codec: match value.get("codec").and_then(serde_json::Value::as_str).unwrap_or("H264") {
            "H264" | "h264" => 0,
            "Hevc" | "HEVC" | "H265" | "h265" => 1,
            _ => return Err(ERR_INVALID_ARGUMENT),
        },
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
    let frames = (f64::from(AUDIO_SAMPLE_RATE) * f64::from(fps.1.max(1)) / f64::from(fps.0.max(1)))
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

/// One channel picture, described without owning it. The buffers live in the playout
/// ring, so a decoded frame is published by moving the buffer it was decoded into
/// rather than copying it a second time.
struct DecodedMeta {
    pts: i64,
    substituted: bool,
    /// True when the incoming frame decoded and is waiting in the secondary buffer.
    secondary: bool,
    /// `(kind, direction, mix)` while a transition is in progress.
    transition: Option<(u8, u8, f32)>,
}

/// True while a plan is mid-transition and both clips are on screen.
fn wants_secondary(plan: &channel::FramePlan) -> bool {
    plan.secondary.is_some() && plan.mix > 0.0 && plan.mix < 1.0
}

/// Decodes the frames a plan asks for directly into the caller's playout buffers.
///
/// A transition in progress keeps both frames separate: the GPU composites them, so
/// blending here would be work done twice — once on the CPU and again on the device
/// that samples the result.
fn render_plan_into(
    store: &Mutex<Box<dyn ReplayStore>>,
    plan: &channel::FramePlan,
    width: u32,
    height: u32,
    primary: &mut Vec<u8>,
    secondary: Option<&mut Vec<u8>>,
) -> Option<DecodedMeta> {
    let (angle, frame) = plan.primary?;
    if width == 0 || height == 0 {
        return None;
    }
    let bytes = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(2);
    if bytes == 0 {
        return None;
    }
    if primary.len() < bytes {
        primary.resize(bytes, 0);
    }
    let mut guard = store.lock().unwrap_or_else(|error| error.into_inner());
    let first = guard.decode(angle, frame, &mut primary[..bytes]).ok()?;
    let stride = first.stride as usize;
    if stride == 0 || stride.saturating_mul(first.height as usize) > primary.len() {
        return None;
    }
    let mut pts = first.pts_100ns;
    let mut substituted = first.substituted;
    let mut secondary_decoded = false;
    let mut transition = None;
    if wants_secondary(plan) {
        let second_buf = secondary?;
        if second_buf.len() < bytes {
            second_buf.resize(bytes, 0);
        }
        let (second_angle, second_frame) = plan.secondary?;
        if let Ok(second) = guard.decode(second_angle, second_frame, &mut second_buf[..bytes]) {
            substituted |= second.substituted;
            // A transition is a picture of its own, so it carries its own timestamp.
            // Without this the compositor would see the same timestamp for every
            // frame of an inserted transition and never upload the dissolve.
            pts = blend_pts(first.pts_100ns, second.pts_100ns, plan.mix);
            transition = Some((
                transition_kind_code(plan.kind),
                transition_direction_code(plan.direction),
                plan.mix as f32,
            ));
            secondary_decoded = true;
        }
    }
    drop(guard);
    Some(DecodedMeta {
        pts,
        substituted,
        secondary: secondary_decoded,
        transition,
    })
}

fn transition_kind_code(kind: channel::TransitionKind) -> u8 {
    match kind {
        channel::TransitionKind::Cut => 0,
        channel::TransitionKind::Dissolve => 1,
        channel::TransitionKind::Dip => 2,
        channel::TransitionKind::Wipe => 3,
        channel::TransitionKind::Slide => 4,
        channel::TransitionKind::Push => 5,
    }
}

fn transition_direction_code(direction: channel::TransitionDirection) -> u8 {
    match direction {
        channel::TransitionDirection::Left => 0,
        channel::TransitionDirection::Right => 1,
        channel::TransitionDirection::Up => 2,
        channel::TransitionDirection::Down => 3,
    }
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
    use eiviz_control::session::replay::{ReplayTransitionKind, ReplayTransitionTiming};
    // A channel that names no playlist plays the first one. That is the rule the host
    // already applies, and a project that has never had its channel pointed anywhere is
    // exactly what a fresh one looks like: without this the transport has no list, so a
    // cue plays a single clip and stops instead of running the playlist.
    let list = match playlist_id {
        Some(id) => config.playlists.iter().find(|list| list.id == id),
        None => config.playlists.first(),
    };
    let Some(list) = list else {
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
            return Err(eiviz_pro_api::ProError::unavailable(
                "replay write is missing",
            ));
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
            return Err(eiviz_pro_api::ProError::unavailable(
                "replay decode is missing",
            ));
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
            return Err(eiviz_pro_api::ProError::unavailable(
                "replay frame is not recorded",
            ));
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

    fn read_packet(
        &mut self,
        angle_id: u32,
        frame_index: u64,
        out: &mut Vec<u8>,
    ) -> eiviz_pro_api::ProResult<ReplayPacket> {
        let Some(read) = self.api.replay_read_packet else {
            return Err(eiviz_pro_api::ProError::unavailable(
                "this replay module cannot hand out packets",
            ));
        };
        // The module writes into the caller's buffer. Keep the buffer with the
        // caller so the channel worker can reuse it for the next frame.
        let mut buffer = std::mem::take(out);
        loop {
            let mut required = 0usize;
            let mut meta = ProReplayPacket {
                struct_size: std::mem::size_of::<ProReplayPacket>() as u32,
                ..ProReplayPacket::default()
            };
            let status = unsafe {
                read(
                    self.handle,
                    angle_id,
                    frame_index,
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut required,
                    &mut meta,
                )
            };
            if status.code == 0 {
                let used = meta.required_len.min(buffer.len());
                buffer.truncate(used);
                *out = buffer;
                return Ok(ReplayPacket {
                    actual_frame_index: meta.actual_frame_index,
                    width: meta.width,
                    height: meta.height,
                    pts_100ns: meta.pts_100ns,
                    substituted: meta.substituted != 0,
                    codec: meta.codec,
                });
            }
            if status.code == PRO_BUSY && required > buffer.len() {
                buffer.resize(required, 0);
                continue;
            }
            *out = buffer;
            return Err(eiviz_pro_api::ProError::io("replay packet is not recorded"));
        }
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

    fn export_start(
        &mut self,
        spec: eiviz_pro_api::ReplayExportSpec,
    ) -> eiviz_pro_api::ProResult<()> {
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
            codec: spec.codec,
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

pub fn open_ffi_store(
    config: &ReplayStoreConfig,
) -> eiviz_pro_api::ProResult<Box<dyn ReplayStore>> {
    let loaded = crate::pro_load::loaded_api()
        .ok_or_else(|| eiviz_pro_api::ProError::unavailable("replay module is not loaded"))?;
    let (api, module) = loaded;
    let Some(open) = api.replay_open else {
        return Err(eiviz_pro_api::ProError::unavailable(
            "replay open is missing",
        ));
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

/// Attaches a finished recording for playback only, through the loaded module.
///
/// The returned store refuses writes, so clips from an earlier run can be played and
/// exported without risking the recording on disk.
pub fn attach_ffi(
    api: eiviz_pro_api::ProApi,
    module: *mut std::ffi::c_void,
    path: &str,
) -> eiviz_pro_api::ProResult<Box<dyn eiviz_pro_api::ReplayStore>> {
    let Some(attach) = api.replay_attach else {
        return Err(eiviz_pro_api::ProError::unavailable(
            "replay attach is missing",
        ));
    };
    let mut handle = std::ptr::null_mut();
    let status = unsafe { attach(module, path.as_ptr(), path.len(), &mut handle) };
    if status.code != 0 || handle.is_null() {
        return Err(eiviz_pro_api::ProError::io("replay attach failed"));
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
    let (api, module) = crate::pro_load::loaded_api()
        .ok_or_else(|| eiviz_pro_api::ProError::unavailable("replay module is not loaded"))?;
    let Some(estimate) = api.replay_estimate else {
        return Err(eiviz_pro_api::ProError::unavailable(
            "replay estimate is missing",
        ));
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
mod recording_registry_tests {
    use super::*;
    use eiviz_control::session::replay::{
        ReplayRecordingDto, ReplayStorageDto,
    };

    fn clip(uuid: &str) -> eiviz_control::session::replay::PlaylistDto {
        use eiviz_control::session::replay::*;
        PlaylistDto {
            id: 1,
            name: "Show".into(),
            revision: 1,
            clips: vec![ClipDto {
                uid: format!("clip-{uuid}"),
                recording_uuid: uuid.into(),
                ..ClipDto::default()
            }],
            default_transition: ClipTransitionDto::default(),
        }
    }

    #[test]
    fn the_session_list_says_where_a_recording_lives() {
        let mut dto = ReplayDto::default();
        dto.storage = vec![ReplayStorageDto {
            path: "D:/first".into(),
            angle_ids: vec![1],
        }];
        dto.recordings = vec![ReplayRecordingDto {
            uuid: "rec-a".into(),
            storage: "E:/second".into(),
            ..ReplayRecordingDto::default()
        }];
        dto.playlists = vec![clip("rec-a")];
        let found = referenced_recordings(&dto);
        assert_eq!(found, vec![("rec-a".to_string(), "E:/second".to_string())]);
    }

    #[test]
    fn an_old_document_falls_back_to_the_storage_that_holds_the_id() {
        let dir = std::env::temp_dir().join(format!("eiviz-registry-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("rec-old")).unwrap();
        let mut dto = ReplayDto::default();
        dto.storage = vec![
            ReplayStorageDto {
                path: "D:/empty".into(),
                angle_ids: vec![1],
            },
            ReplayStorageDto {
                path: dir.display().to_string(),
                angle_ids: vec![2],
            },
        ];
        dto.playlists = vec![clip("rec-old")];
        let found = referenced_recordings(&dto);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "rec-old");
        assert_eq!(found[0].1, dir.display().to_string());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_recording_is_listed_once_even_when_several_clips_use_it() {
        let mut dto = ReplayDto::default();
        dto.storage = vec![ReplayStorageDto {
            path: "D:/only".into(),
            angle_ids: vec![1],
        }];
        let mut playlist = clip("rec-a");
        use eiviz_control::session::replay::ClipDto;
        let second = ClipDto {
            uid: "clip-b".into(),
            recording_uuid: "rec-a".into(),
            ..ClipDto::default()
        };
        playlist.clips.push(second);
        dto.playlists = vec![playlist];
        let found = referenced_recordings(&dto);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "rec-a");
    }
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
        assert_eq!(spec.codec, 0);
    }

    #[test]
    fn an_unknown_export_encoder_is_refused() {
        assert!(
            parse_export_spec(
                "{\"path\":\"out.mp4\",\"codec\":\"Av1\",\"segments\":[{\"angleId\":1,\"fromFrame\":0,\"toFrame\":1}]}"
            )
            .is_err()
        );
        let spec = parse_export_spec(
            "{\"path\":\"out.mp4\",\"codec\":\"Hevc\",\"segments\":[{\"angleId\":1,\"fromFrame\":0,\"toFrame\":1}]}",
        )
        .unwrap();
        assert_eq!(spec.codec, 1);
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
            layout: PixelLayout::Bgra,
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
            [
                128, 16, 128, 16, 128, 16, 128, 16, 128, 16, 128, 16, 128, 16, 128, 16
            ]
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
    fn the_playout_context_carries_the_decode_ahead_setting() {
        // The setting comes from the replay document and the workers read it from
        // here, so this is the join between the operator's checkbox and the queue.
        let engine = engine();
        let previous = engine.lock().decode_ahead;
        engine.lock().decode_ahead = false;
        let (_, context) = engine.playout_setup();
        assert!(!context.decode_ahead, "off must reach the workers");
        engine.lock().decode_ahead = true;
        let (_, context) = engine.playout_setup();
        assert!(context.decode_ahead, "on must reach the workers");
        engine.lock().decode_ahead = previous;
    }

    #[test]
    fn running_ahead_needs_a_decode_that_fits_in_half_a_frame() {
        let frame = Duration::from_nanos(16_666_667);
        // A real GPU decodes a 1080p frame in about a millisecond.
        assert!(may_run_ahead(Duration::from_millis(1), frame));
        // Half a frame is the limit.
        assert!(may_run_ahead(Duration::from_millis(8), frame));
        // A software adapter (CI) and a CPU that is already full take much longer,
        // and there the extra decode would lose frames rather than save them.
        assert!(!may_run_ahead(Duration::from_millis(20), frame));
        assert!(!may_run_ahead(Duration::from_millis(90), frame));
    }

    #[test]
    fn decode_ahead_stays_inside_the_picture_memory_budget() {
        // Two frames of slack is what the queue is for: one takes the decode off the
        // publish path, the second absorbs a tick that overran.
        assert_eq!(ahead_depth(1920, 1080, true), 2);
        assert_eq!(ahead_depth(1280, 720, true), 2);
        // A 4K BGRA frame is 33 MiB, so the queue keeps one rather than doubling that.
        assert_eq!(ahead_depth(3840, 2160, true), 1);
        // A zero-sized configuration is not a reason to queue nothing.
        assert_eq!(ahead_depth(0, 0, true), 2);
        // The operator turned it off in Replay Setup.
        assert_eq!(ahead_depth(1920, 1080, false), 0);
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
        fn new(
            level: f32,
        ) -> (
            Arc<Mutex<Box<dyn ReplayStore>>>,
            Arc<Mutex<Vec<(u32, u64)>>>,
        ) {
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

        let paused = block(
            &transport(clip(ClipAudioSource::Follow, 1.0), false),
            &store,
        );
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

        let other = block(
            &transport(clip(ClipAudioSource::Angle(3), 1.0), true),
            &store,
        );
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
        assert!(
            !asked.contains(&1),
            "the picture angle is not read: {asked:?}"
        );
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
