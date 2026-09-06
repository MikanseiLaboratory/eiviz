use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::codec::{WS_SUBPROTOCOL, decode_envelope, encode_envelope};
use crate::proto::{
    ClientHello, Cut, Envelope, GetSnapshot, Request, Response, Role, envelope, request,
};
use eiviz_control::error::{ControlError, ControlResult};

#[derive(Debug, Clone)]
pub struct ControlClient {
    pub endpoint: String,
    pub token: String,
}

impl ControlClient {
    pub fn websocket(endpoint: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            token: token.into(),
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
        self.roundtrip_ws(request).await
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
