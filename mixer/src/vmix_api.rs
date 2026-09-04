//! vMix-compatible HTTP API hosted inside the mixer.

use std::collections::HashMap;
use std::ffi::c_char;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::abi::{ERR_INVALID_ARGUMENT, INCOMING_PREVIEW, OK, TRANSITION_FADE, UnitState};
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
        crate::diag::http_info("disabled");
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
                crate::diag::http_info(&format!("listen {addr}"));
                slot.server = Some(Arc::new(server));
            }
            Err(error) => {
                let owner = crate::tcp_listen_owner::name(config.port);
                match owner.as_deref() {
                    Some(name) => {
                        crate::diag::http_error(&format!("listen {addr}: {error} ({name})"))
                    }
                    None => crate::diag::http_error(&format!("listen {addr}: {error}")),
                }
                slot.listen_owner = owner;
                slot.config.enabled = false;
                slot.server = None;
                return crate::abi::ERR_IO;
            }
        }
    } else {
        crate::diag::http_info(&format!("restart 0.0.0.0:{}", config.port));
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
}

#[cfg(test)]
pub fn shutdown() {
    stop_worker();
    if let Ok(mut slot) = api_slot().lock() {
        slot.server = None;
    }
}

fn stop_worker() {
    let Ok(mut slot) = api_slot().lock() else {
        return;
    };
    slot.stop.store(true, Ordering::Relaxed);
    if let Some(server) = slot.server.as_ref() {
        server.unblock();
    }
    if let Some(join) = slot.join.take() {
        let _ = join.join();
    }
    slot.stop.store(false, Ordering::Relaxed);
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
                    Ok(Some(request)) => handle_request(request, &config),
                    Ok(None) => {}
                    Err(error) => {
                        if !stop.load(Ordering::Relaxed) {
                            crate::diag::http_warn(&format!("recv: {error}"));
                        }
                    }
                }
            }
            crate::diag::http_info("stopped");
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
enum DispatchError {
    Unknown(String),
    BadRequest(String),
    Failed(String),
}

fn dispatch_function(name: &str, params: &HashMap<String, String>) -> Result<(), DispatchError> {
    match name {
        "Cut" | "CutDirect" | "Fade" | "PreviewInput" | "ActiveInput" => {}
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
            if !input_raw.is_empty() && input_raw != "0" && input_raw != "-1" {
                set_preview(unit_id, incoming)?;
            }
            cut(unit_id, true, INCOMING_PREVIEW)
        }
        "CutDirect" => {
            require_input(input_raw)?;
            let incoming = resolve_incoming(&flat, input_raw, &live)?;
            cut(unit_id, false, incoming)
        }
        "Fade" => {
            let incoming = resolve_incoming(&flat, input_raw, &live)?;
            if !input_raw.is_empty() && input_raw != "0" && input_raw != "-1" {
                set_preview(unit_id, incoming)?;
            }
            let unit = doc.units.iter().find(|item| item.id == unit_id);
            let duration = params
                .get("Duration")
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|value| *value > 0)
                .unwrap_or_else(|| {
                    fade_duration_ms(
                        unit,
                        unit.map(|item| item.fps_num).unwrap_or(60_000),
                        unit.map(|item| item.fps_den).unwrap_or(1_001),
                    )
                });
            fade(unit_id, duration)
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
        _ => unreachable!("function allow-list"),
    }
}

fn require_input(raw: &str) -> Result<(), DispatchError> {
    if raw.is_empty() {
        return Err(DispatchError::BadRequest("Input is required".into()));
    }
    Ok(())
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
    let code = crate::mixer_unit_cut(unit_id, u32::from(swap), incoming);
    if code == OK {
        Ok(())
    } else {
        Err(DispatchError::Failed(format!("cut failed ({code})")))
    }
}

fn fade(unit_id: u64, duration_ms: u32) -> Result<(), DispatchError> {
    let code = crate::mixer_unit_auto(
        unit_id,
        TRANSITION_FADE,
        duration_ms.max(1),
        1,
        1,
        0,
        0,
        0.0,
        0.0,
        0.0,
        1.0,
        INCOMING_PREVIEW,
        0.02,
        0.0,
    );
    if code == OK {
        Ok(())
    } else {
        Err(DispatchError::Failed(format!("fade failed ({code})")))
    }
}

fn set_preview(unit_id: u64, source_id: u64) -> Result<(), DispatchError> {
    let mut state = UnitState::default();
    let get = unsafe { crate::mixer_unit_get_state(unit_id, &mut state) };
    if get != OK {
        return Err(DispatchError::Failed(format!("get state failed ({get})")));
    }
    state.preview_source = source_id;
    let set = unsafe { crate::mixer_unit_set_state(unit_id, &state) };
    if set == OK {
        Ok(())
    } else {
        Err(DispatchError::Failed(format!("set preview failed ({set})")))
    }
}

fn current_xml() -> Result<String, String> {
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
        settings: Default::default(),
        inputs: Vec::new(),
        scenes: Vec::new(),
        units: Vec::new(),
        outputs: Vec::new(),
        multiviews: Vec::new(),
        buses: Vec::new(),
        next_input_id: 0,
        next_scene_id: 0,
        next_unit_id: 0,
        next_output_id: 0,
        next_multiview_id: 0,
        next_bus_id: 0,
        selected_unit_id: 0,
        headphone_copy_master: false,
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

fn parse_query(query: &str) -> HashMap<String, String> {
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
            b'%' if i + 2 < bytes.len() => {
                let hex = &input[i + 1..i + 3];
                if let Ok(value) = u8::from_str_radix(hex, 16) {
                    out.push(value);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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
    use super::*;

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

    #[test]
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
