//! NDI adapter boundary.
//!
//! The `ndi` feature is a real `grafton-ndi`/NDI 6 SDK integration. There is
//! deliberately no simulator or alternate protocol path in this crate.

#[cfg(any(feature = "ndi", test))]
use eiviz_time::{MediaTime, Rational};

#[cfg(feature = "ndi")]
mod native;

#[cfg(feature = "ndi")]
pub use native::{
    NdiConfig, NdiError, NdiSink, NdiSource, NdiSourceInfo, discover_sources, probe,
};

/// NDI timestamps and timecodes are signed 100 ns ticks.
pub const NDI_TICKS_PER_SECOND: i64 = 10_000_000;

#[cfg(any(feature = "ndi", test))]
fn ndi_ticks_to_media_time(ticks: i64) -> MediaTime {
    MediaTime::new(
        ticks,
        Rational::new(1, NDI_TICKS_PER_SECOND).expect("constant NDI timebase"),
    )
}

#[cfg(any(feature = "ndi", test))]
fn media_time_to_ndi_ticks(time: MediaTime) -> i64 {
    let value = time.ticks() as i128
        * time.timebase().numerator() as i128
        * NDI_TICKS_PER_SECOND as i128
        / time.timebase().denominator() as i128;
    value.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

#[cfg(any(feature = "ndi", test))]
fn ndi_ticks_to_sample_index(ticks: i64, sample_rate: u32) -> u64 {
    if ticks <= 0 {
        return 0;
    }
    let samples =
        ticks as u128 * sample_rate as u128 / NDI_TICKS_PER_SECOND as u128;
    samples.min(u64::MAX as u128) as u64
}

#[cfg(any(feature = "ndi", test))]
fn sample_index_to_ndi_ticks(sample_index: u64, sample_rate: u32) -> i64 {
    if sample_rate == 0 {
        return 0;
    }
    let ticks =
        sample_index as u128 * NDI_TICKS_PER_SECOND as u128 / sample_rate as u128;
    ticks.min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_time::{MediaTime, NTSC_5994};

    #[test]
    fn ndi_timestamp_roundtrip_is_exact_at_one_second() {
        let media = ndi_ticks_to_media_time(NDI_TICKS_PER_SECOND);
        assert_eq!(media_time_to_ndi_ticks(media), NDI_TICKS_PER_SECOND);
    }

    #[test]
    fn ntsc_timestamp_conversion_does_not_accumulate_rounded_periods() {
        let frame = MediaTime::from_frame_index(1_000_000, NTSC_5994).unwrap();
        let expected = 1_000_000_i128 * 1001 * NDI_TICKS_PER_SECOND as i128 / 60_000;
        assert_eq!(media_time_to_ndi_ticks(frame), expected as i64);
    }

    #[test]
    fn audio_timestamp_conversion_uses_absolute_sample_index() {
        assert_eq!(sample_index_to_ndi_ticks(48_000, 48_000), 10_000_000);
        assert_eq!(ndi_ticks_to_sample_index(10_000_000, 48_000), 48_000);
        assert_eq!(ndi_ticks_to_sample_index(-1, 48_000), 0);
        assert_eq!(sample_index_to_ndi_ticks(1, 0), 0);
    }
}
