use eiviz_codec_software::{avcc_parameter_sets_to_annexb, avcc_sample_to_annexb};
use eiviz_media::{MediaError, Result};
use eiviz_time::{MediaTime, Rational};
use shiguredo_mp4::{
    TrackKind,
    boxes::SampleEntry,
    demux::{Input, Mp4FileDemuxer},
};
use std::path::Path;

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
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
    pub timescale: u32,
    pub decoder_preamble: Vec<u8>,
    pub samples: Vec<H264Sample>,
}

impl H264Mp4Index {
    pub fn open(path: &Path) -> Result<Self> {
        let metadata =
            std::fs::metadata(path).map_err(|error| MediaError::Other(error.to_string()))?;
        if metadata.len() > MAX_FILE_BYTES {
            return Err(MediaError::Unsupported("MP4 exceeds 2 GiB limit".into()));
        }
        let bytes = std::fs::read(path).map_err(|error| MediaError::Other(error.to_string()))?;
        Self::parse(&bytes)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut demuxer = Mp4FileDemuxer::new();
        while let Some(required) = demuxer.required_input() {
            let start = usize::try_from(required.position)
                .map_err(|_| MediaError::Other("MP4 offset overflow".into()))?;
            let size = required.size.unwrap_or(bytes.len().saturating_sub(start));
            let end = start
                .checked_add(size)
                .map(|end| end.min(bytes.len()))
                .ok_or_else(|| MediaError::Other("MP4 range overflow".into()))?;
            let data = bytes
                .get(start..end)
                .ok_or_else(|| MediaError::Other("truncated MP4 input request".into()))?;
            demuxer.handle_input(Input {
                position: required.position,
                data,
            });
        }
        let tracks = demuxer
            .tracks()
            .map_err(|error| MediaError::Other(format!("MP4 tracks: {error}")))?;
        let video = tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Video)
            .collect::<Vec<_>>();
        if video.len() != 1 {
            return Err(MediaError::Unsupported(format!(
                "expected one video track, found {}",
                video.len()
            )));
        }
        let track_id = video[0].track_id;
        let timescale = video[0].timescale.get();
        let timebase = Rational::new(1, i64::from(timescale))
            .map_err(|error| MediaError::Other(error.to_string()))?;
        let mut config = None;
        let mut samples = Vec::new();
        while let Some(sample) = demuxer
            .next_sample()
            .map_err(|error| MediaError::Other(format!("MP4 sample: {error}")))?
        {
            if sample.track.track_id != track_id {
                continue;
            }
            if let Some(entry) = sample.sample_entry {
                if config.is_some() {
                    return Err(MediaError::Unsupported(
                        "multiple H.264 sample descriptions are unsupported".into(),
                    ));
                }
                let SampleEntry::Avc1(avc1) = entry else {
                    return Err(MediaError::Unsupported("video track is not avc1".into()));
                };
                let avcc = &avc1.avcc_box;
                if avcc.avc_profile_indication != 66 || avcc.profile_compatibility & 0x40 == 0 {
                    return Err(MediaError::Unsupported(
                        "only H.264 Constrained Baseline is supported".into(),
                    ));
                }
                config = Some((
                    avc1.visual.width,
                    avc1.visual.height,
                    avcc.length_size_minus_one.get() as usize + 1,
                    avcc_parameter_sets_to_annexb(
                        &avcc.sps_list,
                        &avcc.pps_list,
                        MAX_ACCESS_UNIT_BYTES,
                    )
                    .map_err(|error| MediaError::Other(error.to_string()))?,
                ));
            }
            let (_, _, length_size, _) = config.as_ref().ok_or_else(|| {
                MediaError::Unsupported("H.264 sample precedes avc1 configuration".into())
            })?;
            let start = usize::try_from(sample.data_offset)
                .map_err(|_| MediaError::Other("sample offset overflow".into()))?;
            let end = start
                .checked_add(sample.data_size)
                .ok_or_else(|| MediaError::Other("sample range overflow".into()))?;
            let data = bytes
                .get(start..end)
                .ok_or_else(|| MediaError::Other("truncated sample".into()))?;
            let cts = sample
                .timestamp
                .saturating_add_signed(sample.composition_time_offset.unwrap_or(0));
            samples.push(H264Sample {
                annexb: avcc_sample_to_annexb(data, *length_size, MAX_ACCESS_UNIT_BYTES)
                    .map_err(|error| MediaError::Other(error.to_string()))?,
                dts: sample.timestamp,
                pts: MediaTime::new(cts as i64, timebase),
                duration: MediaTime::new(i64::from(sample.duration), timebase),
                keyframe: sample.keyframe,
            });
        }
        let (width, height, _, decoder_preamble) =
            config.ok_or_else(|| MediaError::Unsupported("avc1 configuration missing".into()))?;
        Ok(Self {
            width,
            height,
            timescale,
            decoder_preamble,
            samples,
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

    #[test]
    fn malformed_input_is_rejected() {
        assert!(H264Mp4Index::parse(b"not an mp4").is_err());
    }
}
