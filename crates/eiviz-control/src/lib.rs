//! Authenticated control adapters for the versioned Command API.
//!
//! Stream Deck plugins live out of tree and use these HTTP, TCP, or WebSocket
//! contracts. This crate deliberately contains no Deck-specific protocol or
//! action map.

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use eiviz_command::{Command, CommandAck, CommandEnvelope, CommandError};
use eiviz_engine::{Engine, EngineError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tiny_http::{Header, Method, Response, Server, StatusCode};
use tungstenite::handshake::server::{ErrorResponse, Request, Response as WsHandshakeResponse};
use tungstenite::{Error as WsError, Message, accept_hdr};

pub const CONTROL_API_VERSION: u32 = 1;
const MAX_BODY_BYTES: u64 = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    WebSocket(#[from] WsError),
    #[error("{0}")]
    Other(String),
}

#[derive(Clone, Debug)]
pub struct ControlConfig {
    pub http_bind: String,
    pub tcp_bind: String,
    pub websocket_bind: String,
    /// A bearer token for HTTP/WebSocket and the per-request token for TCP.
    /// A non-loopback bind is rejected when this is absent.
    pub require_token: Option<String>,
    pub max_requests_per_sec: u32,
    pub command_queue_capacity: usize,
    pub event_queue_capacity: usize,
    pub max_connections: usize,
}

impl Default for ControlConfig {
    fn default() -> Self {
        Self {
            http_bind: "127.0.0.1:0".into(),
            tcp_bind: "127.0.0.1:0".into(),
            websocket_bind: "127.0.0.1:0".into(),
            require_token: None,
            max_requests_per_sec: 60,
            command_queue_capacity: 256,
            event_queue_capacity: 128,
            max_connections: 64,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ControlPorts {
    pub http: u16,
    pub tcp: u16,
    pub websocket: u16,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Query {
    Status,
    Project,
    Metrics,
    Events,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApiOperation {
    Query { query: Query },
    Command { envelope: Box<CommandEnvelope> },
    Transaction { envelopes: Vec<CommandEnvelope> },
    Subscribe,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiRequest {
    pub version: u32,
    pub request_id: String,
    /// Used only by TCP. HTTP uses Authorization and WebSocket authenticates
    /// the upgrade handshake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(flatten)]
    pub operation: ApiOperation,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionRequest {
    pub version: u32,
    pub envelopes: Vec<CommandEnvelope>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApiResponse {
    pub version: u32,
    pub request_id: Option<String>,
    /// Latest accepted command revision; accepted commands may still be pending.
    pub revision: u64,
    pub applied_revision: u64,
    pub state_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ControlEvent {
    pub version: u32,
    pub event: &'static str,
    pub revision: u64,
    pub applied_revision: u64,
    pub state_hash: String,
    pub command_ids: Vec<String>,
}

#[derive(Debug)]
enum DispatchTask {
    Command {
        envelope: Box<CommandEnvelope>,
        reply: Sender<Result<Vec<CommandAck>, ApiError>>,
    },
    Transaction {
        envelopes: Vec<CommandEnvelope>,
        reply: Sender<Result<Vec<CommandAck>, ApiError>>,
    },
}

#[derive(Clone)]
struct Dispatcher {
    tx: Sender<DispatchTask>,
}

impl Dispatcher {
    fn submit(&self, envelope: CommandEnvelope) -> Result<Vec<CommandAck>, ApiError> {
        let (reply, receive) = bounded(1);
        self.enqueue(
            DispatchTask::Command {
                envelope: Box::new(envelope),
                reply,
            },
            receive,
        )
    }

    fn transaction(&self, envelopes: Vec<CommandEnvelope>) -> Result<Vec<CommandAck>, ApiError> {
        let (reply, receive) = bounded(1);
        self.enqueue(DispatchTask::Transaction { envelopes, reply }, receive)
    }

    fn enqueue(
        &self,
        task: DispatchTask,
        receive: Receiver<Result<Vec<CommandAck>, ApiError>>,
    ) -> Result<Vec<CommandAck>, ApiError> {
        self.tx.try_send(task).map_err(|error| match error {
            TrySendError::Full(_) => ApiError {
                code: "busy",
                message: "command queue is full".into(),
            },
            TrySendError::Disconnected(_) => ApiError {
                code: "unavailable",
                message: "command dispatcher is stopped".into(),
            },
        })?;
        receive.recv().unwrap_or_else(|_| {
            Err(ApiError {
                code: "unavailable",
                message: "command dispatcher stopped before replying".into(),
            })
        })
    }
}

struct EventBroker {
    capacity: usize,
    subscribers: Mutex<Vec<Sender<ControlEvent>>>,
}

impl EventBroker {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            subscribers: Mutex::new(Vec::new()),
        }
    }

    fn subscribe(&self) -> Receiver<ControlEvent> {
        let (send, receive) = bounded(self.capacity);
        self.subscribers.lock().unwrap().push(send);
        receive
    }

    fn publish(&self, event: ControlEvent) {
        self.subscribers.lock().unwrap().retain(|subscriber| {
            match subscriber.try_send(event.clone()) {
                Ok(()) => true,
                // A slow event consumer is disconnected instead of growing an
                // unbounded queue or silently receiving an incomplete stream.
                Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
            }
        });
    }
}

pub fn spawn_control(
    engine: Arc<Engine>,
    config: ControlConfig,
    stop: Arc<AtomicBool>,
) -> std::io::Result<ControlPorts> {
    validate_config(&config)?;
    let broker = Arc::new(EventBroker::new(config.event_queue_capacity));
    let dispatcher = spawn_dispatcher(
        engine.clone(),
        config.command_queue_capacity,
        broker.clone(),
        stop.clone(),
    );

    let http = match spawn_http_server(
        engine.clone(),
        config.clone(),
        dispatcher.clone(),
        stop.clone(),
    ) {
        Ok(port) => port,
        Err(error) => {
            stop.store(true, Ordering::Release);
            return Err(error);
        }
    };
    let tcp = match spawn_tcp_server(
        engine.clone(),
        config.clone(),
        dispatcher.clone(),
        stop.clone(),
    ) {
        Ok(port) => port,
        Err(error) => {
            stop.store(true, Ordering::Release);
            return Err(error);
        }
    };
    let websocket = match spawn_websocket_server(engine, config, dispatcher, broker, stop.clone()) {
        Ok(port) => port,
        Err(error) => {
            stop.store(true, Ordering::Release);
            return Err(error);
        }
    };
    Ok(ControlPorts {
        http,
        tcp,
        websocket,
    })
}

fn validate_config(config: &ControlConfig) -> std::io::Result<()> {
    if config.max_requests_per_sec == 0
        || config.command_queue_capacity == 0
        || config.event_queue_capacity == 0
        || config.max_connections == 0
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "rate, queue capacities, and max_connections must be greater than zero",
        ));
    }
    for bind in [&config.http_bind, &config.tcp_bind, &config.websocket_bind] {
        validate_bind(bind, config.require_token.as_deref())?;
    }
    if config.require_token.as_deref() == Some("") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "control token must not be empty",
        ));
    }
    Ok(())
}

fn validate_bind(bind: &str, token: Option<&str>) -> std::io::Result<SocketAddr> {
    let address = bind.parse::<SocketAddr>().map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("control bind must be an IP socket address: {error}"),
        )
    })?;
    if !address.ip().is_loopback() && token.is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "remote control requires an explicit authentication token",
        ));
    }
    Ok(address)
}

