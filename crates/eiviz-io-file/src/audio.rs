use crate::timeline::{PlaybackTimeline, parse_movie_timeline, scale_u64};
use crate::video::{H264Mp4Index, VideoFileSource};
use eiviz_codec_software::{AacLcConfig, FdkAacDecoder, OpenH264Decoder};
use eiviz_core::{AudioResamplingPolicy, ColorSpace, InputId, Playback};
use eiviz_media::{AudioBuffer, MediaError, MediaSource, Result, VideoFrame};
use eiviz_time::{FrameRate, MediaTime};
use shiguredo_mp4::{
    TrackKind,
    boxes::SampleEntry,
    demux::{Input, Mp4FileDemuxer},
};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_AAC_ACCESS_UNIT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct AacSample {
    pub bytes: Vec<u8>,
    pub dts: u64,
    pub duration: u32,
    /// Start on the edited presentation timeline, in decoded PCM frames.
    pub presentation_start_frame: i64,
}

#[derive(Clone, Debug)]
pub struct AacMp4Index {
    pub track_id: u32,
    pub timescale: u32,
    pub config: AacLcConfig,
    pub samples: Vec<AacSample>,
    pub priming_frames: u64,
    pub presentation_lead_frames: u64,
}

impl AacMp4Index {
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
        Self::parse_optional(bytes)?
            .ok_or_else(|| MediaError::Unsupported("MP4 contains no AAC audio track".into()))
    }

    pub(crate) fn parse_optional(bytes: &[u8]) -> Result<Option<Self>> {
        let movie_timeline = parse_movie_timeline(bytes)?;
        let mut demuxer = Mp4FileDemuxer::new();
        feed_demuxer(&mut demuxer, bytes)?;
        let tracks = demuxer
            .tracks()
            .map_err(|error| MediaError::Other(format!("MP4 tracks: {error}")))?;
        let audio = tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Audio)
            .collect::<Vec<_>>();
        if audio.is_empty() {
            return Ok(None);
        }
        if audio.len() != 1 {
            return Err(MediaError::Unsupported(format!(
                "expected at most one audio track, found {}",
                audio.len()
            )));
        }
        let track_id = audio[0].track_id;
        let timescale = audio[0].timescale.get();
        let edit = movie_timeline.edit(track_id);
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
                        "multiple AAC sample descriptions are unsupported".into(),
                    ));
                }
                let SampleEntry::Mp4a(mp4a) = entry else {
                    return Err(MediaError::Unsupported(
                        "audio track is not an mp4a sample entry".into(),
                    ));
                };
                let descriptor = &mp4a.esds_box.es.dec_config_descr;
                if descriptor.object_type_indication != 0x40 || descriptor.stream_type.get() != 5 {
                    return Err(MediaError::Unsupported(
                        "mp4a/esds does not describe MPEG-4 AAC audio".into(),
                    ));
                }
                let asc = descriptor
                    .dec_specific_info
                    .as_ref()
                    .ok_or_else(|| {
                        MediaError::Unsupported("mp4a/esds is missing AudioSpecificConfig".into())
                    })?
                    .payload
                    .as_slice();
                let parsed = AacLcConfig::parse(asc)
                    .map_err(|error| MediaError::Unsupported(error.to_string()))?;
                if mp4a.audio.channelcount != parsed.channels
                    || u32::from(mp4a.audio.samplerate.integer) != parsed.sample_rate
                {
                    return Err(MediaError::Unsupported(format!(
                        "mp4a fields {} Hz/{} ch disagree with AudioSpecificConfig {} Hz/{} ch",
                        mp4a.audio.samplerate.integer,
                        mp4a.audio.channelcount,
                        parsed.sample_rate,
                        parsed.channels
                    )));
                }
                config = Some(parsed);
            }
            let parsed = config.as_ref().ok_or_else(|| {
                MediaError::Unsupported("AAC sample precedes mp4a/esds configuration".into())
            })?;
            if sample.data_size > MAX_AAC_ACCESS_UNIT_BYTES {
                return Err(MediaError::Unsupported(format!(
                    "AAC access unit exceeds {MAX_AAC_ACCESS_UNIT_BYTES} byte limit"
                )));
            }
            let start = usize::try_from(sample.data_offset)
                .map_err(|_| MediaError::Other("AAC sample offset overflow".into()))?;
            let end = start
                .checked_add(sample.data_size)
                .ok_or_else(|| MediaError::Other("AAC sample range overflow".into()))?;
            let data = bytes
                .get(start..end)
                .ok_or_else(|| MediaError::Other("truncated AAC sample".into()))?;
            let presentation_ticks = movie_timeline.presentation_ticks(
                i64::try_from(sample.timestamp)
                    .map_err(|_| MediaError::Other("AAC timestamp exceeds i64".into()))?,
                timescale,
                edit,
            )?;
            let presentation_start_frame =
                scale_i64(presentation_ticks, parsed.sample_rate, timescale)?;
            samples.push(AacSample {
                bytes: data.to_vec(),
                dts: sample.timestamp,
                duration: sample.duration,
                presentation_start_frame,
            });
        }
        let config = config
            .ok_or_else(|| MediaError::Unsupported("mp4a/esds configuration missing".into()))?;
        if samples.is_empty() {
            return Err(MediaError::Unsupported(
                "AAC MP4 has no audio samples".into(),
            ));
        }
        Ok(Some(Self {
            track_id,
            timescale,
            priming_frames: scale_u64(
                edit.media_start,
                u64::from(config.sample_rate),
                u64::from(timescale),
            )?,
            presentation_lead_frames: scale_u64(
                edit.presentation_start_us,
                u64::from(config.sample_rate),
                1_000_000,
            )?,
            config,
            samples,
        }))
    }
}

