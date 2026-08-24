use eiviz_codec_software::{OpenH264Decoder, avcc_parameter_sets_to_annexb, avcc_sample_to_annexb};
use eiviz_core::{InputId, Playback};
use eiviz_media::{MediaError, MediaSource, Result, VideoFrame};
use eiviz_time::{ClockDomain, FrameRate, MediaTime, Rational};
use shiguredo_mp4::{
    TrackKind,
    boxes::SampleEntry,
    demux::{Input, Mp4FileDemuxer},
};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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

pub struct VideoFileSource {
    id: InputId,
    index: H264Mp4Index,
    sample_pts_us: Vec<u64>,
    binary_path: PathBuf,
    state: Mutex<VideoState>,
}

struct VideoState {
    decoder: OpenH264Decoder,
    cursor: PlaybackCursor,
    decoded_sample: Option<usize>,
    frame: Option<VideoFrame>,
}

impl VideoFileSource {
    /// Opens an MP4 using only the explicitly supplied Cisco OpenH264 binary.
    ///
    /// The binary is loaded and verified first so its absence is always a hard
    /// construction error and cannot be hidden by another decoder or media path.
    pub fn open(id: InputId, path: &Path, binary_path: &Path, playback: Playback) -> Result<Self> {
        let mut decoder = OpenH264Decoder::new(binary_path)
            .map_err(|error| MediaError::Other(error.to_string()))?;
        let index = H264Mp4Index::open(path)?;
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
        let final_sample = index.samples.last().expect("samples checked non-empty");
        let media_end_us = media_time_us(final_sample.pts)?
            .checked_add(media_time_us(final_sample.duration)?)
            .ok_or_else(|| MediaError::Other("video duration overflow".into()))?;
        let cursor = PlaybackCursor::new(playback, media_end_us)?;

        Ok(Self {
            id,
            index,
            sample_pts_us,
            binary_path: binary_path.to_path_buf(),
            state: Mutex::new(VideoState {
                decoder,
                cursor,
                decoded_sample: None,
                frame: None,
            }),
        })
    }

    pub fn set_playback(&self, playback: Playback) -> Result<()> {
        self.state
            .lock()
            .map_err(|_| MediaError::Other("video playback lock poisoned".into()))?
            .cursor
            .apply(playback)
    }

