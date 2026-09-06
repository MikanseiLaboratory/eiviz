use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::codec::{
    WS_SUBPROTOCOL, decode_envelope, decode_tcp_header, encode_envelope, encode_tcp_frame,
};
use crate::proto::{
    ClientHello, Cut, Envelope, GetSnapshot, Request, Response, Role, envelope, request,
};
use eiviz_control::error::{ControlError, ControlResult};

#[derive(Debug, Clone)]
pub struct ControlClient {
    pub endpoint: String,
    pub token: String,
    pub tcp: bool,
}

impl ControlClient {
    pub fn websocket(endpoint: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            token: token.into(),
            tcp: false,
        }
    }

    pub async fn snapshot_json(&self) -> ControlResult<String> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::GetSnapshot(GetSnapshot {})),
            })
            .await?;
        if let Some(crate::proto::response::Payload::Snapshot(snapshot)) = response.payload {
            String::from_utf8(snapshot.document_json)
                .map_err(|error| ControlError::internal(error.to_string()))
        } else {
            Err(ControlError::unavailable("snapshot missing"))
        }
    }

    pub async fn cut(&self, unit_id: u64, swap: bool) -> ControlResult<()> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::Cut(Cut {
                    unit: Some(crate::proto::ResourceRef {
                        kind: "unit".into(),
                        id: unit_id,
                        guid: String::new(),
                        name: String::new(),
                    }),
                    input: None,
                    swap,
                })),
            })
            .await?;
        status_ok(&response)
    }

    pub async fn preview(&self, unit_id: u64, scene_id: u64) -> ControlResult<()> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::Preview(crate::proto::Preview {
                    unit: Some(crate::proto::ResourceRef {
                        kind: "unit".into(),
                        id: unit_id,
                        guid: String::new(),
                        name: String::new(),
                    }),
                    scene: Some(crate::proto::ResourceRef {
                        kind: "scene".into(),
                        id: scene_id,
                        guid: String::new(),
                        name: String::new(),
                    }),
                })),
            })
            .await?;
        status_ok(&response)
    }

    pub async fn auto(&self, unit_id: u64, duration_ms: u32, swap: bool) -> ControlResult<()> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::Auto(crate::proto::Auto {
                    unit: Some(crate::proto::ResourceRef {
                        kind: "unit".into(),
                        id: unit_id,
                        guid: String::new(),
                        name: String::new(),
                    }),
                    input: None,
                    kind: 0,
                    duration_ms,
                    swap,
                    keep_preview: true,
                })),
            })
            .await?;
        status_ok(&response)
    }

    pub async fn replace_session(
        &self,
        document_json: Vec<u8>,
        expected_revision: u64,
    ) -> ControlResult<()> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision,
                payload: Some(request::Payload::ReplaceSession(
                    crate::proto::ReplaceSession {
                        document_json,
                        expected_revision,
                    },
                )),
            })
            .await?;
        status_ok(&response)
    }

    pub async fn shutdown(&self) -> ControlResult<()> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::Shutdown(crate::proto::Shutdown {})),
            })
            .await?;
        status_ok(&response)
    }

    async fn roundtrip(&self, request: Request) -> ControlResult<Response> {
        if self.tcp {
            self.roundtrip_tcp(request).await
        } else {
            self.roundtrip_ws(request).await
        }
    }

    async fn roundtrip_ws(&self, request: Request) -> ControlResult<Response> {
        let mut req = self
            .endpoint
            .as_str()
            .into_client_request()
            .map_err(|error| ControlError::io(error.to_string()))?;
        req.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            tokio_tungstenite::tungstenite::http::HeaderValue::from_static(WS_SUBPROTOCOL),
        );
        let (mut ws, _) = connect_async(req)
            .await
            .map_err(|error| ControlError::io(error.to_string()))?;
        let hello = Envelope {
            kind: Some(envelope::Kind::Hello(ClientHello {
                protocol: WS_SUBPROTOCOL.into(),
                client_name: "eivizctl".into(),
                client_instance_id: uuid::Uuid::new_v4().to_string(),
                token: self.token.clone(),
                role: Role::Admin as i32,
            })),
        };
        ws.send(Message::Binary(encode_envelope(&hello).into()))
            .await
            .map_err(|error| ControlError::io(error.to_string()))?;
        let req = Envelope {
            kind: Some(envelope::Kind::Request(request)),
        };
        ws.send(Message::Binary(encode_envelope(&req).into()))
            .await
            .map_err(|error| ControlError::io(error.to_string()))?;
        while let Some(msg) = ws.next().await {
            let msg = msg.map_err(|error| ControlError::io(error.to_string()))?;
            if let Message::Binary(bytes) = msg {
                let env = decode_envelope(&bytes).map_err(ControlError::invalid)?;
                if let Some(envelope::Kind::Response(response)) = env.kind {
                    return Ok(response);
                }
            }
        }
        Err(ControlError::unavailable("connection closed"))
    }

    async fn roundtrip_tcp(&self, request: Request) -> ControlResult<Response> {
        let mut stream = TcpStream::connect(&self.endpoint)
            .await
            .map_err(|error| ControlError::io(error.to_string()))?;
        stream
            .set_nodelay(true)
            .map_err(|error| ControlError::io(error.to_string()))?;
        let hello = Envelope {
            kind: Some(envelope::Kind::Hello(ClientHello {
                protocol: "eiviz.tcp.v1".into(),
                client_name: "eivizctl".into(),
                client_instance_id: uuid::Uuid::new_v4().to_string(),
                token: String::new(),
                role: Role::Admin as i32,
            })),
        };
        stream
            .write_all(&encode_tcp_frame(&hello))
            .await
            .map_err(|error| ControlError::io(error.to_string()))?;
        let challenge = read_tcp_envelope(&mut stream).await?;
        let mut token = self.token.clone();
        if let Some(envelope::Kind::Event(event)) = challenge.kind {
            if !event.request_id.is_empty() {
                token = format!("{}:{}", event.request_id, self.token);
            }
        }
        let authed = Envelope {
            kind: Some(envelope::Kind::Hello(ClientHello {
                protocol: "eiviz.tcp.v1".into(),
                client_name: "eivizctl".into(),
                client_instance_id: uuid::Uuid::new_v4().to_string(),
                token,
                role: Role::Admin as i32,
            })),
        };
        stream
            .write_all(&encode_tcp_frame(&authed))
            .await
            .map_err(|error| ControlError::io(error.to_string()))?;
        let req = Envelope {
            kind: Some(envelope::Kind::Request(request)),
        };
        stream
            .write_all(&encode_tcp_frame(&req))
            .await
            .map_err(|error| ControlError::io(error.to_string()))?;
        let env = read_tcp_envelope(&mut stream).await?;
        match env.kind {
            Some(envelope::Kind::Response(response)) => Ok(response),
            _ => Err(ControlError::unavailable("unexpected tcp response")),
        }
    }
}

fn status_ok(response: &Response) -> ControlResult<()> {
    let Some(status) = &response.status else {
        return Ok(());
    };
    if status.code.is_empty() || status.code == "OK" {
        Ok(())
    } else {
        Err(ControlError::invalid(status.message.clone()))
    }
}

async fn read_tcp_envelope(stream: &mut TcpStream) -> ControlResult<Envelope> {
    let mut header = [0u8; 9];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|error| ControlError::io(error.to_string()))?;
    let len = decode_tcp_header(&header).map_err(ControlError::invalid)?;
    let mut payload = vec![0u8; len];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(|error| ControlError::io(error.to_string()))?;
    decode_envelope(&payload).map_err(ControlError::invalid)
}

pub async fn wait_ready(endpoint: &str, token: &str, timeout: Duration) -> ControlResult<()> {
    let client = ControlClient::websocket(endpoint, token);
    let start = std::time::Instant::now();
    loop {
        if client.snapshot_json().await.is_ok() {
            return Ok(());
        }
        if start.elapsed() > timeout {
            return Err(ControlError::unavailable("api not ready"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub fn shared_client(endpoint: String, token: String) -> Arc<ControlClient> {
    Arc::new(ControlClient::websocket(endpoint, token))
}
