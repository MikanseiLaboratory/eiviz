use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::codec::{WS_SUBPROTOCOL, decode_envelope, encode_envelope};
use crate::media::DEFAULT_CHUNK_SIZE;
use crate::proto::{
    AbortMediaUpload, BeginMediaUpload, ClientHello, CommitMediaUpload, Cut, Envelope, Event,
    GetSnapshot, OverlayAuto, Request, Response, Role, Subscribe, UploadMediaChunk, envelope,
    request, response,
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
        if let Some(response::Payload::Snapshot(snapshot)) = response.payload {
            String::from_utf8(snapshot.document_json)
                .map_err(|error| ControlError::internal(error.to_string()))
        } else {
            Err(ControlError::unavailable("snapshot missing"))
        }
    }

    pub async fn discover(&self, kind: &str, query: &str) -> ControlResult<String> {
        discover_payload(self.roundtrip(discover_req(kind, query)).await?)
    }

    pub async fn cut(&self, unit_id: u64, swap: bool) -> ControlResult<()> {
        let response = self.roundtrip(live_cut(unit_id, swap)).await?;
        status_ok(&response)
    }

    pub async fn preview(&self, unit_id: u64, scene_id: u64) -> ControlResult<()> {
        status_ok(&self.roundtrip(preview_req(unit_id, scene_id)).await?)
    }

    pub async fn auto(&self, unit_id: u64, duration_ms: u32, swap: bool) -> ControlResult<()> {
        self.auto_full(
            unit_id,
            0,
            duration_ms,
            swap,
            true,
            0,
            0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.02,
            0.0,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn auto_full(
        &self,
        unit_id: u64,
        kind: u32,
        duration_ms: u32,
        swap: bool,
        keep_preview: bool,
        easing: u32,
        direction: u32,
        dip_r: f32,
        dip_g: f32,
        dip_b: f32,
        dip_a: f32,
        softness: f32,
        param: f32,
    ) -> ControlResult<()> {
        status_ok(
            &self
                .roundtrip(auto_req(
                    unit_id,
                    kind,
                    duration_ms,
                    swap,
                    keep_preview,
                    easing,
                    direction,
                    dip_r,
                    dip_g,
                    dip_b,
                    dip_a,
                    softness,
                    param,
                ))
                .await?,
        )
    }

    pub async fn overlay_auto(
        &self,
        unit_id: u64,
        index: u32,
        duration_ms: u32,
        to_on: bool,
    ) -> ControlResult<()> {
        status_ok(
            &self
                .roundtrip(overlay_req(unit_id, index, duration_ms, to_on))
                .await?,
        )
    }

    pub async fn mutate_session(
        &self,
        mutation_json: Vec<u8>,
        expected_revision: u64,
    ) -> ControlResult<()> {
        status_ok(
            &self
                .roundtrip(mutate_req(mutation_json, expected_revision))
                .await?,
        )
    }

    pub async fn replace_session(
        &self,
        document_json: Vec<u8>,
        expected_revision: u64,
    ) -> ControlResult<()> {
        let response = self
            .roundtrip(replace_req(document_json, expected_revision))
            .await?;
        status_ok(&response)
    }

    pub async fn save_session(&self) -> ControlResult<(String, u32)> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::SaveSession(crate::proto::SaveSession {})),
            })
            .await?;
        status_ok(&response)?;
        match response.payload {
            Some(response::Payload::SavedSession(saved)) => Ok((saved.path, saved.history_count)),
            _ => Err(ControlError::unavailable("save session missing")),
        }
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

    pub async fn connect(&self) -> ControlResult<ControlSession> {
        ControlSession::open(self.endpoint.clone(), self.token.clone()).await
    }

    async fn roundtrip(&self, request: Request) -> ControlResult<Response> {
        let session = self.connect().await?;
        session.roundtrip(request).await
    }
}

