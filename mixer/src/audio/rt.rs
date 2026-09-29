//! Real-time helpers for audio device callbacks.
//!
//! Device callbacks (WASAPI, ASIO, Core Audio via cpal) run on driver-owned threads with
//! hard deadlines. They must not block on locks the mixer thread may hold, must not
//! allocate in steady state and must not unwind into the driver. This module provides the
//! small set of primitives that make that possible without extra dependencies.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Single-producer single-consumer ring of `f32` samples.
///
/// One thread may call the producer methods (`push_slice`) and one thread the consumer
/// methods (`pop_slice`, `discard`); `len` is safe from either. Storage is atomic so no
/// `unsafe` is needed. When full the producer drops the samples that do not fit.
pub struct SpscF32 {
    buf: Box<[AtomicU32]>,
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl SpscF32 {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(2);
        Self {
            buf: (0..capacity).map(|_| AtomicU32::new(0)).collect(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    pub fn len(&self) -> usize {
        let tail = self.tail.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Acquire);
        tail.wrapping_sub(head).min(self.buf.len())
    }

    /// Producer side. Returns how many samples were stored.
    pub fn push_slice(&self, samples: &[f32]) -> usize {
        let cap = self.buf.len();
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        let free = cap - tail.wrapping_sub(head).min(cap);
        let n = samples.len().min(free);
        for (i, sample) in samples[..n].iter().enumerate() {
            self.buf[tail.wrapping_add(i) % cap].store(sample.to_bits(), Ordering::Relaxed);
        }
        self.tail.store(tail.wrapping_add(n), Ordering::Release);
        n
    }

    /// Consumer side. Returns how many samples were written to `out`.
    pub fn pop_slice(&self, out: &mut [f32]) -> usize {
        let cap = self.buf.len();
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let n = out.len().min(tail.wrapping_sub(head).min(cap));
        for (i, slot) in out[..n].iter_mut().enumerate() {
            *slot = f32::from_bits(self.buf[head.wrapping_add(i) % cap].load(Ordering::Relaxed));
        }
        self.head.store(head.wrapping_add(n), Ordering::Release);
        n
    }

    /// Consumer side. Drops up to `n` of the oldest samples.
    pub fn discard(&self, n: usize) -> usize {
        let cap = self.buf.len();
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let n = n.min(tail.wrapping_sub(head).min(cap));
        self.head.store(head.wrapping_add(n), Ordering::Release);
        n
    }
}

/// A value the control thread replaces and a real-time callback reads.
///
/// The callback never waits: it checks a version counter and only when it changed does it
/// `try_lock` the value to refresh its private copy. If the control thread happens to hold
/// the lock the callback keeps using its previous copy and retries on the next period.
pub struct Published<T> {
    value: Mutex<T>,
    version: AtomicU64,
}

impl<T: Clone> Published<T> {
    pub fn new(value: T) -> Arc<Self> {
        Arc::new(Self {
            value: Mutex::new(value),
            version: AtomicU64::new(1),
        })
    }

    pub fn set(&self, value: T) {
        let mut guard = self.value.lock().unwrap_or_else(|e| e.into_inner());
        *guard = value;
        self.version.fetch_add(1, Ordering::Release);
    }
}

pub struct PublishedReader<T> {
    local: T,
    seen: u64,
}

impl<T: Clone + Default> PublishedReader<T> {
    pub fn new() -> Self {
        Self {
            local: T::default(),
            seen: 0,
        }
    }

    /// Returns the current local copy and whether it was refreshed by this call.
    pub fn get(&mut self, shared: &Published<T>) -> (&T, bool) {
        let mut refreshed = false;
        if shared.version.load(Ordering::Acquire) != self.seen
            && let Ok(guard) = shared.value.try_lock()
        {
            self.local = guard.clone();
            self.seen = shared.version.load(Ordering::Acquire);
            refreshed = true;
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
    fn published_reader_keeps_copy_while_locked() {
        let shared = Published::new(vec![1]);
        let mut reader = PublishedReader::<Vec<i32>>::new();
        reader.get(&shared);
        let guard = shared.value.lock().unwrap();
        shared.version.fetch_add(1, Ordering::Release);
        let (value, refreshed) = reader.get(&shared);
        assert!(!refreshed);
        assert_eq!(value, &vec![1]);
        drop(guard);
    }

    #[test]
    fn guard_callback_contains_panic() {
        let flag = AtomicBool::new(false);
        assert!(!guard_callback("test", &flag, || panic!("boom")));
        assert!(flag.load(Ordering::Relaxed));
        assert!(guard_callback("test", &flag, || {}));
    }
}
