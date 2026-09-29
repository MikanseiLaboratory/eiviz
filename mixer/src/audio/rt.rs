//! Real-time helpers for audio device callbacks.
//!
//! Device callbacks (WASAPI, ASIO, Core Audio via cpal) run on driver-owned threads with
//! hard deadlines. They must not block on locks the mixer thread may hold, must not
//! allocate in steady state and must not unwind into the driver. This module wraps the
//! `rtrb` ring buffer and `arc-swap` so the rest of the audio code can share them through
//! `Arc` handles without ever waiting.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

use arc_swap::{ArcSwap, Guard};
use rtrb::{Consumer, Producer, RingBuffer};

/// Never blocks. A poisoned lock (a panic that a callback guard already contained) is
/// still usable because the guarded ring halves hold no invariant a panic can break.
fn try_grab<T>(mutex: &Mutex<T>) -> Option<MutexGuard<'_, T>> {
    match mutex.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

/// Single-producer single-consumer ring of `f32` samples backed by `rtrb`.
///
/// `rtrb` hands out exclusive producer/consumer halves; they are kept behind per-side
/// `try_lock`s so the ring can be shared through `Arc`. Each side has exactly one user in
/// practice, so the locks are never contended; if a second user ever appears (for example
/// two streams briefly overlapping while a device is reopened) it skips that period
/// instead of corrupting the queue or blocking. When full the producer drops the samples
/// that do not fit.
pub struct SpscF32 {
    producer: Mutex<Producer<f32>>,
    consumer: Mutex<Consumer<f32>>,
    capacity: usize,
    /// Signed because the consumer may account a pop before the producer accounts the
    /// matching push; `len` clamps the transient negative value.
    queued: AtomicIsize,
}

impl SpscF32 {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(2);
        let (producer, consumer) = RingBuffer::new(capacity);
        Self {
            producer: Mutex::new(producer),
            consumer: Mutex::new(consumer),
            capacity,
            queued: AtomicIsize::new(0),
        }
    }

    pub fn len(&self) -> usize {
        (self.queued.load(Ordering::Acquire).max(0) as usize).min(self.capacity)
    }

    /// Producer side. Returns how many samples were stored.
    pub fn push_slice(&self, samples: &[f32]) -> usize {
        let Some(mut producer) = try_grab(&self.producer) else {
            return 0;
        };
        let (pushed, _) = producer.push_partial_slice(samples);
        let n = pushed.len();
        self.queued.fetch_add(n as isize, Ordering::Release);
        n
    }

    /// Consumer side. Returns how many samples were written to `out`.
    pub fn pop_slice(&self, out: &mut [f32]) -> usize {
        let Some(mut consumer) = try_grab(&self.consumer) else {
            return 0;
        };
        let (popped, _) = consumer.pop_partial_slice(out);
        let n = popped.len();
        self.queued.fetch_sub(n as isize, Ordering::Release);
        n
    }

    /// Consumer side. Drops up to `n` of the oldest samples.
    pub fn discard(&self, n: usize) -> usize {
        let Some(mut consumer) = try_grab(&self.consumer) else {
            return 0;
        };
        let n = n.min(consumer.slots());
        if n == 0 {
            return 0;
        }
        if let Ok(chunk) = consumer.read_chunk(n) {
            chunk.commit_all();
        }
        self.queued.fetch_sub(n as isize, Ordering::Release);
        n
    }
}

/// A value the control thread replaces and a real-time callback reads.
///
/// Publishing swaps an `Arc` atomically (`arc-swap`); a reader compares pointers and only
/// takes a new reference when the value changed, so the callback never waits or clones the
/// payload.
pub struct Published<T> {
    value: ArcSwap<T>,
}

impl<T> Published<T> {
    pub fn new(value: T) -> Arc<Self> {
        Arc::new(Self {
            value: ArcSwap::from_pointee(value),
        })
    }

    pub fn set(&self, value: T) {
        self.value.store(Arc::new(value));
    }
}

pub struct PublishedReader<T> {
    local: Arc<T>,
}

impl<T: Default> PublishedReader<T> {
    pub fn new() -> Self {
        Self {
            local: Arc::new(T::default()),
        }
    }