#[derive(Debug, Clone, Default)]
pub struct SessionView {
    pub connected: bool,
    pub epoch: String,
    pub revision: u64,
    pub sequence: u64,
    pub document_json: Vec<u8>,
    pub live_json: Vec<u8>,
    /// Session revision last time `document_json` was replaced. Live ops keep
    /// `revision` in sync without implying a document reload.
    pub document_revision: u64,
    pub error: String,
    pub lag: bool,
}

pub struct ControlSession {
    tx: mpsc::Sender<SessionOp>,
    events: Arc<Mutex<Vec<String>>>,
    view: Arc<Mutex<SessionView>>,
}

enum SessionOp {
    Request {
        request: Box<Request>,
        reply: oneshot::Sender<ControlResult<Response>>,
    },
    Close {
        reply: oneshot::Sender<()>,
    },
}

impl ControlSession {
    async fn open(endpoint: String, token: String) -> ControlResult<Self> {
        let (tx, rx) = mpsc::channel::<SessionOp>(32);
        let events = Arc::new(Mutex::new(Vec::new()));
        let view = Arc::new(Mutex::new(SessionView::default()));
        let events_worker = Arc::clone(&events);
        let view_worker = Arc::clone(&view);
        let (ready_tx, ready_rx) = oneshot::channel();
        tokio::spawn(async move {
            session_supervisor(endpoint, token, rx, events_worker, view_worker, ready_tx).await;
        });
        ready_rx
            .await
            .map_err(|_| ControlError::unavailable("client worker"))??;
        Ok(Self { tx, events, view })
    }

    pub fn view(&self) -> SessionView {
        self.with_view(SessionView::clone).unwrap_or_default()
    }

    pub fn with_view<R>(&self, f: impl FnOnce(&SessionView) -> R) -> Option<R> {
        self.view.lock().ok().map(|slot| f(&slot))
    }

    pub async fn close(&self) {
        let (reply, done) = oneshot::channel();
        if self.tx.send(SessionOp::Close { reply }).await.is_err() {
            return;
        }
        let _ = done.await;
    }

