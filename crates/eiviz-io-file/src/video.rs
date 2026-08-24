use eiviz_codec_software::{avcc_parameter_sets_to_annexb, avcc_sample_to_annexb};
use eiviz_media::{MediaError, Result};
use eiviz_time::{MediaTime, Rational};
use mp4io::{
    Codec, CodecConfig, ColourInfo, FourCC, Limits, Mp4, SampleEntry, Strictness, TrackKind,
};
use std::path::Path;

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_INDEX_BYTES: usize = 128 * 1024 * 1024;
const MAX_ACCESS_UNIT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct H264Sample {
    pub annexb: Vec<u8>,
    pub dts: u64,
    pub pts: MediaTime,
    pub duration: MediaTime,
    pub keyframe: bool,
}

#[derive(Clone, Debug)]
pub struct H264Mp4Index {
    pub width: u16,
    pub height: u16,
    pub movie_timescale: u32,
    pub track_timescale: u32,
    pub decoder_preamble: Vec<u8>,
    pub samples: Vec<H264Sample>,
    pub colour: ColourInfo,
}

impl H264Mp4Index {
    pub fn open(path: &Path) -> Result<Self> {
        let metadata =
            std::fs::metadata(path).map_err(|error| MediaError::Other(error.to_string()))?;
        if metadata.len() > MAX_FILE_BYTES {
            return Err(MediaError::Unsupported(format!(
                "MP4 exceeds configured {} byte limit",
                MAX_FILE_BYTES
            )));
        }
        let bytes = std::fs::read(path).map_err(|error| MediaError::Other(error.to_string()))?;
        Self::parse(&bytes)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mp4 = Mp4::parse_with_limits(
            bytes,
            Strictness::Normal,
            Limits::default().with_max_index_bytes(MAX_INDEX_BYTES),
        )
        .map_err(|error| MediaError::Other(format!("MP4 parse: {error}")))?;
        let video_tracks = mp4
            .tracks()
            .iter()
            .filter(|track| track.kind() == TrackKind::Video)
            .collect::<Vec<_>>();
        if video_tracks.len() != 1 {
            return Err(MediaError::Unsupported(format!(
                "expected exactly one video track, found {}",
                video_tracks.len()
            )));
        }
        let track = video_tracks[0];
        if track.codec() != Some(Codec::H264) {
            return Err(MediaError::Unsupported("video track is not H.264".into()));
        }
        if track.sample_entries().len() != 1 {
            return Err(MediaError::Unsupported(
                "multiple sample descriptions are not supported".into(),
            ));
        }
        let entry = match track.sample_entry() {
            Some(SampleEntry::Video(entry)) => entry,
            _ => {
                return Err(MediaError::Unsupported(
                    "H.264 video sample entry missing".into(),
                ));
            }
        };
        if entry.kind != FourCC::new(*b"avc1") {
            return Err(MediaError::Unsupported(
                "only avc1 is supported; avc3 is rejected".into(),
            ));
        }
        if !entry.protection_info.is_empty() {
            return Err(MediaError::Unsupported(
                "encrypted MP4 is unsupported".into(),
            ));
        }
        let colour = entry.colour_info.clone().ok_or_else(|| {
            MediaError::Unsupported(
                "MP4 has no explicit colour metadata; no implicit matrix is selected".into(),
            )
        })?;
        if !matches!(colour, ColourInfo::Nclx { .. }) {
            return Err(MediaError::Unsupported(
                "ICC colour profiles are not supported by the first H.264 profile".into(),
            ));
        }
        let config = match &entry.config {
            CodecConfig::Avc(config) => config,
            _ => {
                return Err(MediaError::Unsupported(
                    "AVC decoder configuration is missing".into(),
                ));
            }
        };
        if config.profile_indication != 66 || config.profile_compatibility & 0x40 == 0 {
            return Err(MediaError::Unsupported(format!(
                "only Constrained Baseline is supported (profile={}, compatibility={:#04x})",
                config.profile_indication, config.profile_compatibility
            )));
        }
        let decoder_preamble =
            avcc_parameter_sets_to_annexb(&config.sps, &config.pps, MAX_ACCESS_UNIT_BYTES)
                .map_err(|error| MediaError::Other(error.to_string()))?;
        let movie_timescale = mp4.timescale();
        let track_timescale = track.timescale();
        if movie_timescale == 0 || track_timescale == 0 {
            return Err(MediaError::Unsupported("zero MP4 timescale".into()));
        }
        let movie_timebase = Rational::new(1, i64::from(movie_timescale))
            .map_err(|error| MediaError::Other(error.to_string()))?;
        let track_timebase = Rational::new(1, i64::from(track_timescale))
            .map_err(|error| MediaError::Other(error.to_string()))?;
        let mut samples = Vec::with_capacity(track.sample_count());
        for index in 0..track.sample_count() {
            let sample = track
                .sample(index)
                .map_err(|error| MediaError::Other(format!("MP4 sample {index}: {error}")))?
                .ok_or_else(|| MediaError::Other(format!("MP4 sample {index} missing")))?;
            if sample.description_index != 1 {
                return Err(MediaError::Unsupported(
                    "sample description changes are unsupported".into(),
                ));
            }
            let presentation = track.presentation_time(sample.cts).ok_or_else(|| {
                MediaError::Unsupported(format!(
                    "sample {index} is outside the supported edit-list timeline"
                ))
            })?;
            if presentation < 0 {
                return Err(MediaError::Unsupported(
                    "negative presentation timestamps are unsupported".into(),
                ));
            }
            let annexb = avcc_sample_to_annexb(
                sample.data,
                usize::from(config.nalu_size_len),
                MAX_ACCESS_UNIT_BYTES,
            )
            .map_err(|error| MediaError::Other(format!("sample {index}: {error}")))?;
            samples.push(H264Sample {
                annexb,
                dts: sample.dts,
                pts: MediaTime::new(presentation, movie_timebase),
                duration: MediaTime::new(i64::from(sample.duration), track_timebase),
                keyframe: sample.is_sync,
            });
        }
        Ok(Self {
            width: entry.width,
            height: entry.height,
            movie_timescale,
            track_timescale,
            decoder_preamble,
            samples,
            colour,
        })
    }

