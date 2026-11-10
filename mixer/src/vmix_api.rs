//! vMix-compatible HTTP API hosted inside the mixer.

use std::collections::HashMap;
use std::ffi::c_char;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

#[cfg(test)]
use crate::abi::UnitState;
use crate::abi::{
    ERR_INVALID_ARGUMENT, INCOMING_PREVIEW, OK, OUTPUT_PREVIEW, OUTPUT_PROGRAM, OUTPUT_SOURCE,
    TRANSITION_FADE,
};
use crate::guard::LockExt;
use crate::session::Document;
use crate::vmix_xml::{FlatMap, UnitLive, fade_duration_ms, render_xml, resolve_mix};

#[derive(Clone, Debug)]
struct ApiConfig {
    #[allow(dead_code)]
    enabled: bool,
    port: u16,
    user: String,
    pass: String,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 8088,
            user: String::new(),
            pass: String::new(),
        }
    }
}

struct ApiState {
    config: ApiConfig,
    document: Option<Document>,
    stop: Arc<AtomicBool>,
    server: Option<Arc<Server>>,
    join: Option<JoinHandle<()>>,
    listen_owner: Option<String>,
}

fn api_slot() -> &'static Mutex<ApiState> {
    static API: OnceLock<Mutex<ApiState>> = OnceLock::new();
    API.get_or_init(|| {
        Mutex::new(ApiState {
            config: ApiConfig::default(),
            document: None,
            stop: Arc::new(AtomicBool::new(false)),
            server: None,
            join: None,
            listen_owner: None,
        })
    })
}

pub fn publish_bytes(bytes: &[u8]) -> i32 {
    match crate::session::parse(bytes) {
        Ok(doc) => {
            let Ok(mut slot) = api_slot().lock() else {
                return ERR_INVALID_ARGUMENT;
            };
            slot.document = Some(doc);
            OK
        }
        Err(error) => {
            crate::diag::http_error(&format!("session publish: {error}"));
            ERR_INVALID_ARGUMENT
        }
    }
}

pub fn configure(enabled: bool, port: u32, user: &str, pass: &str) -> i32 {
    if enabled && (port == 0 || port > u32::from(u16::MAX)) {
        return ERR_INVALID_ARGUMENT;
    }
    let config = ApiConfig {
        enabled,
        port: port as u16,
        user: user.to_string(),
        pass: pass.to_string(),
    };
    stop_worker();
    let Ok(mut slot) = api_slot().lock() else {
        return ERR_INVALID_ARGUMENT;
    };
    slot.config = config.clone();
    if !enabled {
        slot.server = None;
        crate::diag::http_info("http disabled");
        return OK;
    }
    slot.listen_owner = None;
    let reuse = slot
        .server
        .as_ref()
        .and_then(|server| server.server_addr().to_ip())
        .is_some_and(|addr| addr.port() == config.port);
    if !reuse {
        slot.server = None;
        let addr = format!("0.0.0.0:{}", config.port);
        match Server::http(&addr) {
            Ok(server) => {
                crate::diag::http_info(&format!("http listen {addr}"));
                slot.server = Some(Arc::new(server));
            }
            Err(error) => {
                let owner = crate::tcp_listen_owner::name(config.port);
                match owner.as_deref() {
                    Some(name) => {
                        crate::diag::http_error(&format!("http listen {addr}: {error} ({name})"))
                    }
                    None => crate::diag::http_error(&format!("http listen {addr}: {error}")),
                }
                slot.listen_owner = owner;
                slot.config.enabled = false;
                slot.server = None;
                return crate::abi::ERR_IO;
            }
        }
    } else {
        crate::diag::http_info(&format!("http restart 0.0.0.0:{}", config.port));
    }
    let server = slot.server.clone().expect("http listener");
    match spawn_worker(server, Arc::clone(&slot.stop), config) {
        Ok(join) => {
            slot.join = Some(join);
            OK
        }
        Err(error) => {
            crate::diag::http_error(&error);
            slot.config.enabled = false;
            slot.server = None;
            crate::abi::ERR_IO
        }
    }
}

pub fn suspend() {
    stop_worker();
    crate::vmix_tcp::configure(false);
    crate::native_ws::configure(false, 0);
}