    pub fn playback(&self) -> Result<Playback> {
        let state = self
            .state
            .lock()
            .map_err(|_| MediaError::Other("video playback lock poisoned".into()))?;
        let mut playback = state.cursor.config.clone();
        playback.position_us = state.cursor.position_us;
        Ok(playback)
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
        let mut state = self
            .state
            .lock()
            .map_err(|_| MediaError::Other("video playback lock poisoned".into()))?;
        let step = state.cursor.tick(clock_us);
        let target = self
            .sample_pts_us
            .partition_point(|sample_pts| *sample_pts <= step.position_us)
            .saturating_sub(1)
            .min(self.index.samples.len().saturating_sub(1));
        let mut frame = self.decode_to(&mut state, target, step.discontinuity)?;
        frame.pts = pts;
        frame.capture_domain = ClockDomain::SourceMedia;
        frame.source = Some(self.id);
        frame.discontinuity = step.discontinuity;
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

#[derive(Clone, Copy, Debug)]
struct CursorStep {
    position_us: u64,
    discontinuity: bool,
}

#[derive(Debug)]
struct PlaybackCursor {
    config: Playback,
    media_end_us: u64,
    position_us: u64,
    last_clock_us: Option<u64>,
    fractional_us: f64,
    seek_pending: bool,
}

impl PlaybackCursor {
    fn new(config: Playback, media_end_us: u64) -> Result<Self> {
        validate_playback(&config, media_end_us)?;
        let position_us = clamp_position(&config, media_end_us, config.position_us);
        Ok(Self {
            config,
            media_end_us,
            position_us,
            last_clock_us: None,
            fractional_us: 0.0,
            seek_pending: false,
        })
    }

    fn apply(&mut self, config: Playback) -> Result<()> {
        validate_playback(&config, self.media_end_us)?;
        if config.position_us != self.config.position_us
            || config.in_us != self.config.in_us
            || config.out_us != self.config.out_us
        {
            self.position_us = clamp_position(&config, self.media_end_us, config.position_us);
            self.fractional_us = 0.0;
            self.seek_pending = true;
        }
        self.config = config;
        Ok(())
    }

    fn tick(&mut self, clock_us: u64) -> CursorStep {
        let elapsed = self
            .last_clock_us
            .replace(clock_us)
            .map(|last| clock_us.saturating_sub(last))
            .unwrap_or(0);
        let mut discontinuity = std::mem::take(&mut self.seek_pending);
        if self.config.playing && elapsed > 0 {
            let scaled = elapsed as f64 * f64::from(self.config.speed) + self.fractional_us;
            let advance = scaled.floor().max(0.0) as u64;
            self.fractional_us = scaled - advance as f64;
            let end = playback_end(&self.config, self.media_end_us);
            let next = self.position_us.saturating_add(advance);
            if next >= end {
                if self.config.loop_playback {
                    let length = end.saturating_sub(self.config.in_us);
                    self.position_us = if length == 0 {
                        self.config.in_us
                    } else {
                        self.config.in_us + next.saturating_sub(self.config.in_us) % length
                    };
                    discontinuity = true;
                } else {
                    self.position_us = end.saturating_sub(1).max(self.config.in_us);
                    self.config.playing = false;
                }
            } else {
                self.position_us = next;
            }
        }
        CursorStep {
            position_us: self.position_us,
            discontinuity,
        }
    }
}

fn validate_playback(playback: &Playback, media_end_us: u64) -> Result<()> {
    if !playback.speed.is_finite() || playback.speed <= 0.0 {
        return Err(MediaError::Unsupported(
            "video playback speed must be finite and greater than zero".into(),
        ));
    }
    let end = playback_end(playback, media_end_us);
    if playback.in_us >= end {
        return Err(MediaError::Unsupported(
            "video playback out point must be after in point".into(),
        ));
    }
    Ok(())
}

fn playback_end(playback: &Playback, media_end_us: u64) -> u64 {
    playback.out_us.unwrap_or(media_end_us).min(media_end_us)
}

fn clamp_position(playback: &Playback, media_end_us: u64, position_us: u64) -> u64 {
    let end = playback_end(playback, media_end_us);
    position_us
        .max(playback.in_us)
        .min(end.saturating_sub(1).max(playback.in_us))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_input_is_rejected() {
        assert!(H264Mp4Index::parse(b"not an mp4").is_err());
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
    fn playback_cursor_pauses_seeks_and_loops_without_decoder() {
        let mut cursor = PlaybackCursor::new(
            Playback {
                playing: true,
                loop_playback: true,
                position_us: 100,
                in_us: 100,
                out_us: Some(400),
                speed: 1.0,
            },
            1_000,
        )
        .unwrap();
        assert_eq!(cursor.tick(1_000).position_us, 100);
        assert_eq!(cursor.tick(1_150).position_us, 250);
        let wrapped = cursor.tick(1_350);
        assert_eq!(wrapped.position_us, 150);
        assert!(wrapped.discontinuity);

        let mut paused = cursor.config.clone();
        paused.playing = false;
        cursor.apply(paused.clone()).unwrap();
        assert_eq!(cursor.tick(2_000).position_us, 150);

        paused.position_us = 300;
        cursor.apply(paused).unwrap();
        let seek = cursor.tick(2_100);
        assert_eq!(seek.position_us, 300);
        assert!(seek.discontinuity);
    }

    #[test]
    fn non_looping_cursor_holds_last_position() {
        let mut cursor = PlaybackCursor::new(
            Playback {
                playing: true,
                loop_playback: false,
                position_us: 0,
                in_us: 0,
                out_us: Some(100),
                speed: 1.0,
            },
            1_000,
        )
        .unwrap();
        cursor.tick(0);
        assert_eq!(cursor.tick(200).position_us, 99);
        assert!(!cursor.config.playing);
        assert_eq!(cursor.tick(1_000).position_us, 99);
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
}
