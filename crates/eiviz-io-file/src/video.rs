use crate::timeline::{PlaybackTimeline, parse_movie_timeline};
use eiviz_codec_software::{OpenH264Decoder, avcc_parameter_sets_to_annexb, avcc_sample_to_annexb};
use eiviz_core::{ColorSpace, InputId, Playback};
use eiviz_media::{MediaError, MediaSource, Result, VideoFrame};
use eiviz_time::{ClockDomain, ClockObservation, ClockTimestamp, FrameRate, MediaTime, Rational};
use shiguredo_mp4::{
    TrackKind,
    boxes::SampleEntry,
    demux::{Input, Mp4FileDemuxer},
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
    pub color_metadata_sources: Vec<VideoColorMetadataSource>,
    pub samples: Vec<H264Sample>,
    pub presentation_duration_us: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoColorMetadataSource {
    Mp4Nclx,
    H264Vui,
}

impl H264Mp4Index {
    pub fn open(path: &Path, expected_color: ColorSpace) -> Result<Self> {
        let metadata =
            std::fs::metadata(path).map_err(|error| MediaError::Other(error.to_string()))?;
        if metadata.len() > MAX_FILE_BYTES {
            return Err(MediaError::Unsupported("MP4 exceeds 2 GiB limit".into()));
        }
        let bytes = std::fs::read(path).map_err(|error| MediaError::Other(error.to_string()))?;
        Self::parse(&bytes, expected_color)
    }

    pub fn parse(bytes: &[u8], expected_color: ColorSpace) -> Result<Self> {
        let movie_timeline = parse_movie_timeline(bytes)?;
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
        let edit = movie_timeline.edit(track_id);
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
                let color_metadata_sources =
                    validate_color_signaling(&avc1.unknown_boxes, &avcc.sps_list, expected_color)?;
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
                    color_metadata_sources,
                ));
            }
            let (_, _, length_size, _, _) = config.as_ref().ok_or_else(|| {
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
            let cts = i64::try_from(sample.timestamp)
                .map_err(|_| MediaError::Other("video timestamp exceeds i64".into()))?
                .checked_add(sample.composition_time_offset.unwrap_or(0))
                .ok_or_else(|| MediaError::Other("video composition timestamp overflow".into()))?;
            let presentation_ticks = movie_timeline.presentation_ticks(cts, timescale, edit)?;
            samples.push(H264Sample {
                annexb: avcc_sample_to_annexb(data, *length_size, MAX_ACCESS_UNIT_BYTES)
                    .map_err(|error| MediaError::Other(error.to_string()))?,
                dts: sample.timestamp,
                pts: MediaTime::new(presentation_ticks, timebase),
                duration: MediaTime::new(i64::from(sample.duration), timebase),
                keyframe: sample.keyframe,
            });
        }
        let (width, height, _, decoder_preamble, color_metadata_sources) =
            config.ok_or_else(|| MediaError::Unsupported("avc1 configuration missing".into()))?;
        let sample_end_us = samples
            .last()
            .map(|sample| {
                media_time_us(sample.pts)?
                    .checked_add(media_time_us(sample.duration)?)
                    .ok_or_else(|| MediaError::Other("video duration overflow".into()))
            })
            .transpose()?
            .unwrap_or(0);
        Ok(Self {
            width,
            height,
            timescale,
            decoder_preamble,
            color_metadata_sources,
            samples,
            presentation_duration_us: movie_timeline.duration_us.max(sample_end_us),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ParsedColorSignal {
    primaries: u16,
    transfer: u16,
    matrix: u16,
    full_range: bool,
    source: VideoColorMetadataSource,
}

fn validate_color_signaling(
    boxes: &[shiguredo_mp4::boxes::UnknownBox],
    sps_list: &[Vec<u8>],
    expected_color: ColorSpace,
) -> Result<Vec<VideoColorMetadataSource>> {
    if expected_color != ColorSpace::Bt709Sdr {
        return Err(MediaError::Unsupported(format!(
            "H.264 file decode requires an explicit Bt709Sdr Project profile, got {expected_color:?}"
        )));
    }
    let mut signals = Vec::new();
    for color_box in boxes
        .iter()
        .filter(|child| child.box_type.as_bytes() == b"colr")
    {
        signals.push(parse_nclx(&color_box.payload)?);
    }
    for sps in sps_list {
        if let Some(signal) = parse_h264_vui_color(sps)? {
            signals.push(signal);
        }
    }
    if signals.is_empty() {
        return Err(MediaError::Unsupported(
            "H.264 MP4 has no colr/nclx or SPS VUI colour description; Bt709Sdr is not assumed"
                .into(),
        ));
    }
    for signal in &signals {
        if signal.primaries != 1 || signal.transfer != 1 || signal.matrix != 1 || signal.full_range
        {
            return Err(MediaError::Unsupported(format!(
                "{:?} color metadata is primaries={} transfer={} matrix={} full_range={}; explicit Bt709Sdr requires 1/1/1 limited range",
                signal.source, signal.primaries, signal.transfer, signal.matrix, signal.full_range
            )));
        }
    }
    let mut sources = Vec::new();
    for signal in signals {
        if !sources.contains(&signal.source) {
            sources.push(signal.source);
        }
    }
    Ok(sources)
}

fn parse_nclx(payload: &[u8]) -> Result<ParsedColorSignal> {
    if payload.len() < 11 {
        return Err(MediaError::Unsupported(
            "MP4 colr/nclx payload is truncated".into(),
        ));
    }
    if &payload[..4] != b"nclx" {
        return Err(MediaError::Unsupported(format!(
            "MP4 colr type {:?} is unsupported; explicit nclx metadata is required",
            &payload[..4]
        )));
    }
    if payload[10] & 0x7f != 0 {
        return Err(MediaError::Unsupported(
            "MP4 colr/nclx reserved range bits are non-zero".into(),
        ));
    }
    Ok(ParsedColorSignal {
        primaries: u16::from_be_bytes([payload[4], payload[5]]),
        transfer: u16::from_be_bytes([payload[6], payload[7]]),
        matrix: u16::from_be_bytes([payload[8], payload[9]]),
        full_range: payload[10] & 0x80 != 0,
        source: VideoColorMetadataSource::Mp4Nclx,
    })
}

fn parse_h264_vui_color(sps_nal: &[u8]) -> Result<Option<ParsedColorSignal>> {
    let Some((&header, encoded_rbsp)) = sps_nal.split_first() else {
        return Err(MediaError::Unsupported("empty H.264 SPS".into()));
    };
    if header & 0x1f != 7 {
        return Err(MediaError::Unsupported(
            "avcC sequence parameter set is not an SPS NAL".into(),
        ));
    }
    let mut rbsp = Vec::with_capacity(encoded_rbsp.len());
    let mut zeros = 0u8;
    for &byte in encoded_rbsp {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        rbsp.push(byte);
        zeros = if byte == 0 {
            zeros.saturating_add(1)
        } else {
            0
        };
    }
    let mut bits = BitReader::new(&rbsp);
    let profile_idc = bits.read_bits(8)?;
    bits.skip(16)?; // constraint flags/reserved_zero_2bits and level_idc
    bits.read_ue()?; // seq_parameter_set_id
    if matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        return Err(MediaError::Unsupported(
            "high-profile SPS is outside the explicit Constrained Baseline profile".into(),
        ));
    }
    bits.read_ue()?; // log2_max_frame_num_minus4
    match bits.read_ue()? {
        0 => {
            bits.read_ue()?;
        }
        1 => {
            bits.read_bit()?;
            bits.read_se()?;
            bits.read_se()?;
            let cycle = bits.read_ue()?;
            for _ in 0..cycle {
                bits.read_se()?;
            }
        }
        2 => {}
        value => {
            return Err(MediaError::Unsupported(format!(
                "invalid H.264 pic_order_cnt_type {value}"
            )));
        }
    }
    bits.read_ue()?; // max_num_ref_frames
    bits.read_bit()?; // gaps_in_frame_num_value_allowed_flag
    bits.read_ue()?; // pic_width_in_mbs_minus1
    bits.read_ue()?; // pic_height_in_map_units_minus1
    if !bits.read_bit()? {
        bits.read_bit()?; // mb_adaptive_frame_field_flag
    }
    bits.read_bit()?; // direct_8x8_inference_flag
    if bits.read_bit()? {
        for _ in 0..4 {
            bits.read_ue()?;
        }
    }
    if !bits.read_bit()? {
        return Ok(None);
    }
    if bits.read_bit()? {
        let aspect_ratio_idc = bits.read_bits(8)?;
        if aspect_ratio_idc == 255 {
            bits.skip(32)?;
        }
    }
    if bits.read_bit()? {
        bits.read_bit()?; // overscan_appropriate_flag
    }
    if !bits.read_bit()? {
        return Ok(None);
    }
    bits.skip(3)?; // video_format
    let full_range = bits.read_bit()?;
    if !bits.read_bit()? {
        return Ok(None);
    }
    Ok(Some(ParsedColorSignal {
        primaries: bits.read_bits(8)? as u16,
        transfer: bits.read_bits(8)? as u16,
        matrix: bits.read_bits(8)? as u16,
        full_range,
        source: VideoColorMetadataSource::H264Vui,
    }))
}

struct BitReader<'a> {
    bytes: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, bit: 0 }
    }

    fn read_bit(&mut self) -> Result<bool> {
        let byte = *self
            .bytes
            .get(self.bit / 8)
            .ok_or_else(|| MediaError::Unsupported("truncated H.264 SPS VUI".into()))?;
        let value = byte & (1 << (7 - self.bit % 8)) != 0;
        self.bit += 1;
        Ok(value)
    }

    fn read_bits(&mut self, count: usize) -> Result<u32> {
        if count > 32 {
            return Err(MediaError::Unsupported(
                "invalid H.264 bit-field width".into(),
            ));
        }
        let mut value = 0u32;
        for _ in 0..count {
            value = (value << 1) | u32::from(self.read_bit()?);
        }
        Ok(value)
    }

    fn skip(&mut self, count: usize) -> Result<()> {
        self.read_bits(count).map(|_| ())
    }

    fn read_ue(&mut self) -> Result<u32> {
        let mut zeros = 0usize;
        while !self.read_bit()? {
            zeros += 1;
            if zeros > 31 {
                return Err(MediaError::Unsupported(
                    "H.264 Exp-Golomb value is too large".into(),
                ));
            }
        }
        let suffix = self.read_bits(zeros)?;
        Ok((1u32 << zeros) - 1 + suffix)
    }

    fn read_se(&mut self) -> Result<i32> {
        let value = self.read_ue()?;
        let magnitude = value.div_ceil(2) as i32;
        Ok(if value.is_multiple_of(2) {
            -magnitude
        } else {
            magnitude
        })
    }
}

