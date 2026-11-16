use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) enum MixerSlot {
    Empty,
    Initializing,
    Running(Mixer),
    Stopping,
}

static MIXER: OnceLock<Mutex<MixerSlot>> = OnceLock::new();

/// Incremented for every successful mixer_create. GPU resources belong to one
/// wgpu::Instance; a cached device from an earlier generation must never be used
/// after New/Open recreates the mixer.
static GENERATION: AtomicU64 = AtomicU64::new(1);

pub(crate) fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

pub(crate) fn mixer_slot() -> &'static Mutex<MixerSlot> {
    MIXER.get_or_init(|| Mutex::new(MixerSlot::Empty))
}

pub(crate) fn with_mixer<T>(f: impl FnOnce(&mut Mixer) -> T) -> Result<T, i32> {
    let start = Instant::now();
    let mut slot = mixer_slot().lock_or_recover();
    let result = match &mut *slot {
        MixerSlot::Running(mixer) => Ok(f(mixer)),
        _ => Err(ERR_NOT_CREATED),
    };
    crate::diag::lock_held("mixer_slot", start, result)
}

pub(crate) fn reserve_mixer_create() -> i32 {
    let mut slot = mixer_slot().lock_or_recover();
    match *slot {
        MixerSlot::Empty => {
            *slot = MixerSlot::Initializing;
            OK
        }
        _ => ERR_ALREADY_CREATED,
    }
}

pub(crate) fn commit_mixer_create(mixer: Mixer) -> i32 {
    let mut slot = mixer_slot().lock_or_recover();
    match *slot {
        MixerSlot::Initializing => {
            *slot = MixerSlot::Running(mixer);
            GENERATION.fetch_add(1, Ordering::AcqRel);
            OK
        }
        _ => ERR_ALREADY_CREATED,
    }
}

pub(crate) fn abort_mixer_create() {
    let mut slot = mixer_slot().lock_or_recover();
    if matches!(*slot, MixerSlot::Initializing) {
        *slot = MixerSlot::Empty;
    }
}
