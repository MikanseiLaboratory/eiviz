//! NDI adapter boundary.
//!
//! The `ndi` feature is a real `grafton-ndi`/NDI 6 SDK integration. There is
//! deliberately no simulator or alternate protocol path in this crate.

#[cfg(any(feature = "ndi", test))]
use eiviz_media::{PixelFormat, VideoFrame};
#[cfg(any(feature = "ndi", test))]
use eiviz_time::{MediaTime, Rational};
#[cfg(any(feature = "ndi", test))]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[cfg(feature = "ndi")]
mod native;

#[cfg(feature = "ndi")]
pub use native::{NdiError, NdiSink, NdiSource, NdiSourceInfo, discover_sources, probe};

/// NDI timestamps and timecodes are signed 100 ns ticks.
pub const NDI_TICKS_PER_SECOND: i64 = 10_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NdiColorProfile {
    Bt709Limited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NdiOutputPixelFormat {
    Rgba,
    Nv12,
}

#[derive(Clone, Debug)]
pub struct NdiConfig {
    pub video_queue_capacity: usize,
    pub audio_queue_capacity: usize,
    pub metadata_queue_capacity: usize,
    pub output_queue_capacity: usize,
    pub capture_poll: Duration,
    pub output_pixel_format: NdiOutputPixelFormat,
    /// Required when output conversion crosses between RGB and YUV.
    pub output_color_profile: Option<NdiColorProfile>,
    /// Required by production output construction. Kept optional so receive
    /// configurations do not invent a project profile.
    pub output_video_profile: Option<eiviz_core::VideoFormat>,
}

impl Default for NdiConfig {
    fn default() -> Self {
        Self {
            video_queue_capacity: 2,
            audio_queue_capacity: 8,
            metadata_queue_capacity: 60,
            output_queue_capacity: 8,
            capture_poll: Duration::from_millis(10),
            output_pixel_format: NdiOutputPixelFormat::Rgba,
            output_color_profile: None,
            output_video_profile: None,
        }
    }
}

impl NdiConfig {
    #[cfg(any(feature = "ndi", test))]
    pub(crate) fn validation_error(&self) -> Option<&'static str> {
        if self.video_queue_capacity == 0
            || self.audio_queue_capacity == 0
            || self.metadata_queue_capacity == 0
            || self.output_queue_capacity == 0
        {
            return Some("queue capacities must be greater than zero");
        }
        if self.capture_poll.is_zero() {
            return Some("capture poll interval must be greater than zero");
        }
        if self.output_pixel_format == NdiOutputPixelFormat::Nv12
            && self.output_color_profile.is_none()
        {
            return Some("NV12 output requires an explicit color profile");
        }
        if let Some(video) = &self.output_video_profile
            && validate_video_format(video).is_err()
        {
            return Some("NDI adapter supports only 8-bit progressive BT.709 SDR project profiles");
        }
        None
    }

    pub fn for_output(video: &eiviz_core::VideoFormat) -> Result<Self, String> {
        validate_video_format(video)?;
        Ok(Self {
            output_video_profile: Some(video.clone()),
            ..Self::default()
        })
    }
}

pub fn validate_video_format(video: &eiviz_core::VideoFormat) -> Result<(), String> {
    if video.bit_depth != 8 || video.interlaced || video.color != eiviz_core::ColorSpace::Bt709Sdr {
        return Err(format!(
            "NDI adapter does not support {:?} {}-bit interlaced={}; only 8-bit progressive BT.709 SDR is implemented and no conversion fallback is permitted",
            video.color, video.bit_depth, video.interlaced
        ));
    }
    Ok(())
}

#[cfg(any(feature = "ndi", test))]
fn ndi_ticks_to_media_time(ticks: i64) -> MediaTime {
    MediaTime::new(
        ticks,
        Rational::new(1, NDI_TICKS_PER_SECOND).expect("constant NDI timebase"),
    )
}