pub struct VideoFileSource {
    id: InputId,
    index: H264Mp4Index,
    sample_pts_us: Vec<u64>,
    binary_path: PathBuf,
    timeline: Arc<Mutex<PlaybackTimeline>>,
    state: Mutex<VideoState>,
}

struct VideoState {
    decoder: OpenH264Decoder,
    decoded_sample: Option<usize>,
    frame: Option<VideoFrame>,
    seen_generation: u64,
}

impl VideoFileSource {
    /// Opens an MP4 using only the explicitly supplied Cisco OpenH264 binary.
    ///
    /// The binary is loaded and verified first so its absence is always a hard
    /// construction error and cannot be hidden by another decoder or media path.
    pub fn open(
        id: InputId,
        path: &Path,
        binary_path: &Path,
        expected_color: ColorSpace,
        playback: Playback,
    ) -> Result<Self> {
        OpenH264Decoder::new(binary_path).map_err(|error| MediaError::Other(error.to_string()))?;
        let index = H264Mp4Index::open(path, expected_color)?;
        let timeline = Arc::new(Mutex::new(PlaybackTimeline::new(
            playback,
            index.presentation_duration_us,
            false,
        )?));
        Self::from_index(id, index, binary_path, timeline)
    }

    pub(crate) fn from_index(
        id: InputId,
        index: H264Mp4Index,
        binary_path: &Path,
        timeline: Arc<Mutex<PlaybackTimeline>>,
    ) -> Result<Self> {
        let mut decoder = OpenH264Decoder::new(binary_path)
            .map_err(|error| MediaError::Other(error.to_string()))?;
        if index.samples.is_empty() {
            return Err(MediaError::Unsupported(
                "H.264 MP4 has no video samples".into(),
            ));
        }
        if !index.samples.iter().any(|sample| sample.keyframe) {
            return Err(MediaError::Unsupported(
                "H.264 MP4 has no sync sample".into(),
            ));
        }
        feed_preamble(&mut decoder, &index, id)?;
        let sample_pts_us = index
            .samples
            .iter()
            .map(|sample| media_time_us(sample.pts))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            id,
            index,
            sample_pts_us,
            binary_path: binary_path.to_path_buf(),
            timeline,
            state: Mutex::new(VideoState {
                decoder,
                decoded_sample: None,
                frame: None,
                seen_generation: 0,
            }),
        })
    }

    pub fn set_playback(&self, playback: Playback) -> Result<()> {
        self.set_playback_mode(playback, false)
    }

    pub(crate) fn set_playback_mode(&self, playback: Playback, has_audio: bool) -> Result<()> {
        self.timeline
            .lock()
            .map_err(|_| MediaError::Other("video playback lock poisoned".into()))?
            .apply(playback, has_audio)
    }

    pub fn playback(&self) -> Result<Playback> {
        let timeline = self
            .timeline
            .lock()
            .map_err(|_| MediaError::Other("video playback lock poisoned".into()))?;
        Ok(timeline.playback())
    }

    fn reset_decoder(&self, state: &mut VideoState) -> Result<()> {
        let mut decoder = OpenH264Decoder::new(&self.binary_path)
            .map_err(|error| MediaError::Other(error.to_string()))?;
        feed_preamble(&mut decoder, &self.index, self.id)?;
        state.decoder = decoder;
        state.decoded_sample = None;
        state.frame = None;
        Ok(())
    }

    fn decode_to(&self, state: &mut VideoState, target: usize, reset: bool) -> Result<VideoFrame> {
        if reset || state.decoded_sample.is_some_and(|decoded| target < decoded) {
            self.reset_decoder(state)?;
        }
        if state.decoded_sample == Some(target) {
            return state
                .frame
                .clone()
                .ok_or_else(|| MediaError::Other("decoded sample has no frame".into()));
        }

        let start = match state.decoded_sample {
            Some(decoded) => decoded.saturating_add(1),
            None => self
                .index
                .samples
                .iter()
                .enumerate()
                .take(target.saturating_add(1))
                .filter(|(_, sample)| sample.keyframe)
                .map(|(index, _)| index)
                .next_back()
                .ok_or_else(|| {
                    MediaError::Unsupported("no sync sample at or before playback position".into())
                })?,
        };
        for sample_index in start..=target {
            let sample = &self.index.samples[sample_index];
            if let Some(frame) = state
                .decoder
                .decode_bt709_limited(&sample.annexb, sample_index as u64, self.id, sample.pts)
                .map_err(|error| MediaError::Other(error.to_string()))?
            {
                if frame.width != u32::from(self.index.width)
                    || frame.height != u32::from(self.index.height)
                {
                    return Err(MediaError::Unsupported(format!(
                        "decoded dimensions {}x{} do not match avc1 {}x{}",
                        frame.width, frame.height, self.index.width, self.index.height
                    )));
                }
                state.frame = Some(frame);
            }
            state.decoded_sample = Some(sample_index);
        }
        state.frame.clone().ok_or_else(|| {
            MediaError::Other("OpenH264 produced no frame for playback position".into())
        })
    }
}