    pub async fn subscribe(&self, after_sequence: u64) -> ControlResult<Response> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::Subscribe(Subscribe { after_sequence })),
            })
            .await?;
        apply_response(&self.view, &response);
        Ok(response)
    }

    pub async fn snapshot(&self) -> ControlResult<Response> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::GetSnapshot(GetSnapshot {})),
            })
            .await?;
        apply_response(&self.view, &response);
        status_ok(&response)?;
        Ok(response)
    }

    pub async fn snapshot_json(&self) -> ControlResult<String> {
        let response = self.snapshot().await?;
        if let Some(response::Payload::Snapshot(snapshot)) = response.payload {
            String::from_utf8(snapshot.document_json)
                .map_err(|error| ControlError::internal(error.to_string()))
        } else {
            Err(ControlError::unavailable("snapshot missing"))
        }
    }

    pub async fn cut(&self, unit_id: u64, swap: bool) -> ControlResult<()> {
        let response = self.roundtrip(live_cut(unit_id, swap)).await?;
        apply_response(&self.view, &response);
        status_ok(&response)
    }

    pub async fn preview(&self, unit_id: u64, scene_id: u64) -> ControlResult<()> {
        let response = self.roundtrip(preview_req(unit_id, scene_id)).await?;
        apply_response(&self.view, &response);
        status_ok(&response)
    }

    pub async fn auto(&self, unit_id: u64, duration_ms: u32, swap: bool) -> ControlResult<()> {
        self.auto_full(
            unit_id,
            0,
            duration_ms,
            swap,
            true,
            0,
            0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.02,
            0.0,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn auto_full(
        &self,
        unit_id: u64,
        kind: u32,
        duration_ms: u32,
        swap: bool,
        keep_preview: bool,
        easing: u32,
        direction: u32,
        dip_r: f32,
        dip_g: f32,
        dip_b: f32,
        dip_a: f32,
        softness: f32,
        param: f32,
    ) -> ControlResult<()> {
        let response = self
            .roundtrip(auto_req(
                unit_id,
                kind,
                duration_ms,
                swap,
                keep_preview,
                easing,
                direction,
                dip_r,
                dip_g,
                dip_b,
                dip_a,
                softness,
                param,
            ))
            .await?;
        apply_response(&self.view, &response);
        status_ok(&response)
    }

    pub async fn set_mix(&self, unit_id: u64, value: f32) -> ControlResult<()> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::SetMix(crate::proto::SetMix {
                    unit: Some(ref_unit(unit_id)),
                    value,
                })),
            })
            .await?;
        apply_response(&self.view, &response);
        status_ok(&response)
    }

    pub async fn overlay_auto(
        &self,
        unit_id: u64,
        index: u32,
        duration_ms: u32,
        to_on: bool,
    ) -> ControlResult<()> {
        let response = self
            .roundtrip(overlay_req(unit_id, index, duration_ms, to_on))
            .await?;
        apply_response(&self.view, &response);
        status_ok(&response)
    }

    pub async fn mutate_session(
        &self,
        mutation_json: Vec<u8>,
        expected_revision: u64,
    ) -> ControlResult<()> {
        let response = self
            .roundtrip(mutate_req(mutation_json, expected_revision))
            .await?;
        apply_response(&self.view, &response);
        status_ok(&response)
    }

    pub async fn replace_session(
        &self,
        document_json: Vec<u8>,
        expected_revision: u64,
    ) -> ControlResult<()> {
        let response = self
            .roundtrip(replace_req(document_json, expected_revision))
            .await?;
        apply_response(&self.view, &response);
        status_ok(&response)
    }

    pub async fn save_session(&self) -> ControlResult<(String, u32)> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::SaveSession(crate::proto::SaveSession {})),
            })
            .await?;
        apply_response(&self.view, &response);
        status_ok(&response)?;
        match response.payload {
            Some(response::Payload::SavedSession(saved)) => Ok((saved.path, saved.history_count)),
            _ => Err(ControlError::unavailable("save session missing")),
        }
    }

    pub async fn shutdown(&self) -> ControlResult<()> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::Shutdown(crate::proto::Shutdown {})),
            })
            .await?;
        apply_response(&self.view, &response);
        status_ok(&response)
    }

    pub async fn video_play(&self, input_id: u64, playing: bool) -> ControlResult<()> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::VideoPlay(crate::proto::VideoPlay {
                    input: Some(ref_input(input_id)),
                    playing,
                })),
            })
            .await?;
        apply_response(&self.view, &response);
        status_ok(&response)
    }

    pub async fn video_loop(&self, input_id: u64, looping: bool) -> ControlResult<()> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::VideoLoop(crate::proto::VideoLoop {
                    input: Some(ref_input(input_id)),
                    looping,
                })),
            })
            .await?;
        apply_response(&self.view, &response);
        status_ok(&response)
    }

    pub async fn video_seek(&self, input_id: u64, position_hns: i64) -> ControlResult<()> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::VideoSeek(crate::proto::VideoSeek {
                    input: Some(ref_input(input_id)),
                    position_hns,
                })),
            })
            .await?;
        apply_response(&self.view, &response);
        status_ok(&response)
    }

    pub async fn audio_set_input(
        &self,
        input_id: u64,
        bus_mask: u32,
        gain: f32,
        mute: bool,
    ) -> ControlResult<()> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::AudioSetInput(
                    crate::proto::AudioSetInput {
                        input: Some(ref_input(input_id)),
                        bus_mask,
                        gain,
                        mute,
                    },
                )),
            })
            .await?;
        apply_response(&self.view, &response);
        status_ok(&response)
    }

    pub async fn audio_set_bus(&self, bus_id: u64, gain: f32, mute: bool) -> ControlResult<()> {
        let response = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::AudioSetBus(crate::proto::AudioSetBus {
                    bus: Some(crate::proto::ResourceRef {
                        kind: "bus".into(),
                        id: bus_id,
                        guid: String::new(),
                        name: String::new(),
                    }),
                    gain,
                    mute,
                })),
            })
            .await?;
        apply_response(&self.view, &response);
        status_ok(&response)
    }

    pub async fn discover(&self, kind: &str, query: &str) -> ControlResult<String> {
        let response = self.roundtrip(discover_req(kind, query)).await?;
        apply_response(&self.view, &response);
        discover_payload(response)
    }

    pub async fn upload_file(
        &self,
        path: &Path,
        kind: &str,
        input_name: &str,
        video_loop: bool,
        expected_revision: u64,
    ) -> ControlResult<()> {
        let bytes = std::fs::read(path).map_err(|error| ControlError::io(error.to_string()))?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ControlError::invalid("file name"))?;
        let sha = format!("{:x}", Sha256::digest(&bytes));
        let begin = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                payload: Some(request::Payload::BeginMediaUpload(BeginMediaUpload {
                    file_name: file_name.into(),
                    media_kind: kind.into(),
                    size_bytes: bytes.len() as u64,
                    sha256_hex: sha,
                    input_name: input_name.into(),
                    video_loop,
                })),
            })
            .await?;
        status_ok(&begin)?;
        let Some(response::Payload::UploadAccepted(accepted)) = begin.payload else {
            return Err(ControlError::unavailable("upload not accepted"));
        };
        let chunk = if accepted.chunk_size == 0 {
            DEFAULT_CHUNK_SIZE as usize
        } else {
            accepted.chunk_size as usize
        };
        let mut offset = 0u64;
        for piece in bytes.chunks(chunk) {
            let sent = self
                .roundtrip(Request {
                    request_id: uuid::Uuid::new_v4().to_string(),
                    expected_revision: 0,
                    payload: Some(request::Payload::UploadMediaChunk(UploadMediaChunk {
                        upload_id: accepted.upload_id.clone(),
                        offset,
                        data: piece.to_vec(),
                    })),
                })
                .await;
            let failed = match sent {
                Ok(response) => status_ok(&response).err(),
                Err(error) => Some(error),
            };
            if let Some(error) = failed {
                let _ = self
                    .roundtrip(Request {
                        request_id: uuid::Uuid::new_v4().to_string(),
                        expected_revision: 0,
                        payload: Some(request::Payload::AbortMediaUpload(AbortMediaUpload {
                            upload_id: accepted.upload_id.clone(),
                        })),
                    })
                    .await;
                return Err(error);
            }
            offset += piece.len() as u64;
        }
        let commit = self
            .roundtrip(Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                expected_revision,
                payload: Some(request::Payload::CommitMediaUpload(CommitMediaUpload {
                    upload_id: accepted.upload_id,
                    expected_revision,
                })),
            })
            .await?;
        apply_response(&self.view, &commit);
        status_ok(&commit)
    }

    pub fn take_events(&self) -> Vec<String> {
        self.events
            .lock()
            .map(|mut slot| std::mem::take(&mut *slot))
            .unwrap_or_default()
    }

    async fn roundtrip(&self, request: Request) -> ControlResult<Response> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(SessionOp::Request {
                request: Box::new(request),
                reply: reply_tx,
            })
            .await
            .map_err(|_| ControlError::unavailable("client closed"))?;
        reply_rx
            .await
            .map_err(|_| ControlError::unavailable("client closed"))?
    }
}

