use eiviz_core::InputId;
use eiviz_media::{AdapterHealth, AudioBuffer, MediaError, MediaSink, MediaSource, VideoFrame};
use eiviz_time::{
    ClockDomain, ClockObservation, ClockTimestamp, FrameRate, MediaTime, Rational, monotonic_nanos,
};
use openmediatransport::{
    ColorSpace, Discovery, FrameType, MediaFrame, ReceiverConfig, ReceiverSession, Sender,
    SessionState, VideoFlags,
};
use parking_lot::Mutex;

const OMT_TICKS_PER_SECOND: i64 = 10_000_000;

#[derive(Debug, thiserror::Error)]
pub enum OmtError {
    #[error(transparent)]
    Protocol(#[from] openmediatransport::OmtError),
    #[error("invalid OMT frame: {0}")]
    InvalidFrame(String),
}

pub struct OmtSource {
    id: InputId,
    address: String,
    session: Mutex<ReceiverSession>,
    last_video: Mutex<Option<VideoFrame>>,
    last_error: Mutex<Option<String>>,
}

impl OmtSource {
    pub fn connect(id: InputId, address: impl Into<String>) -> Result<Self, OmtError> {
        let address = address.into();
        let session = ReceiverSession::connect(
            address.clone(),
            ReceiverConfig {
                frame_types: FrameType::VIDEO | FrameType::AUDIO | FrameType::METADATA,
                ..ReceiverConfig::default()
            },
        )?;
        Ok(Self {
            id,
            address,
            session: Mutex::new(session),
            last_video: Mutex::new(None),
            last_error: Mutex::new(None),
        })
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn health(&self) -> AdapterHealth {
        match self.session.lock().state() {
            SessionState::Connected => AdapterHealth::Running,
            SessionState::Connecting | SessionState::Reconnecting => AdapterHealth::Degraded,
            SessionState::Stopping | SessionState::Stopped => AdapterHealth::Unavailable,
        }
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().clone()
    }
}

impl MediaSource for OmtSource {
    fn id(&self) -> InputId {
        self.id
    }

    fn pull_video(
        &self,
        _pts: MediaTime,
        _rate: FrameRate,
    ) -> eiviz_media::Result<Option<VideoFrame>> {
        if let Some(frame) = self.session.lock().try_recv_video() {
            let mut converted = convert_video(self.id, frame).map_err(MediaError::from)?;
            let mut last_video = self.last_video.lock();
            converted.discontinuity = last_video
                .as_ref()
                .is_some_and(|previous| converted.pts <= previous.pts);
            if let Some(observation) = converted.clock_observation.as_mut() {
                observation.discontinuity = converted.discontinuity;
            }
            *last_video = Some(converted.clone());
            return Ok(Some(converted));
        }
        Ok(self.last_video.lock().clone())
    }

