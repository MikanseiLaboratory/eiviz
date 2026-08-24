use crate::fanout::EncodedSink;
use bytes::Bytes;
use eiviz_media::{EncodedAccessUnit, EncodedKind, EncodedStreamConfig, MediaError, Result};
use rml_rtmp::handshake::{Handshake, HandshakeProcessResult, PeerType};
use rml_rtmp::sessions::{
    ClientSession, ClientSessionConfig, ClientSessionEvent, ClientSessionResult, PublishRequestType,
};
use rml_rtmp::time::RtmpTimestamp;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RtmpEndpoint {
    pub host: String,
    pub port: u16,
    pub app: String,
    pub stream_key: String,
}

impl RtmpEndpoint {
    pub fn parse(url: &str) -> Result<Self> {
        let remainder = url
            .strip_prefix("rtmp://")
            .ok_or_else(|| MediaError::Unsupported("RTMP URL must start with rtmp://".into()))?;
        let (authority, path) = remainder.split_once('/').ok_or_else(|| {
            MediaError::Unsupported("RTMP URL must contain /app/stream-key".into())
        })?;
        let (app, stream_key) = path.split_once('/').ok_or_else(|| {
            MediaError::Unsupported("RTMP URL must contain /app/stream-key".into())
        })?;
        if authority.is_empty() || app.is_empty() || stream_key.is_empty() {
            return Err(MediaError::Unsupported(
                "RTMP host, app, and stream key must be non-empty".into(),
            ));
        }
        let (host, port) = parse_authority(authority, 1935)?;
        Ok(Self {
            host,
            port,
            app: app.into(),
            stream_key: stream_key.into(),
        })
    }

    fn tc_url(&self) -> String {
        format!("rtmp://{}:{}/{}", self.host, self.port, self.app)
    }
}

pub struct RtmpPublisher {
    endpoint: RtmpEndpoint,
    connect_timeout: Duration,
    chunk_size: u32,
    name: String,
    connection: Option<RtmpConnection>,
}

struct RtmpConnection {
    stream: TcpStream,
    session: ClientSession,
}

impl RtmpPublisher {
    pub fn new(url: &str, connect_timeout: Duration, chunk_size: u32) -> Result<Self> {
        if !(128..=65_536).contains(&chunk_size) {
            return Err(MediaError::Unsupported(
                "RTMP chunk size must be in 128..=65536".into(),
            ));
        }
        let endpoint = RtmpEndpoint::parse(url)?;
        let name = format!("RTMP {}:{}/{}", endpoint.host, endpoint.port, endpoint.app);
        Ok(Self {
            endpoint,
            connect_timeout,
            chunk_size,
            name,
            connection: None,
        })
    }
}

impl EncodedSink for RtmpPublisher {
    fn name(&self) -> &str {
        &self.name
    }

    fn connect(&mut self, config: &EncodedStreamConfig) -> Result<()> {
        let mut connection =
            RtmpConnection::connect(&self.endpoint, self.connect_timeout, self.chunk_size)?;
        connection.publish_headers(config)?;
        self.connection = Some(connection);
        Ok(())
    }

    fn send(&mut self, access_unit: &Arc<EncodedAccessUnit>) -> Result<()> {
        let connection = self
            .connection
            .as_mut()
            .ok_or_else(|| MediaError::Disconnected(self.name.clone()))?;
        connection.service_control_messages()?;
        let timestamp =
            RtmpTimestamp::new(media_time_ms(access_unit.dts.unwrap_or(access_unit.pts)));
        let result = match access_unit.kind {
            EncodedKind::Avc => connection.session.publish_video_data(
                Bytes::from(eiviz_codec_software::avc_nalu_payload(access_unit)),
                timestamp,
                !access_unit.keyframe,
            ),
            EncodedKind::Aac => connection.session.publish_audio_data(
                Bytes::from(eiviz_codec_software::aac_raw_payload(access_unit)),
                timestamp,
                false,
            ),
            EncodedKind::Pcm => {
                return Err(MediaError::Unsupported(
                    "RTMP baseline requires AAC, not PCM".into(),
                ));
            }
        }
        .map_err(|error| MediaError::Disconnected(error.to_string()))?;
        write_client_result(&mut connection.stream, result)?;
        Ok(())
    }