async fn session_supervisor(
    endpoint: String,
    token: String,
    mut rx: mpsc::Receiver<SessionOp>,
    events: Arc<Mutex<Vec<String>>>,
    view: Arc<Mutex<SessionView>>,
    ready: oneshot::Sender<ControlResult<()>>,
) {
    let mut ready = Some(ready);
    let mut delay = Duration::from_millis(200);
    loop {
        match session_once(&endpoint, &token, &mut rx, &events, &view, ready.take()).await {
            Ok(()) => return,
            Err(error) => {
                if let Ok(mut slot) = view.lock() {
                    slot.connected = false;
                    slot.error = error.clone();
                }
                if rx.is_closed() {
                    return;
                }
                tokio::select! {
                    op = rx.recv() => match op {
                        None => return,
                        Some(SessionOp::Close { reply }) => {
                            let _ = reply.send(());
                            return;
                        }
                        Some(SessionOp::Request { reply, .. }) => {
                            let _ = reply.send(Err(ControlError::unavailable(error.clone())));
                        }
                    },
                    _ = tokio::time::sleep(delay) => {}
                }
                delay = (delay * 2).min(Duration::from_secs(5));
            }
        }
    }
}

async fn session_once(
    endpoint: &str,
    token: &str,
    rx: &mut mpsc::Receiver<SessionOp>,
    events: &Arc<Mutex<Vec<String>>>,
    view: &Arc<Mutex<SessionView>>,
    ready: Option<oneshot::Sender<ControlResult<()>>>,
) -> Result<(), String> {
    let mut req = endpoint
        .into_client_request()
        .map_err(|error| error.to_string())?;
    req.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        tokio_tungstenite::tungstenite::http::HeaderValue::from_static(WS_SUBPROTOCOL),
    );
    let (mut ws, _) = match connect_async(req).await {
        Ok(pair) => pair,
        Err(error) => {
            if let Some(ready) = ready {
                let _ = ready.send(Err(ControlError::io(error.to_string())));
            }
            return Err(error.to_string());
        }
    };
    let hello = Envelope {
        kind: Some(envelope::Kind::Hello(ClientHello {
            protocol: WS_SUBPROTOCOL.into(),
            client_name: "eiviz-host".into(),
            client_instance_id: uuid::Uuid::new_v4().to_string(),
            token: token.into(),
            role: Role::Admin as i32,
        })),
    };
    if let Err(error) = ws
        .send(Message::Binary(encode_envelope(&hello).into()))
        .await
    {
        if let Some(ready) = ready {
            let _ = ready.send(Err(ControlError::io(error.to_string())));
        }
        return Err(error.to_string());
    }
    if let Ok(mut slot) = view.lock() {
        slot.connected = true;
        slot.error.clear();
        slot.lag = false;
    }
    if let Some(ready) = ready {
        let _ = ready.send(Ok(()));
    }
    let mut pending: HashMap<String, oneshot::Sender<ControlResult<Response>>> = HashMap::new();
    let mut last_seq = 0u64;
    let mut ping = tokio::time::interval(Duration::from_secs(20));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            op = rx.recv() => {
                match op {
                    None => {
                        if let Ok(mut slot) = view.lock() {
                            slot.connected = false;
                        }
                        let _ = ws.close(None).await;
                        return Ok(());
                    }
                    Some(SessionOp::Close { reply }) => {
                        if let Ok(mut slot) = view.lock() {
                            slot.connected = false;
                        }
                        let _ = ws.close(None).await;
                        let _ = reply.send(());
                        return Ok(());
                    }
                    Some(SessionOp::Request { request, reply }) => {
                        pending.insert(request.request_id.clone(), reply);
                        let env = Envelope {
                            kind: Some(envelope::Kind::Request(*request)),
                        };
                        ws.send(Message::Binary(encode_envelope(&env).into()))
                            .await
                            .map_err(|error| error.to_string())?;
                    }
                }
            }
            msg = ws.next() => {
                let Some(msg) = msg else { return Err("disconnected".into()); };
                let msg = msg.map_err(|error| error.to_string())?;
                match msg {
                    Message::Binary(bytes) => {
                        let env = decode_envelope(&bytes)?;
                        match env.kind {
                            Some(envelope::Kind::Response(response)) => {
                                apply_response(view, &response);
                                if let Some(reply) = pending.remove(&response.request_id) {
                                    let _ = reply.send(Ok(response));
                                }
                            }
                            Some(envelope::Kind::Event(event)) => {
                                apply_event(view, events, &event, &mut last_seq);
                            }
                            _ => {}
                        }
                    }
                    Message::Ping(payload) => {
                        ws.send(Message::Pong(payload)).await.ok();
                    }
                    Message::Pong(_) => {}
                    Message::Close(_) => return Err("closed".into()),
                    _ => {}
                }
            }
            _ = ping.tick() => {
                ws.send(Message::Ping(Vec::new().into()))
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }
    }
}