    fn pull_audio(
        &self,
        _sample_index: u64,
        _frames: usize,
    ) -> eiviz_media::Result<Option<AudioBuffer>> {
        self.session
            .lock()
            .try_recv_audio()
            .map(convert_audio)
            .transpose()
            .map_err(MediaError::from)
    }
}

fn convert_video(
    id: InputId,
    frame: openmediatransport::DecodedVideoFrame,
) -> Result<VideoFrame, OmtError> {
    if frame.width == 0 || frame.height == 0 || frame.stride < frame.width * 4 {
        return Err(OmtError::InvalidFrame("invalid BGRA shape".into()));
    }
    let required = frame.stride as usize * frame.height as usize;
    if frame.pixels.len() < required {
        return Err(OmtError::InvalidFrame("truncated BGRA frame".into()));
    }
    let mut rgba = Vec::with_capacity(frame.width as usize * frame.height as usize * 4);
    for row in 0..frame.height as usize {
        let start = row * frame.stride as usize;
        rgba.extend(openmediatransport::bgra_to_rgba(
            &frame.pixels[start..start + frame.width as usize * 4],
        ));
    }
    let pts = MediaTime::new(
        frame.timestamp,
        Rational::new(1, OMT_TICKS_PER_SECOND).expect("constant"),
    );
    let clock_observation = ClockObservation {
        source: ClockTimestamp::from_media(ClockDomain::SourceMedia, pts)
            .map_err(|error| OmtError::InvalidFrame(error.to_string()))?,
        target: ClockTimestamp::nanoseconds(ClockDomain::Monotonic, monotonic_nanos())
            .map_err(|error| OmtError::InvalidFrame(error.to_string()))?,
        discontinuity: false,
    };
    Ok(VideoFrame {
        id: frame.timestamp.max(0) as u64,
        source: Some(id),
        pts,
        capture_domain: ClockDomain::SourceMedia,
        clock_observation: Some(clock_observation),
        width: frame.width,
        height: frame.height,
        format: eiviz_media::PixelFormat::Rgba8,
        data: rgba.into(),
        discontinuity: false,
    })
}

fn convert_audio(frame: openmediatransport::DecodedAudioFrame) -> Result<AudioBuffer, OmtError> {
    if frame.sample_rate <= 0 || frame.channels <= 0 || frame.samples_per_channel <= 0 {
        return Err(OmtError::InvalidFrame("invalid FPA1 shape".into()));
    }
    let channels = frame.channels as usize;
    let samples = frame.samples_per_channel as usize;
    let required = channels * samples * 4;
    if frame.pcm_planar_f32.len() < required {
        return Err(OmtError::InvalidFrame("truncated FPA1 frame".into()));
    }
    let mut planes = vec![vec![0.0; samples]; channels];
    for (channel, plane) in planes.iter_mut().enumerate() {
        let offset = channel * samples * 4;
        for (sample, bytes) in plane
            .iter_mut()
            .zip(frame.pcm_planar_f32[offset..offset + samples * 4].chunks_exact(4))
        {
            *sample = f32::from_le_bytes(bytes.try_into().expect("four bytes"));
        }
    }
    let sample_index = (frame.timestamp.max(0) as u128 * frame.sample_rate as u128
        / OMT_TICKS_PER_SECOND as u128) as u64;
    let capture_nanos = monotonic_nanos();
    Ok(AudioBuffer {
        sample_index,
        sample_rate: frame.sample_rate as u32,
        channels: frame.channels as u16,
        planes,
        capture_timestamp: Some(eiviz_media::AudioCaptureTimestamp {
            device_sample_index: sample_index,
            callback_nanos: capture_nanos,
            capture_nanos,
        }),
        discontinuity: false,
    })
}

pub struct OmtSink {
    name: String,
    frame_rate: FrameRate,
    sender: Mutex<Sender>,
}

impl OmtSink {
    pub fn create(name: impl Into<String>, frame_rate: FrameRate) -> Result<Self, OmtError> {
        let name = name.into();
        let sender = Sender::create(name.clone(), FrameType::VIDEO | FrameType::AUDIO)?;
        Ok(Self {
            name,
            frame_rate,
            sender: Mutex::new(sender),
        })
    }
}

impl MediaSink for OmtSink {
    fn name(&self) -> &str {
        &self.name
    }

    fn push_video(&self, frame: &VideoFrame) -> eiviz_media::Result<()> {
        let mut bgra = Vec::with_capacity(frame.data.len());
        for pixel in frame.data.chunks_exact(4) {
            bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
        let media = MediaFrame {
            frame_type: FrameType::VIDEO,
            timestamp: media_time_to_omt(frame.pts),
            codec: openmediatransport::Codec::Bgra as i32,
            width: frame.width as i32,
            height: frame.height as i32,
            stride: frame.width.saturating_mul(4) as i32,
            flags: VideoFlags::ALPHA,
            frame_rate_n: self.frame_rate.numerator() as i32,
            frame_rate_d: self.frame_rate.denominator() as i32,
            aspect_ratio: frame.width as f32 / frame.height.max(1) as f32,
            color_space: ColorSpace::Bt709,
            data: bgra,
            ..MediaFrame::default()
        };
        self.sender
            .lock()
            .send_video(media)
            .map_err(|error| MediaError::Other(error.to_string()))
    }

    fn push_audio(&self, audio: &AudioBuffer) -> eiviz_media::Result<()> {
        let mut data = Vec::new();
        for plane in &audio.planes {
            for sample in plane {
                data.extend_from_slice(&sample.to_le_bytes());
            }
        }
        let media = MediaFrame {
            frame_type: FrameType::AUDIO,
            timestamp: (audio.sample_index as u128 * OMT_TICKS_PER_SECOND as u128
                / audio.sample_rate as u128) as i64,
            codec: openmediatransport::Codec::Fpa1 as i32,
            sample_rate: audio.sample_rate as i32,
            channels: audio.channels as i32,
            samples_per_channel: audio.planes.first().map_or(0, Vec::len) as i32,
            active_channels: (1u32 << audio.channels.min(32)) - 1,
            data,
            ..MediaFrame::default()
        };
        self.sender
            .lock()
            .send_audio(media)
            .map_err(|error| MediaError::Other(error.to_string()))
    }
}

pub fn discover_sources() -> Result<Vec<String>, OmtError> {
    let mut discovery = Discovery::new()?;
    discovery.refresh()?;
    Ok(discovery
        .sources()
        .iter()
        .map(|source| source.to_url())
        .collect())
}

fn media_time_to_omt(time: MediaTime) -> i64 {
    let value =
        time.ticks() as i128 * time.timebase().numerator() as i128 * OMT_TICKS_PER_SECOND as i128
            / time.timebase().denominator() as i128;
    value.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

impl From<OmtError> for MediaError {
    fn from(value: OmtError) -> Self {
        MediaError::Other(value.to_string())
    }
}