fn spawn_dispatcher(
    engine: Arc<Engine>,
    capacity: usize,
    broker: Arc<EventBroker>,
    stop: Arc<AtomicBool>,
) -> Dispatcher {
    let (send, receive) = bounded::<DispatchTask>(capacity);
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            let task = match receive.recv_timeout(Duration::from_millis(100)) {
                Ok(task) => task,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            };
            let (result, reply) = match task {
                DispatchTask::Command { envelope, reply } => {
                    (engine.submit(*envelope).map(|ack| vec![ack]), reply)
                }
                DispatchTask::Transaction { envelopes, reply } => {
                    (engine.submit_transaction(envelopes), reply)
                }
            };
            let result = result.map_err(api_engine_error);
            if let Ok(acknowledgements) = &result {
                broker.publish(ControlEvent {
                    version: CONTROL_API_VERSION,
                    event: "command_accepted",
                    revision: engine.revision(),
                    applied_revision: engine.applied_revision(),
                    state_hash: engine.state_hash(),
                    command_ids: acknowledgements
                        .iter()
                        .map(|ack| ack.id.to_string())
                        .collect(),
                });
            }
            let _ = reply.send(result);
        }
    });
    Dispatcher { tx: send }
}

fn api_engine_error(error: EngineError) -> ApiError {
    let code = match &error {
        EngineError::Command(CommandError::RevisionMismatch { .. })
        | EngineError::Command(CommandError::ClientSequence { .. }) => "conflict",
        EngineError::Command(CommandError::Busy) => "busy",
        EngineError::Command(CommandError::UnsupportedVersion { .. })
        | EngineError::Command(CommandError::EmptyTransaction)
        | EngineError::Command(CommandError::Domain(_))
        | EngineError::Command(CommandError::Rejected(_))
        | EngineError::Command(CommandError::Duplicate(_)) => "invalid_command",
        EngineError::Admission(_) => "admission_denied",
        _ => "engine_error",
    };
    ApiError {
        code,
        message: error.to_string(),
    }
}

#[derive(Clone)]
struct RateWindow {
    started: Instant,
    count: u32,
}

impl RateWindow {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            count: 0,
        }
    }

    fn allow(&mut self, max: u32) -> bool {
        if self.started.elapsed() >= Duration::from_secs(1) {
            self.started = Instant::now();
            self.count = 0;
        }
        if self.count >= max {
            return false;
        }
        self.count += 1;
        true
    }
}

fn spawn_http_server(
    engine: Arc<Engine>,
    config: ControlConfig,
    dispatcher: Dispatcher,
    stop: Arc<AtomicBool>,
) -> std::io::Result<u16> {
    let server = Server::http(&config.http_bind)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| std::io::Error::other("HTTP server did not bind an IP address"))?
        .port();
    thread::spawn(move || {
        let mut rate = RateWindow::new();
        while !stop.load(Ordering::Acquire) {
            let mut request = match server.recv_timeout(Duration::from_millis(100)) {
                Ok(Some(request)) => request,
                Ok(None) => continue,
                Err(_) => break,
            };
            if !http_authorized(&request, config.require_token.as_deref()) {
                let _ = request.respond(text_response(401, "unauthorized"));
                continue;
            }
            if !rate.allow(config.max_requests_per_sec) {
                let _ = request.respond(text_response(429, "rate limited"));
                continue;
            }
            let url = request
                .url()
                .split('?')
                .next()
                .unwrap_or(request.url())
                .to_owned();
            let method = request.method().clone();
            let mut body = String::new();
            let read = request
                .as_reader()
                .take(MAX_BODY_BYTES + 1)
                .read_to_string(&mut body);
            if read.is_err() || body.len() as u64 > MAX_BODY_BYTES {
                let _ = request.respond(text_response(413, "body too large"));
                continue;
            }
            let response = handle_http(&engine, &dispatcher, &method, &url, &body);
            let _ = request.respond(response);
        }
    });
    Ok(port)
}

