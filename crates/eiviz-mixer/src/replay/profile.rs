//! Opt-in stage timings for Replay.
//!
//! Replay used to be measured only by its end result: frames recorded and frames
//! dropped. That says a recording is short but not which stage lost it. These
//! counters are off by default so the hot path stays a relaxed atomic add, and a
//! single line every two seconds names the stage that is behind.
//!
//! The operator turns them on with "パフォーマンスログ" in Replay Setup, which writes
//! the `profiling` field of the replay document; there is no environment variable.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Whether the counters run at all. The operator turns this on from Replay Setup;
/// it starts off, and a session that was saved with it on keeps it on.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// The operator's choice from Replay Setup. Cheap enough to call on every configure.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn is_enabled() -> bool {
    enabled()
}

/// A count, a total and a maximum, in microseconds.
pub struct Counter {
    total_us: AtomicU64,
    max_us: AtomicU64,
    count: AtomicU64,
}

impl Counter {
    const fn new() -> Self {
        Self {
            total_us: AtomicU64::new(0),
            max_us: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    pub fn add(&self, elapsed: Duration) {
        if !enabled() {
            return;
        }
        let us = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
        self.total_us.fetch_add(us, Ordering::Relaxed);
        self.max_us.fetch_max(us, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    fn take(&self) -> Option<(f64, f64, u64)> {
        let count = self.count.swap(0, Ordering::Relaxed);
        if count == 0 {
            return None;
        }
        let total = self.total_us.swap(0, Ordering::Relaxed);
        let max = self.max_us.swap(0, Ordering::Relaxed);
        Some((
            total as f64 / count as f64 / 1000.0,
            max as f64 / 1000.0,
            count,
        ))
    }
}

pub static DECODE: Counter = Counter::new();
pub static PUBLISH: Counter = Counter::new();
pub static READBACK: Counter = Counter::new();

static QUEUED_SHOWS: AtomicU64 = AtomicU64::new(0);
static LATE_TICKS: AtomicU64 = AtomicU64::new(0);
static LATE_FRAMES: AtomicU64 = AtomicU64::new(0);

pub fn decode(elapsed: Duration) {
    DECODE.add(elapsed);
}

pub fn publish(elapsed: Duration) {
    PUBLISH.add(elapsed);
}

/// Time spent copying a mapped GPU readback into a CPU frame. This is the stage
/// CPU encoding pays for every angle, every frame.
pub fn readback(elapsed: Duration) {
    READBACK.add(elapsed);
}

/// Counts playout ticks that showed a picture decoded during an earlier tick.
pub fn queued() {
    if enabled() {
        QUEUED_SHOWS.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn late(frames: u32) {
    if !enabled() {
        return;
    }
    LATE_TICKS.fetch_add(1, Ordering::Relaxed);
    LATE_FRAMES.fetch_add(u64::from(frames), Ordering::Relaxed);
}

/// Writes one line every two seconds. Called from the playout loop; a no-op unless
/// profiling was asked for.
pub fn log_if_due() {
    if !enabled() {
        return;
    }
    static LAST: OnceLock<Mutex<Instant>> = OnceLock::new();
    let last = LAST.get_or_init(|| Mutex::new(Instant::now()));
    let Ok(mut guard) = last.lock() else {
        return;
    };
    if guard.elapsed() < Duration::from_secs(2) {
        return;
    }
    *guard = Instant::now();
    let decode = DECODE.take();
    let publish = PUBLISH.take();
    let readback = READBACK.take();
    let late_ticks = LATE_TICKS.swap(0, Ordering::Relaxed);
    let late_frames = LATE_FRAMES.swap(0, Ordering::Relaxed);
    let queued = QUEUED_SHOWS.swap(0, Ordering::Relaxed);
    crate::diag::info(&format!(
        "replay profile decode={} publish={} readback={} queued={queued} late_ticks={late_ticks} late_frames={late_frames}",
        timing_text(decode),
        timing_text(publish),
        timing_text(readback),
    ));
}

fn timing_text(value: Option<(f64, f64, u64)>) -> String {
    match value {
        Some((avg, max, count)) => format!("{avg:.2}/{max:.2}ms n={count}"),
        None => "-".to_string(),
    }
}
