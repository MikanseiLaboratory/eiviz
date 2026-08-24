//! Encoded distribution fan-out, muxers, and network transports.
//!
//! This crate never treats the in-tree I_PCM test encoder or PCM audio as
//! production H.264/AAC. Product activation requires an explicitly registered
//! encoder factory: the production slice uses hash/version-verified dynamic
//! Cisco OpenH264 2.6.0 plus an explicit license-reviewed dynamic FDK AAC-LC
//! binary.

mod fanout;
mod recording;
mod rtmp;
mod srt;

#[cfg(test)]
pub(crate) static NETWORK_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub use fanout::{EncodedFanout, EncodedSink, SinkDiagnostics, SinkState, WorkerRecovery};
pub use recording::{FragmentedMp4Sink, RecoveryReport, recover_fragmented_mp4};
pub use rtmp::{RtmpEndpoint, RtmpPublisher};
pub use srt::SrtMpegTsPublisher;

use eiviz_core::{
    AacEncoderProfile, DistributionProfile, H264EncoderProfile, Output, OutputKind,
    TransportProfile,
};
use eiviz_media::{Capability, MediaError, MediaSink, Result};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EncoderCapabilities {
    pub cisco_openh264_26: bool,
    pub fdk_aac_lc: bool,
    pub h264_annexb_adapters: BTreeSet<String>,
    pub raw_aac_lc_adapters: BTreeSet<String>,
}

impl EncoderCapabilities {
    pub fn dynamic_openh264_fdk() -> Self {
        Self {
            cisco_openh264_26: true,
            fdk_aac_lc: true,
            ..Default::default()
        }
    }

    pub fn validate(&self, profile: &DistributionProfile) -> Result<()> {
        match &profile.video {
            H264EncoderProfile::CiscoOpenH26426 { .. } if !self.cisco_openh264_26 => {
                return Err(MediaError::Unsupported(
                    "Cisco OpenH264 2.6.0 dynamic encoder is not registered; the I_PCM test encoder is not a substitute".into(),
                ));
            }
            H264EncoderProfile::CiscoOpenH26426 { .. } => {}
            H264EncoderProfile::ExternalAnnexB { adapter, .. }
                if !self.h264_annexb_adapters.contains(adapter) =>
            {
                return Err(MediaError::Unsupported(format!(
                    "selected H.264 Annex-B encoder adapter {adapter:?} is unavailable"
                )));
            }
            H264EncoderProfile::ExternalAnnexB { .. } => {}
        }
        match &profile.audio {
            AacEncoderProfile::FdkAacLc { .. } if !self.fdk_aac_lc => {
                return Err(MediaError::Unsupported(
                    "FDK AAC-LC dynamic adapter is not registered; an explicit license-reviewed binary is required and PCM/test bytes are not substituted".into(),
                ));
            }
            AacEncoderProfile::FdkAacLc { .. } => {}
            AacEncoderProfile::ExternalRawAacLc { adapter, .. }
                if !self.raw_aac_lc_adapters.contains(adapter) =>
            {
                return Err(MediaError::Unsupported(format!(
                    "selected raw AAC-LC encoder adapter {adapter:?} is unavailable"
                )));
            }
            AacEncoderProfile::ExternalRawAacLc { .. } => {}
        }
        Ok(())
    }
}

pub fn capabilities() -> Vec<Capability> {
    vec![
        Capability {
            id: "distribution-fanout-mux".into(),
            available: true,
            detail: "shared encoded-AU fanout, H.264/AAC FLV, MPEG-TS, and fragmented MP4 framing"
                .into(),
        },
        Capability {
            id: "distribution-rtmp".into(),
            available: true,
            detail: "pure-Rust RTMP publisher transport; real-server HIL pending".into(),
        },
        Capability {
            id: "distribution-srt".into(),
            available: true,
            detail: "pure-Rust srt-tokio caller transport; real-server/loss HIL pending".into(),
        },
        Capability {
            id: "distribution-h264-encoder".into(),
            available: false,
            detail: "no production H.264 encoder adapter is compiled; I_PCM is test-only".into(),
        },
        Capability {
            id: "distribution-aac-encoder".into(),
            available: false,
            detail: "no production AAC encoder adapter is compiled".into(),
        },
    ]
}