fn feed_demuxer(demuxer: &mut Mp4FileDemuxer, bytes: &[u8]) -> Result<()> {
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
    Ok(())
}

fn scale_i64(value: i64, numerator: u32, denominator: u32) -> Result<i64> {
    let scaled = i128::from(value)
        .checked_mul(i128::from(numerator))
        .ok_or_else(|| MediaError::Other("AAC presentation timestamp overflow".into()))?
        / i128::from(denominator);
    i64::try_from(scaled)
        .map_err(|_| MediaError::Other("AAC presentation timestamp overflow".into()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileMediaStatus {
    VideoOnly,
    AudioVideo {
        source_sample_rate: u32,
        channels: u16,
        project_sample_rate: u32,
        uses_asrc: bool,
    },
}

impl std::fmt::Display for FileMediaStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VideoOnly => formatter.write_str("video-only (H.264)"),
            Self::AudioVideo {
                source_sample_rate,
                channels,
                project_sample_rate,
                uses_asrc,
            } => write!(
                formatter,
                "A/V (H.264 + AAC-LC, {source_sample_rate} Hz/{channels} ch -> {project_sample_rate} Hz, {})",
                if *uses_asrc {
                    "explicit ASRC"
                } else {
                    "exact rate"
                }
            ),
        }
    }
}

pub struct FileMediaSource {
    id: InputId,
    video: VideoFileSource,
    audio: Option<AudioTrackSource>,
    status: FileMediaStatus,
    project_sample_rate: u32,
}

impl FileMediaSource {
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        id: InputId,
        path: &Path,
        openh264_binary: &Path,
        fdk_aac_binary: Option<&Path>,
        project_sample_rate: u32,
        resampling: AudioResamplingPolicy,
        expected_color: ColorSpace,
        playback: Playback,
    ) -> Result<Self> {
        // Verify the mandatory video backend before reading media. There is no alternate path.
        OpenH264Decoder::new(openh264_binary)
            .map_err(|error| MediaError::Other(error.to_string()))?;
        let metadata =
            std::fs::metadata(path).map_err(|error| MediaError::Other(error.to_string()))?;
        if metadata.len() > MAX_FILE_BYTES {
            return Err(MediaError::Unsupported("MP4 exceeds 2 GiB limit".into()));
        }
        let bytes = std::fs::read(path).map_err(|error| MediaError::Other(error.to_string()))?;
        let video_index = H264Mp4Index::parse(&bytes, expected_color)?;
        let audio_index = AacMp4Index::parse_optional(&bytes)?;
        let has_audio = audio_index.is_some();
        let timeline = Arc::new(Mutex::new(PlaybackTimeline::new(
            playback,
            video_index.presentation_duration_us,
            has_audio,
        )?));
        let video =
            VideoFileSource::from_index(id, video_index, openh264_binary, Arc::clone(&timeline))?;
        let (audio, status) = if let Some(index) = audio_index {
            let fdk_path = fdk_aac_binary.ok_or_else(|| {
                MediaError::Unsupported(
                    "MP4 contains AAC-LC, but no explicit license-reviewed FDK AAC decoder binary path was supplied; audio is not dropped or replaced with PCM".into(),
                )
            })?;
            let uses_asrc = if index.config.sample_rate == project_sample_rate {
                false
            } else {
                match resampling {
                    AudioResamplingPolicy::ExactRate => {
                        return Err(MediaError::Unsupported(format!(
                            "AAC source rate {} Hz does not match project {} Hz under ExactRate; explicitly select an ASRC policy",
                            index.config.sample_rate, project_sample_rate
                        )));
                    }
                    AudioResamplingPolicy::Asrc { .. } => true,
                }
            };
            let source = AudioTrackSource::new(index, fdk_path, timeline)?;
            let status = FileMediaStatus::AudioVideo {
                source_sample_rate: source.index.config.sample_rate,
                channels: source.index.config.channels,
                project_sample_rate,
                uses_asrc,
            };
            (Some(source), status)
        } else {
            (None, FileMediaStatus::VideoOnly)
        };
        Ok(Self {
            id,
            video,
            audio,
            status,
            project_sample_rate,
        })
    }

    pub fn status(&self) -> &FileMediaStatus {
        &self.status
    }

    pub fn set_playback(&self, playback: Playback) -> Result<()> {
        self.video.set_playback_mode(playback, self.audio.is_some())
    }

    pub fn playback(&self) -> Result<Playback> {
        self.video.playback()
    }
}

