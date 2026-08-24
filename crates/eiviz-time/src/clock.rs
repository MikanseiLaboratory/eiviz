use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Distinct clock domains. Values from different domains must not be compared
/// without an affine mapper.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClockDomain {
    Monotonic,
    DeckLinkStream,
    DeckLinkGenlock,
    AudioSample,
    SourceMedia,
    Ptp,
    Gpu,
    Virtual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockInstant {
    pub domain: ClockDomain,
    pub nanos: u64,
}

pub trait Clock: Send + Sync {
    fn domain(&self) -> ClockDomain;
    fn now(&self) -> ClockInstant;
}

#[derive(Debug)]
pub struct MonotonicClock {
    _private: (),
}

impl MonotonicClock {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    fn domain(&self) -> ClockDomain {
        ClockDomain::Monotonic
    }

    fn now(&self) -> ClockInstant {
        ClockInstant {
            domain: ClockDomain::Monotonic,
            nanos: monotonic_nanos(),
        }
    }
}

/// Nanoseconds from one process-wide [`Instant`] origin.
///
/// This value is suitable for correlating adapter callbacks and scheduler
/// deadlines inside this process. It is deliberately unrelated to UTC.
pub fn monotonic_nanos() -> u64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    let elapsed = ORIGIN.get_or_init(Instant::now).elapsed().as_nanos();
    u64::try_from(elapsed).unwrap_or(u64::MAX)
}

/// Deterministic clock for tests and replay. Advance only via `advance_nanos`.
#[derive(Debug)]
pub struct VirtualClock {
    nanos: AtomicU64,
}

impl VirtualClock {
    pub fn new() -> Self {
        Self {
            nanos: AtomicU64::new(0),
        }
    }

    pub fn set_nanos(&self, nanos: u64) {
        self.nanos.store(nanos, Ordering::SeqCst);
    }

    pub fn advance_nanos(&self, delta: u64) {
        self.nanos.fetch_add(delta, Ordering::SeqCst);
    }

    /// Advance to the exact PTS of `frame` at `rate`, computed from the frame
    /// index rather than by adding a rounded period.
    pub fn seek_frame(&self, frame: u64, rate: crate::FrameRate) {
        let num = frame as u128 * rate.denominator() as u128 * 1_000_000_000u128;
        let nanos = (num / rate.numerator() as u128) as u64;
        self.set_nanos(nanos);
    }
}

impl Default for VirtualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for VirtualClock {
    fn domain(&self) -> ClockDomain {
        ClockDomain::Virtual
    }

    fn now(&self) -> ClockInstant {
        ClockInstant {
            domain: ClockDomain::Virtual,
            nanos: self.nanos.load(Ordering::SeqCst),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framerate::NTSC_5994;

    #[test]
    fn virtual_seek_is_exact_ratio() {
        let clock = VirtualClock::new();
        clock.seek_frame(60000, NTSC_5994);
        // 60000 * 1001 / 60000 seconds = 1001s
        assert_eq!(clock.now().nanos, 1001 * 1_000_000_000);
    }
}