#[cfg(test)]
pub fn shutdown() {
    stop_worker();
    crate::vmix_tcp::configure(false);
    crate::native_ws::configure(false, 0);
    if let Ok(mut slot) = api_slot().lock() {
        slot.server = None;
    }
}

fn stop_worker() {
    let (stop, server, join) = {
        let mut slot = api_slot().lock_or_recover();
        (
            Arc::clone(&slot.stop),
            slot.server.clone(),
            slot.join.take(),
        )
    };
    stop.store(true, Ordering::Relaxed);
    if let Some(server) = server.as_ref() {
        server.unblock();
    }
    // The worker may itself be waiting on the slot lock, so joining must happen outside it.
    if let Some(join) = join {
        let _ = join.join();
    }
    stop.store(false, Ordering::Relaxed);
}

fn spawn_worker(
    server: Arc<Server>,
    stop: Arc<AtomicBool>,
    config: ApiConfig,
) -> Result<JoinHandle<()>, String> {
    thread::Builder::new()
        .name("eiviz-vmix-http".into())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match server.recv_timeout(Duration::from_millis(200)) {
                    Ok(Some(request)) => {
                        if let Err(payload) = std::panic::catch_unwind(AssertUnwindSafe(|| {
                            handle_request(request, &config)
                        })) {
                            crate::diag::http_error(&format!(
                                "request handler panicked: {}",
                                crate::guard::panic_message(payload.as_ref())
                            ));
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        if !stop.load(Ordering::Relaxed) {
                            crate::diag::http_warn(&format!("recv: {error}"));
                        }
                    }
                }
            }
            crate::diag::http_info("http stopped");
        })
        .map_err(|error| error.to_string())
}

fn handle_request(request: Request, config: &ApiConfig) {
    if request.method() != &Method::Get {
        let _ = request.respond(Response::from_string("method not allowed").with_status_code(405));
        return;
    }
    let url = request.url().to_string();
    let (path, query) = split_url(&url);
    if !is_api_path(path) {
        let _ = request.respond(Response::from_string("not found").with_status_code(404));
        return;
    }
    if !check_auth(&request, config) {
        crate::diag::http_warn("401 unauthorized");
        let mut response = Response::from_string("unauthorized").with_status_code(401);
        if let Ok(header) =
            Header::from_bytes(&b"WWW-Authenticate"[..], &b"Basic realm=\"eiviz\""[..])
        {
            response = response.with_header(header);
        }
        let _ = request.respond(response);
        return;
    }

    let params = parse_query(query);
    let function = params
        .get("Function")
        .or_else(|| params.get("function"))
        .cloned();
    if let Some(name) = function {
        match dispatch_function(&name, &params) {
            Ok(()) => {
                crate::diag::http_info(&format!(
                    "200 Function={name} {}",
                    summarize_params(&params)
                ));
            }
            Err(DispatchError::Unknown(message)) => {
                crate::diag::http_warn(&format!("404 Function={name} {message}"));
                let _ = request.respond(Response::from_string(message).with_status_code(404));
                return;
            }
            Err(DispatchError::BadRequest(message)) => {
                crate::diag::http_warn(&format!("400 Function={name} {message}"));
                let _ = request.respond(Response::from_string(message).with_status_code(400));
                return;
            }
            Err(DispatchError::Failed(message)) => {
                crate::diag::http_error(&format!("500 Function={name} {message}"));
                let _ = request.respond(Response::from_string(message).with_status_code(500));
                return;
            }
        }
    }

    match current_xml() {
        Ok(xml) => {
            let mut response = Response::from_string(xml).with_status_code(StatusCode(200));
            if let Ok(header) =
                Header::from_bytes(&b"Content-Type"[..], &b"application/xml; charset=utf-8"[..])
            {
                response = response.with_header(header);
            }
            let _ = request.respond(response);
        }
        Err(error) => {
            crate::diag::http_error(&format!("xml: {error}"));
            let _ = request.respond(Response::from_string(error).with_status_code(500));
        }
    }
}

#[derive(Debug)]
pub(crate) enum DispatchError {
    Unknown(String),
    BadRequest(String),
    Failed(String),
}

