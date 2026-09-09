use super::*;

pub(crate) enum MixerSlot {
    Empty,
    Initializing,
    Running(Mixer),
    Stopping,
}

static MIXER: OnceLock<Mutex<MixerSlot>> = OnceLock::new();

pub(crate) fn mixer_slot() -> &'static Mutex<MixerSlot> {
    MIXER.get_or_init(|| Mutex::new(MixerSlot::Empty))
}

pub(crate) fn with_mixer<T>(f: impl FnOnce(&mut Mixer) -> T) -> Result<T, i32> {
    let start = Instant::now();
    let mut slot = mixer_slot().lock().expect("mixer mutex poisoned");
    let result = match &mut *slot {
        MixerSlot::Running(mixer) => Ok(f(mixer)),
        _ => Err(ERR_NOT_CREATED),
    };
    crate::diag::lock_held("mixer_slot", start, result)
}

pub(crate) fn reserve_mixer_create() -> i32 {
    let mut slot = mixer_slot().lock().expect("mixer mutex poisoned");
    match *slot {
        MixerSlot::Empty => {
            *slot = MixerSlot::Initializing;
            OK
        }
        _ => ERR_ALREADY_CREATED,
    }
}

pub(crate) fn commit_mixer_create(mixer: Mixer) -> i32 {
    let mut slot = mixer_slot().lock().expect("mixer mutex poisoned");
    match *slot {
        MixerSlot::Initializing => {
            *slot = MixerSlot::Running(mixer);
            OK
        }
        _ => ERR_ALREADY_CREATED,
    }
}

pub(crate) fn abort_mixer_create() {
    let mut slot = mixer_slot().lock().expect("mixer mutex poisoned");
    if matches!(*slot, MixerSlot::Initializing) {
        *slot = MixerSlot::Empty;
    }
}
