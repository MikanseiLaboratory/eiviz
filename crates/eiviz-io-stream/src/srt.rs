use crate::fanout::EncodedSink;
use bytes::Bytes;
use eiviz_media::{
    EncodedAccessUnit, EncodedKind, EncodedStreamConfig, MediaError, Result,
};
use futures::SinkExt;
use srt_tokio::{SrtSocket, options::SocketAddress};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct SrtMpegTsPublisher {
    remote: String,
    stream_id: Option<String>,
    latency: Duration,
    connect_timeout: Duration,
    name: String,
    runtime: Option<tokio::runtime::Runtime>,
    socket: Option<SrtSocket>,
    video_continuity: u8,
    audio_continuity: u8,
    config: Option<EncodedStreamConfig>,
}

impl SrtMpegTsPublisher {
    pub fn new(
        url: &str,
        stream_id: Option<String>,
        latency: Duration,
        connect_timeout: Duration,
    ) -> Result<Self> {
        let remote = parse_srt_url(url)?;
        Ok(Self {
            name: format!("SRT {remote}"),
            remote,
            stream_id,
            latency,
            connect_timeout,
            runtime: None,
            socket: None,
            video_continuity: 0,
            audio_continuity: 0,
            config: None,
        })
    }

    fn send_packets(&mut self, packets: impl IntoIterator<Item = [u8; 188]>) -> Result<()> {
        let mut payload = Vec::new();
        for packet in packets {
            payload.extend_from_slice(&packet);
        }
        if payload.is_empty() {
            return Ok(());
        }
        let runtime = self
            .runtime
            .as_mut()
            .ok_or_else(|| MediaError::Disconnected(self.name.clone()))?;
        let socket = self
            .socket
            .as_mut()
            .ok_or_else(|| MediaError::Disconnected(self.name.clone()))?;
        runtime
            .block_on(socket.send((Instant::now(), Bytes::from(payload))))
            .map_err(|error| MediaError::Disconnected(error.to_string()))
    }
}

impl EncodedSink for SrtMpegTsPublisher {
    fn name(&self) -> &str {
        &self.name
    }

    fn connect(&mut self, config: &EncodedStreamConfig) -> Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|error| MediaError::Other(error.to_string()))?;
        let remote: SocketAddress = self
            .remote
            .as_str()
            .try_into()
            .map_err(|_| MediaError::Unsupported("invalid SRT socket address".into()))?;
        let builder = SrtSocket::builder().latency(self.latency);
        let socket = runtime
            .block_on(async {
                tokio::time::timeout(
                    self.connect_timeout,
                    builder.call(remote, self.stream_id.as_deref()),
                )
                .await
            })
            .map_err(|_| MediaError::Disconnected("SRT connect timeout".into()))?
            .map_err(|error| MediaError::Disconnected(error.to_string()))?;
        self.runtime = Some(runtime);
        self.socket = Some(socket);
        self.config = Some(config.clone());
        self.video_continuity = 0;
        self.audio_continuity = 0;
        self.send_packets([eiviz_codec_software::pat(), eiviz_codec_software::pmt()])
    }

    fn send(&mut self, access_unit: &Arc<EncodedAccessUnit>) -> Result<()> {
        if access_unit.kind == EncodedKind::Avc && access_unit.keyframe {
            self.send_packets([eiviz_codec_software::pat(), eiviz_codec_software::pmt()])?;
        }
        let pts = eiviz_codec_software::media_time_90k(access_unit.pts);
        match access_unit.kind {
            EncodedKind::Avc => {
                let packets =
                    eiviz_codec_software::pes_video(access_unit, pts, &mut self.video_continuity);
                self.send_packets(packets)
            }
            EncodedKind::Aac => {
                let config = self
                    .config
                    .as_ref()
                    .ok_or_else(|| MediaError::Disconnected(self.name.clone()))?;
                let packets = eiviz_codec_software::pes_aac(
                    access_unit,
                    pts,
                    config.audio_sample_rate,
                    config.audio_channels,
                    &mut self.audio_continuity,
                )
                .map_err(|error| MediaError::Unsupported(error.into()))?;
                self.send_packets(packets)
            }
            EncodedKind::Pcm => Err(MediaError::Unsupported(
                "SRT MPEG-TS baseline requires AAC, not PCM".into(),
            )),
        }
    }

    fn disconnect(&mut self) {
        if let (Some(runtime), Some(mut socket)) = (self.runtime.as_mut(), self.socket.take()) {
            let _ = runtime.block_on(socket.close());
        }
        self.runtime = None;
        self.config = None;
    }
}

fn parse_srt_url(url: &str) -> Result<String> {
    let remote = url
        .strip_prefix("srt://")
        .ok_or_else(|| MediaError::Unsupported("SRT URL must start with srt://".into()))?;
    if remote.is_empty() || remote.contains('/') || remote.contains('?') || !remote.contains(':') {
        return Err(MediaError::Unsupported(
            "SRT URL must be srt://host:port; stream ID and latency are explicit profile fields"
                .into(),
        ));
    }
    Ok(remote.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_does_not_accept_implicit_query_options() {
        assert_eq!(parse_srt_url("srt://127.0.0.1:9000").unwrap(), "127.0.0.1:9000");
        assert!(parse_srt_url("udp://127.0.0.1:9000").is_err());
        assert!(parse_srt_url("srt://127.0.0.1:9000?latency=20").is_err());
    }
}