    fn disconnect(&mut self) {
        self.connection = None;
    }
}

impl RtmpConnection {
    fn connect(endpoint: &RtmpEndpoint, timeout: Duration, chunk_size: u32) -> Result<Self> {
        let address = format!("{}:{}", endpoint.host, endpoint.port);
        let addresses = address
            .to_socket_addrs()
            .map_err(|error| MediaError::Disconnected(error.to_string()))?;
        let mut last_error = None;
        let mut stream = None;
        for address in addresses {
            match TcpStream::connect_timeout(&address, timeout) {
                Ok(value) => {
                    stream = Some(value);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let mut stream = stream.ok_or_else(|| {
            MediaError::Disconnected(
                last_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "RTMP host resolved to no addresses".into()),
            )
        })?;
        stream
            .set_read_timeout(Some(timeout))
            .and_then(|_| stream.set_write_timeout(Some(timeout)))
            .map_err(|error| MediaError::Disconnected(error.to_string()))?;
        perform_handshake(&mut stream)?;

        let mut session_config = ClientSessionConfig::new();
        session_config.tc_url = Some(endpoint.tc_url());
        session_config.chunk_size = chunk_size;
        let (mut session, initial) = ClientSession::new(session_config)
            .map_err(|error| MediaError::Disconnected(error.to_string()))?;
        write_client_results(&mut stream, initial)?;
        let request = session
            .request_connection(endpoint.app.clone())
            .map_err(|error| MediaError::Disconnected(error.to_string()))?;
        write_client_result(&mut stream, request)?;

        let mut publish_requested = false;
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let count = stream
                .read(&mut buffer)
                .map_err(|error| MediaError::Disconnected(error.to_string()))?;
            if count == 0 {
                return Err(MediaError::Disconnected(
                    "RTMP server closed during publish negotiation".into(),
                ));
            }
            let results = session
                .handle_input(&buffer[..count])
                .map_err(|error| MediaError::Disconnected(error.to_string()))?;
            let mut connected = false;
            let mut published = false;
            for result in results {
                match result {
                    ClientSessionResult::OutboundResponse(packet) => stream
                        .write_all(&packet.bytes)
                        .map_err(|error| MediaError::Disconnected(error.to_string()))?,
                    ClientSessionResult::RaisedEvent(
                        ClientSessionEvent::ConnectionRequestAccepted,
                    ) => connected = true,
                    ClientSessionResult::RaisedEvent(
                        ClientSessionEvent::PublishRequestAccepted,
                    ) => published = true,
                    ClientSessionResult::RaisedEvent(
                        ClientSessionEvent::ConnectionRequestRejected { description },
                    ) => return Err(MediaError::Disconnected(description)),
                    _ => {}
                }
            }
            if connected && !publish_requested {
                let request = session
                    .request_publishing(endpoint.stream_key.clone(), PublishRequestType::Live)
                    .map_err(|error| MediaError::Disconnected(error.to_string()))?;
                write_client_result(&mut stream, request)?;
                publish_requested = true;
            }
            if published {
                stream
                    .set_read_timeout(Some(Duration::from_millis(1)))
                    .map_err(|error| MediaError::Disconnected(error.to_string()))?;
                return Ok(Self { stream, session });
            }
        }
    }

    fn publish_headers(&mut self, config: &EncodedStreamConfig) -> Result<()> {
        let video = self
            .session
            .publish_video_data(
                Bytes::from(eiviz_codec_software::avc_sequence_header_payload(
                    &config.h264_sps,
                    &config.h264_pps,
                )),
                RtmpTimestamp::new(0),
                false,
            )
            .map_err(|error| MediaError::Disconnected(error.to_string()))?;
        write_client_result(&mut self.stream, video)?;
        let audio = self
            .session
            .publish_audio_data(
                Bytes::from(eiviz_codec_software::aac_sequence_header_payload(
                    &config.aac_audio_specific_config,
                )),
                RtmpTimestamp::new(0),
                false,
            )
            .map_err(|error| MediaError::Disconnected(error.to_string()))?;
        write_client_result(&mut self.stream, audio)
    }

    fn service_control_messages(&mut self) -> Result<()> {
        let mut buffer = [0u8; 8192];
        loop {
            match self.stream.read(&mut buffer) {
                Ok(0) => {
                    return Err(MediaError::Disconnected(
                        "RTMP server closed the connection".into(),
                    ));
                }
                Ok(count) => {
                    let results = self
                        .session
                        .handle_input(&buffer[..count])
                        .map_err(|error| MediaError::Disconnected(error.to_string()))?;
                    write_client_results(&mut self.stream, results)?;
                }
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    return Ok(());
                }
                Err(error) => return Err(MediaError::Disconnected(error.to_string())),
            }
        }
    }
}

