use crossbeam_channel::{Receiver, Sender, TrySendError};
use eiviz_media::{EncodedAccessUnit, EncodedKind, EncodedStreamConfig, MediaError, Result};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub trait EncodedSink: Send + 'static {
    fn name(&self) -> &str;
    fn connect(&mut self, config: &EncodedStreamConfig) -> Result<()>;
    fn send(&mut self, access_unit: &Arc<EncodedAccessUnit>) -> Result<()>;
    fn disconnect(&mut self) {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SinkState {
    WaitingForKeyframe,
    Connecting,
    Running,
    Backoff,
    Failed,
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SinkDiagnostics {
    pub name: String,
    pub state: SinkState,
    pub queue_depth: usize,
    pub queue_high_water: usize,
    pub enqueued: u64,
    pub sent: u64,
    pub dropped: u64,
    pub reconnects: u64,
    pub last_error: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct WorkerRecovery {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    /// Zero retries forever.
    pub max_attempts: u32,
}

struct SharedDiagnostics {
    name: String,
    state: Mutex<SinkState>,
    queue_depth: AtomicUsize,
    queue_high_water: AtomicUsize,
    enqueued: AtomicU64,
    sent: AtomicU64,
    dropped: AtomicU64,
    reconnects: AtomicU64,
    last_error: Mutex<Option<String>>,
    stopping: AtomicBool,
    recovery_required: AtomicBool,
    recovery_generation: AtomicU64,
}

impl SharedDiagnostics {
    fn snapshot(&self) -> SinkDiagnostics {
        SinkDiagnostics {
            name: self.name.clone(),
            state: *self.state.lock().expect("diagnostic state"),
            queue_depth: self.queue_depth.load(Ordering::Relaxed),
            queue_high_water: self.queue_high_water.load(Ordering::Relaxed),
            enqueued: self.enqueued.load(Ordering::Relaxed),
            sent: self.sent.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            reconnects: self.reconnects.load(Ordering::Relaxed),
            last_error: self.last_error.lock().expect("diagnostic error").clone(),
        }
    }

    fn set_state(&self, state: SinkState) {
        *self.state.lock().expect("diagnostic state") = state;
    }

    fn set_error(&self, error: impl Into<String>) {
        *self.last_error.lock().expect("diagnostic error") = Some(error.into());
    }
}

struct QueuedAccessUnit {
    generation: u64,
    access_unit: Arc<EncodedAccessUnit>,
}

struct SinkQueue {
    sender: Sender<QueuedAccessUnit>,
    diagnostics: Arc<SharedDiagnostics>,
    thread: Option<JoinHandle<()>>,
}

/// One encoded access unit is shared across independent bounded sink queues.
pub struct EncodedFanout {
    config: EncodedStreamConfig,
    sinks: Mutex<BTreeMap<String, SinkQueue>>,
}

impl EncodedFanout {
    pub fn new(config: EncodedStreamConfig) -> Self {
        Self {
            config,
            sinks: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn add_sink(
        &self,
        sink: Box<dyn EncodedSink>,
        capacity: usize,
        recovery: WorkerRecovery,
    ) -> Result<()> {
        if capacity == 0 {
            return Err(MediaError::Unsupported(
                "encoded sink queue capacity must be non-zero".into(),
            ));
        }
        let name = sink.name().to_owned();
        let mut sinks = self.sinks.lock().expect("fanout sinks");
        if sinks.contains_key(&name) {
            return Err(MediaError::Other(format!(
                "encoded sink {name} is already attached"
            )));
        }
        let (sender, receiver): (Sender<QueuedAccessUnit>, Receiver<QueuedAccessUnit>) =
            crossbeam_channel::bounded(capacity);
        let diagnostics = Arc::new(SharedDiagnostics {
            name: name.clone(),
            state: Mutex::new(SinkState::WaitingForKeyframe),
            queue_depth: AtomicUsize::new(0),
            queue_high_water: AtomicUsize::new(0),
            enqueued: AtomicU64::new(0),
            sent: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            reconnects: AtomicU64::new(0),
            last_error: Mutex::new(None),
            stopping: AtomicBool::new(false),
            recovery_required: AtomicBool::new(true),
            recovery_generation: AtomicU64::new(0),
        });
        let worker_diagnostics = diagnostics.clone();
        let config = self.config.clone();
        let thread_name = format!("encoded-sink-{name}");
        let thread = std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || run_worker(sink, receiver, config, recovery, worker_diagnostics))
            .map_err(|error| MediaError::Other(error.to_string()))?;
        sinks.insert(
            name,
            SinkQueue {
                sender,
                diagnostics,
                thread: Some(thread),
            },
        );
        Ok(())
    }

    pub fn publish(&self, access_unit: Arc<EncodedAccessUnit>) {
        let sinks = self.sinks.lock().expect("fanout sinks");
        for queue in sinks.values() {
            let generation = queue
                .diagnostics
                .recovery_generation
                .load(Ordering::Acquire);
            match queue.sender.try_send(QueuedAccessUnit {
                generation,
                access_unit: access_unit.clone(),
            }) {
                Ok(()) => {
                    queue.diagnostics.enqueued.fetch_add(1, Ordering::Relaxed);
                    let depth = queue.sender.len();
                    queue
                        .diagnostics
                        .queue_depth
                        .store(depth, Ordering::Relaxed);
                    queue
                        .diagnostics
                        .queue_high_water
                        .fetch_max(depth, Ordering::Relaxed);
                    tracing::debug!(
                        sink = %queue.diagnostics.name,
                        queue_depth = depth,
                        queue_high_water = queue
                            .diagnostics
                            .queue_high_water
                            .load(Ordering::Relaxed),
                        "distribution access unit queued"
                    );
                }
                Err(TrySendError::Full(_)) => {
                    queue.diagnostics.dropped.fetch_add(1, Ordering::Relaxed);
                    queue
                        .diagnostics
                        .recovery_generation
                        .fetch_add(1, Ordering::AcqRel);
                    queue
                        .diagnostics
                        .recovery_required
                        .store(true, Ordering::Release);
                    tracing::warn!(
                        sink = %queue.diagnostics.name,
                        queue_depth = queue.sender.len(),
                        dropped = queue.diagnostics.dropped.load(Ordering::Relaxed),
                        "distribution queue full"
                    );
                }
                Err(TrySendError::Disconnected(_)) => {
                    queue.diagnostics.dropped.fetch_add(1, Ordering::Relaxed);
                    queue.diagnostics.set_state(SinkState::Failed);
                    queue.diagnostics.set_error("sink worker stopped");
                    tracing::error!(
                        sink = %queue.diagnostics.name,
                        "distribution worker disconnected"
                    );
                }
            }
        }
    }

    pub fn diagnostics(&self) -> Vec<SinkDiagnostics> {
        self.sinks
            .lock()
            .expect("fanout sinks")
            .values()
            .map(|queue| queue.diagnostics.snapshot())
            .collect()
    }

    /// True while at least one sink is waiting to establish/re-establish its
    /// stream at a clean random-access point.
    pub fn keyframe_required(&self) -> bool {
        self.sinks
            .lock()
            .expect("fanout sinks")
            .values()
            .any(|queue| queue.diagnostics.recovery_required.load(Ordering::Acquire))
    }

    pub fn sink_count(&self) -> usize {
        self.sinks.lock().expect("fanout sinks").len()
    }

    pub fn remove_sink(&self, name: &str) {
        let queue = self.sinks.lock().expect("fanout sinks").remove(name);
        if let Some(mut queue) = queue {
            queue.diagnostics.stopping.store(true, Ordering::Release);
            drop(queue.sender);
            if let Some(thread) = queue.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

impl Drop for EncodedFanout {
    fn drop(&mut self) {
        let queues = std::mem::take(self.sinks.get_mut().expect("fanout sinks"));
        for (_, mut queue) in queues {
            queue.diagnostics.stopping.store(true, Ordering::Release);
            drop(queue.sender);
            if let Some(thread) = queue.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

fn run_worker(
    mut sink: Box<dyn EncodedSink>,
    receiver: Receiver<QueuedAccessUnit>,
    config: EncodedStreamConfig,
    recovery: WorkerRecovery,
    diagnostics: Arc<SharedDiagnostics>,
) {
    let mut connected = false;
    while let Ok(queued) = receiver.recv() {
        if diagnostics.stopping.load(Ordering::Acquire) {
            break;
        }
        let access_unit = queued.access_unit;
        diagnostics
            .queue_depth
            .store(receiver.len(), Ordering::Relaxed);
        let recovering = diagnostics.recovery_required.load(Ordering::Acquire);
        let required_generation = diagnostics.recovery_generation.load(Ordering::Acquire);
        if recovering
            && (queued.generation < required_generation || !is_video_keyframe(&access_unit))
        {
            diagnostics.dropped.fetch_add(1, Ordering::Relaxed);
            diagnostics.set_state(SinkState::WaitingForKeyframe);
            continue;
        }
        if recovering {
            if connected {
                sink.disconnect();
                connected = false;
                diagnostics.reconnects.fetch_add(1, Ordering::Relaxed);
            }
            if !connect_with_backoff(&mut *sink, &config, recovery, &diagnostics) {
                break;
            }
            connected = true;
            diagnostics
                .recovery_required
                .store(false, Ordering::Release);
        }
        if let Err(error) = sink.send(&access_unit) {
            tracing::warn!(
                sink = %diagnostics.name,
                error = %error,
                "distribution send failed"
            );
            diagnostics.set_error(error.to_string());
            diagnostics.set_state(SinkState::WaitingForKeyframe);
            diagnostics.reconnects.fetch_add(1, Ordering::Relaxed);
            diagnostics
                .recovery_generation
                .fetch_add(1, Ordering::AcqRel);
            diagnostics.recovery_required.store(true, Ordering::Release);
            sink.disconnect();
            connected = false;
            diagnostics.dropped.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        diagnostics.sent.fetch_add(1, Ordering::Relaxed);
        diagnostics.set_state(SinkState::Running);
    }
    if connected {
        sink.disconnect();
    }
    if diagnostics.snapshot().state != SinkState::Failed {
        diagnostics.set_state(SinkState::Stopped);
    }
}

fn connect_with_backoff(
    sink: &mut dyn EncodedSink,
    config: &EncodedStreamConfig,
    recovery: WorkerRecovery,
    diagnostics: &SharedDiagnostics,
) -> bool {
    let mut attempt = 0u32;
    let mut delay = recovery.initial_delay.max(Duration::from_millis(1));
    loop {
        if diagnostics.stopping.load(Ordering::Acquire) {
            return false;
        }
        diagnostics.set_state(SinkState::Connecting);
        match sink.connect(config) {
            Ok(()) => return true,
            Err(error) => {
                attempt = attempt.saturating_add(1);
                diagnostics.set_error(error.to_string());
                tracing::warn!(
                    sink = %diagnostics.name,
                    attempt,
                    error = %error,
                    "distribution connect failed"
                );
                if recovery.max_attempts != 0 && attempt >= recovery.max_attempts {
                    diagnostics.set_state(SinkState::Failed);
                    return false;
                }
                diagnostics.set_state(SinkState::Backoff);
                if !interruptible_sleep(delay, diagnostics) {
                    return false;
                }
                delay = delay.saturating_mul(2).min(recovery.max_delay.max(delay));
                diagnostics.reconnects.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn interruptible_sleep(delay: Duration, diagnostics: &SharedDiagnostics) -> bool {
    let started = std::time::Instant::now();
    loop {
        if diagnostics.stopping.load(Ordering::Acquire) {
            return false;
        }
        let remaining = delay.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return true;
        }
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

fn is_video_keyframe(access_unit: &EncodedAccessUnit) -> bool {
    access_unit.kind == EncodedKind::Avc && access_unit.keyframe
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_time::MediaTime;

    struct PointerSink {
        pointers: Arc<Mutex<Vec<usize>>>,
    }

    struct ReconnectingSink {
        connects: Arc<AtomicU64>,
        successful_ids: Arc<Mutex<Vec<u8>>>,
        fail_first_send: bool,
    }

    impl EncodedSink for ReconnectingSink {
        fn name(&self) -> &str {
            "reconnecting"
        }

        fn connect(&mut self, _config: &EncodedStreamConfig) -> Result<()> {
            self.connects.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn send(&mut self, access_unit: &Arc<EncodedAccessUnit>) -> Result<()> {
            if self.fail_first_send {
                self.fail_first_send = false;
                return Err(MediaError::Disconnected("mock loss".into()));
            }
            self.successful_ids
                .lock()
                .unwrap()
                .push(*access_unit.bytes.last().unwrap());
            Ok(())
        }
    }

    impl EncodedSink for PointerSink {
        fn name(&self) -> &str {
            "pointer"
        }

        fn connect(&mut self, _config: &EncodedStreamConfig) -> Result<()> {
            Ok(())
        }

        fn send(&mut self, access_unit: &Arc<EncodedAccessUnit>) -> Result<()> {
            self.pointers
                .lock()
                .unwrap()
                .push(Arc::as_ptr(access_unit) as usize);
            Ok(())
        }
    }

    fn config() -> EncodedStreamConfig {
        EncodedStreamConfig {
            h264_sps: vec![0x67, 66, 0, 31].into(),
            h264_pps: vec![0x68, 0].into(),
            aac_audio_specific_config: vec![0x11, 0x90].into(),
            video_width: 1920,
            video_height: 1080,
            video_timescale: 60_000,
            video_sample_duration: 1001,
            audio_sample_rate: 48_000,
            audio_channels: 2,
        }
    }

    #[test]
    fn fanout_keeps_shared_access_unit_allocation() {
        let pointers = Arc::new(Mutex::new(Vec::new()));
        let fanout = EncodedFanout::new(config());
        fanout
            .add_sink(
                Box::new(PointerSink {
                    pointers: pointers.clone(),
                }),
                4,
                WorkerRecovery {
                    initial_delay: Duration::from_millis(1),
                    max_delay: Duration::from_millis(2),
                    max_attempts: 1,
                },
            )
            .unwrap();
        let access_unit = Arc::new(EncodedAccessUnit {
            pts: MediaTime::ZERO,
            dts: Some(MediaTime::ZERO),
            keyframe: true,
            bytes: vec![0, 0, 0, 1, 0x65].into(),
            kind: EncodedKind::Avc,
        });
        let expected = Arc::as_ptr(&access_unit) as usize;
        fanout.publish(access_unit);
        for _ in 0..50 {
            if !pointers.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(pointers.lock().unwrap().as_slice(), &[expected]);
    }

    #[test]
    fn reconnect_discards_until_next_keyframe() {
        let connects = Arc::new(AtomicU64::new(0));
        let successful_ids = Arc::new(Mutex::new(Vec::new()));
        let fanout = EncodedFanout::new(config());
        fanout
            .add_sink(
                Box::new(ReconnectingSink {
                    connects: connects.clone(),
                    successful_ids: successful_ids.clone(),
                    fail_first_send: true,
                }),
                8,
                WorkerRecovery {
                    initial_delay: Duration::from_millis(1),
                    max_delay: Duration::from_millis(2),
                    max_attempts: 1,
                },
            )
            .unwrap();
        let publish = |kind, keyframe, id| {
            fanout.publish(Arc::new(EncodedAccessUnit {
                pts: MediaTime::ZERO,
                dts: Some(MediaTime::ZERO),
                keyframe,
                bytes: vec![0, 0, 0, 1, id].into(),
                kind,
            }));
        };
        publish(EncodedKind::Avc, true, 1);
        for _ in 0..100 {
            if fanout.diagnostics()[0].reconnects >= 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        for (kind, keyframe, id) in [
            (EncodedKind::Aac, false, 2),
            (EncodedKind::Avc, false, 3),
            (EncodedKind::Avc, true, 4),
        ] {
            publish(kind, keyframe, id);
        }
        for _ in 0..100 {
            if successful_ids.lock().unwrap().as_slice() == [4] {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(successful_ids.lock().unwrap().as_slice(), &[4]);
        assert_eq!(connects.load(Ordering::Relaxed), 2);
        let diagnostics = fanout.diagnostics();
        assert!(diagnostics[0].dropped >= 3);
        assert!(diagnostics[0].reconnects >= 1);
    }
}
