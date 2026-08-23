use crate::TimeError;
use crate::framerate::FrameRate;
use crate::rational::Rational;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;

/// Media timestamp stored as integer ticks on a rational timebase.
/// Seconds = `ticks * timebase.numerator / timebase.denominator`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MediaTime {
    ticks: i64,
    timebase: Rational,
}

impl MediaTime {
    pub const ZERO: Self = Self {
        ticks: 0,
        timebase: Rational::ONE,
    };

    pub fn new(ticks: i64, timebase: Rational) -> Self {
        Self { ticks, timebase }
    }

    pub fn from_seconds_rational(seconds: Rational) -> Self {
        Self {
            ticks: seconds.numerator(),
            timebase: Rational::new(1, seconds.denominator() as i64).expect("den"),
        }
    }

    /// Presentation time of frame `index` at `rate`: `index * den / num` seconds.
    pub fn from_frame_index(index: u64, rate: FrameRate) -> Result<Self, TimeError> {
        let ticks = i64::try_from(index).map_err(|_| TimeError::Overflow)?;
        Ok(Self {
            ticks,
            timebase: rate.frame_duration(),
        })
    }

    pub fn ticks(self) -> i64 {
        self.ticks
    }

    pub fn timebase(self) -> Rational {
        self.timebase
    }

    pub fn frame_index(self, rate: FrameRate) -> Result<u64, TimeError> {
        if self.ticks < 0 {
            return Ok(0);
        }
        // seconds = ticks * tb_num / tb_den
        // frame = floor(seconds * rate_num / rate_den)
        let num = (self.ticks as i128)
            .checked_mul(self.timebase.numerator() as i128)
            .and_then(|v| v.checked_mul(rate.numerator() as i128))
            .ok_or(TimeError::Overflow)?;
        let den = (self.timebase.denominator() as i128)
            .checked_mul(rate.denominator() as i128)
            .ok_or(TimeError::Overflow)?;
        if den == 0 {
            return Err(TimeError::Overflow);
        }
        Ok((num / den).max(0) as u64)
    }

    fn as_cross(self) -> (i128, i128) {
        (
            self.ticks as i128 * self.timebase.numerator() as i128,
            self.timebase.denominator() as i128,
        )
    }
}

impl PartialEq for MediaTime {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for MediaTime {}

impl PartialOrd for MediaTime {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MediaTime {
    fn cmp(&self, other: &Self) -> Ordering {
        let (a, ad) = self.as_cross();
        let (b, bd) = other.as_cross();
        (a * bd).cmp(&(b * ad))
    }
}

impl fmt::Display for MediaTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}*{}", self.ticks, self.timebase)
    }
}

/// `floor(frame * sample_rate * rate.den / rate.num)` — exact, no float.
pub fn audio_sample_index(frame: u64, sample_rate: u32, rate: FrameRate) -> Result<u64, TimeError> {
    let n = (frame as u128)
        .checked_mul(sample_rate as u128)
        .and_then(|v| v.checked_mul(rate.denominator() as u128))
        .ok_or(TimeError::Overflow)?;
    Ok((n / rate.numerator() as u128) as u64)
}

pub fn audio_frame_sample_span(
    frame: u64,
    sample_rate: u32,
    rate: FrameRate,
) -> Result<(u64, u64), TimeError> {
    let start = audio_sample_index(frame, sample_rate, rate)?;
    let end = audio_sample_index(frame.saturating_add(1), sample_rate, rate)?;
    Ok((start, end.saturating_sub(start)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framerate::NTSC_5994;

    #[test]
    fn pts_is_n_times_1001_over_60000() {
        let t = MediaTime::from_frame_index(60000, NTSC_5994).unwrap();
        // 60000 * 1001 / 60000 = 1001 seconds
        let one_frame = MediaTime::from_frame_index(1, NTSC_5994).unwrap();
        assert_eq!(one_frame.timebase().numerator(), 1001);
        assert_eq!(one_frame.timebase().denominator(), 60000);
        assert_eq!(t.ticks(), 60000);
        // 1001 seconds of video is exactly 60000 frames.
        assert_eq!(t.frame_index(NTSC_5994).unwrap(), 60000);
    }

    #[test]
    fn no_accumulated_rounding_over_million_frames() {
        let a = MediaTime::from_frame_index(1_000_000, NTSC_5994).unwrap();
        let b = MediaTime::from_frame_index(1_000_001, NTSC_5994).unwrap();
        assert!(b > a);
        assert_eq!(a.frame_index(NTSC_5994).unwrap(), 1_000_000);
    }

    #[test]
    fn audio_cadence_uses_floor_formula() {
        let s0 = audio_sample_index(0, 48000, NTSC_5994).unwrap();
        let s1 = audio_sample_index(1, 48000, NTSC_5994).unwrap();
        let s2 = audio_sample_index(2, 48000, NTSC_5994).unwrap();
        assert_eq!(s0, 0);
        assert_eq!(s1, 800); // floor(800.8)
        assert_eq!(s2, 1601); // floor(1601.6)
        let (_start, n) = audio_frame_sample_span(0, 48000, NTSC_5994).unwrap();
        assert_eq!(n, 800);
        let (_, n1) = audio_frame_sample_span(1, 48000, NTSC_5994).unwrap();
        assert_eq!(n1, 801);
    }

    #[test]
    fn thousand_one_seconds_are_sixty_thousand_frames() {
        let mut count = 0u64;
        // Count deadlines in 1001 seconds: frames 0..60000 inclusive start is 60000 frames.
        while MediaTime::from_frame_index(count, NTSC_5994).unwrap()
            < MediaTime::from_frame_index(60000, NTSC_5994).unwrap()
        {
            count += 1;
        }
        assert_eq!(count, 60000);
    }
}