fn apply_response(view: &Arc<Mutex<SessionView>>, response: &Response) {
    let Ok(mut slot) = view.lock() else {
        return;
    };
    if let Some(response::Payload::Snapshot(snapshot)) = &response.payload {
        slot.revision = snapshot.revision;
        slot.sequence = snapshot.sequence;
        slot.epoch = snapshot.epoch.clone();
        if !snapshot.document_json.is_empty() {
            slot.document_json = snapshot.document_json.clone();
            slot.document_revision = slot.revision;
        }
        if !snapshot.live_json.is_empty() {
            slot.live_json = snapshot.live_json.clone();
        }
        slot.lag = false;
        slot.error.clear();
    } else if response.revision != 0 {
        slot.revision = response.revision;
        slot.sequence = response.sequence;
    }
    if let Some(status) = &response.status {
        if !status.code.is_empty() && status.code != "OK" {
            slot.error = status.message.clone();
        }
    }
}

fn apply_event(
    view: &Arc<Mutex<SessionView>>,
    events: &Arc<Mutex<Vec<String>>>,
    event: &Event,
    last_seq: &mut u64,
) {
    if let Ok(mut slot) = events.lock() {
        slot.push(event.kind.clone());
    }
    let Ok(mut slot) = view.lock() else {
        return;
    };
    if !event.epoch.is_empty() && !slot.epoch.is_empty() && event.epoch != slot.epoch {
        slot.lag = true;
        slot.epoch = event.epoch.clone();
    }
    if *last_seq != 0 && event.sequence > *last_seq + 1 {
        slot.lag = true;
    }
    if event.kind == "Lag" {
        slot.lag = true;
    }
    *last_seq = (*last_seq).max(event.sequence);
    slot.sequence = slot.sequence.max(event.sequence);
    if !event.document_json.is_empty() {
        slot.document_json = event.document_json.clone();
        if event.session_revision != 0 {
            slot.revision = event.session_revision;
            slot.document_revision = event.session_revision;
        }
    } else if event.kind == "SessionChanged" && event.session_revision != 0 {
        slot.revision = event.session_revision;
        slot.document_revision = event.session_revision;
    }
    if !event.live_json.is_empty() {
        slot.live_json = event.live_json.clone();
    }
}

