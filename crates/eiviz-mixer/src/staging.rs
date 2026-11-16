//! GPU completion tracking for CPU-visible staging memory.
//!
//! Host-visible upload buffers are written by the CPU and read later by a copy that only runs
//! when the GPU reaches it. Rewriting such a buffer before that copy has executed corrupts the
//! frame, so every slot carries a [`Fence`] that records the submission that reads it.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::Duration;

/// How long a writer may block for the GPU to release a staging slot before giving up.
const SLOT_WAIT: Duration = Duration::from_secs(2);

/// Marker prefix for the recoverable "every slot is still in use" error.
pub(crate) const EXHAUSTED: &str = "staging ring exhausted";

pub(crate) fn is_exhausted(error: &str) -> bool {
    error.starts_with(EXHAUSTED)
}

#[derive(Default)]
pub(crate) struct Fence(Option<wgpu::SubmissionIndex>);

impl Fence {
    pub(crate) fn arm(&mut self, index: wgpu::SubmissionIndex) {
        self.0 = Some(index);
    }

    /// True when no submission that reads the slot is still queued or running.
    pub(crate) fn is_idle(&self, device: &wgpu::Device) -> bool {
        let Some(index) = self.0.clone() else {
            return true;
        };
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: Some(Duration::ZERO),
            })
            .is_ok()
    }

    pub(crate) fn wait(&mut self, device: &wgpu::Device) -> Result<(), String> {
        let Some(index) = self.0.clone() else {
            return Ok(());
        };
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: Some(SLOT_WAIT),
            })
            .map_err(|error| format!("waiting for GPU to release staging slot: {error}"))?;
        self.0 = None;
        Ok(())
    }
}

pub(crate) struct PoolSlot<T> {
    pub(crate) inner: T,
    fence: Fence,
    pending: bool,
}

struct SlotGroup<T> {
    slots: Vec<PoolSlot<T>>,
    last_used: u64,
}

/// Staging slots grouped by a size key. One group serves every source that uploads frames of
/// that size; slots are handed out only once the GPU has finished reading them.
pub(crate) struct StagingPool<K: Eq + Hash + Copy, T> {
    groups: HashMap<K, SlotGroup<T>>,
    max_slots: usize,
    epoch: u64,
}

/// Groups that were not used for this many flushes are released.
const GROUP_IDLE_EPOCHS: u64 = 600;

impl<K: Eq + Hash + Copy, T> StagingPool<K, T> {
    pub(crate) fn new(max_slots: usize) -> Self {
        Self {
            groups: HashMap::new(),
            max_slots: max_slots.max(2),
            epoch: 0,
        }
    }

    /// Returns the index of a slot in `key`'s group that is safe to write. `flush` must submit
    /// whatever copies are still recorded and return their submission index; it is only called
    /// when every slot is busy.
    pub(crate) fn acquire(
        &mut self,
        device: &wgpu::Device,
        key: K,
        create: impl FnOnce() -> Result<T, String>,
        flush: impl FnOnce() -> Option<wgpu::SubmissionIndex>,
    ) -> Result<usize, String> {
        let epoch = self.epoch;
        let group = self.groups.entry(key).or_insert_with(|| SlotGroup {
            slots: Vec::new(),
            last_used: epoch,
        });
        group.last_used = epoch;
        if let Some(index) = group
            .slots
            .iter()
            .position(|slot| !slot.pending && slot.fence.is_idle(device))
        {
            return Ok(index);
        }
        if group.slots.len() < self.max_slots {
            group.slots.push(PoolSlot {
                inner: create()?,
                fence: Fence::default(),
                pending: false,
            });
            return Ok(group.slots.len() - 1);
        }
        if let Some(index) = flush() {
            for other in self.groups.values_mut() {
                for slot in other.slots.iter_mut().filter(|slot| slot.pending) {
                    slot.fence.arm(index.clone());
                    slot.pending = false;
                }
            }
        }
        let Some(group) = self.groups.get_mut(&key) else {
            return Err(EXHAUSTED.into());
        };
        let Some(index) = group.slots.iter().position(|slot| !slot.pending) else {
            return Err(format!("{EXHAUSTED}: every slot has unsubmitted copies"));
        };
        group.slots[index].fence.wait(device)?;
        Ok(index)
    }

    pub(crate) fn slot(&self, key: K, index: usize) -> Option<&T> {
        self.groups
            .get(&key)
            .and_then(|group| group.slots.get(index))
            .map(|slot| &slot.inner)
    }

    /// The slot's copy has been recorded but not submitted yet.
    pub(crate) fn mark_pending(&mut self, key: K, index: usize) {
        if let Some(slot) = self
            .groups
            .get_mut(&key)
            .and_then(|group| group.slots.get_mut(index))
        {
            slot.pending = true;
        }
    }

    /// Records the submission that carries every copy recorded since the last flush.
    pub(crate) fn arm_pending(&mut self, index: &wgpu::SubmissionIndex) {
        for group in self.groups.values_mut() {
            for slot in group.slots.iter_mut().filter(|slot| slot.pending) {
                slot.fence.arm(index.clone());
                slot.pending = false;
            }
        }
    }

    /// Call once per flush to release groups whose frame size is no longer in use.
    pub(crate) fn end_frame(&mut self) {
        self.epoch += 1;
        let epoch = self.epoch;
        self.groups.retain(|_, group| {
            epoch.saturating_sub(group.last_used) < GROUP_IDLE_EPOCHS
                || group.slots.iter().any(|slot| slot.pending)
        });
    }

    pub(crate) fn clear(&mut self) {
        self.groups.clear();
    }

    pub(crate) fn for_each(&self, mut visit: impl FnMut(&T)) {
        for group in self.groups.values() {
            for slot in &group.slots {
                visit(&slot.inner);
            }
        }
    }
}