fn handle_http(
    engine: &Engine,
    dispatcher: &Dispatcher,
    method: &Method,
    url: &str,
    body: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    if *method == Method::Get {
        let query = match url {
            "/v1/health" | "/v1/status" => Query::Status,
            "/v1/project" => Query::Project,
            "/v1/metrics" => Query::Metrics,
            "/v1/events" => Query::Events,
            _ => return text_response(404, "not found"),
        };
        return api_http_response(engine, None, execute_query(engine, query));
    }
    if *method == Method::Post && url == "/v1/command" {
        let envelope = match serde_json::from_str::<CommandEnvelope>(body) {
            Ok(envelope) => envelope,
            Err(error) => return text_response(400, &error.to_string()),
        };
        if let Err(error) = validate_external_envelopes(std::slice::from_ref(&envelope)) {
            return api_http_response(engine, None, Err(error));
        }
        return api_http_response(
            engine,
            None,
            dispatcher
                .submit(envelope)
                .and_then(|mut values| serialize_value(values.remove(0))),
        );
    }
    if *method == Method::Post && url == "/v1/transaction" {
        let transaction = match serde_json::from_str::<TransactionRequest>(body) {
            Ok(transaction) if transaction.version == CONTROL_API_VERSION => transaction,
            Ok(transaction) => {
                return text_response(
                    400,
                    &format!(
                        "unsupported API version {}; expected {}",
                        transaction.version, CONTROL_API_VERSION
                    ),
                );
            }
            Err(error) => return text_response(400, &error.to_string()),
        };
        if let Err(error) = validate_external_envelopes(&transaction.envelopes) {
            return api_http_response(engine, None, Err(error));
        }
        return api_http_response(
            engine,
            None,
            dispatcher
                .transaction(transaction.envelopes)
                .and_then(serialize_value),
        );
    }
    text_response(404, "not found")
}

fn execute_query(engine: &Engine, query: Query) -> Result<Value, ApiError> {
    match query {
        Query::Status => Ok(json!({
            "healthy": true,
            "accepted_revision": engine.revision(),
            "applied_revision": engine.applied_revision(),
            "state_hash": engine.state_hash(),
            "staged_state_hash": engine.staged_state_hash(),
            "commands": engine.command_diagnostics(),
        })),
        Query::Project => serialize_value(engine.snapshot()),
        Query::Metrics => serialize_value(engine.metrics()),
        Query::Events => serialize_value(engine.flight_log()),
    }
}

fn serialize_value(value: impl Serialize) -> Result<Value, ApiError> {
    serde_json::to_value(value).map_err(|error| ApiError {
        code: "serialization",
        message: error.to_string(),
    })
}

fn success_response(engine: &Engine, request_id: Option<String>, result: Value) -> ApiResponse {
    ApiResponse {
        version: CONTROL_API_VERSION,
        request_id,
        revision: engine.revision(),
        applied_revision: engine.applied_revision(),
        state_hash: engine.state_hash(),
        result: Some(result),
        error: None,
    }
}

fn error_response(engine: &Engine, request_id: Option<String>, error: ApiError) -> ApiResponse {
    ApiResponse {
        version: CONTROL_API_VERSION,
        request_id,
        revision: engine.revision(),
        applied_revision: engine.applied_revision(),
        state_hash: engine.state_hash(),
        result: None,
        error: Some(error),
    }
}

fn api_http_response(
    engine: &Engine,
    request_id: Option<String>,
    result: Result<Value, ApiError>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let (status, response) = match result {
        Ok(value) => (200, success_response(engine, request_id, value)),
        Err(error) => {
            let status = match error.code {
                "conflict" => 409,
                "busy" | "unavailable" => 503,
                "admission_denied" => 422,
                _ => 400,
            };
            (status, error_response(engine, request_id, error))
        }
    };
    json_response(status, &response)
}

fn json_response(status: u16, value: &impl Serialize) -> Response<std::io::Cursor<Vec<u8>>> {
    let bytes =
        serde_json::to_vec(value).unwrap_or_else(|_| b"{\"error\":\"serialization\"}".into());
    Response::from_data(bytes)
        .with_status_code(StatusCode(status))
        .with_header(
            Header::from_bytes("Content-Type", "application/json; charset=utf-8")
                .expect("static HTTP header"),
        )
        .with_header(Header::from_bytes("Cache-Control", "no-store").expect("static HTTP header"))
}

fn text_response(status: u16, text: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(text)
        .with_status_code(StatusCode(status))
        .with_header(Header::from_bytes("Cache-Control", "no-store").expect("static HTTP header"))
}

fn http_authorized(request: &tiny_http::Request, token: Option<&str>) -> bool {
    let Some(token) = token else {
        return true;
    };
    request.headers().iter().any(|header| {
        header.field.equiv("Authorization")
            && header
                .value
                .as_str()
                .strip_prefix("Bearer ")
                .is_some_and(|provided| constant_time_eq(provided, token))
    })
}

fn spawn_tcp_server(
    engine: Arc<Engine>,
    config: ControlConfig,
    dispatcher: Dispatcher,
    stop: Arc<AtomicBool>,
) -> std::io::Result<u16> {
    let listener = TcpListener::bind(&config.tcp_bind)?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let active = Arc::new(AtomicUsize::new(0));
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, _)) => {
                    if active.load(Ordering::Acquire) >= config.max_connections {
                        let mut stream = stream;
                        let _ = stream.write_all(b"{\"error\":\"connection limit\"}\n");
                        continue;
                    }
                    active.fetch_add(1, Ordering::AcqRel);
                    let engine = engine.clone();
                    let dispatcher = dispatcher.clone();
                    let config = config.clone();
                    let active = active.clone();
                    thread::spawn(move || {
                        let _guard = ActiveConnection(active);
                        let _ = serve_tcp(engine, dispatcher, config, stream);
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });
    Ok(port)
}

