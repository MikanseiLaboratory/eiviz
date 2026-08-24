use eiviz_core::{ColorMetadata, FieldKind, InputId, VideoFormat};
use eiviz_media::{
    AdapterHealth, AudioBuffer, BoundedMetadataQueue, InputTally, MediaError, MediaSink,
    MediaSource, PixelFormat, SourceControlDiagnostics, SourceMetadata, VideoFrame,
};
use eiviz_time::{
    ClockDomain, ClockObservation, ClockTimestamp, FrameRate, MediaTime, Rational, monotonic_nanos,
};
use openmediatransport::{
    ColorSpace, Discovery, FrameType, MediaFrame, Metadata, ReceiverConfig, ReceiverSession,
    Sender, SenderConfig, SessionState, Tally, VideoFlags,
};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

const OMT_TICKS_PER_SECOND: i64 = 10_000_000;
const OMT_METADATA_CAPACITY: usize = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OmtColorProfile {
    Bt601Limited,
    Bt709Limited,
}

impl OmtColorProfile {
    fn omt(self) -> ColorSpace {
        match self {
            Self::Bt601Limited => ColorSpace::Bt601,
            Self::Bt709Limited => ColorSpace::Bt709,
        }
    }

    fn metadata(self) -> ColorMetadata {
        match self {
            Self::Bt601Limited => eiviz_core::ColorMetadata {
                matrix: eiviz_core::ColorMatrix::Bt601,
                range: eiviz_core::ColorRange::Limited,
                transfer: eiviz_core::TransferFunction::Bt709,
            },
            Self::Bt709Limited => eiviz_core::ColorSpace::Bt709Sdr.metadata(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OmtOutputPixelFormat {
    Bgra,
    Uyvy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OmtOutputConfig {
    pub pixel_format: OmtOutputPixelFormat,
    pub color_profile: OmtColorProfile,
    pub send_queue_depth: usize,
}

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
    metadata: BoundedMetadataQueue,
    last_video_reconnects: Mutex<u64>,
    last_audio: Mutex<Option<(u64, u64)>>,
    discontinuities: AtomicU64,
    metadata_received: AtomicU64,
    tally_updates: AtomicU64,
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
            metadata: BoundedMetadataQueue::new(OMT_METADATA_CAPACITY),
            last_video_reconnects: Mutex::new(0),
            last_audio: Mutex::new(None),
            discontinuities: AtomicU64::new(0),
            metadata_received: AtomicU64::new(0),
            tally_updates: AtomicU64::new(0),
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
        self.last_error
            .lock()
            .clone()
            .or_else(|| self.session.lock().last_error())
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
        let (frame, state, reconnects) = {
            let session = self.session.lock();
            (
                session.try_recv_video(),
                session.state(),
                session.statistics().reconnects,
            )
        };
        if let Some(frame) = frame {
            if let Some(metadata) = frame.frame_metadata.clone() {
                self.queue_metadata(frame.timestamp, metadata);
            }
            let mut converted = convert_video(self.id, frame).map_err(MediaError::from)?;
            let mut last_video = self.last_video.lock();
            let mut last_reconnects = self.last_video_reconnects.lock();
            converted.discontinuity = reconnects > *last_reconnects
                || last_video
                    .as_ref()
                    .is_some_and(|previous| converted.pts <= previous.pts);
            if let Some(observation) = converted.clock_observation.as_mut() {
                observation.discontinuity = converted.discontinuity;
            }
            if converted.discontinuity {
                self.discontinuities.fetch_add(1, Ordering::Relaxed);
            }
            *last_reconnects = reconnects;
            *last_video = Some(converted.clone());
            return Ok(Some(converted));
        }
        if state != SessionState::Connected {
            return Ok(None);
        }
        Ok(self.last_video.lock().clone())
    }

    fn pull_audio(
        &self,
        _sample_index: u64,
        _frames: usize,
    ) -> eiviz_media::Result<Option<AudioBuffer>> {
        let (frame, reconnects) = {
            let session = self.session.lock();
            (session.try_recv_audio(), session.statistics().reconnects)
        };
        let Some(frame) = frame else {
            return Ok(None);
        };
        if let Some(metadata) = frame.frame_metadata.clone() {
            self.queue_metadata(frame.timestamp, metadata);
        }
        let mut converted = convert_audio(frame).map_err(MediaError::from)?;
        let mut last_audio = self.last_audio.lock();
        let previous_reconnects = last_audio.map_or(0, |(_, count)| count);
        converted.discontinuity = reconnects > previous_reconnects
            || last_audio.is_some_and(|(sample_index, _)| converted.sample_index <= sample_index);
        if converted.discontinuity {
            self.discontinuities.fetch_add(1, Ordering::Relaxed);
        }
        *last_audio = Some((converted.sample_index, reconnects));
        Ok(Some(converted))
    }

    fn supports_tally(&self) -> bool {
        true
    }

    fn set_tally(&self, tally: InputTally) -> eiviz_media::Result<()> {
        self.session
            .lock()
            .set_tally(Tally::new(
                i32::from(tally.preview),
                i32::from(tally.program),
            ))
            .map_err(OmtError::from)
            .map_err(MediaError::from)?;
        self.tally_updates.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn poll_metadata(&self) -> eiviz_media::Result<Vec<SourceMetadata>> {
        {
            let session = self.session.lock();
            while let Some(frame) = session.try_recv_metadata() {
                self.queue_metadata(frame.timestamp, frame.xml);
            }
        }
        Ok(self.metadata.drain())
    }

    fn control_diagnostics(&self) -> Option<SourceControlDiagnostics> {
        let reconnects = self.session.lock().statistics().reconnects;
        Some(SourceControlDiagnostics {
            reconnects,
            discontinuities: self.discontinuities.load(Ordering::Relaxed),
            metadata_received: self.metadata_received.load(Ordering::Relaxed),
            metadata_dropped: self.metadata.dropped(),
            tally_updates: self.tally_updates.load(Ordering::Relaxed),
        })
    }
}

impl OmtSource {
    fn queue_metadata(&self, timestamp: i64, xml: impl Into<std::sync::Arc<str>>) {
        let xml = xml.into();
        self.metadata_received.fetch_add(1, Ordering::Relaxed);
        self.metadata
            .push(convert_metadata(self.id, timestamp, xml));
    }
}

fn convert_video(
    id: InputId,
    frame: openmediatransport::DecodedVideoFrame,
) -> Result<VideoFrame, OmtError> {
    match frame.color_space {
        ColorSpace::Bt601 | ColorSpace::Bt709 => {}
        ColorSpace::Undefined => {
            return Err(OmtError::InvalidFrame(
                "OMT video color space is undefined; implicit SD/HD matrix selection is forbidden"
                    .into(),
            ));
        }
    }
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
        color: match frame.color_space {
            ColorSpace::Bt601 => OmtColorProfile::Bt601Limited.metadata(),
            ColorSpace::Bt709 => OmtColorProfile::Bt709Limited.metadata(),
            ColorSpace::Undefined => unreachable!("rejected above"),
        },
        field: FieldKind::Progressive,
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

fn convert_metadata(input: InputId, timestamp: i64, xml: std::sync::Arc<str>) -> SourceMetadata {
    let categories = openmediatransport::parse_metadata(&xml)
        .iter()
        .map(metadata_category)
        .map(str::to_owned)
        .collect();
    SourceMetadata {
        input,
        protocol: "omt",
        timestamp: MediaTime::new(
            timestamp,
            Rational::new(1, OMT_TICKS_PER_SECOND).expect("constant"),
        ),
        payload: xml,
        categories,
    }
}

fn metadata_category(metadata: &Metadata) -> &'static str {
    match metadata {
        Metadata::Subscribe { .. } => "subscribe",
        Metadata::Settings { .. } => "settings",
        Metadata::Tally(_) => "tally",
        Metadata::SenderInfo(_) => "sender-info",
        Metadata::Redirect { .. } => "redirect",
        Metadata::Address { .. } => "address",
        Metadata::Web { .. } => "web",
        Metadata::Ptz(_) => "ptz",
        Metadata::Ancillary(_) => "ancillary",
        Metadata::Unknown(_) => "unknown",
    }
}

pub struct OmtSink {
    name: String,
    frame_rate: FrameRate,
    config: OmtOutputConfig,
    sender: Mutex<Sender>,
}

impl OmtSink {
    pub fn create_for_video_format(
        name: impl Into<String>,
        video: &VideoFormat,
        config: OmtOutputConfig,
    ) -> Result<Self, OmtError> {
        validate_video_format(video, config.color_profile)?;
        Self::create(name, video.frame_rate, config)
    }

    pub fn create(
        name: impl Into<String>,
        frame_rate: FrameRate,
        config: OmtOutputConfig,
    ) -> Result<Self, OmtError> {
        if config.send_queue_depth == 0 {
            return Err(OmtError::InvalidFrame(
                "OMT output send queue depth must be non-zero".into(),
            ));
        }
        let name = name.into();
        let sender = Sender::create_with_config(
            name.clone(),
            FrameType::VIDEO | FrameType::AUDIO | FrameType::METADATA,
            SenderConfig {
                send_queue_depth: config.send_queue_depth,
                ..SenderConfig::default()
            },
        )?;
        Ok(Self {
            name,
            frame_rate,
            config,
            sender: Mutex::new(sender),
        })
    }

    pub fn send_metadata(&self, timestamp: MediaTime, xml: &str) -> Result<(), OmtError> {
        Ok(self
            .sender
            .lock()
            .send_metadata(media_time_to_omt(timestamp), xml)?)
    }

    pub fn receiver_tally(&self) -> Tally {
        let mut sender = self.sender.lock();
        let _ = sender.poll_accept();
        let _ = sender.poll_peer_metadata();
        sender.tally()
    }
}

impl MediaSink for OmtSink {
    fn name(&self) -> &str {
        &self.name
    }

    fn push_video(&self, frame: &VideoFrame) -> eiviz_media::Result<()> {
        if frame.color != self.config.color_profile.metadata() {
            return Err(MediaError::Unsupported(format!(
                "OMT output color {:?} does not match selected {:?}; implicit conversion is forbidden",
                frame.color, self.config.color_profile
            )));
        }
        if frame.field != FieldKind::Progressive {
            return Err(MediaError::Unsupported(
                "OMT output adapter does not support interlaced field frames".into(),
            ));
        }
        let (codec, stride, data, flags) = match self.config.pixel_format {
            OmtOutputPixelFormat::Bgra => (
                openmediatransport::Codec::Bgra,
                frame.width.saturating_mul(4),
                frame_to_bgra(frame).map_err(MediaError::from)?,
                VideoFlags::ALPHA,
            ),
            OmtOutputPixelFormat::Uyvy => (
                openmediatransport::Codec::Uyvy,
                frame.width.saturating_mul(2),
                frame_to_uyvy(frame, self.config.color_profile).map_err(MediaError::from)?,
                VideoFlags::NONE,
            ),
        };
        let media = MediaFrame {
            frame_type: FrameType::VIDEO,
            timestamp: media_time_to_omt(frame.pts),
            codec: codec as i32,
            width: frame.width as i32,
            height: frame.height as i32,
            stride: stride as i32,
            flags,
            frame_rate_n: self.frame_rate.numerator() as i32,
            frame_rate_d: self.frame_rate.denominator() as i32,
            aspect_ratio: frame.width as f32 / frame.height.max(1) as f32,
            color_space: self.config.color_profile.omt(),
            data,
            ..MediaFrame::default()
        };
        let mut sender = self.sender.lock();
        sender
            .poll_accept()
            .and_then(|_| sender.poll_peer_metadata())
            .and_then(|()| sender.send_video(media))
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
        let mut sender = self.sender.lock();
        sender
            .poll_accept()
            .and_then(|_| sender.poll_peer_metadata())
            .and_then(|()| sender.send_audio(media))
            .map_err(|error| MediaError::Other(error.to_string()))
    }
}

fn frame_to_bgra(frame: &VideoFrame) -> Result<Vec<u8>, OmtError> {
    let required = frame.width as usize * frame.height as usize * 4;
    if frame.data.len() < required {
        return Err(OmtError::InvalidFrame(format!(
            "truncated {:?} frame: {} bytes, expected {required}",
            frame.format,
            frame.data.len()
        )));
    }
    match frame.format {
        PixelFormat::Rgba8 => Ok(frame.data[..required]
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[2], pixel[1], pixel[0], pixel[3]])
            .collect()),
        PixelFormat::Bgra8 => Ok(frame.data[..required].to_vec()),
        PixelFormat::Nv12
        | PixelFormat::P010
        | PixelFormat::P216
        | PixelFormat::Rgba16Float => Err(OmtError::InvalidFrame(
            "OMT BGRA output accepts only 8-bit packed RGB; implicit format/profile conversion is not supported".into(),
        )),
    }
}

fn frame_to_uyvy(frame: &VideoFrame, profile: OmtColorProfile) -> Result<Vec<u8>, OmtError> {
    if !frame.width.is_multiple_of(2) {
        return Err(OmtError::InvalidFrame(
            "UYVY output requires an even frame width".into(),
        ));
    }
    let required = frame.width as usize * frame.height as usize * 4;
    if !matches!(frame.format, PixelFormat::Rgba8 | PixelFormat::Bgra8) {
        return Err(OmtError::InvalidFrame(
            "UYVY output accepts only explicit RGBA8 or BGRA8 input".into(),
        ));
    }
    if frame.data.len() < required {
        return Err(OmtError::InvalidFrame(format!(
            "truncated {:?} frame: {} bytes, expected {required}",
            frame.format,
            frame.data.len()
        )));
    }

    let mut uyvy = Vec::with_capacity(frame.width as usize * frame.height as usize * 2);
    for pair in frame.data[..required].chunks_exact(8) {
        let first = rgb_from_pixel(&pair[..4], frame.format);
        let second = rgb_from_pixel(&pair[4..], frame.format);
        let (y0, u0, v0) = rgb_to_studio_yuv(first, profile);
        let (y1, u1, v1) = rgb_to_studio_yuv(second, profile);
        uyvy.extend_from_slice(&[
            (u0 as u16 + u1 as u16).div_ceil(2) as u8,
            y0,
            (v0 as u16 + v1 as u16).div_ceil(2) as u8,
            y1,
        ]);
    }
    Ok(uyvy)
}

fn rgb_from_pixel(pixel: &[u8], format: PixelFormat) -> [u8; 3] {
    match format {
        PixelFormat::Rgba8 => [pixel[0], pixel[1], pixel[2]],
        PixelFormat::Bgra8 => [pixel[2], pixel[1], pixel[0]],
        PixelFormat::Nv12 | PixelFormat::P010 | PixelFormat::P216 | PixelFormat::Rgba16Float => {
            unreachable!("validated packed RGB format")
        }
    }
}

pub fn validate_video_format(
    video: &VideoFormat,
    color_profile: OmtColorProfile,
) -> Result<(), OmtError> {
    if video.bit_depth != 8
        || video.interlaced
        || video.color_metadata() != color_profile.metadata()
    {
        return Err(OmtError::InvalidFrame(format!(
            "OMT adapter supports 8-bit progressive {:?} only; selected project is {:?} {}-bit interlaced={}",
            color_profile, video.color, video.bit_depth, video.interlaced
        )));
    }
    Ok(())
}

fn rgb_to_studio_yuv(rgb: [u8; 3], profile: OmtColorProfile) -> (u8, u8, u8) {
    let [r, g, b] = rgb.map(i32::from);
    let (y, u, v) = match profile {
        OmtColorProfile::Bt601Limited => (
            ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16,
            ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128,
            ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128,
        ),
        OmtColorProfile::Bt709Limited => (
            ((47 * r + 157 * g + 16 * b + 128) >> 8) + 16,
            ((-26 * r - 87 * g + 113 * b + 128) >> 8) + 128,
            ((112 * r - 102 * g - 10 * b + 128) >> 8) + 128,
        ),
    };
    (
        y.clamp(16, 235) as u8,
        u.clamp(16, 240) as u8,
        v.clamp(16, 240) as u8,
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_time::MediaTime;

    fn frame(format: PixelFormat, data: Vec<u8>) -> VideoFrame {
        VideoFrame {
            id: 1,
            source: None,
            pts: MediaTime::ZERO,
            capture_domain: ClockDomain::Virtual,
            clock_observation: None,
            width: 2,
            height: 1,
            format,
            color: eiviz_core::ColorSpace::Bt709Sdr.metadata(),
            field: FieldKind::Progressive,
            data: data.into(),
            discontinuity: false,
        }
    }

    #[test]
    fn bgra_output_respects_declared_input_layout() {
        let rgba = frame(PixelFormat::Rgba8, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        let bgra = frame(PixelFormat::Bgra8, vec![3, 2, 1, 4, 7, 6, 5, 8]);
        assert_eq!(frame_to_bgra(&rgba).unwrap(), frame_to_bgra(&bgra).unwrap());
    }

    #[test]
    fn uyvy_conversion_uses_explicit_profile() {
        let white = frame(
            PixelFormat::Rgba8,
            vec![255, 255, 255, 255, 255, 255, 255, 255],
        );
        assert_eq!(
            frame_to_uyvy(&white, OmtColorProfile::Bt709Limited).unwrap(),
            [128, 235, 128, 235]
        );
        let red = frame(PixelFormat::Rgba8, vec![255, 0, 0, 255, 255, 0, 0, 255]);
        assert_ne!(
            frame_to_uyvy(&red, OmtColorProfile::Bt601Limited).unwrap(),
            frame_to_uyvy(&red, OmtColorProfile::Bt709Limited).unwrap()
        );
    }

    #[test]
    fn omt_metadata_is_classified_for_engine_surface() {
        let converted = convert_metadata(
            InputId::new(),
            10,
            std::sync::Arc::<str>::from(
                r#"<OMTInfo ProductName="Camera" Manufacturer="Example" Version="1" />"#,
            ),
        );
        assert_eq!(converted.categories, ["sender-info"]);
    }

    #[test]
    fn nv12_is_not_implicitly_interpreted_as_packed_rgb() {
        let nv12 = frame(PixelFormat::Nv12, vec![0; 8]);
        assert!(frame_to_bgra(&nv12).is_err());
        assert!(frame_to_uyvy(&nv12, OmtColorProfile::Bt709Limited).is_err());
    }
}