fn live_cut(unit_id: u64, swap: bool) -> Request {
    Request {
        request_id: uuid::Uuid::new_v4().to_string(),
        expected_revision: 0,
        payload: Some(request::Payload::Cut(Cut {
            unit: Some(ref_unit(unit_id)),
            input: None,
            swap,
        })),
    }
}

fn preview_req(unit_id: u64, scene_id: u64) -> Request {
    Request {
        request_id: uuid::Uuid::new_v4().to_string(),
        expected_revision: 0,
        payload: Some(request::Payload::Preview(crate::proto::Preview {
            unit: Some(ref_unit(unit_id)),
            scene: Some(crate::proto::ResourceRef {
                kind: "scene".into(),
                id: scene_id,
                guid: String::new(),
                name: String::new(),
            }),
        })),
    }
}

#[allow(clippy::too_many_arguments)]
fn auto_req(
    unit_id: u64,
    kind: u32,
    duration_ms: u32,
    swap: bool,
    keep_preview: bool,
    easing: u32,
    direction: u32,
    dip_r: f32,
    dip_g: f32,
    dip_b: f32,
    dip_a: f32,
    softness: f32,
    param: f32,
) -> Request {
    Request {
        request_id: uuid::Uuid::new_v4().to_string(),
        expected_revision: 0,
        payload: Some(request::Payload::Auto(crate::proto::Auto {
            unit: Some(ref_unit(unit_id)),
            input: None,
            kind,
            duration_ms,
            swap,
            keep_preview,
            easing,
            direction,
            dip_r,
            dip_g,
            dip_b,
            dip_a,
            softness,
            param,
        })),
    }
}