struct ActiveConnection(Arc<AtomicUsize>);

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn serve_tcp(
    engine: Arc<Engine>,
    dispatcher: Dispatcher,
    config: ControlConfig,
    mut stream: TcpStream,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut rate = RateWindow::new();
    let mut buffer = Vec::new();
    let mut temporary = [0u8; 4096];
    loop {
        let count = match stream.read(&mut temporary) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(error) => return Err(error),
        };
        buffer.extend_from_slice(&temporary[..count]);
        if buffer.len() as u64 > MAX_BODY_BYTES {
            stream.write_all(b"{\"error\":\"frame too large\"}\n")?;
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "TCP frame too large",
            ));
        }
        while let Some(index) = buffer.iter().position(|byte| *byte == b'\n') {
            let line = buffer.drain(..=index).collect::<Vec<_>>();
            if !rate.allow(config.max_requests_per_sec) {
                write_tcp_error(&mut stream, &engine, None, "rate_limited", "rate limited")?;
                continue;
            }
            let request = match serde_json::from_slice::<ApiRequest>(&line[..line.len() - 1]) {
                Ok(request) => request,
                Err(error) => {
                    write_tcp_error(
                        &mut stream,
                        &engine,
                        None,
                        "invalid_request",
                        &error.to_string(),
                    )?;
                    continue;
                }
            };
            if !request_authorized(request.token.as_deref(), config.require_token.as_deref()) {
                write_tcp_error(
                    &mut stream,
                    &engine,
                    Some(request.request_id),
                    "unauthorized",
                    "unauthorized",
                )?;
                continue;
            }
            let response = execute_request(&engine, &dispatcher, request, false);
            let mut encoded = serde_json::to_vec(&response).map_err(std::io::Error::other)?;
            encoded.push(b'\n');
            stream.write_all(&encoded)?;
        }
    }
    Ok(())
}

fn write_tcp_error(
    stream: &mut TcpStream,
    engine: &Engine,
    request_id: Option<String>,
    code: &'static str,
    message: &str,
) -> std::io::Result<()> {
    let response = error_response(
        engine,
        request_id,
        ApiError {
            code,
            message: message.into(),
        },
    );
    let mut encoded = serde_json::to_vec(&response).map_err(std::io::Error::other)?;
    encoded.push(b'\n');
    stream.write_all(&encoded)
}

fn spawn_websocket_server(
    engine: Arc<Engine>,
    config: ControlConfig,
    dispatcher: Dispatcher,
    broker: Arc<EventBroker>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<u16> {
    let listener = TcpListener::bind(&config.websocket_bind)?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let active = Arc::new(AtomicUsize::new(0));
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, _)) => {
                    if active.load(Ordering::Acquire) >= config.max_connections {
                        drop(stream);
                        continue;
                    }
                    active.fetch_add(1, Ordering::AcqRel);
                    let engine = engine.clone();
                    let dispatcher = dispatcher.clone();
                    let broker = broker.clone();
                    let config = config.clone();
                    let active = active.clone();
                    thread::spawn(move || {
                        let _guard = ActiveConnection(active);
                        let _ = serve_websocket(engine, dispatcher, broker, config, stream);
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });
    Ok(port)
}