fn perform_handshake(stream: &mut TcpStream) -> Result<()> {
    let mut handshake = Handshake::new(PeerType::Client);
    let initial = handshake
        .generate_outbound_p0_and_p1()
        .map_err(|error| MediaError::Disconnected(error.to_string()))?;
    stream
        .write_all(&initial)
        .map_err(|error| MediaError::Disconnected(error.to_string()))?;
    let mut buffer = [0u8; 4096];
    loop {
        let count = stream
            .read(&mut buffer)
            .map_err(|error| MediaError::Disconnected(error.to_string()))?;
        if count == 0 {
            return Err(MediaError::Disconnected(
                "RTMP server closed during handshake".into(),
            ));
        }
        match handshake
            .process_bytes(&buffer[..count])
            .map_err(|error| MediaError::Disconnected(error.to_string()))?
        {
            HandshakeProcessResult::InProgress { response_bytes } => stream
                .write_all(&response_bytes)
                .map_err(|error| MediaError::Disconnected(error.to_string()))?,
            HandshakeProcessResult::Completed {
                response_bytes,
                remaining_bytes,
            } => {
                stream
                    .write_all(&response_bytes)
                    .map_err(|error| MediaError::Disconnected(error.to_string()))?;
                if !remaining_bytes.is_empty() {
                    return Err(MediaError::Disconnected(
                        "RTMP server sent session data before handshake completion".into(),
                    ));
                }
                return Ok(());
            }
        }
    }
}

fn write_client_results(stream: &mut TcpStream, results: Vec<ClientSessionResult>) -> Result<()> {
    for result in results {
        if let ClientSessionResult::OutboundResponse(packet) = result {
            stream
                .write_all(&packet.bytes)
                .map_err(|error| MediaError::Disconnected(error.to_string()))?;
        }
    }
    Ok(())
}

fn write_client_result(stream: &mut TcpStream, result: ClientSessionResult) -> Result<()> {
    write_client_results(stream, vec![result])
}

fn media_time_ms(time: eiviz_time::MediaTime) -> u32 {
    let base = time.timebase();
    let value =
        time.ticks() as i128 * base.numerator() as i128 * 1_000 / base.denominator() as i128;
    value.max(0).min(u32::MAX as i128) as u32
}