impl MediaSource for VideoFileSource {
    fn id(&self) -> InputId {
        self.id
    }

    fn pull_video(&self, pts: MediaTime, _rate: FrameRate) -> Result<Option<VideoFrame>> {
        let clock_us = media_time_us(pts)?;
        let step = self
            .timeline
            .lock()
            .map_err(|_| MediaError::Other("video playback lock poisoned".into()))?
            .resolve(clock_us);
        let mut state = self
            .state
            .lock()
            .map_err(|_| MediaError::Other("video playback lock poisoned".into()))?;
        let discontinuity = state.seen_generation != step.generation;
        state.seen_generation = step.generation;
        let target = self
            .sample_pts_us
            .partition_point(|sample_pts| *sample_pts <= step.position_us)
            .saturating_sub(1)
            .min(self.index.samples.len().saturating_sub(1));
        let mut frame = self.decode_to(&mut state, target, discontinuity)?;
        let source_pts = frame.pts;
        frame.pts = pts;
        frame.capture_domain = ClockDomain::SourceMedia;
        frame.clock_observation = Some(ClockObservation {
            source: ClockTimestamp::from_media(ClockDomain::SourceMedia, source_pts)
                .map_err(|error| MediaError::Other(error.to_string()))?,
            target: ClockTimestamp::from_media(ClockDomain::Virtual, pts)
                .map_err(|error| MediaError::Other(error.to_string()))?,
            discontinuity,
        });
        frame.source = Some(self.id);
        frame.discontinuity = discontinuity;
        Ok(Some(frame))
    }

