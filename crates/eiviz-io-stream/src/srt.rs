use crate::fanout::EncodedSink;
use bytes::Bytes;
use eiviz_media::{EncodedAccessUnit, EncodedKind, EncodedStreamConfig, MediaError, Result};
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
    use eiviz_time::MediaTime;
    use futures::StreamExt;
    use std::net::UdpSocket;
    use std::sync::mpsc;

    #[test]
    fn endpoint_does_not_accept_implicit_query_options() {
        assert_eq!(
            parse_srt_url("srt://127.0.0.1:9000").unwrap(),
            "127.0.0.1:9000"
        );
        assert!(parse_srt_url("udp://127.0.0.1:9000").is_err());
        assert!(parse_srt_url("srt://127.0.0.1:9000?latency=20").is_err());
    }

    #[test]
    fn local_srt_listener_receives_mpegts_avc_and_aac_pids() {
        let reservation = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let (result_tx, result_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let mut socket = SrtSocket::builder()
                    .latency(Duration::from_millis(20))
                    .listen_on(format!("127.0.0.1:{port}"))
                    .await
                    .unwrap();
                let mut pids = std::collections::BTreeSet::new();
                while ![
                    0,
                    eiviz_codec_software::PMT_PID,
                    eiviz_codec_software::VIDEO_PID,
                    eiviz_codec_software::AUDIO_PID,
                ]
                .iter()
                .all(|pid| pids.contains(pid))
                {
                    let (_, bytes) = socket.next().await.unwrap().unwrap();
                    for packet in bytes.chunks_exact(188) {
                        assert_eq!(packet[0], 0x47);
                        pids.insert((((packet[1] & 0x1f) as u16) << 8) | packet[2] as u16);
                    }
                }
                result_tx.send(pids).unwrap();
            });
        });

        let mut publisher = SrtMpegTsPublisher::new(
            &format!("srt://127.0.0.1:{port}"),
            None,
            Duration::from_millis(20),
            Duration::from_secs(2),
        )
        .unwrap();
        let config = test_config();
        publisher.connect(&config).unwrap();
        publisher
            .send(&Arc::new(EncodedAccessUnit {
                pts: MediaTime::ZERO,
                dts: Some(MediaTime::ZERO),
                keyframe: true,
                bytes: vec![0, 0, 0, 1, 0x65, 1].into(),
                kind: EncodedKind::Avc,
            }))
            .unwrap();
        publisher
            .send(&Arc::new(EncodedAccessUnit {
                pts: MediaTime::ZERO,
                dts: Some(MediaTime::ZERO),
                keyframe: false,
                bytes: vec![0x21, 0x10].into(),
                kind: EncodedKind::Aac,
            }))
            .unwrap();
        let pids = result_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(pids.contains(&eiviz_codec_software::VIDEO_PID));
        assert!(pids.contains(&eiviz_codec_software::AUDIO_PID));
        publisher.disconnect();
        server.join().unwrap();
    }

    fn test_config() -> EncodedStreamConfig {
        EncodedStreamConfig {
            h264_sps: vec![0x67, 66, 0, 31].into(),
            h264_pps: vec![0x68, 0].into(),
            aac_audio_specific_config: vec![0x11, 0x90].into(),
            video_width: 1920,
            video_height: 1080,
            video_timescale: 60_000,
            video_sample_duration: 1001,
            audio_sample_rate: 48_000,
            audio_channels: 2,
        }
    }
}