#[allow(clippy::result_large_err)] // tungstenite's callback requires ErrorResponse by value.
fn serve_websocket(
    engine: Arc<Engine>,
    dispatcher: Dispatcher,
    broker: Arc<EventBroker>,
    config: ControlConfig,
    stream: TcpStream,
) -> Result<(), ControlError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(WsError::Io)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(WsError::Io)?;
    let required_token = config.require_token.clone();
    let mut websocket = accept_hdr(
        stream,
        move |request: &Request,
              mut response: WsHandshakeResponse|
              -> Result<WsHandshakeResponse, ErrorResponse> {
            let authorization = request
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "));
            let protocol = request
                .headers()
                .get("sec-websocket-protocol")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .find(|value| value.starts_with("eiviz.bearer."))
                });
            let protocol_token = protocol.and_then(|value| value.strip_prefix("eiviz.bearer."));
            if !request_authorized(authorization.or(protocol_token), required_token.as_deref()) {
                let mut rejection = ErrorResponse::new(Some("unauthorized".into()));
                *rejection.status_mut() = tungstenite::http::StatusCode::UNAUTHORIZED;
                return Err(rejection);
            }
            if let Some(protocol) = protocol {
                let value = tungstenite::http::HeaderValue::from_str(protocol)
                    .map_err(|_| ErrorResponse::new(Some("invalid subprotocol".into())))?;
                response
                    .headers_mut()
                    .insert("sec-websocket-protocol", value);
            }
            Ok(response)
        },
    )
    .map_err(|error| ControlError::Other(format!("WebSocket handshake failed: {error:?}")))?;
    websocket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(WsError::Io)?;

    let mut rate = RateWindow::new();
    let mut events: Option<Receiver<ControlEvent>> = None;
    loop {
        if let Some(receiver) = &events {
            loop {
                match receiver.try_recv() {
                    Ok(event) => {
                        let text = serde_json::to_string(&event)
                            .map_err(|error| WsError::Io(std::io::Error::other(error)))?;
                        websocket.send(Message::Text(text.into()))?;
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        websocket.close(None)?;
                        return Ok(());
                    }
                }
            }
        }
        let message = match websocket.read() {
            Ok(message) => message,
            Err(WsError::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        match message {
            Message::Text(text) => {
                if text.len() as u64 > MAX_BODY_BYTES {
                    websocket.close(None)?;
                    return Ok(());
                }
                if !rate.allow(config.max_requests_per_sec) {
                    let response = error_response(
                        &engine,
                        None,
                        ApiError {
                            code: "rate_limited",
                            message: "rate limited".into(),
                        },
                    );
                    websocket.send(Message::Text(
                        serde_json::to_string(&response).unwrap().into(),
                    ))?;
                    continue;
                }
                let request = match serde_json::from_str::<ApiRequest>(&text) {
                    Ok(request) => request,
                    Err(error) => {
                        let response = error_response(
                            &engine,
                            None,
                            ApiError {
                                code: "invalid_request",
                                message: error.to_string(),
                            },
                        );
                        websocket.send(Message::Text(
                            serde_json::to_string(&response).unwrap().into(),
                        ))?;
                        continue;
                    }
                };
                let subscribe = matches!(request.operation, ApiOperation::Subscribe);
                let response = execute_request(&engine, &dispatcher, request, true);
                websocket.send(Message::Text(
                    serde_json::to_string(&response).unwrap().into(),
                ))?;
                if subscribe && response.error.is_none() && events.is_none() {
                    events = Some(broker.subscribe());
                }
            }
            Message::Ping(payload) => websocket.send(Message::Pong(payload))?,
            Message::Close(frame) => {
                websocket.close(frame)?;
                return Ok(());
            }
            Message::Binary(_) => {
                websocket.close(None)?;
                return Ok(());
            }
            Message::Pong(_) | Message::Frame(_) => {}
        }
    }
}

fn execute_request(
    engine: &Engine,
    dispatcher: &Dispatcher,
    request: ApiRequest,
    allow_subscribe: bool,
) -> ApiResponse {
    let request_id = Some(request.request_id);
    if request.version != CONTROL_API_VERSION {
        return error_response(
            engine,
            request_id,
            ApiError {
                code: "unsupported_version",
                message: format!(
                    "unsupported API version {}; expected {}",
                    request.version, CONTROL_API_VERSION
                ),
            },
        );
    }
    let result = match request.operation {
        ApiOperation::Query { query } => execute_query(engine, query),
        ApiOperation::Command { envelope } => {
            validate_external_envelopes(std::slice::from_ref(envelope.as_ref()))
                .and_then(|()| dispatcher.submit(*envelope))
                .and_then(|mut values| serialize_value(values.remove(0)))
        }
        ApiOperation::Transaction { envelopes } => validate_external_envelopes(&envelopes)
            .and_then(|()| dispatcher.transaction(envelopes))
            .and_then(serialize_value),
        ApiOperation::Subscribe if allow_subscribe => Ok(json!({"subscribed": true})),
        ApiOperation::Subscribe => Err(ApiError {
            code: "unsupported_operation",
            message: "subscriptions require WebSocket".into(),
        }),
    };
    match result {
        Ok(value) => success_response(engine, request_id, value),
        Err(error) => error_response(engine, request_id, error),
    }
}

fn validate_external_envelopes(envelopes: &[CommandEnvelope]) -> Result<(), ApiError> {
    if envelopes.iter().any(|envelope| envelope.client_seq == 0) {
        return Err(ApiError {
            code: "invalid_command",
            message: "external command client_seq must be greater than zero".into(),
        });
    }
    Ok(())
}

fn request_authorized(provided: Option<&str>, required: Option<&str>) -> bool {
    required.is_none_or(|required| {
        provided.is_some_and(|provided| constant_time_eq(provided, required))
    })
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());
    for index in 0..length {
        difference |= usize::from(
            left.get(index).copied().unwrap_or_default()
                ^ right.get(index).copied().unwrap_or_default(),
        );
    }
    difference == 0
}

#[derive(Clone, Debug, Serialize)]
pub struct MidiCapability {
    pub compiled: bool,
    pub detail: &'static str,
}

pub const fn midi_capability() -> MidiCapability {
    #[cfg(feature = "midi")]
    {
        MidiCapability {
            compiled: true,
            detail: "midir native backend compiled; select a physical port explicitly",
        }
    }
    #[cfg(not(feature = "midi"))]
    {
        MidiCapability {
            compiled: false,
            detail: "not compiled; enable the explicit `midir` feature",
        }
    }
}

#[cfg(feature = "midi")]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MidiDeviceInfo {
    /// Backend-provided opaque stable identifier. Selection never falls back
    /// to an index or the platform default.
    pub id: String,
    pub name: String,
}

#[cfg(feature = "midi")]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MidiTrigger {
    NoteOn {
        channel: u8,
        note: u8,
        minimum_velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
    },
    ControlChange {
        channel: u8,
        controller: u8,
        minimum_value: u8,
        maximum_value: u8,
    },
    ProgramChange {
        channel: u8,
        program: u8,
    },
}

#[cfg(feature = "midi")]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MidiMapping {
    pub trigger: MidiTrigger,
    pub command: Command,
}

#[cfg(feature = "midi")]
#[derive(Clone, Debug)]
pub struct MidiConfig {
    pub device_id: String,
    pub mappings: Vec<MidiMapping>,
    pub queue_capacity: usize,
}

#[cfg(feature = "midi")]
#[derive(Clone, Debug, Serialize)]
pub struct MidiStatus {
    pub device_id: String,
    pub device_name: String,
    pub received_messages: u64,
    pub matched_messages: u64,
    pub queue_overflows: u64,
    pub submit_errors: u64,
    pub last_error: Option<String>,
}

#[cfg(feature = "midi")]
struct MidiCounters {
    received_messages: std::sync::atomic::AtomicU64,
    matched_messages: std::sync::atomic::AtomicU64,
    queue_overflows: std::sync::atomic::AtomicU64,
    submit_errors: std::sync::atomic::AtomicU64,
    last_error: Mutex<Option<String>>,
}

