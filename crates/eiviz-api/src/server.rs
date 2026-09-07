use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::auth::{AuthConfig, Role};
use crate::codec::{MAX_MESSAGE_BYTES, WS_SUBPROTOCOL, decode_envelope, encode_envelope};
use crate::media::{DEFAULT_CHUNK_SIZE, MediaStorage, parse_kind};
use crate::proto::{
    Envelope, Event as ProtoEvent, Response as ProtoResponse, Snapshot as ProtoSnapshot, Status,
    UploadAccepted, envelope, request, response,
};
use eiviz_control::error::ControlError;
use eiviz_control::{Command, ControlFacade, Incoming, RequestKey, SessionMutation};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::accept_hdr_async_with_config;
use tokio_tungstenite::tungstenite::{
    Message,
    handshake::server::{Request, Response},
    protocol::WebSocketConfig,
};

#[derive(Clone)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub auth: AuthConfig,
    pub idle_timeout: Duration,
    pub max_clients: usize,
    pub media: Option<Arc<dyn MediaStorage>>,
}

impl ServerConfig {
    pub fn loopback(port: u16, auth: AuthConfig) -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], port)),
            auth,
            idle_timeout: Duration::from_secs(60),
            max_clients: 32,
            media: None,
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
    let state = Arc::new(State {
        control,
        auth: config.auth.clone(),
        clients: std::sync::atomic::AtomicUsize::new(0),
        max_clients: config.max_clients,
        idle_timeout: config.idle_timeout,
        media: config.media.clone(),
        pending: Mutex::new(HashMap::new()),
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

struct PendingUpload {
    kind: eiviz_control::session::InputKind,
    input_name: String,
    video_loop: bool,
}

struct State {
    control: Arc<dyn ControlFacade>,
    auth: AuthConfig,
    clients: std::sync::atomic::AtomicUsize,
    max_clients: usize,
    idle_timeout: Duration,
    media: Option<Arc<dyn MediaStorage>>,
    pending: Mutex<HashMap<String, PendingUpload>>,
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
            Ok(response)
        } else {
            Err(tokio_tungstenite::tungstenite::http::Response::builder()
                .status(400)
                .body(None)
                .unwrap())
        }
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
    let mut subscribed = false;
    let mut last_activity = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(16));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut meter_ticks = 0u8;
    loop {
        if last_activity.elapsed() > state.idle_timeout {
            return Err("idle timeout".into());
        }
        tokio::select! {
            msg = ws.next() => {
                let Some(msg) = msg else { return Ok(()); };
                let msg = msg.map_err(|error| error.to_string())?;
                last_activity = Instant::now();
                match msg {
                    Message::Binary(bytes) => {
                        let env = decode_envelope(&bytes)?;
                        match env.kind {
                            Some(envelope::Kind::Hello(hello)) => {
                                if hello.protocol != WS_SUBPROTOCOL && !hello.protocol.is_empty() {
                                    let deny = status_envelope("", "INVALID_ARGUMENT", "protocol mismatch");
                                    ws.send(Message::Binary(encode_envelope(&deny).into())).await.ok();
                                    return Err("protocol mismatch".into());
                                }
                                if !state.auth.check(&hello.token) {
                                    let deny = status_envelope("", "PERMISSION_DENIED", "unauthorized");
                                    ws.send(Message::Binary(encode_envelope(&deny).into())).await.ok();
                                    return Err("unauthorized".into());
                                }
                                instance = if hello.client_instance_id.is_empty() {
                                    uuid::Uuid::new_v4().to_string()
                                } else {
                                    hello.client_instance_id
                                };
                                let requested = Role::from_proto(
                                    crate::proto::Role::try_from(hello.role).unwrap_or(crate::proto::Role::Read),
                                );
                                role = state.auth.granted_role(requested);
                                authed = true;
                            }
                            Some(envelope::Kind::Request(request)) => {
                                if !authed {
                                    return Err("hello required".into());
                                }
                                if let Some(request::Payload::Subscribe(sub)) = &request.payload {
                                    last_seq = sub.after_sequence;
                                    subscribed = true;
                                }
                                let response = dispatch(&state, &instance, role, request);
                                ws.send(Message::Binary(encode_envelope(&response).into()))
                                    .await
                                    .map_err(|error| error.to_string())?;
                                last_activity = Instant::now();
                            }
                            _ => return Err("unexpected envelope".into()),
                        }
                    }
                    Message::Ping(payload) => {
                        ws.send(Message::Pong(payload)).await.ok();
                    }
                    Message::Pong(_) => {}
                    Message::Close(_) | Message::Text(_) => return Err("invalid frame".into()),
                    _ => {}
                }
            }
            _ = tick.tick() => {
                if !subscribed {
                    continue;
                }
                meter_ticks = meter_ticks.wrapping_add(1);
                if meter_ticks.is_multiple_of(3) {
                    state.control.publish_meters();
                }
                last_activity = Instant::now();
                let events = drain_events(&state, &mut last_seq);
                for event in events {
                    ws.send(Message::Binary(encode_envelope(&event).into()))
                        .await
                        .map_err(|error| error.to_string())?;
                    last_activity = Instant::now();
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
        | Some(request::Payload::OverlayAuto(_))
        | Some(request::Payload::VideoPlay(_))
        | Some(request::Payload::VideoLoop(_))
        | Some(request::Payload::VideoSeek(_))
        | Some(request::Payload::AudioSetInput(_))
        | Some(request::Payload::AudioSetBus(_))
        | Some(request::Payload::Discover(_)) => Role::Operate,
        Some(request::Payload::ReplaceSession(_))
        | Some(request::Payload::MutateSession(_))
        | Some(request::Payload::BeginMediaUpload(_))
        | Some(request::Payload::UploadMediaChunk(_))
        | Some(request::Payload::CommitMediaUpload(_))
        | Some(request::Payload::AbortMediaUpload(_)) => Role::Configure,
        Some(request::Payload::SnapshotCmd(_)) | Some(request::Payload::Shutdown(_)) => Role::Admin,
        None => Role::Read,
    };
    if !role.allows(required) {
        return status_envelope(&request_id, "PERMISSION_DENIED", "role too low");
    }
    let result = match request.payload {
        Some(request::Payload::GetCapabilities(_)) => state
            .control
            .snapshot()
            .map(|snap| cap_response(snap, request_id.clone())),
        Some(request::Payload::GetSnapshot(_)) | Some(request::Payload::Subscribe(_)) => state
            .control
            .snapshot()
            .map(|snap| proto_snapshot(snap, request_id.clone())),
        Some(request::Payload::Preview(preview)) => {
            let unit_id = preview.unit.as_ref().map(|r| r.id).unwrap_or(0);
            let scene_id = preview.scene.as_ref().map(|r| r.id).unwrap_or(0);
            exec_cmd(
                state,
                instance,
                &request_id,
                Command::Preview { unit_id, scene_id },
            )
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
            exec_cmd(
                state,
                instance,
                &request_id,
                Command::Cut {
                    unit_id,
                    swap: cut.swap && !named,
                    incoming,
                },
            )
        }
        Some(request::Payload::Auto(auto)) => {
            let unit_id = auto.unit.as_ref().map(|r| r.id).unwrap_or(0);
            exec_cmd(
                state,
                instance,
                &request_id,
                Command::Auto {
                    unit_id,
                    kind: auto.kind,
                    duration_ms: auto.duration_ms,
                    swap: auto.swap,
                    keep_preview: auto.keep_preview,
                    easing: auto.easing,
                    direction: auto.direction,
                    dip_r: auto.dip_r,
                    dip_g: auto.dip_g,
                    dip_b: auto.dip_b,
                    dip_a: if auto.dip_a <= 0.0 { 1.0 } else { auto.dip_a },
                    incoming: Incoming::Preview,
                    softness: if auto.softness <= 0.0 {
                        0.02
                    } else {
                        auto.softness
                    },
                    param: auto.param,
                },
            )
        }
        Some(request::Payload::SetMix(mix)) => {
            let unit_id = mix.unit.as_ref().map(|r| r.id).unwrap_or(0);
            exec_cmd(
                state,
                instance,
                &request_id,
                Command::SetMix {
                    unit_id,
                    value: mix.value,
                },
            )
        }
        Some(request::Payload::OverlayAuto(overlay)) => exec_cmd(
            state,
            instance,
            &request_id,
            Command::OverlayAuto {
                unit_id: overlay.unit.as_ref().map(|item| item.id).unwrap_or(0),
                index: overlay.index,
                duration_ms: overlay.duration_ms,
                to_on: overlay.to_on,
            },
        ),
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
                Ok(document) => exec_cmd(
                    state,
                    instance,
                    &request_id,
                    Command::ReplaceSession {
                        document: Box::new(document),
                        expected_revision: if replace.expected_revision == 0 {
                            None
                        } else {
                            Some(replace.expected_revision)
                        },
                    },
                ),
                Err(error) => Err(ControlError::invalid(error)),
            }
        }
        Some(request::Payload::MutateSession(mutate)) => {
            match serde_json::from_slice::<SessionMutation>(&mutate.mutation_json) {
                Ok(mutation) => exec_cmd(
                    state,
                    instance,
                    &request_id,
                    Command::MutateSession {
                        mutation: Box::new(mutation),
                        expected_revision: if mutate.expected_revision == 0 {
                            None
                        } else {
                            Some(mutate.expected_revision)
                        },
                    },
                ),
                Err(error) => Err(ControlError::invalid(error.to_string())),
            }
        }
        Some(request::Payload::BeginMediaUpload(begin)) => begin_upload(state, &request_id, begin),
        Some(request::Payload::UploadMediaChunk(chunk)) => write_chunk(state, &request_id, chunk),
        Some(request::Payload::CommitMediaUpload(commit)) => {
            commit_upload(state, instance, &request_id, commit)
        }
        Some(request::Payload::AbortMediaUpload(abort)) => {
            if let Some(media) = &state.media {
                let _ = media.abort(&abort.upload_id);
            }
            if let Ok(mut pending) = state.pending.lock() {
                pending.remove(&abort.upload_id);
            }
            state
                .control
                .snapshot()
                .map(|snap| proto_snapshot(snap, request_id.clone()))
        }
        Some(request::Payload::Shutdown(_)) => {
            exec_cmd(state, instance, &request_id, Command::Shutdown)
        }
        None => Err(ControlError::invalid("empty request")),
    };
    match result {
        Ok(response) => Envelope {
            kind: Some(envelope::Kind::Response(response)),
        },
        Err(error) => status_envelope(&request_id, error.code(), error.message()),
    }
}

fn begin_upload(
    state: &State,
    request_id: &str,
    begin: crate::proto::BeginMediaUpload,
) -> Result<ProtoResponse, ControlError> {
    let media = state
        .media
        .as_ref()
        .ok_or_else(|| ControlError::unavailable("media storage is not configured"))?;
    let kind = parse_kind(&begin.media_kind)?;
    let upload_id = media.begin(&begin.file_name, kind, begin.size_bytes, &begin.sha256_hex)?;
    if let Ok(mut pending) = state.pending.lock() {
        pending.insert(
            upload_id.clone(),
            PendingUpload {
                kind,
                input_name: begin.input_name,
                video_loop: begin.video_loop,
            },
        );
    }
    Ok(ProtoResponse {
        request_id: request_id.into(),
        status: Some(Status {
            code: "OK".into(),
            message: String::new(),
        }),
        revision: 0,
        sequence: 0,
        payload: Some(response::Payload::UploadAccepted(UploadAccepted {
            upload_id,
            chunk_size: DEFAULT_CHUNK_SIZE,
        })),
    })
}

fn write_chunk(
    state: &State,
    request_id: &str,
    chunk: crate::proto::UploadMediaChunk,
) -> Result<ProtoResponse, ControlError> {
    let media = state
        .media
        .as_ref()
        .ok_or_else(|| ControlError::unavailable("media storage is not configured"))?;
    media.write_chunk(&chunk.upload_id, chunk.offset, &chunk.data)?;
    Ok(ProtoResponse {
        request_id: request_id.into(),
        status: Some(Status {
            code: "OK".into(),
            message: String::new(),
        }),
        revision: 0,
        sequence: 0,
        payload: None,
    })
}

fn commit_upload(
    state: &State,
    instance: &str,
    request_id: &str,
    commit: crate::proto::CommitMediaUpload,
) -> Result<ProtoResponse, ControlError> {
    let media = state
        .media
        .as_ref()
        .ok_or_else(|| ControlError::unavailable("media storage is not configured"))?;
    let pending = state
        .pending
        .lock()
        .ok()
        .and_then(|mut slot| slot.remove(&commit.upload_id))
        .ok_or_else(|| ControlError::not_found("upload"))?;
    let host_path = media.commit(&commit.upload_id)?;
    let result = exec_cmd(
        state,
        instance,
        request_id,
        Command::MutateSession {
            mutation: Box::new(SessionMutation::AddMediaInput {
                name: pending.input_name,
                media_kind: pending.kind,
                host_path: host_path.to_string_lossy().into_owned(),
                video_loop: pending.video_loop,
                tags: vec![],
            }),
            expected_revision: if commit.expected_revision == 0 {
                None
            } else {
                Some(commit.expected_revision)
            },
        },
    );
    if result.is_err() {
        let _ = std::fs::remove_file(&host_path);
    }
    result
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
    let live = serde_json::to_vec(&snapshot.live).unwrap_or_default();
    let resources = serde_json::to_vec(&snapshot.resources).unwrap_or_default();
    let capabilities = snapshot.capabilities.clone();
    ProtoResponse {
        request_id,
        status: Some(Status {
            code: "OK".into(),
            message: String::new(),
        }),
        revision: snapshot.revision,
        sequence: snapshot.sequence,
        payload: Some(response::Payload::Snapshot(ProtoSnapshot {
            revision: snapshot.revision,
            sequence: snapshot.sequence,
            document_json: json,
            lifecycle: snapshot.lifecycle.as_str().into(),
            live_json: live,
            resources_json: resources,
            epoch: snapshot.epoch,
            capabilities: Some(crate::proto::Capabilities {
                protocol_version: capabilities.protocol_version,
                mixer_version: capabilities.mixer_version,
                platforms: capabilities.platforms,
                commands: capabilities.commands,
                presentation_abi: capabilities.presentation_abi,
                native_api: capabilities.native_api,
                vmix_http: capabilities.vmix_http,
            }),
        })),
    }
}

fn cap_response(snapshot: eiviz_control::Snapshot, request_id: String) -> ProtoResponse {
    let mut response = proto_snapshot(snapshot, request_id);
    if let Some(response::Payload::Snapshot(snap)) = &response.payload {
        response.payload = Some(response::Payload::Capabilities(
            snap.capabilities.clone().unwrap_or_default(),
        ));
    }
    response
}

fn status_envelope(request_id: &str, code: &str, message: &str) -> Envelope {
    Envelope {
        kind: Some(envelope::Kind::Response(ProtoResponse {
            request_id: request_id.into(),
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

fn drain_events(state: &State, last_seq: &mut u64) -> Vec<Envelope> {
    let epoch = state.control.epoch();
    let events = state.control.events_after(*last_seq);
    let mut out = Vec::new();
    for event in events {
        *last_seq = (*last_seq).max(event.meta().sequence);
        out.push(Envelope {
            kind: Some(envelope::Kind::Event(proto_event(&event, &epoch))),
        });
    }
    out
}

fn proto_event(event: &eiviz_control::Event, epoch: &str) -> ProtoEvent {
    let meta = event.meta();
    let mut proto = ProtoEvent {
        sequence: meta.sequence,
        session_revision: meta.session_revision,
        unix_ms: meta.unix_ms,
        request_id: meta.request_id.clone(),
        kind: event.kind_name().into(),
        status: None,
        snapshot: None,
        live_json: Vec::new(),
        document_json: Vec::new(),
        epoch: epoch.into(),
        discover_kind: String::new(),
        discover_payload: String::new(),
    };
    match event {
        eiviz_control::Event::SessionChanged { document, .. } => {
            proto.document_json = eiviz_control::session::to_vec(document).unwrap_or_default();
        }
        eiviz_control::Event::LiveChanged { live, .. } => {
            proto.live_json = serde_json::to_vec(live).unwrap_or_default();
        }
        eiviz_control::Event::Failed { error, .. } => {
            proto.status = Some(Status {
                code: error.code().into(),
                message: error.message().into(),
            });
        }
        eiviz_control::Event::Discovered { kind, payload, .. } => {
            proto.discover_kind = kind.clone();
            proto.discover_payload = payload.clone();
        }
        _ => {}
    }
    proto
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

    fn config(auth: AuthConfig) -> ServerConfig {
        ServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            auth,
            idle_timeout: Duration::from_secs(5),
            max_clients: 4,
            media: None,
        }
    }

    #[tokio::test]
    async fn websocket_snapshot_and_auth() {
        let control = ready_control();
        let (bind, task) = listen(
            config(AuthConfig {
                token: "secret".into(),
                require_auth: true,
                max_role: Role::Admin,
            }),
            control,
        )
        .await
        .unwrap();
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
        let (bind, task) = listen(
            config(AuthConfig {
                token: String::new(),
                require_auth: false,
                max_role: Role::Admin,
            }),
            Arc::clone(&control),
        )
        .await
        .unwrap();
        let url = format!("ws://{}", bind.ws_addr);
        ControlClient::websocket(&url, "")
            .cut(1, true)
            .await
            .unwrap();
        let snap = control.snapshot().unwrap();
        assert_eq!(snap.revision, 1);
        task.abort();
    }

    #[tokio::test]
    async fn client_cannot_elevate_past_max_role() {
        let control = ready_control();
        let (bind, task) = listen(
            config(AuthConfig {
                token: "secret".into(),
                require_auth: true,
                max_role: Role::Read,
            }),
            control,
        )
        .await
        .unwrap();
        let url = format!("ws://{}", bind.ws_addr);
        let client = ControlClient::websocket(&url, "secret");
        assert!(client.snapshot_json().await.is_ok());
        assert!(client.cut(1, true).await.is_err());
        task.abort();
    }

    #[tokio::test]
    async fn two_subscribers_see_the_same_cut() {
        let control = ready_control();
        let (bind, task) = listen(
            config(AuthConfig {
                token: String::new(),
                require_auth: false,
                max_role: Role::Admin,
            }),
            Arc::clone(&control),
        )
        .await
        .unwrap();
        let url = format!("ws://{}", bind.ws_addr);
        let a = ControlClient::websocket(&url, "").connect().await.unwrap();
        let b = ControlClient::websocket(&url, "").connect().await.unwrap();
        a.subscribe(0).await.unwrap();
        b.subscribe(0).await.unwrap();
        a.cut(1, true).await.unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
        let events_a = a.take_events();
        let events_b = b.take_events();
        assert!(
            events_a
                .iter()
                .any(|kind| kind == "LiveChanged" || kind == "CommandApplied")
        );
        assert!(
            events_b
                .iter()
                .any(|kind| kind == "LiveChanged" || kind == "CommandApplied")
        );
        task.abort();
    }

    #[tokio::test]
    async fn upload_commit_adds_still_input() {
        let dir = std::env::temp_dir().join(format!("eiviz-upload-{}", uuid::Uuid::new_v4()));
        let store =
            crate::FileMediaStorage::new(crate::MediaStorageConfig::new(dir.clone())).unwrap();
        let control = ready_control();
        let (bind, task) = listen(
            ServerConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                auth: AuthConfig {
                    token: String::new(),
                    require_auth: false,
                    max_role: Role::Admin,
                },
                idle_timeout: Duration::from_secs(5),
                max_clients: 4,
                media: Some(Arc::new(store)),
            },
            Arc::clone(&control),
        )
        .await
        .unwrap();
        let url = format!("ws://{}", bind.ws_addr);
        let session = ControlClient::websocket(&url, "").connect().await.unwrap();
        let src = std::env::temp_dir().join(format!("eiviz-src-{}.png", uuid::Uuid::new_v4()));
        std::fs::write(&src, b"\x89PNG\r\n").unwrap();
        session
            .upload_file(&src, "still", "Logo", true, 1)
            .await
            .unwrap();
        let snap = control.snapshot().unwrap();
        assert!(
            snap.document
                .inputs
                .iter()
                .any(|input| input.name == "Logo")
        );
        task.abort();
        let _ = std::fs::remove_dir_all(dir);
    }
}