pub(crate) fn dispatch_function(
    name: &str,
    params: &HashMap<String, String>,
) -> Result<(), DispatchError> {
    if let Some(result) = dispatch_replay(name, params) {
        return result;
    }
    match name {
        "Cut" | "CutDirect" | "Fade" | "PreviewInput" | "ActiveInput" | "Snapshot"
        | "SnapshotInput" => {}
        _ => return Err(DispatchError::Unknown(format!("unknown Function {name}"))),
    }
    let doc = {
        let slot = api_slot()
            .lock()
            .map_err(|_| DispatchError::Failed("api lock".into()))?;
        slot.document
            .clone()
            .ok_or_else(|| DispatchError::BadRequest("session not published".into()))?
    };
    let mix_raw = params.get("Mix").map(String::as_str);
    let unit_id = resolve_mix(&doc, mix_raw).map_err(DispatchError::BadRequest)?;
    let live =
        crate::live_unit(unit_id).ok_or_else(|| DispatchError::BadRequest("unknown Mix".into()))?;
    let flat = FlatMap::build(&doc);
    let input_raw = params.get("Input").map(String::as_str).unwrap_or("");

    match name {
        "Cut" => {
            let incoming = resolve_incoming(&flat, input_raw, &live)?;
            if takes_from_preview(input_raw) {
                cut(unit_id, true, INCOMING_PREVIEW)
            } else {
                cut(unit_id, false, incoming)
            }
        }
        "CutDirect" => {
            require_input(input_raw)?;
            let incoming = resolve_incoming(&flat, input_raw, &live)?;
            cut(unit_id, false, incoming)
        }
        "Fade" => {
            let incoming = resolve_incoming(&flat, input_raw, &live)?;
            let from_preview = takes_from_preview(input_raw);
            let unit = doc.units.iter().find(|item| item.id == unit_id);
            let duration = params
                .get("Duration")
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|value| *value > 0)
                .unwrap_or_else(|| {
                    fade_duration_ms(
                        &doc.transitions,
                        unit.map(|item| item.fps_num).unwrap_or(60_000),
                        unit.map(|item| item.fps_den).unwrap_or(1_001),
                    )
                });
            fade(
                unit_id,
                duration,
                from_preview,
                if from_preview {
                    INCOMING_PREVIEW
                } else {
                    incoming
                },
            )
        }
        "PreviewInput" => {
            require_input(input_raw)?;
            let incoming = resolve_incoming(&flat, input_raw, &live)?;
            set_preview(unit_id, incoming)
        }
        "ActiveInput" => {
            require_input(input_raw)?;
            let incoming = resolve_incoming(&flat, input_raw, &live)?;
            cut(unit_id, false, incoming)
        }
        "Snapshot" => {
            let value = params.get("Value").map(String::as_str).unwrap_or("");
            snapshot(unit_id, OUTPUT_PROGRAM, value)
        }
        "SnapshotInput" => {
            require_input(input_raw)?;
            let value = params.get("Value").map(String::as_str).unwrap_or("");
            if input_raw == "0" {
                snapshot(unit_id, OUTPUT_PREVIEW, value)
            } else if input_raw == "-1" {
                snapshot(unit_id, OUTPUT_PROGRAM, value)
            } else {
                let found = flat
                    .resolve_input(input_raw)
                    .map_err(DispatchError::BadRequest)?
                    .ok_or_else(|| {
                        DispatchError::BadRequest(format!("unknown Input {input_raw}"))
                    })?;
                let kind = if found.is_scene { 0 } else { OUTPUT_SOURCE };
                snapshot(found.source_id, kind, value)
            }
        }
        _ => unreachable!("function allow-list"),
    }
}

fn dispatch_replay(
    name: &str,
    params: &HashMap<String, String>,
) -> Option<Result<(), DispatchError>> {
    if !REPLAY_FUNCTIONS.contains(&name) {
        return None;
    }
    Some(
        replay_command(name, params)
            .map(execute_replay)
            .and_then(|inner| inner),
    )
}

/// Every vMix function this layer maps to Replay.
const REPLAY_FUNCTIONS: [&str; 9] = [
    "ReplayStartRecording",
    "ReplayStopRecording",
    "ReplayMarkIn",
    "ReplayMarkOut",
    "ReplayCue",
    "ReplayPlay",
    "ReplayPause",
    "ReplayJumpToNow",
    "ReplayChangeSpeed",
];