    fn pull_audio(
        &self,
        _sample_index: u64,
        _frames: usize,
    ) -> Result<Option<eiviz_media::AudioBuffer>> {
        Ok(None)
    }

    fn update_playback(&self, playback: &Playback) {
        let _ = self.set_playback(playback.clone());
    }
}

fn feed_preamble(decoder: &mut OpenH264Decoder, index: &H264Mp4Index, id: InputId) -> Result<()> {
    let frame = decoder
        .decode_bt709_limited(&index.decoder_preamble, 0, id, MediaTime::ZERO)
        .map_err(|error| MediaError::Other(error.to_string()))?;
    if frame.is_some() {
        return Err(MediaError::Other(
            "OpenH264 unexpectedly emitted a frame from SPS/PPS".into(),
        ));
    }
    Ok(())
}

fn media_time_us(time: MediaTime) -> Result<u64> {
    if time.ticks() <= 0 {
        return Ok(0);
    }
    let micros = i128::from(time.ticks())
        .checked_mul(i128::from(time.timebase().numerator()))
        .and_then(|value| value.checked_mul(1_000_000))
        .ok_or_else(|| MediaError::Other("media timestamp overflow".into()))?
        / i128::from(time.timebase().denominator());
    u64::try_from(micros).map_err(|_| MediaError::Other("media timestamp overflow".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_input_is_rejected() {
        assert!(H264Mp4Index::parse(b"not an mp4", ColorSpace::Bt709Sdr).is_err());
    }

    #[test]
    fn missing_openh264_binary_fails_before_media_fallback() {
        let missing_binary =
            std::env::temp_dir().join(format!("eiviz-missing-openh264-{}", std::process::id()));
        let missing_video =
            std::env::temp_dir().join(format!("eiviz-missing-video-{}", std::process::id()));
        let result = VideoFileSource::open(
            InputId::new(),
            &missing_video,
            &missing_binary,
            ColorSpace::Bt709Sdr,
            Playback::default(),
        );
        let error = match result {
            Ok(_) => panic!("missing OpenH264 must not construct a source"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("Cisco OpenH264 2.6.0"));
        assert!(
            error
                .to_string()
                .contains(missing_binary.to_string_lossy().as_ref())
        );
    }

    #[test]
    #[ignore = "requires explicit Cisco binary and representative MP4 paths"]
    fn decodes_real_openh264_binary_and_mp4() {
        let binary =
            std::env::var_os("EIVIZ_OPENH264_HIL_BINARY").expect("set EIVIZ_OPENH264_HIL_BINARY");
        let mp4 = std::env::var_os("EIVIZ_OPENH264_HIL_MP4").expect("set EIVIZ_OPENH264_HIL_MP4");
        let source = VideoFileSource::open(
            InputId::new(),
            Path::new(&mp4),
            Path::new(&binary),
            ColorSpace::Bt709Sdr,
            Playback::default(),
        )
        .unwrap();
        let frame = source
            .pull_video(MediaTime::ZERO, eiviz_time::NTSC_5994)
            .unwrap()
            .expect("decoder must emit an authentic frame");
        assert_eq!(frame.format, eiviz_media::PixelFormat::Rgba8);
        assert_eq!(
            frame.data.len(),
            frame.width as usize * frame.height as usize * 4
        );
    }

    #[test]
    fn nclx_and_vui_require_explicit_limited_bt709() {
        assert_eq!(
            parse_nclx(b"nclx\0\x01\0\x01\0\x01\0").unwrap(),
            ParsedColorSignal {
                primaries: 1,
                transfer: 1,
                matrix: 1,
                full_range: false,
                source: VideoColorMetadataSource::Mp4Nclx,
            }
        );
        let full_range = parse_nclx(b"nclx\0\x01\0\x01\0\x01\x80").unwrap();
        assert!(full_range.full_range);

        let sps = baseline_sps_with_vui(1, 1, 1, false);
        assert_eq!(
            parse_h264_vui_color(&sps).unwrap(),
            Some(ParsedColorSignal {
                primaries: 1,
                transfer: 1,
                matrix: 1,
                full_range: false,
                source: VideoColorMetadataSource::H264Vui,
            })
        );
        assert_eq!(
            validate_color_signaling(&[], &[sps], ColorSpace::Bt709Sdr).unwrap(),
            [VideoColorMetadataSource::H264Vui]
        );
        assert!(
            validate_color_signaling(
                &[],
                &[baseline_sps_with_vui(1, 1, 6, false)],
                ColorSpace::Bt709Sdr
            )
            .unwrap_err()
            .to_string()
            .contains("matrix=6")
        );
    }

    #[test]
    fn absent_color_metadata_is_rejected_instead_of_assuming_bt709() {
        let error = validate_color_signaling(&[], &[], ColorSpace::Bt709Sdr).unwrap_err();
        assert!(error.to_string().contains("is not assumed"));
        let error = validate_color_signaling(&[], &[], ColorSpace::Bt2020Pq).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("explicit Bt709Sdr Project profile")
        );
    }

    fn baseline_sps_with_vui(primaries: u8, transfer: u8, matrix: u8, full_range: bool) -> Vec<u8> {
        let mut bits = TestBitWriter::default();
        bits.write_bits(66, 8);
        bits.write_bits(0x40, 8);
        bits.write_bits(30, 8);
        bits.write_ue(0); // seq_parameter_set_id
        bits.write_ue(0); // log2_max_frame_num_minus4
        bits.write_ue(0); // pic_order_cnt_type
        bits.write_ue(0); // log2_max_pic_order_cnt_lsb_minus4
        bits.write_ue(1); // max_num_ref_frames
        bits.write_bit(false); // gaps
        bits.write_ue(0); // width
        bits.write_ue(0); // height
        bits.write_bit(true); // frame_mbs_only
        bits.write_bit(true); // direct_8x8
        bits.write_bit(false); // crop
        bits.write_bit(true); // vui
        bits.write_bit(false); // aspect ratio
        bits.write_bit(false); // overscan
        bits.write_bit(true); // video signal type
        bits.write_bits(5, 3); // unspecified video format
        bits.write_bit(full_range);
        bits.write_bit(true); // colour description
        bits.write_bits(u32::from(primaries), 8);
        bits.write_bits(u32::from(transfer), 8);
        bits.write_bits(u32::from(matrix), 8);
        let mut sps = vec![0x67];
        sps.extend(bits.finish());
        sps
    }

    #[derive(Default)]
    struct TestBitWriter {
        bytes: Vec<u8>,
        current: u8,
        count: u8,
    }

    impl TestBitWriter {
        fn write_bit(&mut self, value: bool) {
            self.current = (self.current << 1) | u8::from(value);
            self.count += 1;
            if self.count == 8 {
                self.bytes.push(self.current);
                self.current = 0;
                self.count = 0;
            }
        }

        fn write_bits(&mut self, value: u32, count: usize) {
            for shift in (0..count).rev() {
                self.write_bit(value & (1 << shift) != 0);
            }
        }

        fn write_ue(&mut self, value: u32) {
            let encoded = value + 1;
            let bits = 32 - encoded.leading_zeros();
            for _ in 1..bits {
                self.write_bit(false);
            }
            self.write_bits(encoded, bits as usize);
        }

        fn finish(mut self) -> Vec<u8> {
            self.write_bit(true);
            while self.count != 0 {
                self.write_bit(false);
            }
            self.bytes
        }
    }
}
