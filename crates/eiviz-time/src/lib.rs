//! Rational arithmetic, media timestamps, and clock domains.
//!
//! 59.94 fps is always `60000/1001`. Frame presentation time is computed from
//! the frame index; rounded periods are never accumulated.

mod clock;
mod framerate;
mod rational;
mod timestamp;

pub use clock::{Clock, ClockDomain, ClockInstant, MonotonicClock, VirtualClock};
pub use framerate::{FrameRate, NTSC_5994, PAL_50, RATE_24, RATE_25, RATE_30, RATE_60};
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
}