fn replay_command(
    name: &str,
    params: &HashMap<String, String>,
) -> Result<eiviz_control::Command, DispatchError> {
    let command = match name {
        "ReplayStartRecording" => eiviz_control::Command::ReplayRecord { on: true },
        "ReplayStopRecording" => eiviz_control::Command::ReplayRecord { on: false },
        "ReplayMarkIn" => eiviz_control::Command::ReplayMarkIn,
        "ReplayMarkOut" => eiviz_control::Command::ReplayMarkOut {
            name: params.get("Value").cloned().unwrap_or_default(),
        },
        // Cue a clip onto a channel. An empty Value cues the newest clip on the
        // channel, which is the clip the operator just marked.
        "ReplayCue" => eiviz_control::Command::ReplayCue {
            channel: replay_channel(params)?,
            clip_uid: params.get("Value").cloned().unwrap_or_default(),
        },
        "ReplayPlay" => eiviz_control::Command::ReplayTransport {
            channel: replay_channel(params)?,
            playing: true,
            speed_milli: replay_speed(params)?,
            seek_frame: -1,
        },
        // A pause keeps the speed the channel already has; the mixer reads zero as
        // "do not change it".
        "ReplayPause" => eiviz_control::Command::ReplayTransport {
            channel: replay_channel(params)?,
            playing: false,
            speed_milli: 0,
            seek_frame: -1,
        },
        "ReplayJumpToNow" => eiviz_control::Command::ReplayJumpToNow {
            channel: replay_channel(params)?,
        },
        // Changing the speed also starts playback, which is what an operator means
        // by pressing a speed button.
        "ReplayChangeSpeed" => eiviz_control::Command::ReplayTransport {
            channel: replay_channel(params)?,
            playing: true,
            speed_milli: replay_speed(params)?,
            seek_frame: -1,
        },
        other => {
            return Err(DispatchError::Unknown(format!("unknown Function {other}")));
        }
    };
    Ok(command)
}

/// Replay channel from `Channel`, which vMix writes as a letter (`A`) or a number.
/// A channel nobody has is an error: playing the wrong one silently would be worse
/// than refusing.
fn replay_channel(params: &HashMap<String, String>) -> Result<u32, DispatchError> {
    let Some(value) = params.get("Channel") else {
        return Ok(0);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(0);
    }
    if let Ok(index) = value.parse::<u32>() {
        return (index < 26)
            .then_some(index)
            .ok_or_else(|| DispatchError::BadRequest(format!("bad replay channel {value}")));
    }
    match value.as_bytes() {
        [byte] => {
            let letter = byte.to_ascii_uppercase();
            if letter.is_ascii_uppercase() {
                Ok(u32::from(letter - b'A'))
            } else {
                Err(DispatchError::BadRequest(format!(
                    "bad replay channel {value}"
                )))
            }
        }
        _ => Err(DispatchError::BadRequest(format!(
            "bad replay channel {value}"
        ))),
    }
}

/// Playback speed from `Value`, a multiple of normal speed. A missing value means
/// normal speed; a value that is not a positive number is refused.
fn replay_speed(params: &HashMap<String, String>) -> Result<i32, DispatchError> {
    let Some(value) = params.get("Value") else {
        return Ok(1000);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(1000);
    }
    let speed = value
        .parse::<f64>()
        .ok()
        .filter(|speed| speed.is_finite() && *speed > 0.0)
        .ok_or_else(|| DispatchError::BadRequest(format!("bad replay speed {value}")))?;
    Ok((speed * 1000.0).round() as i32)
}

fn execute_replay(command: eiviz_control::Command) -> Result<(), DispatchError> {
    let result = crate::runtime::control().lock().map(|mut svc| {
        svc.execute(
            eiviz_control::RequestKey {
                client_instance_id: "vmix".into(),
                request_id: String::new(),
            },
            command,
        )
    });
    match result {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => Err(DispatchError::Failed(error.message().to_string())),
        Err(_) => Err(DispatchError::Failed("control lock".into())),
    }
}

fn snapshot(source_id: u64, kind: u32, value: &str) -> Result<(), DispatchError> {
    let path = crate::snapshot::resolve_remote_path(value).map_err(DispatchError::BadRequest)?;
    let code = crate::take_snapshot(source_id, kind, &path);
    if code == OK {
        Ok(())
    } else {
        Err(DispatchError::Failed(format!("snapshot failed ({code})")))
    }
}