#[cfg(feature = "midi")]
pub struct MidiHandle {
    device_id: String,
    device_name: String,
    local_stop: Arc<AtomicBool>,
    counters: Arc<MidiCounters>,
    connection: Option<midir::MidiInputConnection<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

#[cfg(feature = "midi")]
impl MidiHandle {
    pub fn status(&self) -> MidiStatus {
        MidiStatus {
            device_id: self.device_id.clone(),
            device_name: self.device_name.clone(),
            received_messages: self.counters.received_messages.load(Ordering::Relaxed),
            matched_messages: self.counters.matched_messages.load(Ordering::Relaxed),
            queue_overflows: self.counters.queue_overflows.load(Ordering::Relaxed),
            submit_errors: self.counters.submit_errors.load(Ordering::Relaxed),
            last_error: self.counters.last_error.lock().unwrap().clone(),
        }
    }
}

#[cfg(feature = "midi")]
impl Drop for MidiHandle {
    fn drop(&mut self) {
        self.local_stop.store(true, Ordering::Release);
        self.connection.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(feature = "midi")]
#[derive(Clone, Copy)]
struct RawMidiMessage {
    bytes: [u8; 3],
    len: u8,
}

#[cfg(feature = "midi")]
pub fn list_midi_inputs() -> Result<Vec<MidiDeviceInfo>, ControlError> {
    let input = midir::MidiInput::new("eiviz-midi-enumeration").map_err(|error| {
        ControlError::Other(format!("MIDI backend initialization failed: {error}"))
    })?;
    input
        .ports()
        .into_iter()
        .map(|port| {
            let name = input
                .port_name(&port)
                .map_err(|error| ControlError::Other(format!("MIDI port query failed: {error}")))?;
            Ok(MidiDeviceInfo {
                id: port.id(),
                name,
            })
        })
        .collect()
}

#[cfg(feature = "midi")]
pub fn spawn_midi(
    engine: Arc<Engine>,
    config: MidiConfig,
    process_stop: Arc<AtomicBool>,
) -> Result<MidiHandle, ControlError> {
    validate_midi_config(&config)?;
    let mut input = midir::MidiInput::new("eiviz-midi-control").map_err(|error| {
        ControlError::Other(format!("MIDI backend initialization failed: {error}"))
    })?;
    input.ignore(midir::Ignore::All);
    let port = input.find_port_by_id(&config.device_id).ok_or_else(|| {
        ControlError::Other(format!(
            "selected MIDI input {} is not available",
            config.device_id
        ))
    })?;
    let device_name = input
        .port_name(&port)
        .map_err(|error| ControlError::Other(format!("MIDI port query failed: {error}")))?;
    let (send, receive) = bounded::<RawMidiMessage>(config.queue_capacity);
    let counters = Arc::new(MidiCounters {
        received_messages: std::sync::atomic::AtomicU64::new(0),
        matched_messages: std::sync::atomic::AtomicU64::new(0),
        queue_overflows: std::sync::atomic::AtomicU64::new(0),
        submit_errors: std::sync::atomic::AtomicU64::new(0),
        last_error: Mutex::new(None),
    });
    let callback_counters = counters.clone();
    let connection = input
        .connect(
            &port,
            "eiviz-midi-input",
            move |_timestamp, message, _| {
                callback_counters
                    .received_messages
                    .fetch_add(1, Ordering::Relaxed);
                if message.len() > 3 {
                    return;
                }
                let mut raw = RawMidiMessage {
                    bytes: [0; 3],
                    len: message.len() as u8,
                };
                raw.bytes[..message.len()].copy_from_slice(message);
                if send.try_send(raw).is_err() {
                    callback_counters
                        .queue_overflows
                        .fetch_add(1, Ordering::Relaxed);
                }
            },
            (),
        )
        .map_err(|error| ControlError::Other(format!("MIDI input open failed: {error}")))?;

    let local_stop = Arc::new(AtomicBool::new(false));
    let worker_stop = local_stop.clone();
    let worker_counters = counters.clone();
    let mappings = config.mappings;
    let client = eiviz_core::ClientId::new();
    let worker = thread::spawn(move || {
        let mut client_sequence = 0u64;
        while !process_stop.load(Ordering::Acquire) && !worker_stop.load(Ordering::Acquire) {
            let message = match receive.recv_timeout(Duration::from_millis(20)) {
                Ok(message) => message,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            };
            let commands = mappings
                .iter()
                .filter(|mapping| midi_trigger_matches(&mapping.trigger, message))
                .map(|mapping| mapping.command.clone())
                .collect::<Vec<_>>();
            if commands.is_empty() {
                continue;
            }
            worker_counters
                .matched_messages
                .fetch_add(1, Ordering::Relaxed);
            let envelopes = commands
                .into_iter()
                .map(|command| {
                    client_sequence = client_sequence.saturating_add(1);
                    let mut envelope = CommandEnvelope::new(client, command);
                    envelope.client_seq = client_sequence;
                    envelope
                })
                .collect();
            if let Err(error) = engine.submit_transaction(envelopes) {
                worker_counters
                    .submit_errors
                    .fetch_add(1, Ordering::Relaxed);
                *worker_counters.last_error.lock().unwrap() = Some(error.to_string());
            }
        }
    });

    Ok(MidiHandle {
        device_id: config.device_id,
        device_name,
        local_stop,
        counters,
        connection: Some(connection),
        worker: Some(worker),
    })
}

#[cfg(feature = "midi")]
fn validate_midi_config(config: &MidiConfig) -> Result<(), ControlError> {
    if config.device_id.is_empty() || config.queue_capacity == 0 || config.mappings.is_empty() {
        return Err(ControlError::Other(
            "MIDI device, at least one mapping, and a non-zero queue are required".into(),
        ));
    }
    for mapping in &config.mappings {
        let valid = match mapping.trigger {
            MidiTrigger::NoteOn {
                channel,
                note,
                minimum_velocity,
            } => channel < 16 && note < 128 && minimum_velocity < 128,
            MidiTrigger::NoteOff { channel, note } => channel < 16 && note < 128,
            MidiTrigger::ControlChange {
                channel,
                controller,
                minimum_value,
                maximum_value,
            } => {
                channel < 16
                    && controller < 128
                    && minimum_value < 128
                    && maximum_value < 128
                    && minimum_value <= maximum_value
            }
            MidiTrigger::ProgramChange { channel, program } => channel < 16 && program < 128,
        };
        if !valid {
            return Err(ControlError::Other("invalid MIDI mapping range".into()));
        }
    }
    Ok(())
}

#[cfg(feature = "midi")]
fn midi_trigger_matches(trigger: &MidiTrigger, message: RawMidiMessage) -> bool {
    let status = message.bytes[0];
    let channel = status & 0x0f;
    let kind = status & 0xf0;
    match *trigger {
        MidiTrigger::NoteOn {
            channel: expected_channel,
            note,
            minimum_velocity,
        } => {
            message.len >= 3
                && kind == 0x90
                && channel == expected_channel
                && message.bytes[1] == note
                && message.bytes[2] >= minimum_velocity.max(1)
        }
        MidiTrigger::NoteOff {
            channel: expected_channel,
            note,
        } => {
            message.len >= 3
                && channel == expected_channel
                && message.bytes[1] == note
                && (kind == 0x80 || (kind == 0x90 && message.bytes[2] == 0))
        }
        MidiTrigger::ControlChange {
            channel: expected_channel,
            controller,
            minimum_value,
            maximum_value,
        } => {
            message.len >= 3
                && kind == 0xb0
                && channel == expected_channel
                && message.bytes[1] == controller
                && (minimum_value..=maximum_value).contains(&message.bytes[2])
        }
        MidiTrigger::ProgramChange {
            channel: expected_channel,
            program,
        } => {
            message.len >= 2
                && kind == 0xc0
                && channel == expected_channel
                && message.bytes[1] == program
        }
    }
}

pub fn keyboard_take(engine: &Engine) {
    let unit = engine.primary_unit();
    let _ = engine.submit_payload(Command::Take {
        unit,
        swap: false,
        style: eiviz_core::TransitionStyle::Cut,
        duration_frames: 0,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_core::ClientId;
    use std::io::{Read, Write};
    use tungstenite::client::IntoClientRequest;

    fn config() -> ControlConfig {
        ControlConfig {
            require_token: Some("integration-secret".into()),
            ..ControlConfig::default()
        }
    }

    fn http_request(port: u16, request: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        stream.flush().unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn authenticated_http_envelope_is_idempotent() {
        let engine = Engine::new("control-http").shared();
        let stop = Arc::new(AtomicBool::new(false));
        let ports = spawn_control(engine.clone(), config(), stop.clone()).unwrap();
        let mut envelope = CommandEnvelope::new(
            ClientId::new(),
            Command::SetName {
                name: "http-renamed".into(),
            },
        );
        envelope.client_seq = 1;
        let body = serde_json::to_string(&envelope).unwrap();
        let request = format!(
            "POST /v1/command HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer integration-secret\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let first = http_request(ports.http, &request);
        let second = http_request(ports.http, &request);
        assert!(first.contains("200 OK"), "{first}");
        assert!(second.contains("\"duplicate\":true"), "{second}");
        assert!(first.contains("\"applied_revision\":null"), "{first}");
        assert_eq!(engine.snapshot().name, "control-http");
        assert_eq!(engine.staged_snapshot().name, "http-renamed");
        assert_eq!(engine.revision(), 1);
        assert_eq!(engine.applied_revision(), 0);
        engine.tick().unwrap();
        assert_eq!(engine.snapshot().name, "http-renamed");
        assert_eq!(engine.applied_revision(), 1);
        stop.store(true, Ordering::Release);
    }

    #[test]
    fn auth_applies_to_queries_and_remote_requires_token() {
        let engine = Engine::new("auth").shared();
        let stop = Arc::new(AtomicBool::new(false));
        let ports = spawn_control(engine.clone(), config(), stop.clone()).unwrap();
        let response = http_request(
            ports.http,
            "GET /v1/project HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert!(response.contains("401 Unauthorized"), "{response}");
        stop.store(true, Ordering::Release);

        let remote = spawn_control(
            engine,
            ControlConfig {
                http_bind: "0.0.0.0:0".into(),
                tcp_bind: "0.0.0.0:0".into(),
                websocket_bind: "0.0.0.0:0".into(),
                require_token: None,
                ..ControlConfig::default()
            },
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(
            remote.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn transaction_is_atomic_over_http() {
        let engine = Engine::new("transaction").shared();
        let stop = Arc::new(AtomicBool::new(false));
        let ports = spawn_control(engine.clone(), config(), stop.clone()).unwrap();
        let client = ClientId::new();
        let mut rename = CommandEnvelope::new(
            client,
            Command::SetName {
                name: "must-not-commit".into(),
            },
        );
        rename.client_seq = 1;
        let mut invalid = CommandEnvelope::new(
            client,
            Command::RemoveMixingUnit {
                id: engine.primary_unit(),
            },
        );
        invalid.client_seq = 2;
        let body = serde_json::to_string(&TransactionRequest {
            version: CONTROL_API_VERSION,
            envelopes: vec![rename, invalid],
        })
        .unwrap();
        let response = http_request(
            ports.http,
            &format!(
                "POST /v1/transaction HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer integration-secret\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            ),
        );
        assert!(response.contains("400 Bad Request"), "{response}");
        assert_eq!(engine.snapshot().name, "transaction");
        assert_eq!(engine.revision(), 0);
        stop.store(true, Ordering::Release);
    }

    #[test]
    fn tcp_requires_versioned_authenticated_request() {
        let engine = Engine::new("tcp").shared();
        let stop = Arc::new(AtomicBool::new(false));
        let ports = spawn_control(engine.clone(), config(), stop.clone()).unwrap();
        let mut envelope = CommandEnvelope::new(
            ClientId::new(),
            Command::SetName {
                name: "tcp-renamed".into(),
            },
        );
        envelope.client_seq = 1;
        let request = ApiRequest {
            version: CONTROL_API_VERSION,
            request_id: "tcp-1".into(),
            token: Some("integration-secret".into()),
            operation: ApiOperation::Command {
                envelope: Box::new(envelope),
            },
        };
        let mut stream = TcpStream::connect(("127.0.0.1", ports.tcp)).unwrap();
        writeln!(stream, "{}", serde_json::to_string(&request).unwrap()).unwrap();
        stream.flush().unwrap();
        let mut bytes = [0; 4096];
        let count = stream.read(&mut bytes).unwrap();
        let response = String::from_utf8_lossy(&bytes[..count]);
        assert!(response.contains("\"result\""), "{response}");
        assert!(!response.contains("\"error\""), "{response}");
        assert_eq!(engine.snapshot().name, "tcp");
        assert_eq!(engine.staged_snapshot().name, "tcp-renamed");
        stop.store(true, Ordering::Release);
    }

    #[test]
    fn websocket_auth_query_command_and_event_subscription() {
        let engine = Engine::new("websocket").shared();
        let stop = Arc::new(AtomicBool::new(false));
        let ports = spawn_control(engine.clone(), config(), stop.clone()).unwrap();
        // The accept loop is intentionally non-blocking; allow its worker to
        // enter the poll loop before beginning the synchronous handshake.
        std::thread::sleep(Duration::from_millis(50));
        let stream = TcpStream::connect(("127.0.0.1", ports.websocket)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut upgrade = format!("ws://127.0.0.1:{}/v1/ws", ports.websocket)
            .into_client_request()
            .unwrap();
        upgrade.headers_mut().insert(
            "authorization",
            tungstenite::http::HeaderValue::from_static("Bearer integration-secret"),
        );
        let (mut websocket, _) = tungstenite::client(upgrade, stream).unwrap();

        let subscribe = ApiRequest {
            version: CONTROL_API_VERSION,
            request_id: "subscribe-1".into(),
            token: None,
            operation: ApiOperation::Subscribe,
        };
        websocket
            .send(Message::Text(
                serde_json::to_string(&subscribe).unwrap().into(),
            ))
            .unwrap();
        let response = websocket.read().unwrap().into_text().unwrap();
        assert!(response.contains("\"subscribed\":true"), "{response}");

        let mut envelope = CommandEnvelope::new(
            ClientId::new(),
            Command::SetName {
                name: "ws-renamed".into(),
            },
        );
        envelope.client_seq = 1;
        let command = ApiRequest {
            version: CONTROL_API_VERSION,
            request_id: "command-1".into(),
            token: None,
            operation: ApiOperation::Command {
                envelope: Box::new(envelope),
            },
        };
        websocket
            .send(Message::Text(
                serde_json::to_string(&command).unwrap().into(),
            ))
            .unwrap();
        let response = websocket.read().unwrap().into_text().unwrap();
        assert!(
            response.contains("\"request_id\":\"command-1\""),
            "{response}"
        );
        let event = websocket.read().unwrap().into_text().unwrap();
        assert!(event.contains("\"event\":\"command_accepted\""), "{event}");
        assert_eq!(engine.snapshot().name, "websocket");
        assert_eq!(engine.staged_snapshot().name, "ws-renamed");
        stop.store(true, Ordering::Release);
    }

    #[test]
    fn bounded_command_and_event_queues_fail_closed() {
        let (send, _receive) = bounded(1);
        let dispatcher = Dispatcher { tx: send };
        let (reply, _reply_receive) = bounded(1);
        dispatcher
            .tx
            .send(DispatchTask::Command {
                envelope: Box::new(CommandEnvelope::new(ClientId::new(), Command::Noop)),
                reply,
            })
            .unwrap();
        let error = dispatcher
            .submit(CommandEnvelope::new(ClientId::new(), Command::Noop))
            .unwrap_err();
        assert_eq!(error.code, "busy");

        let broker = EventBroker::new(1);
        let events = broker.subscribe();
        let event = ControlEvent {
            version: CONTROL_API_VERSION,
            event: "command_accepted",
            revision: 1,
            applied_revision: 0,
            state_hash: "hash".into(),
            command_ids: vec![],
        };
        broker.publish(event.clone());
        broker.publish(event);
        assert!(events.recv().is_ok());
        assert!(matches!(
            events.try_recv(),
            Err(crossbeam_channel::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn http_rate_limit_is_enforced() {
        let engine = Engine::new("rate").shared();
        let stop = Arc::new(AtomicBool::new(false));
        let ports = spawn_control(
            engine,
            ControlConfig {
                max_requests_per_sec: 1,
                require_token: Some("integration-secret".into()),
                ..ControlConfig::default()
            },
            stop.clone(),
        )
        .unwrap();
        let request = "GET /v1/status HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer integration-secret\r\nConnection: close\r\n\r\n";
        let first = http_request(ports.http, request);
        let second = http_request(ports.http, request);
        assert!(first.contains("200 OK"), "{first}");
        assert!(second.contains("429 Too Many Requests"), "{second}");
        stop.store(true, Ordering::Release);
    }

    #[cfg(feature = "midi")]
    #[test]
    fn midi_channel_messages_match_only_explicit_mapping() {
        let trigger = MidiTrigger::ControlChange {
            channel: 2,
            controller: 7,
            minimum_value: 64,
            maximum_value: 127,
        };
        assert!(midi_trigger_matches(
            &trigger,
            RawMidiMessage {
                bytes: [0xb2, 7, 100],
                len: 3,
            }
        ));
        assert!(!midi_trigger_matches(
            &trigger,
            RawMidiMessage {
                bytes: [0xb1, 7, 100],
                len: 3,
            }
        ));
        assert!(!midi_trigger_matches(
            &trigger,
            RawMidiMessage {
                bytes: [0xb2, 7, 63],
                len: 3,
            }
        ));
    }
}