fn parse_authority(authority: &str, default_port: u16) -> Result<(String, u16)> {
    if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, suffix) = bracketed.split_once(']').ok_or_else(|| {
            MediaError::Unsupported("invalid bracketed IPv6 RTMP authority".into())
        })?;
        let port = if suffix.is_empty() {
            default_port
        } else {
            suffix
                .strip_prefix(':')
                .ok_or_else(|| MediaError::Unsupported("invalid RTMP authority".into()))?
                .parse()
                .map_err(|_| MediaError::Unsupported("invalid RTMP port".into()))?
        };
        return Ok((host.into(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => Ok((
            host.into(),
            port.parse()
                .map_err(|_| MediaError::Unsupported("invalid RTMP port".into()))?,
        )),
        _ => Ok((authority.into(), default_port)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_time::MediaTime;
    use rml_rtmp::handshake::{Handshake, HandshakeProcessResult, PeerType};
    use rml_rtmp::sessions::{
        ServerSession, ServerSessionConfig, ServerSessionEvent, ServerSessionResult,
    };
    use std::net::TcpListener;
    use std::sync::mpsc;

    #[test]
    fn endpoint_requires_explicit_app_and_key() {
        let endpoint = RtmpEndpoint::parse("rtmp://127.0.0.1:1936/live/key").unwrap();
        assert_eq!(endpoint.port, 1936);
        assert_eq!(endpoint.app, "live");
        assert_eq!(endpoint.stream_key, "key");
        assert!(RtmpEndpoint::parse("http://example/live/key").is_err());
        assert!(RtmpEndpoint::parse("rtmp://example/live").is_err());
    }

    #[test]
    fn local_server_receives_avc_and_aac_rtmp_messages() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (result_tx, result_rx) = mpsc::channel();
        let server = std::thread::spawn(move || run_mock_server(listener, result_tx));

        let mut publisher = RtmpPublisher::new(
            &format!("rtmp://127.0.0.1:{port}/live/key"),
            Duration::from_secs(2),
            4096,
        )
        .unwrap();
        let config = test_config();
        publisher.connect(&config).unwrap();
        publisher
            .send(&Arc::new(EncodedAccessUnit {
                pts: MediaTime::ZERO,
                dts: Some(MediaTime::ZERO),
                keyframe: true,
                bytes: vec![0, 0, 0, 1, 0x65, 1, 2].into(),
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
        publisher.disconnect();

        let (video_messages, audio_messages) =
            result_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(video_messages, 2, "sequence header + AVC access unit");
        assert_eq!(audio_messages, 2, "sequence header + AAC access unit");
        server.join().unwrap();
    }

    fn run_mock_server(listener: TcpListener, result: mpsc::Sender<(u64, u64)>) {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut handshake = Handshake::new(PeerType::Server);
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let count = stream.read(&mut buffer).unwrap();
            let response = handshake.process_bytes(&buffer[..count]).unwrap();
            match response {
                HandshakeProcessResult::InProgress { response_bytes } => {
                    stream.write_all(&response_bytes).unwrap();
                }
                HandshakeProcessResult::Completed { response_bytes, .. } => {
                    stream.write_all(&response_bytes).unwrap();
                    break;
                }
            }
        }

        let (mut session, initial) = ServerSession::new(ServerSessionConfig::new()).unwrap();
        write_server_results(&mut stream, &mut session, initial);
        let mut video = 0;
        let mut audio = 0;
        while video < 2 || audio < 2 {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            let results = session.handle_input(&buffer[..count]).unwrap();
            for item in results {
                match item {
                    ServerSessionResult::OutboundResponse(packet) => {
                        stream.write_all(&packet.bytes).unwrap();
                    }
                    ServerSessionResult::RaisedEvent(
                        ServerSessionEvent::ConnectionRequested { request_id, .. }
                        | ServerSessionEvent::PublishStreamRequested { request_id, .. },
                    ) => {
                        let accepted = session.accept_request(request_id).unwrap();
                        write_server_results(&mut stream, &mut session, accepted);
                    }
                    ServerSessionResult::RaisedEvent(ServerSessionEvent::VideoDataReceived {
                        ..
                    }) => video += 1,
                    ServerSessionResult::RaisedEvent(ServerSessionEvent::AudioDataReceived {
                        ..
                    }) => audio += 1,
                    _ => {}
                }
            }
        }
        result.send((video, audio)).unwrap();
    }

    fn write_server_results(
        stream: &mut TcpStream,
        _session: &mut ServerSession,
        results: Vec<ServerSessionResult>,
    ) {
        for result in results {
            if let ServerSessionResult::OutboundResponse(packet) = result {
                stream.write_all(&packet.bytes).unwrap();
            }
        }
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