fn require_input(raw: &str) -> Result<(), DispatchError> {
    if raw.is_empty() {
        return Err(DispatchError::BadRequest("Input is required".into()));
    }
    Ok(())
}

fn takes_from_preview(raw: &str) -> bool {
    raw.is_empty() || raw == "0"
}

fn resolve_incoming(flat: &FlatMap, raw: &str, live: &UnitLive) -> Result<u64, DispatchError> {
    if raw.is_empty() || raw == "0" {
        return Ok(live.preview_source);
    }
    if raw == "-1" {
        return Ok(live.program_source);
    }
    let input = flat.resolve_scene(raw).map_err(DispatchError::BadRequest)?;
    Ok(input.source_id)
}

fn cut(unit_id: u64, swap: bool, incoming: u64) -> Result<(), DispatchError> {
    execute_live(eiviz_control::Command::Cut {
        unit_id,
        swap,
        incoming: if swap {
            eiviz_control::Incoming::Preview
        } else {
            eiviz_control::Incoming::from_u64(incoming)
        },
    })
}

fn fade(unit_id: u64, duration_ms: u32, swap: bool, incoming: u64) -> Result<(), DispatchError> {
    execute_live(eiviz_control::Command::Auto {
        unit_id,
        kind: TRANSITION_FADE,
        duration_value: duration_ms.max(1),
        duration_unit: 1,
        swap,
        keep_preview: true,
        easing: 0,
        bezier_x1: 0.0,
        bezier_y1: 0.0,
        bezier_x2: 1.0,
        bezier_y2: 1.0,
        direction: 0,
        dip_r: 0.0,
        dip_g: 0.0,
        dip_b: 0.0,
        dip_a: 1.0,
        incoming: if swap {
            eiviz_control::Incoming::Preview
        } else {
            eiviz_control::Incoming::from_u64(incoming)
        },
        softness: 0.02,
        param: 0.0,
    })
}

fn set_preview(unit_id: u64, source_id: u64) -> Result<(), DispatchError> {
    let scene_id = source_id & !crate::SCENE_BASE;
    execute_live(eiviz_control::Command::Preview { unit_id, scene_id })
}

fn execute_live(command: eiviz_control::Command) -> Result<(), DispatchError> {
    match crate::runtime::control().lock() {
        Ok(mut svc) => svc
            .execute(
                eiviz_control::RequestKey {
                    client_instance_id: "vmix".into(),
                    request_id: String::new(),
                },
                command,
            )
            .map(|_| ())
            .map_err(|error| DispatchError::Failed(error.to_string())),
        Err(_) => Err(DispatchError::Failed("control lock".into())),
    }
}

pub(crate) fn published_document() -> Option<Document> {
    api_slot().lock().ok()?.document.clone()
}

pub(crate) fn current_xml() -> Result<String, String> {
    let doc = {
        let slot = api_slot().lock().map_err(|_| "api lock".to_string())?;
        slot.document.clone().unwrap_or_else(empty_document)
    };
    render_xml(&doc, &crate::live_snapshot())
}

fn empty_document() -> Document {
    crate::session::parse(br#"{"version":2}"#).unwrap_or_else(|_| Document {
        version: 2,
        scene_presets: Vec::new(),
        input_tags: Vec::new(),
        scene_tags: Vec::new(),
        replay_tags: Vec::new(),
        settings: Default::default(),
        inputs: Vec::new(),
        scenes: Vec::new(),
        units: Vec::new(),
        transitions: Vec::new(),
        overlays: Vec::new(),
        next_overlay_id: 1,
        stream_encoders: Vec::new(),
        outputs: Vec::new(),
        multiviews: Vec::new(),
        next_input_id: 0,
        next_scene_id: 0,
        next_unit_id: 0,
        next_output_id: 0,
        next_stream_encoder_id: 0,
        next_multiview_id: 0,
        selected_unit_id: 0,
        headphone: Default::default(),
        headphone_copy_monitor: false,
        headphone_listen_kind: 0,
        headphone_listen_id: 0,
        replay: Default::default(),
    })
}