#[cfg(any(feature = "ndi", test))]
fn media_time_to_ndi_ticks(time: MediaTime) -> i64 {
    let value =
        time.ticks() as i128 * time.timebase().numerator() as i128 * NDI_TICKS_PER_SECOND as i128
            / time.timebase().denominator() as i128;
    value.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

#[cfg(any(feature = "ndi", test))]
fn ndi_ticks_to_sample_index(ticks: i64, sample_rate: u32) -> u64 {
    if ticks <= 0 {
        return 0;
    }
    let samples = ticks as u128 * sample_rate as u128 / NDI_TICKS_PER_SECOND as u128;
    samples.min(u64::MAX as u128) as u64
}

#[cfg(any(feature = "ndi", test))]
fn sample_index_to_ndi_ticks(sample_index: u64, sample_rate: u32) -> i64 {
    if sample_rate == 0 {
        return 0;
    }
    let ticks = sample_index as u128 * NDI_TICKS_PER_SECOND as u128 / sample_rate as u128;
    ticks.min(i64::MAX as u128) as i64
}

#[cfg(any(feature = "ndi", test))]
fn push_latest<T>(
    tx: &crossbeam_channel::Sender<T>,
    drop_rx: &crossbeam_channel::Receiver<T>,
    value: T,
    dropped: &AtomicU64,
) {
    match tx.try_send(value) {
        Ok(()) => {}
        Err(crossbeam_channel::TrySendError::Full(value)) => {
            let _ = drop_rx.try_recv();
            let _ = tx.try_send(value);
            dropped.fetch_add(1, Ordering::Relaxed);
        }
        Err(crossbeam_channel::TrySendError::Disconnected(_)) => {}
    }
}

#[cfg(any(feature = "ndi", test))]
fn frame_to_nv12(frame: &VideoFrame, profile: NdiColorProfile) -> Result<Vec<u8>, String> {
    if !frame.width.is_multiple_of(2) || !frame.height.is_multiple_of(2) {
        return Err("NV12 output requires even width and height".into());
    }
    if !matches!(frame.format, PixelFormat::Rgba8 | PixelFormat::Bgra8) {
        return Err("NV12 output conversion accepts only RGBA8 or BGRA8 input".into());
    }
    let required = frame.width as usize * frame.height as usize * 4;
    if frame.data.len() < required {
        return Err(format!(
            "truncated {:?} frame: {} bytes, expected {required}",
            frame.format,
            frame.data.len()
        ));
    }

    let width = frame.width as usize;
    let height = frame.height as usize;
    let mut nv12 = vec![0_u8; width * height * 3 / 2];
    for y in 0..height {
        for x in 0..width {
            let rgb = packed_rgb(frame, x, y);
            nv12[y * width + x] = rgb_to_bt709_limited(rgb, profile).0;
        }
    }
    let uv_start = width * height;
    for y in (0..height).step_by(2) {
        for x in (0..width).step_by(2) {
            let mut u = 0_u16;
            let mut v = 0_u16;
            for dy in 0..2 {
                for dx in 0..2 {
                    let (_, sample_u, sample_v) =
                        rgb_to_bt709_limited(packed_rgb(frame, x + dx, y + dy), profile);
                    u += u16::from(sample_u);
                    v += u16::from(sample_v);
                }
            }
            let offset = uv_start + (y / 2) * width + x;
            nv12[offset] = ((u + 2) / 4) as u8;
            nv12[offset + 1] = ((v + 2) / 4) as u8;
        }
    }
    Ok(nv12)
}

#[cfg(any(feature = "ndi", test))]
fn packed_rgb(frame: &VideoFrame, x: usize, y: usize) -> [u8; 3] {
    let offset = (y * frame.width as usize + x) * 4;
    let pixel = &frame.data[offset..offset + 4];
    match frame.format {
        PixelFormat::Rgba8 => [pixel[0], pixel[1], pixel[2]],
        PixelFormat::Bgra8 => [pixel[2], pixel[1], pixel[0]],
        PixelFormat::Nv12 | PixelFormat::P010 | PixelFormat::P216 | PixelFormat::Rgba16Float => {
            unreachable!("validated packed RGB input")
        }
    }
}

#[cfg(any(feature = "ndi", test))]
fn rgb_to_bt709_limited(rgb: [u8; 3], profile: NdiColorProfile) -> (u8, u8, u8) {
    match profile {
        NdiColorProfile::Bt709Limited => {
            let [r, g, b] = rgb.map(i32::from);
            let y = ((47 * r + 157 * g + 16 * b + 128) >> 8) + 16;
            let u = ((-26 * r - 87 * g + 113 * b + 128) >> 8) + 128;
            let v = ((112 * r - 102 * g - 10 * b + 128) >> 8) + 128;
            (
                y.clamp(16, 235) as u8,
                u.clamp(16, 240) as u8,
                v.clamp(16, 240) as u8,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_media::VideoFrame;
    use eiviz_time::{MediaTime, NTSC_5994};
    use std::sync::atomic::AtomicU64;

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

    #[test]
    fn bounded_capture_queue_keeps_latest_frame() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let dropped = AtomicU64::new(0);
        push_latest(&tx, &rx, 1, &dropped);
        push_latest(&tx, &rx, 2, &dropped);
        assert_eq!(rx.try_recv().unwrap(), 2);
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn nv12_selection_requires_explicit_color_profile() {
        let config = NdiConfig {
            output_pixel_format: NdiOutputPixelFormat::Nv12,
            ..NdiConfig::default()
        };
        assert_eq!(
            config.validation_error(),
            Some("NV12 output requires an explicit color profile")
        );
    }

    #[test]
    fn bt709_nv12_conversion_has_expected_black_and_white_range() {
        let mut data = Vec::new();
        for _ in 0..2 {
            data.extend_from_slice(&[0, 0, 0, 255]);
            data.extend_from_slice(&[255, 255, 255, 255]);
        }
        let frame = VideoFrame {
            id: 1,
            source: None,
            pts: MediaTime::ZERO,
            capture_domain: eiviz_time::ClockDomain::Virtual,
            clock_observation: None,
            width: 2,
            height: 2,
            format: PixelFormat::Rgba8,
            color: eiviz_core::ColorSpace::Bt709Sdr.metadata(),
            field: eiviz_core::FieldKind::Progressive,
            data: data.into(),
            discontinuity: false,
        };
        let converted = frame_to_nv12(&frame, NdiColorProfile::Bt709Limited).unwrap();
        assert_eq!(&converted[..4], &[16, 235, 16, 235]);
        assert_eq!(converted.len(), 6);
    }
}