impl MediaSource for FileMediaSource {
    fn id(&self) -> InputId {
        self.id
    }

    fn pull_video(&self, pts: MediaTime, rate: FrameRate) -> Result<Option<VideoFrame>> {
        self.video.pull_video(pts, rate)
    }

    fn pull_audio(&self, sample_index: u64, frames: usize) -> Result<Option<AudioBuffer>> {
        self.audio
            .as_ref()
            .map(|audio| audio.pull(sample_index, frames, self.project_sample_rate))
            .transpose()
    }

    fn update_playback(&self, playback: &Playback) {
        let _ = self.set_playback(playback.clone());
    }
}

struct AudioTrackSource {
    index: AacMp4Index,
    binary_path: PathBuf,
    timeline: Arc<Mutex<PlaybackTimeline>>,
    state: Mutex<AudioDecodeState>,
}

struct AudioDecodeState {
    decoder: FdkAacDecoder,
    next_sample: usize,
    queue_start: i64,
    planes: Vec<VecDeque<f32>>,
    seen_generation: u64,
    expected_frame: Option<i64>,
}

impl AudioTrackSource {
    fn new(
        index: AacMp4Index,
        binary_path: &Path,
        timeline: Arc<Mutex<PlaybackTimeline>>,
    ) -> Result<Self> {
        let decoder = FdkAacDecoder::new(binary_path, index.config.clone())
            .map_err(|error| MediaError::Other(error.to_string()))?;
        Ok(Self {
            index,
            binary_path: binary_path.to_path_buf(),
            timeline,
            state: Mutex::new(AudioDecodeState {
                decoder,
                next_sample: 0,
                queue_start: 0,
                planes: Vec::new(),
                seen_generation: 0,
                expected_frame: None,
            }),
        })
    }

    fn pull(&self, sample_index: u64, frames: usize, project_rate: u32) -> Result<AudioBuffer> {
        let clock_us = scale_u64(sample_index, 1_000_000, u64::from(project_rate))?;
        let step = self
            .timeline
            .lock()
            .map_err(|_| MediaError::Other("file playback lock poisoned".into()))?
            .resolve(clock_us);
        let target = i64::try_from(scale_u64(
            step.position_us,
            u64::from(self.index.config.sample_rate),
            1_000_000,
        )?)
        .map_err(|_| MediaError::Other("AAC playback position exceeds i64".into()))?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| MediaError::Other("AAC decoder lock poisoned".into()))?;
        let discontinuity = state.seen_generation != step.generation
            || state
                .expected_frame
                .is_some_and(|expected| expected != target);
        if discontinuity {
            self.reset_at(&mut state, target)?;
        }
        state.seen_generation = step.generation;
        self.ensure_decoded(&mut state, target.saturating_add(frames as i64))?;

