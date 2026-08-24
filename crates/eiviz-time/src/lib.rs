//! Rational arithmetic, media timestamps, and clock domains.
//!
//! 59.94 fps is always `60000/1001`. Frame presentation time is computed from
//! the frame index; rounded periods are never accumulated.

mod clock;
mod framerate;
mod mapper;
mod rational;
mod timestamp;

pub use clock::{Clock, ClockDomain, ClockInstant, MonotonicClock, VirtualClock, monotonic_nanos};
pub use framerate::{FrameRate, NTSC_5994, PAL_50, RATE_24, RATE_25, RATE_30, RATE_60};
pub use mapper::{
    ClockLockState, ClockMapper, ClockMapperConfig, ClockMapperDiagnostics, ClockObservation,
    ClockTimestamp, ObservationStatus, TimingIsland, TimingIslandDiagnostics,
};
pub use rational::{Rational, RationalError};
pub use timestamp::{MediaTime, audio_frame_sample_span, audio_sample_index};

pub type Result<T> = std::result::Result<T, TimeError>;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum TimeError {
    #[error(transparent)]
    Rational(#[from] RationalError),
    #[error("overflow computing media time")]
    Overflow,
    #[error("clock domain mismatch: {0:?} vs {1:?}")]
    DomainMismatch(ClockDomain, ClockDomain),
    #[error("clock timebase mismatch")]
    TimebaseMismatch,
    #[error("clock timebase must be positive")]
    InvalidTimebase,
    #[error("source and target clock domains are both {0:?}")]
    SameClockDomain(ClockDomain),
    #[error("invalid clock mapper configuration")]
    InvalidMapperConfig,
    #[error("clock mapper {from_domain:?} -> {to_domain:?} is unlocked")]
    ClockUnlocked {
        from_domain: ClockDomain,
        to_domain: ClockDomain,
    },
    #[error("clock mapper {from_domain:?} -> {to_domain:?} is missing")]
    MapperMissing {
        from_domain: ClockDomain,
        to_domain: ClockDomain,
    },
    #[error("clock counter value is outside its configured modulus")]
    CounterOutsideModulus,
}