    /// Returns the current value and whether it was refreshed by this call.
    pub fn get(&mut self, shared: &Published<T>) -> (&T, bool) {
        let current = shared.value.load();
        let refreshed = !Arc::ptr_eq(&current, &self.local);
        if refreshed {
            self.local = Guard::into_inner(current);
        }
        (&self.local, refreshed)
    }
}

/// Runs `f` and swallows a panic so it never unwinds into a driver thread. Returns
/// `false` when `f` panicked; the first panic is logged.
pub fn guard_callback(name: &str, panicked: &AtomicBool, f: impl FnOnce()) -> bool {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(()) => true,
        Err(_) => {
            if !panicked.swap(true, Ordering::Relaxed) {
                crate::diag::error(&format!("audio callback '{name}' panicked; output muted"));
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spsc_round_trip_wraps() {
        let ring = SpscF32::new(8);
        let mut next = 0.0f32;
        let mut expect = 0.0f32;
        for _ in 0..50 {
            let block = [next, next + 1.0, next + 2.0];
            assert_eq!(ring.push_slice(&block), 3);
            next += 3.0;
            let mut out = [0.0f32; 3];
            assert_eq!(ring.pop_slice(&mut out), 3);
            for value in out {
                assert_eq!(value, expect);
                expect += 1.0;
            }
        }
        assert_eq!(ring.len(), 0);
    }

    #[test]
    fn spsc_full_drops_new_samples() {
        let ring = SpscF32::new(4);
        assert_eq!(ring.push_slice(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]), 4);
        assert_eq!(ring.len(), 4);
        let mut out = [0.0f32; 6];
        assert_eq!(ring.pop_slice(&mut out), 4);
        assert_eq!(&out[..4], &[1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn spsc_discard_drops_oldest() {
        let ring = SpscF32::new(8);
        ring.push_slice(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(ring.discard(2), 2);
        let mut out = [0.0f32; 2];
        ring.pop_slice(&mut out);
        assert_eq!(out, [3.0, 4.0]);
    }

    #[test]
    fn spsc_threads_keep_order() {
        let ring = Arc::new(SpscF32::new(64));
        let producer = Arc::clone(&ring);
        let total = 20_000usize;
        let handle = std::thread::spawn(move || {
            let mut sent = 0usize;
            while sent < total {
                let n = producer.push_slice(&[sent as f32]);
                sent += n;
                if n == 0 {
                    std::thread::yield_now();
                }
            }
        });
        let mut got = 0usize;
        let mut out = [0.0f32; 7];
        while got < total {
            let n = ring.pop_slice(&mut out);
            for value in &out[..n] {
                assert_eq!(*value, got as f32);
                got += 1;
            }
            if n == 0 {
                std::thread::yield_now();
            }
        }
        handle.join().unwrap();
    }

    #[test]
    fn published_reader_refreshes_after_set() {
        let shared = Published::new(vec![1, 2]);
        let mut reader = PublishedReader::<Vec<i32>>::new();
        let (value, refreshed) = reader.get(&shared);
        assert!(refreshed);
        assert_eq!(value, &vec![1, 2]);
        assert!(!reader.get(&shared).1);
        shared.set(vec![3]);
        let (value, refreshed) = reader.get(&shared);
        assert!(refreshed);
        assert_eq!(value, &vec![3]);
    }

    #[test]
    fn published_reader_skips_to_latest_value() {
        let shared = Published::new(vec![1]);
        let mut reader = PublishedReader::<Vec<i32>>::new();
        reader.get(&shared);
        shared.set(vec![2]);
        shared.set(vec![3]);
        let (value, refreshed) = reader.get(&shared);
        assert!(refreshed);
        assert_eq!(value, &vec![3]);
    }

    #[test]
    fn spsc_len_tracks_discard_and_pop() {
        let ring = SpscF32::new(16);
        ring.push_slice(&[0.0; 10]);
        assert_eq!(ring.len(), 10);
        assert_eq!(ring.discard(4), 4);
        assert_eq!(ring.len(), 6);
        let mut out = [0.0f32; 20];
        assert_eq!(ring.pop_slice(&mut out), 6);
        assert_eq!(ring.len(), 0);
        assert_eq!(ring.discard(3), 0);
    }

    #[test]
    fn guard_callback_contains_panic() {
        let flag = AtomicBool::new(false);
        assert!(!guard_callback("test", &flag, || panic!("boom")));
        assert!(flag.load(Ordering::Relaxed));
        assert!(guard_callback("test", &flag, || {}));
    }
}