fn overlay_req(unit_id: u64, index: u32, duration_ms: u32, to_on: bool) -> Request {
    Request {
        request_id: uuid::Uuid::new_v4().to_string(),
        expected_revision: 0,
        payload: Some(request::Payload::OverlayAuto(OverlayAuto {
            unit: Some(ref_unit(unit_id)),
            index,
            duration_ms,
            to_on,
        })),
    }
}

fn mutate_req(mutation_json: Vec<u8>, expected_revision: u64) -> Request {
    Request {
        request_id: uuid::Uuid::new_v4().to_string(),
        expected_revision,
        payload: Some(request::Payload::MutateSession(
            crate::proto::MutateSession {
                expected_revision,
                mutation_json,
            },
        )),
    }
}

fn discover_req(kind: &str, query: &str) -> Request {
    Request {
        request_id: uuid::Uuid::new_v4().to_string(),
        expected_revision: 0,
        payload: Some(request::Payload::Discover(crate::proto::Discover {
            kind: kind.into(),
            query: query.into(),
        })),
    }
}

fn discover_payload(response: Response) -> ControlResult<String> {
    status_ok(&response)?;
    match response.payload {
        Some(response::Payload::Discover(result)) => Ok(result.payload),
        _ => Err(ControlError::unavailable("discover missing")),
    }
}

fn replace_req(document_json: Vec<u8>, expected_revision: u64) -> Request {
    Request {
        request_id: uuid::Uuid::new_v4().to_string(),
        expected_revision,
        payload: Some(request::Payload::ReplaceSession(
            crate::proto::ReplaceSession {
                document_json,
                expected_revision,
            },
        )),
    }
}

fn ref_unit(unit_id: u64) -> crate::proto::ResourceRef {
    crate::proto::ResourceRef {
        kind: "unit".into(),
        id: unit_id,
        guid: String::new(),
        name: String::new(),
    }
}

fn ref_input(input_id: u64) -> crate::proto::ResourceRef {
    crate::proto::ResourceRef {
        kind: "input".into(),
        id: input_id,
        guid: String::new(),
        name: String::new(),
    }
}

fn status_ok(response: &Response) -> ControlResult<()> {
    let Some(status) = &response.status else {
        return Ok(());
    };
    if status.code.is_empty() || status.code == "OK" {
        Ok(())
    } else {
        Err(status_error(&status.code, &status.message))
    }
}

fn status_error(code: &str, message: &str) -> ControlError {
    match code {
        "CONFLICT" => ControlError::conflict(message),
        "NOT_FOUND" => ControlError::not_found(message),
        "UNAVAILABLE" => ControlError::unavailable(message),
        "PERMISSION_DENIED" => ControlError::permission(message),
        "IO" => ControlError::io(message),
        "INTERNAL" => ControlError::internal(message),
        "AMBIGUOUS" => ControlError::ambiguous(message),
        _ => ControlError::invalid(message),
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

#[cfg(test)]
mod tests {
    use super::SessionView;

    #[test]
    fn live_revision_does_not_imply_document_revision() {
        let view = SessionView {
            revision: 9,
            ..SessionView::default()
        };
        assert_eq!(view.document_revision, 0);
        assert_eq!(view.revision, 9);
    }
}
