use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::auth::{AuthConfig, Role};
use crate::codec::{MAX_MESSAGE_BYTES, WS_SUBPROTOCOL, decode_envelope, encode_envelope};
use crate::proto::{
    Envelope, Event as ProtoEvent, Response as ProtoResponse, Snapshot as ProtoSnapshot, Status,
    envelope, request,
};
use eiviz_control::error::ControlError;
use eiviz_control::{Command, ControlFacade, Incoming, RequestKey};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_tungstenite::accept_hdr_async_with_config;
use tokio_tungstenite::tungstenite::{
    Message,
    handshake::server::{Request, Response},
    protocol::WebSocketConfig,
};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub auth: AuthConfig,
    pub idle_timeout: Duration,
    pub max_clients: usize,
}

impl ServerConfig {
    pub fn loopback(port: u16, auth: AuthConfig) -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], port)),
            auth,
            idle_timeout: Duration::from_secs(60),
            max_clients: 32,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ServerBind {
    pub ws_addr: SocketAddr,
}

pub async fn spawn(config: ServerConfig, control: Arc<dyn ControlFacade>) -> std::io::Result<()> {
    let (bind, task) = listen(config, control).await?;
    eprintln!("eiviz-api listening ws={}", bind.ws_addr);
    task.await.map_err(std::io::Error::other)?
}

pub async fn listen(
    config: ServerConfig,
    control: Arc<dyn ControlFacade>,
) -> std::io::Result<(ServerBind, tokio::task::JoinHandle<std::io::Result<()>>)> {
    let ws = TcpListener::bind(config.bind).await?;
    let ws_addr = ws.local_addr()?;
    let idle_timeout = config.idle_timeout;
    let (events, _) = broadcast::channel::<ProtoEvent>(256);
    let state = Arc::new(State {
        control,
        auth: config.auth.clone(),
        events: events.clone(),
        clients: std::sync::atomic::AtomicUsize::new(0),
        max_clients: config.max_clients,
        idle_timeout,
    });
    let task = tokio::spawn(async move { accept_loop(ws, state).await });
    Ok((ServerBind { ws_addr }, task))
}

async fn accept_loop(ws: TcpListener, state: Arc<State>) -> std::io::Result<()> {
    loop {
        let (stream, _) = ws.accept().await?;
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(error) = handle_ws(state, stream).await {
                eprintln!("eiviz api ws: {error}");
            }
        });
    }
}

struct State {
    control: Arc<dyn ControlFacade>,
    auth: AuthConfig,
    events: broadcast::Sender<ProtoEvent>,
    clients: std::sync::atomic::AtomicUsize,
    max_clients: usize,
    idle_timeout: Duration,
}