    pub fn sync_sample_before(&self, target: MediaTime) -> Option<usize> {
        self.samples
            .iter()
            .enumerate()
            .take_while(|(_, sample)| sample.pts <= target)
            .filter(|(_, sample)| sample.keyframe)
            .map(|(index, _)| index)
            .last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(ticks: i64, keyframe: bool) -> H264Sample {
        H264Sample {
            annexb: vec![0, 0, 0, 1, 0x65],
            dts: ticks as u64,
            pts: MediaTime::new(ticks, Rational::new(1, 1000).unwrap()),
            duration: MediaTime::new(40, Rational::new(1, 1000).unwrap()),
            keyframe,
        }
    }

    #[test]
    fn seek_chooses_preceding_sync_sample_without_float_time() {
        let index = H264Mp4Index {
            width: 1920,
            height: 1080,
            movie_timescale: 1000,
            track_timescale: 1000,
            decoder_preamble: vec![0, 0, 0, 1, 0x67],
            samples: vec![
                sample(0, true),
                sample(40, false),
                sample(80, false),
                sample(120, true),
                sample(160, false),
            ],
            colour: ColourInfo::Nclx {
                colour_primaries: 1,
                transfer_characteristics: 1,
                matrix_coefficients: 1,
                full_range: false,
            },
        };
        let target = MediaTime::new(150, Rational::new(1, 1000).unwrap());
        assert_eq!(index.sync_sample_before(target), Some(3));
    }

    #[test]
    fn malformed_input_is_rejected_before_decoder_creation() {
        assert!(H264Mp4Index::parse(b"not an mp4").is_err());
    }
}