pub fn sink_for_output(output: &Output) -> Result<Box<dyn EncodedSink>> {
    let profile = output.distribution.as_ref().ok_or_else(|| {
        MediaError::Unsupported("distribution output has no explicit profile".into())
    })?;
    match (&output.kind, &profile.transport) {
        (
            OutputKind::Rtmp { url },
            TransportProfile::RtmpPublish {
                chunk_size,
                connect_timeout_ms,
            },
        ) => Ok(Box::new(RtmpPublisher::new(
            url,
            Duration::from_millis(*connect_timeout_ms),
            *chunk_size,
        )?)),
        (
            OutputKind::Srt { url },
            TransportProfile::SrtCallerMpegTs {
                latency_ms,
                stream_id,
                connect_timeout_ms,
            },
        ) => Ok(Box::new(SrtMpegTsPublisher::new(
            url,
            stream_id.clone(),
            Duration::from_millis((*latency_ms).into()),
            Duration::from_millis(*connect_timeout_ms),
        )?)),
        (
            OutputKind::Mp4 { path },
            TransportProfile::FragmentedMp4 {
                recover_incomplete_tail,
            },
        ) => Ok(Box::new(FragmentedMp4Sink::new(
            path.into(),
            *recover_incomplete_tail,
        ))),
        _ => Err(MediaError::Unsupported(
            "output kind and transport profile do not match".into(),
        )),
    }
}

/// Always-failing raw sink used to prove that legacy Program outputs remain
/// isolated from adapter failures.
pub struct FailingSink {
    name: String,
}

impl FailingSink {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl MediaSink for FailingSink {
    fn name(&self) -> &str {
        &self.name
    }

    fn push_video(&self, _frame: &eiviz_media::VideoFrame) -> Result<()> {
        Err(MediaError::Disconnected(self.name.clone()))
    }

    fn push_audio(&self, _audio: &eiviz_media::AudioBuffer) -> Result<()> {
        Err(MediaError::Disconnected(self.name.clone()))
    }
}

pub fn attach_profiled_sink(
    fanout: &EncodedFanout,
    output: &Output,
    encoders: &EncoderCapabilities,
) -> Result<String> {
    let profile = output.distribution.as_ref().ok_or_else(|| {
        MediaError::Unsupported("distribution output has no explicit profile".into())
    })?;
    encoders.validate(profile)?;
    let sink = sink_for_output(output)?;
    let sink_name = format!("{}:{}", output.id, sink.name());
    let sink = Box::new(OutputSink {
        name: sink_name.clone(),
        inner: sink,
    });
    fanout.add_sink(
        sink,
        profile.queue_capacity,
        WorkerRecovery {
            initial_delay: Duration::from_millis(profile.reconnect.initial_delay_ms),
            max_delay: Duration::from_millis(profile.reconnect.max_delay_ms),
            max_attempts: profile.reconnect.max_attempts,
        },
    )?;
    Ok(sink_name)
}

struct OutputSink {
    name: String,
    inner: Box<dyn EncodedSink>,
}

impl EncodedSink for OutputSink {
    fn name(&self) -> &str {
        &self.name
    }

    fn connect(&mut self, config: &eiviz_media::EncodedStreamConfig) -> Result<()> {
        self.inner.connect(config)
    }

    fn send(&mut self, access_unit: &Arc<eiviz_media::EncodedAccessUnit>) -> Result<()> {
        self.inner.send(access_unit)
    }

    fn disconnect(&mut self) {
        self.inner.disconnect();
    }
}

/// Test adapters can register exact names without making them implicit
/// production defaults.
pub fn registered_capabilities(
    h264: impl IntoIterator<Item = String>,
    aac: impl IntoIterator<Item = String>,
) -> Arc<EncoderCapabilities> {
    Arc::new(EncoderCapabilities {
        cisco_openh264_26: false,
        fdk_aac_lc: false,
        h264_annexb_adapters: h264.into_iter().collect(),
        raw_aac_lc_adapters: aac.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_core::{
        AacEncoderProfile, DistributionProfile, H264EncoderProfile, ReconnectProfile,
    };

    fn profile() -> DistributionProfile {
        DistributionProfile {
            video: H264EncoderProfile::CiscoOpenH26426 {
                bitrate_bps: 8_000_000,
                keyframe_interval_frames: 120,
                level_idc: 42,
            },
            audio: AacEncoderProfile::FdkAacLc {
                bitrate_bps: 192_000,
                sample_rate: 48_000,
                channels: 2,
            },
            transport: TransportProfile::RtmpPublish {
                chunk_size: 4096,
                connect_timeout_ms: 2_000,
            },
            queue_capacity: 128,
            reconnect: ReconnectProfile {
                initial_delay_ms: 100,
                max_delay_ms: 5_000,
                max_attempts: 0,
            },
        }
    }

    #[test]
    fn unavailable_product_encoders_hard_fail() {
        let error = EncoderCapabilities::default()
            .validate(&profile())
            .unwrap_err();
        assert!(error.to_string().contains("I_PCM test encoder"));
    }
}