fn check_auth(request: &Request, config: &ApiConfig) -> bool {
    if config.user.is_empty() && config.pass.is_empty() {
        return true;
    }
    let expected = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        format!("{}:{}", config.user, config.pass),
    );
    request.headers().iter().any(|header| {
        header.field.equiv("Authorization")
            && header
                .value
                .as_str()
                .strip_prefix("Basic ")
                .is_some_and(|token| token.trim() == expected)
    })
}

fn is_api_path(path: &str) -> bool {
    let trimmed = path.trim_end_matches('/');
    trimmed.eq_ignore_ascii_case("/api")
}

fn split_url(url: &str) -> (&str, &str) {
    match url.split_once('?') {
        Some((path, query)) => (path, query),
        None => (url, ""),
    }
}

pub(crate) fn parse_query(query: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(url_decode(key), url_decode(value));
    }
    map
}

fn url_decode(input: &str) -> String {
    let mut out = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() && hex_pair(bytes[i + 1], bytes[i + 2]).is_some() => {
                out.push(hex_pair(bytes[i + 1], bytes[i + 2]).unwrap_or(b'%'));
                i += 3;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_pair(high: u8, low: u8) -> Option<u8> {
    let digit = |byte: u8| (byte as char).to_digit(16);
    Some((digit(high)? * 16 + digit(low)?) as u8)
}

fn summarize_params(params: &HashMap<String, String>) -> String {
    let mut parts = Vec::new();
    for key in ["Input", "Mix", "Duration"] {
        if let Some(value) = params.get(key) {
            parts.push(format!("{key}={value}"));
        }
    }
    parts.join(" ")
}

pub unsafe fn publish_c(json: *const u8, len: usize) -> i32 {
    if json.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let bytes = unsafe { std::slice::from_raw_parts(json, len) };
    publish_bytes(bytes)
}

pub unsafe fn configure_c(
    enabled: u32,
    port: u32,
    user: *const c_char,
    pass: *const c_char,
) -> i32 {
    configure(enabled != 0, port, &read_cstr(user), &read_cstr(pass))
}

pub fn listen_owner() -> Option<String> {
    api_slot()
        .lock()
        .ok()
        .and_then(|slot| slot.listen_owner.clone())
}

pub unsafe fn listen_owner_c(out: *mut u8, cap: usize) -> i32 {
    if out.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let name = listen_owner().unwrap_or_default();
    let n = name.len().min(cap);
    unsafe {
        std::ptr::copy_nonoverlapping(name.as_ptr(), out, n);
    }
    n as i32
}

fn read_cstr(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_str()
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn url_decode_handles_multibyte_and_trailing_escapes() {
        assert_eq!(super::url_decode("%41"), "A");
        assert_eq!(super::url_decode("a%20b+c"), "a b c");
        assert_eq!(super::url_decode("%E3%81%82"), "あ");
        assert_eq!(super::url_decode("%%あ"), "%%あ");
        assert_eq!(super::url_decode("%あ"), "%あ");
        assert_eq!(super::url_decode("%4"), "%4");
        assert_eq!(super::url_decode("%"), "%");
    }

    use super::*;
    use serial_test::serial;

    #[test]
    fn query_and_path() {
        let (path, query) = split_url("/API/?Function=Fade&Duration=500");
        assert!(is_api_path(path));
        let params = parse_query(query);
        assert_eq!(params.get("Function").map(String::as_str), Some("Fade"));
        assert_eq!(params.get("Duration").map(String::as_str), Some("500"));
    }

    #[test]
    fn unknown_function_without_session() {
        let params = HashMap::new();
        match dispatch_function("Zoom", &params) {
            Err(DispatchError::Unknown(_)) | Err(DispatchError::BadRequest(_)) => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    /// vMix writes the replay channel as a letter, and some tools send a number.
    /// A channel nobody has is refused instead of playing channel A.
    #[test]
    fn replay_channel_accepts_a_letter_or_a_number_and_refuses_junk() {
        let mut params = HashMap::new();
        assert_eq!(
            super::replay_channel(&params).unwrap(),
            0,
            "default is channel A"
        );
        for (value, want) in [
            ("0", 0),
            ("2", 2),
            ("A", 0),
            ("a", 0),
            ("B", 1),
            ("z", 25),
            (" 3 ", 3),
        ] {
            params.insert("Channel".into(), value.into());
            assert_eq!(
                super::replay_channel(&params).unwrap(),
                want,
                "Channel={value}"
            );
        }
        for value in ["26", "-1", "abc", "AB", "?"] {
            params.insert("Channel".into(), value.into());
            assert!(
                super::replay_channel(&params).is_err(),
                "Channel={value} must be refused"
            );
        }
    }

    #[test]
    fn replay_speed_is_a_multiple_of_normal_speed() {
        let mut params = HashMap::new();
        assert_eq!(super::replay_speed(&params).unwrap(), 1000);
        params.insert("Value".into(), String::new());
        assert_eq!(super::replay_speed(&params).unwrap(), 1000);
        for (value, want) in [("0.5", 500), ("1", 1000), ("2", 2000), ("0.25", 250)] {
            params.insert("Value".into(), value.into());
            assert_eq!(super::replay_speed(&params).unwrap(), want, "Value={value}");
        }
        for value in ["fast", "0", "-1", "NaN"] {
            params.insert("Value".into(), value.into());
            assert!(
                super::replay_speed(&params).is_err(),
                "Value={value} must be refused"
            );
        }
    }

    #[test]
    #[serial(mixer)]
    fn cut_with_input_leaves_preview() {
        crate::mixer_destroy();
        assert_eq!(crate::mixer_create(0, 60_000, 1_001), crate::OK);
        assert_eq!(crate::mixer_create_unit(1, 320, 180), crate::OK);
        publish_cut_session();
        let preview = crate::SCENE_BASE | 2;
        let program = crate::SCENE_BASE | 1;
        let incoming = crate::SCENE_BASE | 3;
        set_unit_buses(program, preview);
        let mut params = HashMap::new();
        params.insert("Input".into(), "3".into());
        dispatch_function("Cut", &params).expect("cut");
        let out = unit_state();
        assert_eq!(out.program_source, incoming);
        assert_eq!(out.preview_source, preview);
        crate::mixer_destroy();
    }

    #[test]
    #[serial(mixer)]
    fn cut_without_input_takes_preview() {
        crate::mixer_destroy();
        assert_eq!(crate::mixer_create(0, 60_000, 1_001), crate::OK);
        assert_eq!(crate::mixer_create_unit(1, 320, 180), crate::OK);
        publish_cut_session();
        let preview = crate::SCENE_BASE | 2;
        let program = crate::SCENE_BASE | 1;
        set_unit_buses(program, preview);
        dispatch_function("Cut", &HashMap::new()).expect("cut");
        let out = unit_state();
        assert_eq!(out.program_source, preview);
        assert_eq!(out.preview_source, program);
        crate::mixer_destroy();
    }

    #[test]
    #[serial(mixer)]
    fn fade_with_input_leaves_preview() {
        crate::mixer_destroy();
        assert_eq!(crate::mixer_create(0, 60_000, 1_001), crate::OK);
        assert_eq!(crate::mixer_create_unit(1, 320, 180), crate::OK);
        publish_cut_session();
        let preview = crate::SCENE_BASE | 2;
        let program = crate::SCENE_BASE | 1;
        let incoming = crate::SCENE_BASE | 3;
        set_unit_buses(program, preview);
        let mut params = HashMap::new();
        params.insert("Input".into(), "3".into());
        params.insert("Duration".into(), "1".into());
        dispatch_function("Fade", &params).expect("fade");
        // Transitions run on the master frame clock, which only the render loop advances.
        crate::lifecycle::with_mixer(|mixer| {
            mixer.shared.lock_or_recover().composed_frame += 1;
        })
        .expect("mixer");
        let out = unit_state();
        assert_eq!(out.program_source, incoming);
        assert_eq!(out.preview_source, preview);
        assert_eq!(out.mix, 0.0);
        crate::mixer_destroy();
    }

    #[test]
    #[serial(mixer)]
    fn snapshot_function_requires_published_session() {
        clear_published();
        crate::mixer_destroy();
        let err = dispatch_function("Snapshot", &HashMap::new()).unwrap_err();
        assert!(
            matches!(err, DispatchError::BadRequest(ref message) if message.contains("not published")),
            "{err:?}"
        );
    }

    #[test]
    #[serial(mixer)]
    fn snapshot_function_writes_after_session_publish() {
        crate::mixer_destroy();
        assert_eq!(crate::mixer_create(0, 60_000, 1_001), crate::OK);
        assert_eq!(crate::mixer_create_unit(1, 320, 180), crate::OK);
        thread::sleep(Duration::from_millis(350));
        publish_cut_session();
        let path = crate::snapshot::snapshot_dir().join("eiviz-vmix-http-snapshot.png");
        let _ = std::fs::remove_file(&path);
        let mut params = HashMap::new();
        params.insert("Value".into(), "eiviz-vmix-http-snapshot.png".to_string());
        let mut result = dispatch_function("Snapshot", &params);
        if result.is_err() {
            thread::sleep(Duration::from_millis(250));
            result = dispatch_function("Snapshot", &params);
        }
        result.expect("snapshot");
        let bytes = std::fs::read(&path).expect("png");
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
        let _ = std::fs::remove_file(&path);
        crate::mixer_destroy();
    }

    fn clear_published() {
        if let Ok(mut slot) = api_slot().lock() {
            slot.document = None;
        }
    }

    fn publish_cut_session() {
        let json = br#"{
  "version": 2,
  "inputs": [{ "id": 10, "name": "Bars", "kind": "Bars" }],
  "scenes": [
    { "id": 1, "name": "Scene 1", "layers": [{ "inputId": 10, "width": 1, "height": 1 }] },
    { "id": 2, "name": "Scene 2", "layers": [{ "inputId": 10, "width": 1, "height": 1 }] },
    { "id": 3, "name": "Scene 3", "layers": [{ "inputId": 10, "width": 1, "height": 1 }] }
  ],
  "units": [{ "id": 1, "name": "MU1" }]
}"#;
        assert_eq!(publish_bytes(json), crate::OK);
    }

    fn set_unit_buses(program: u64, preview: u64) {
        let state = UnitState {
            program_source: program,
            preview_source: preview,
            mix: 0.0,
            ..UnitState::default()
        };
        unsafe {
            assert_eq!(crate::mixer_unit_set_state(1, &state), crate::OK);
        }
    }

    fn unit_state() -> UnitState {
        let mut out = UnitState::default();
        unsafe {
            assert_eq!(crate::mixer_unit_get_state(1, &mut out), crate::OK);
        }
        out
    }

    #[test]
    #[serial(mixer)]
    fn http_auth_xml_and_unknown_function() {
        let port = 18721;
        assert_eq!(configure(true, port, "user", "secret"), crate::abi::OK);
        assert_eq!(configure(true, port, "user", "secret"), crate::abi::OK);
        std::thread::sleep(Duration::from_millis(80));
        let unauth = http_get(port, None, "/api");
        assert!(unauth.contains("401"), "{unauth}");
        let xml = http_get(port, Some(("user", "secret")), "/api");
        assert!(xml.contains("<vmix>"), "{xml}");
        let unknown = http_get(port, Some(("user", "secret")), "/api?Function=Zoom");
        assert!(
            unknown.contains("404") || unknown.contains("unknown Function"),
            "{unknown}"
        );
        shutdown();
    }

    #[cfg(unix)]
    #[test]
    fn busy_port_returns_io_and_leaves_http_off() {
        let occupant = std::net::TcpListener::bind("0.0.0.0:0").expect("occupy");
        let port = occupant.local_addr().expect("addr").port() as u32;
        assert_eq!(configure(true, port, "", ""), crate::abi::ERR_IO);
        drop(occupant);
        assert_eq!(configure(true, port, "", ""), crate::abi::OK);
        shutdown();
    }

    fn http_get(port: u32, auth: Option<(&str, &str)>, path: &str) -> String {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        let mut stream = TcpStream::connect(("127.0.0.1", port as u16)).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
        let mut headers = format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\n");
        if let Some((user, pass)) = auth {
            let token = base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                format!("{user}:{pass}"),
            );
            headers.push_str(&format!("Authorization: Basic {token}\r\n"));
        }
        headers.push_str("\r\n");
        stream.write_all(headers.as_bytes()).expect("write");
        let mut body = String::new();
        stream.read_to_string(&mut body).ok();
        body
    }
}