        let mut output = AudioBuffer::silence(
            u64::try_from(target.max(0)).unwrap_or(0),
            self.index.config.sample_rate,
            self.index.config.channels,
            frames,
        );
        output.discontinuity = discontinuity;
        let queue_end = state
            .queue_start
            .saturating_add(state.planes.first().map_or(0, VecDeque::len) as i64);
        let copy_start = target.max(state.queue_start);
        let copy_end = target
            .saturating_add(frames as i64)
            .min(queue_end)
            .max(copy_start);
        if copy_end > copy_start {
            let source_offset = usize::try_from(copy_start - state.queue_start)
                .map_err(|_| MediaError::Other("AAC queue offset overflow".into()))?;
            let output_offset = usize::try_from(copy_start - target)
                .map_err(|_| MediaError::Other("AAC output offset overflow".into()))?;
            let count = usize::try_from(copy_end - copy_start)
                .map_err(|_| MediaError::Other("AAC copy length overflow".into()))?;
            for (output_plane, queue) in output.planes.iter_mut().zip(&state.planes) {
                for (destination, source) in output_plane[output_offset..output_offset + count]
                    .iter_mut()
                    .zip(queue.iter().skip(source_offset).take(count))
                {
                    *destination = *source;
                }
            }
        }
        state.expected_frame = Some(target.saturating_add(frames as i64));
        self.discard_before(&mut state, target);
        Ok(output)
    }

    fn reset_at(&self, state: &mut AudioDecodeState, target: i64) -> Result<()> {
        state.decoder = FdkAacDecoder::new(&self.binary_path, self.index.config.clone())
            .map_err(|error| MediaError::Other(error.to_string()))?;
        let target_sample = self
            .index
            .samples
            .partition_point(|sample| {
                sample
                    .presentation_start_frame
                    .saturating_add(self.index.config.frame_length as i64)
                    <= target
            })
            .min(self.index.samples.len().saturating_sub(1));
        state.next_sample = target_sample.saturating_sub(1);
        state.queue_start = self.index.samples[state.next_sample].presentation_start_frame;
        state.planes = vec![VecDeque::new(); self.index.config.channels as usize];
        state.expected_frame = None;
        Ok(())
    }

    fn ensure_decoded(&self, state: &mut AudioDecodeState, required_end: i64) -> Result<()> {
        while state
            .queue_start
            .saturating_add(state.planes.first().map_or(0, VecDeque::len) as i64)
            < required_end
            && state.next_sample < self.index.samples.len()
        {
            let sample = &self.index.samples[state.next_sample];
            let decoded = state
                .decoder
                .decode(&sample.bytes)
                .map_err(|error| MediaError::Other(error.to_string()))?;
            let queue_end = state
                .queue_start
                .saturating_add(state.planes.first().map_or(0, VecDeque::len) as i64);
            if sample.presentation_start_frame > queue_end {
                let gap = usize::try_from(sample.presentation_start_frame - queue_end)
                    .map_err(|_| MediaError::Other("AAC edit gap overflow".into()))?;
                for plane in &mut state.planes {
                    plane.resize(plane.len().saturating_add(gap), 0.0);
                }
            }
            let queue_end = state
                .queue_start
                .saturating_add(state.planes.first().map_or(0, VecDeque::len) as i64);
            let skip = usize::try_from(
                queue_end
                    .saturating_sub(sample.presentation_start_frame)
                    .max(0),
            )
            .map_err(|_| MediaError::Other("AAC overlap overflow".into()))?;
            for (queue, decoded_plane) in state.planes.iter_mut().zip(decoded) {
                queue.extend(decoded_plane.into_iter().skip(skip));
            }
            state.next_sample = state.next_sample.saturating_add(1);
        }
        Ok(())
    }

    fn discard_before(&self, state: &mut AudioDecodeState, target: i64) {
        let keep_from = target.saturating_sub(self.index.config.frame_length as i64);
        let discard = usize::try_from(keep_from.saturating_sub(state.queue_start).max(0))
            .unwrap_or(usize::MAX)
            .min(state.planes.first().map_or(0, VecDeque::len));
        for plane in &mut state.planes {
            plane.drain(..discard);
        }
        state.queue_start = state.queue_start.saturating_add(discard as i64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_mp4::{
        FixedPointNumber, TrackKind,
        boxes::{AudioSampleEntryFields, Mp4aBox, SampleEntry},
        descriptors::{
            DecoderConfigDescriptor, DecoderSpecificInfo, EsDescriptor, SlConfigDescriptor,
        },
        mux::{Mp4FileMuxer, Sample},
    };
    use std::num::{NonZeroU16, NonZeroU32};

    fn deterministic_aac_mp4() -> Vec<u8> {
        let mut muxer = Mp4FileMuxer::new().unwrap();
        let mut bytes = muxer.initial_boxes_bytes().to_vec();
        let payload = [0xde, 0xad, 0xbe, 0xef];
        let data_offset = bytes.len() as u64;
        bytes.extend_from_slice(&payload);
        let entry = SampleEntry::Mp4a(Mp4aBox {
            audio: AudioSampleEntryFields {
                data_reference_index: NonZeroU16::MIN,
                channelcount: 2,
                samplesize: 16,
                samplerate: FixedPointNumber::new(48_000, 0),
            },
            esds_box: shiguredo_mp4::boxes::EsdsBox {
                es: EsDescriptor {
                    es_id: 1,
                    stream_priority: EsDescriptor::LOWEST_STREAM_PRIORITY,
                    depends_on_es_id: None,
                    url_string: None,
                    ocr_es_id: None,
                    dec_config_descr: DecoderConfigDescriptor {
                        object_type_indication:
                            DecoderConfigDescriptor::OBJECT_TYPE_INDICATION_AUDIO_ISO_IEC_14496_3,
                        stream_type: DecoderConfigDescriptor::STREAM_TYPE_AUDIO,
                        up_stream: DecoderConfigDescriptor::UP_STREAM_FALSE,
                        buffer_size_db: shiguredo_mp4::Uint::new(0),
                        max_bitrate: 0,
                        avg_bitrate: 0,
                        dec_specific_info: Some(DecoderSpecificInfo {
                            payload: vec![0x11, 0x90],
                        }),
                    },
                    sl_config_descr: SlConfigDescriptor,
                },
            },
            unknown_boxes: Vec::new(),
        });
        muxer
            .append_sample(&Sample {
                track_kind: TrackKind::Audio,
                sample_entry: Some(entry),
                keyframe: true,
                timescale: NonZeroU32::new(48_000).unwrap(),
                duration: 1024,
                composition_time_offset: None,
                data_offset,
                data_size: payload.len(),
            })
            .unwrap();
        let finalized = muxer.finalize().unwrap();
        for (offset, replacement) in finalized.offset_and_bytes_pairs() {
            let offset = offset as usize;
            bytes.resize(bytes.len().max(offset + replacement.len()), 0);
            bytes[offset..offset + replacement.len()].copy_from_slice(replacement);
        }
        bytes
    }

    #[test]
    fn deterministically_demuxes_mp4a_esds_and_timeline() {
        let index = AacMp4Index::parse(&deterministic_aac_mp4()).unwrap();
        assert_eq!(index.config.audio_specific_config, [0x11, 0x90]);
        assert_eq!(index.config.sample_rate, 48_000);
        assert_eq!(index.config.channels, 2);
        assert_eq!(index.samples.len(), 1);
        assert_eq!(index.samples[0].bytes, [0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(index.samples[0].presentation_start_frame, 0);
        assert_eq!(index.priming_frames, 0);
    }

    #[test]
    #[ignore = "requires explicit OpenH264, license-reviewed FDK, and representative A/V MP4 paths"]
    fn decodes_real_h264_aac_mp4_with_explicit_binaries() {
        let openh264 =
            std::env::var_os("EIVIZ_FILE_HIL_OPENH264").expect("set EIVIZ_FILE_HIL_OPENH264");
        let fdk = std::env::var_os("EIVIZ_FILE_HIL_FDK_AAC").expect("set EIVIZ_FILE_HIL_FDK_AAC");
        let mp4 = std::env::var_os("EIVIZ_FILE_HIL_MP4").expect("set EIVIZ_FILE_HIL_MP4");
        let index = AacMp4Index::open(Path::new(&mp4)).unwrap();
        let source = FileMediaSource::open(
            InputId::new(),
            Path::new(&mp4),
            Path::new(&openh264),
            Some(Path::new(&fdk)),
            index.config.sample_rate,
            AudioResamplingPolicy::ExactRate,
            ColorSpace::Bt709Sdr,
            Playback::default(),
        )
        .unwrap();
        assert!(matches!(
            source.status(),
            FileMediaStatus::AudioVideo { .. }
        ));
        let video = source
            .pull_video(MediaTime::ZERO, eiviz_time::NTSC_5994)
            .unwrap()
            .expect("H.264 decoder must emit a frame");
        assert!(!video.data.is_empty());
        let audio = source
            .pull_audio(0, index.config.frame_length)
            .unwrap()
            .expect("AAC decoder must emit audio");
        assert_eq!(audio.sample_rate, index.config.sample_rate);
        assert_eq!(audio.channels, index.config.channels);
        assert_eq!(audio.planes[0].len(), index.config.frame_length);
    }
}