async fn handle_ws(state: Arc<State>, stream: TcpStream) -> Result<(), String> {
    let clients = state
        .clients
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if clients >= state.max_clients {
        state
            .clients
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        return Err("too many clients".into());
    }
    struct Guard(Arc<State>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0
                .clients
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let _guard = Guard(Arc::clone(&state));
    let callback = |req: &Request, mut response: Response| {
        let proto = req
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if proto.split(',').any(|item| item.trim() == WS_SUBPROTOCOL) {
            response
                .headers_mut()
                .append("Sec-WebSocket-Protocol", WS_SUBPROTOCOL.parse().unwrap());
        }
        Ok(response)
    };
    let mut ws = accept_hdr_async_with_config(
        stream,
        callback,
        Some(
            WebSocketConfig::default()
                .max_message_size(Some(MAX_MESSAGE_BYTES))
                .max_frame_size(Some(MAX_MESSAGE_BYTES)),
        ),
    )
    .await
    .map_err(|error| error.to_string())?;
    let mut instance = String::new();
    let mut role = Role::Read;
    let mut authed = !state.auth.require_auth;
    let mut last_seq = 0u64;
    let mut rx = state.events.subscribe();
    loop {
        tokio::select! {
            msg = tokio::time::timeout(state.idle_timeout, ws.next()) => {
                let msg = match msg {
                    Ok(Some(msg)) => msg.map_err(|error| error.to_string())?,
                    Ok(None) => return Ok(()),
                    Err(_) => return Err("idle timeout".into()),
                };
                match msg {
                    Message::Binary(bytes) => {
                        let env = decode_envelope(&bytes)?;
                        match env.kind {
                            Some(envelope::Kind::Hello(hello)) => {
                                if !state.auth.check(&hello.token) {
                                    let deny = status_envelope("PERMISSION_DENIED", "unauthorized");
                                    ws.send(Message::Binary(encode_envelope(&deny).into()))
                                        .await
                                        .ok();
                                    return Err("unauthorized".into());
                                }
                                instance = hello.client_instance_id;
                                role = Role::from_proto(crate::proto::Role::try_from(hello.role).unwrap_or(crate::proto::Role::Read));
                                authed = true;
                            }
                            Some(envelope::Kind::Request(request)) => {
                                if !authed {
                                    return Err("hello required".into());
                                }
                                if let Some(request::Payload::Subscribe(sub)) = &request.payload {
                                    last_seq = sub.after_sequence;
                                }
                                let response = dispatch(&state, &instance, role, request);
                                ws.send(Message::Binary(encode_envelope(&response).into()))
                                    .await
                                    .map_err(|error| error.to_string())?;
                                flush_events(&state, &mut last_seq);
                            }
                            _ => return Err("unexpected envelope".into()),
                        }
                    }
                    Message::Ping(payload) => {
                        ws.send(Message::Pong(payload)).await.ok();
                    }
                    Message::Close(_) | Message::Text(_) => return Err("invalid frame".into()),
                    _ => {}
                }
            }
            event = rx.recv() => {
                if let Ok(event) = event {
                    let env = Envelope {
                        kind: Some(envelope::Kind::Event(event)),
                    };
                    ws.send(Message::Binary(encode_envelope(&env).into()))
                        .await
                        .map_err(|error| error.to_string())?;
                }
            }
        }
    }
}

fn dispatch(state: &State, instance: &str, role: Role, request: crate::proto::Request) -> Envelope {
    let request_id = request.request_id.clone();
    let required = match request.payload {
        Some(request::Payload::GetCapabilities(_))
        | Some(request::Payload::GetSnapshot(_))
        | Some(request::Payload::Subscribe(_)) => Role::Read,
        Some(request::Payload::Preview(_))
        | Some(request::Payload::Cut(_))
        | Some(request::Payload::Auto(_))
        | Some(request::Payload::SetMix(_))
        | Some(request::Payload::VideoPlay(_))
        | Some(request::Payload::VideoLoop(_))
        | Some(request::Payload::VideoSeek(_))
        | Some(request::Payload::AudioSetInput(_))
        | Some(request::Payload::AudioSetBus(_))
        | Some(request::Payload::SnapshotCmd(_))
        | Some(request::Payload::Discover(_)) => Role::Operate,
        Some(request::Payload::ReplaceSession(_)) => Role::Configure,
        Some(request::Payload::Shutdown(_)) => Role::Admin,
        None => Role::Read,
    };
    if !role.allows(required) {
        return status_envelope("PERMISSION_DENIED", "role too low");
    }
    let result = match request.payload {
        Some(request::Payload::GetCapabilities(_)) | Some(request::Payload::GetSnapshot(_)) => {
            state
                .control
                .snapshot()
                .map(|snap| proto_snapshot(snap, request_id.clone()))
        }
        Some(request::Payload::Subscribe(sub)) => {
            let _ = state.control.events_after(sub.after_sequence);
            state
                .control
                .snapshot()
                .map(|snap| proto_snapshot(snap, request_id.clone()))
        }
        Some(request::Payload::Preview(preview)) => {
            let unit_id = preview.unit.as_ref().map(|r| r.id).unwrap_or(0);
            let scene_id = preview.scene.as_ref().map(|r| r.id).unwrap_or(0);
            state
                .control
                .execute(
                    RequestKey {
                        client_instance_id: instance.into(),
                        request_id: request_id.clone(),
                    },
                    Command::Preview { unit_id, scene_id },
                )
                .and_then(|_| {
                    state
                        .control
                        .snapshot()
                        .map(|snap| proto_snapshot(snap, request_id.clone()))
                })
        }
        Some(request::Payload::Cut(cut)) => {
            let unit_id = cut.unit.as_ref().map(|r| r.id).unwrap_or(0);
            let named = cut
                .input
                .as_ref()
                .is_some_and(|input| input.id != 0 || !input.name.is_empty());
            let incoming = if named {
                Incoming::Source(cut.input.as_ref().map(|input| input.id).unwrap_or(0))
            } else {
                Incoming::Preview
            };
            state
                .control
                .execute(
                    RequestKey {
                        client_instance_id: instance.into(),
                        request_id: request_id.clone(),
                    },
                    Command::Cut {
                        unit_id,
                        swap: cut.swap && !named,
                        incoming,
                    },
                )
                .and_then(|_| {
                    state
                        .control
                        .snapshot()
                        .map(|snap| proto_snapshot(snap, request_id.clone()))
                })
        }
        Some(request::Payload::Auto(auto)) => {
            let unit_id = auto.unit.as_ref().map(|r| r.id).unwrap_or(0);
            state
                .control
                .execute(
                    RequestKey {
                        client_instance_id: instance.into(),
                        request_id: request_id.clone(),
                    },
                    Command::Auto {
                        unit_id,
                        kind: auto.kind,
                        duration_ms: auto.duration_ms,
                        swap: auto.swap,
                        keep_preview: auto.keep_preview,
                        easing: 0,
                        direction: 0,
                        dip_r: 0.0,
                        dip_g: 0.0,
                        dip_b: 0.0,
                        dip_a: 1.0,
                        incoming: Incoming::Preview,
                        softness: 0.02,
                        param: 0.0,
                    },
                )
                .and_then(|_| {
                    state
                        .control
                        .snapshot()
                        .map(|snap| proto_snapshot(snap, request_id.clone()))
                })
        }
        Some(request::Payload::SetMix(mix)) => {
            let unit_id = mix.unit.as_ref().map(|r| r.id).unwrap_or(0);
            state
                .control
                .execute(
                    RequestKey {
                        client_instance_id: instance.into(),
                        request_id: request_id.clone(),
                    },
                    Command::SetMix {
                        unit_id,
                        value: mix.value,
                    },
                )
                .and_then(|_| {
                    state
                        .control
                        .snapshot()
                        .map(|snap| proto_snapshot(snap, request_id.clone()))
                })
        }
        Some(request::Payload::VideoPlay(play)) => exec_cmd(
            state,
            instance,
            &request_id,
            Command::VideoPlay {
                input_id: play.input.as_ref().map(|item| item.id).unwrap_or(0),
                playing: play.playing,
            },
        ),
        Some(request::Payload::VideoLoop(looping)) => exec_cmd(
            state,
            instance,
            &request_id,
            Command::VideoLoop {
                input_id: looping.input.as_ref().map(|item| item.id).unwrap_or(0),
                looping: looping.looping,
            },
        ),
        Some(request::Payload::VideoSeek(seek)) => exec_cmd(
            state,
            instance,
            &request_id,
            Command::VideoSeek {
                input_id: seek.input.as_ref().map(|item| item.id).unwrap_or(0),
                position_hns: seek.position_hns,
            },
        ),
        Some(request::Payload::AudioSetInput(audio)) => exec_cmd(
            state,
            instance,
            &request_id,
            Command::AudioSetInput {
                input_id: audio.input.as_ref().map(|item| item.id).unwrap_or(0),
                bus_mask: audio.bus_mask,
                gain: audio.gain,
                mute: audio.mute,
            },
        ),
        Some(request::Payload::AudioSetBus(audio)) => exec_cmd(
            state,
            instance,
            &request_id,
            Command::AudioSetBus {
                bus_id: audio.bus.as_ref().map(|item| item.id).unwrap_or(0),
                gain: audio.gain,
                mute: audio.mute,
            },
        ),
        Some(request::Payload::SnapshotCmd(snap)) => exec_cmd(
            state,
            instance,
            &request_id,
            Command::Snapshot {
                unit_id: snap.unit.as_ref().map(|item| item.id).unwrap_or(0),
                kind: match snap.kind {
                    1 => eiviz_control::command::SnapshotKind::Preview,
                    3 => eiviz_control::command::SnapshotKind::Source(0),
                    _ => eiviz_control::command::SnapshotKind::Program,
                },
                path: snap.path,
            },
        ),
        Some(request::Payload::Discover(discover)) => exec_cmd(
            state,
            instance,
            &request_id,
            Command::Discover {
                kind: match discover.kind.as_str() {
                    "ndi" => eiviz_control::command::DiscoverKind::Ndi,
                    "audio" => eiviz_control::command::DiscoverKind::Audio,
                    _ => eiviz_control::command::DiscoverKind::Omt,
                },
            },
        ),
        Some(request::Payload::ReplaceSession(replace)) => {
            match eiviz_control::session::parse(&replace.document_json) {
                Ok(document) => state
                    .control
                    .execute(
                        RequestKey {
                            client_instance_id: instance.into(),
                            request_id: request_id.clone(),
                        },
                        Command::ReplaceSession {
                            document: Box::new(document),
                            expected_revision: if replace.expected_revision == 0 {
                                None
                            } else {
                                Some(replace.expected_revision)
                            },
                        },
                    )
                    .and_then(|_| {
                        state
                            .control
                            .snapshot()
                            .map(|snap| proto_snapshot(snap, request_id.clone()))
                    }),
                Err(error) => Err(ControlError::invalid(error)),
            }
        }
        Some(request::Payload::Shutdown(_)) => state
            .control
            .execute(
                RequestKey {
                    client_instance_id: instance.into(),
                    request_id: request_id.clone(),
                },
                Command::Shutdown,
            )
            .and_then(|_| {
                state
                    .control
                    .snapshot()
                    .map(|snap| proto_snapshot(snap, request_id.clone()))
            }),
        None => Err(ControlError::invalid("empty request")),
    };
    match result {
        Ok(response) => Envelope {
            kind: Some(envelope::Kind::Response(response)),
        },
        Err(error) => status_envelope(error.code(), error.message()),
    }
}

fn exec_cmd(
    state: &State,
    instance: &str,
    request_id: &str,
    command: Command,
) -> Result<ProtoResponse, ControlError> {
    state
        .control
        .execute(
            RequestKey {
                client_instance_id: instance.into(),
                request_id: request_id.into(),
            },
            command,
        )
        .and_then(|_| {
            state
                .control
                .snapshot()
                .map(|snap| proto_snapshot(snap, request_id.into()))
        })
}

fn proto_snapshot(snapshot: eiviz_control::Snapshot, request_id: String) -> ProtoResponse {
    let json = eiviz_control::session::to_vec(&snapshot.document).unwrap_or_default();
    ProtoResponse {
        request_id,
        status: Some(Status {
            code: "OK".into(),
            message: String::new(),
        }),
        revision: snapshot.revision,
        sequence: snapshot.sequence,
        payload: Some(crate::proto::response::Payload::Snapshot(ProtoSnapshot {
            revision: snapshot.revision,
            sequence: snapshot.sequence,
            document_json: json,
            lifecycle: snapshot.lifecycle.as_str().into(),
        })),
    }
}

fn status_envelope(code: &str, message: &str) -> Envelope {
    Envelope {
        kind: Some(envelope::Kind::Response(ProtoResponse {
            request_id: String::new(),
            status: Some(Status {
                code: code.into(),
                message: message.into(),
            }),
            revision: 0,
            sequence: 0,
            payload: None,
        })),
    }
}

fn flush_events(state: &State, last_seq: &mut u64) {
    let events = state.control.events_after(*last_seq);
    for event in events {
        *last_seq = (*last_seq).max(event.meta().sequence);
        let proto = ProtoEvent {
            sequence: event.meta().sequence,
            session_revision: event.meta().session_revision,
            unix_ms: event.meta().unix_ms,
            request_id: event.meta().request_id.clone(),
            kind: event.kind_name().into(),
            status: None,
            snapshot: None,
        };
        let _ = state.events.send(proto);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuthConfig;
    use crate::client::ControlClient;
    use eiviz_control::{ControlFacade, ControlService, NullMixer};

    const BARS: &[u8] = br#"{
      "version": 2,
      "inputs": [{ "id": 2, "name": "Bars", "kind": "Bars" }],
      "scenes": [
        { "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] },
        { "id": 2, "name": "Scene 2", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }
      ],
      "units": [{ "id": 1, "name": "MU 1" }]
    }"#;

    fn ready_control() -> Arc<dyn ControlFacade> {
        let svc = std::sync::Mutex::new(ControlService::new(NullMixer::default()));
        let doc = eiviz_control::parse(BARS).unwrap();
        svc.lock()
            .unwrap()
            .replace_session(doc, None, "boot")
            .unwrap();
        Arc::new(svc)
    }

    #[tokio::test]
    async fn websocket_snapshot_and_auth() {
        let control = ready_control();
        let config = ServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            auth: AuthConfig {
                token: "secret".into(),
                require_auth: true,
            },
            idle_timeout: Duration::from_secs(5),
            max_clients: 4,
        };
        let (bind, task) = listen(config, control).await.unwrap();
        let url = format!("ws://{}", bind.ws_addr);
        let ok = ControlClient::websocket(&url, "secret");
        assert!(ok.snapshot_json().await.is_ok());
        let bad = ControlClient::websocket(&url, "nope");
        assert!(bad.snapshot_json().await.is_err());
        task.abort();
    }

    #[tokio::test]
    async fn websocket_cut_goes_through_control_service() {
        let control = ready_control();
        let config = ServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            auth: AuthConfig {
                token: String::new(),
                require_auth: false,
            },
            idle_timeout: Duration::from_secs(5),
            max_clients: 4,
        };
        let (bind, task) = listen(config, Arc::clone(&control)).await.unwrap();
        let url = format!("ws://{}", bind.ws_addr);
        ControlClient::websocket(&url, "")
            .cut(1, true)
            .await
            .unwrap();
        let snap = control.snapshot().unwrap();
        assert_eq!(snap.revision, 1);
        task.abort();
    }
}
