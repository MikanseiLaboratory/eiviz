//! vMix-compatible TCP API ([vMix TCP API](https://www.vmix.com/help29/TCPAPI.html)).
//! Text commands, `\r\n` terminated, port 8099. Iryx talks to this surface.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::abi::{ERR_INVALID_ARGUMENT, ERR_IO, OK};
use crate::vmix_xml::{FlatMap, UnitLive};

pub const TCP_PORT: u16 = 8099;

struct TcpState {
    stop: Option<Arc<AtomicBool>>,
    join: Option<JoinHandle<()>>,
    listen_owner: Option<String>,
}

fn tcp_slot() -> &'static Mutex<TcpState> {
    static SLOT: OnceLock<Mutex<TcpState>> = OnceLock::new();
    SLOT.get_or_init(|| {
        Mutex::new(TcpState {
            stop: None,
            join: None,
            listen_owner: None,
        })
    })
}

pub fn configure(enabled: bool) -> i32 {
    stop_worker();
    if !enabled {
        crate::diag::http_info("tcp disabled");
        return OK;
    }
    let addr = format!("0.0.0.0:{TCP_PORT}");
    let listener = match TcpListener::bind(&addr) {
        Ok(listener) => {
            let _ = listener.set_nonblocking(true);
            crate::diag::http_info(&format!("tcp listen {addr}"));
            listener
        }
        Err(error) => {
            let owner = crate::tcp_listen_owner::name(TCP_PORT);
            match owner.as_deref() {
                Some(name) => {
                    crate::diag::http_error(&format!("tcp listen {addr}: {error} ({name})"))
                }
                None => crate::diag::http_error(&format!("tcp listen {addr}: {error}")),
            }
            if let Ok(mut slot) = tcp_slot().lock() {
                slot.listen_owner = owner;
            }
            return ERR_IO;
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    let Ok(mut slot) = tcp_slot().lock() else {
        return ERR_INVALID_ARGUMENT;
    };
    slot.listen_owner = None;
    let thread_stop = Arc::clone(&stop);
    match thread::Builder::new()
        .name("eiviz-vmix-tcp".into())
        .spawn(move || accept_loop(listener, thread_stop))
    {
        Ok(join) => {
            slot.stop = Some(stop);
            slot.join = Some(join);
            OK
        }
        Err(error) => {
            crate::diag::http_error(&format!("tcp spawn: {error}"));
            ERR_IO
        }
    }
}

pub fn listen_owner() -> Option<String> {
    tcp_slot()
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

fn stop_worker() {
    let Ok(mut slot) = tcp_slot().lock() else {
        return;
    };
    if let Some(stop) = slot.stop.take() {
        stop.store(true, Ordering::Relaxed);
    }
    if let Some(join) = slot.join.take() {
        let _ = join.join();
    }
    slot.listen_owner = None;
}

fn accept_loop(listener: TcpListener, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let stop = Arc::clone(&stop);
                let _ = thread::Builder::new()
                    .name("eiviz-vmix-tcp-client".into())
                    .spawn(move || handle_client(stream, stop));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                if !stop.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }
}

fn handle_client(stream: TcpStream, stop: Arc<AtomicBool>) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let writer = Arc::new(Mutex::new(clone));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut sub_tally = false;
    let mut sub_acts = false;
    let mut last_tally = String::new();
    let mut last_acts = String::new();
    while !stop.load(Ordering::Relaxed) {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                if let Err(error) = tick_subs(
                    &writer,
                    sub_tally,
                    sub_acts,
                    &mut last_tally,
                    &mut last_acts,
                ) {
                    crate::diag::http_warn(&format!("tcp sub: {error}"));
                    break;
                }
                continue;
            }
            Err(_) => break,
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            continue;
        }
        match dispatch_line(trimmed) {
            Reply::Line(text) => {
                if write_all(&writer, text.as_bytes()).is_err() {
                    break;
                }
            }
            Reply::Xml(xml) => {
                if write_all(&writer, &encode_xml(&xml)).is_err() {
                    break;
                }
            }
            Reply::SubscribeTally => {
                crate::diag::http_info("tcp SUBSCRIBE TALLY");
                sub_tally = true;
                let _ = write_all(&writer, b"SUBSCRIBE OK TALLY\r\n");
                last_tally.clear();
            }
            Reply::SubscribeActs => {
                crate::diag::http_info("tcp SUBSCRIBE ACTS");
                sub_acts = true;
                let _ = write_all(&writer, b"SUBSCRIBE OK ACTS\r\n");
                last_acts.clear();
            }
            Reply::UnsubscribeTally => {
                sub_tally = false;
                let _ = write_all(&writer, b"UNSUBSCRIBE OK TALLY\r\n");
            }
            Reply::UnsubscribeActs => {
                sub_acts = false;
                let _ = write_all(&writer, b"UNSUBSCRIBE OK ACTS\r\n");
            }
            Reply::Quit => break,
        }
    }
}

fn tick_subs(
    writer: &Arc<Mutex<TcpStream>>,
    sub_tally: bool,
    sub_acts: bool,
    last_tally: &mut String,
    last_acts: &mut String,
) -> std::io::Result<()> {
    if sub_tally {
        let now = tally_string();
        if now != *last_tally {
            *last_tally = now.clone();
            write_all(writer, format!("TALLY OK {now}\r\n").as_bytes())?;
        }
    }
    if sub_acts {
        let now = acts_snapshot();
        if now != *last_acts {
            for line in changed_lines(last_acts, &now) {
                write_all(writer, format!("{line}\r\n").as_bytes())?;
            }
            *last_acts = now;
        }
    }
    Ok(())
}

fn write_all(writer: &Arc<Mutex<TcpStream>>, bytes: &[u8]) -> std::io::Result<()> {
    let mut slot = writer
        .lock()
        .map_err(|_| std::io::Error::other("tcp writer lock"))?;
    slot.write_all(bytes)?;
    slot.flush()
}

#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Line(String),
    Xml(String),
    SubscribeTally,
    SubscribeActs,
    UnsubscribeTally,
    UnsubscribeActs,
    Quit,
}

fn dispatch_line(line: &str) -> Reply {
    let line = line.trim();
    let (command, rest) = split_cmd(line);
    let command = command.to_ascii_uppercase();
    match command.as_str() {
        "TALLY" => Reply::Line(format!("TALLY OK {}\r\n", tally_string())),
        "FUNCTION" => Reply::Line(function_reply(rest)),
        "ACTS" => Reply::Line(acts_reply(rest)),
        "XML" => match crate::vmix_api::current_xml() {
            Ok(xml) => Reply::Xml(xml),
            Err(error) => Reply::Line(format!("XML ER {error}\r\n")),
        },
        "XMLTEXT" => Reply::Line(xmltext_reply(rest)),
        "SUBSCRIBE" => match rest.trim().to_ascii_uppercase().as_str() {
            "TALLY" => Reply::SubscribeTally,
            "ACTS" => Reply::SubscribeActs,
            other => Reply::Line(format!("SUBSCRIBE ER unknown command {other}\r\n")),
        },
        "UNSUBSCRIBE" => match rest.trim().to_ascii_uppercase().as_str() {
            "TALLY" => Reply::UnsubscribeTally,
            "ACTS" => Reply::UnsubscribeActs,
            other => Reply::Line(format!("UNSUBSCRIBE ER unknown command {other}\r\n")),
        },
        "QUIT" => Reply::Quit,
        "VERSION" => Reply::Line(format!("VERSION OK {}\r\n", crate::vmix_xml::VERSION)),
        _ => Reply::Line(format!("{command} ER unknown command\r\n")),
    }
}

fn split_cmd(line: &str) -> (&str, &str) {
    match line.find(char::is_whitespace) {
        Some(index) => (&line[..index], line[index..].trim_start()),
        None => (line, ""),
    }
}

fn changed_lines<'a>(old: &str, new: &'a str) -> Vec<&'a str> {
    if old.is_empty() {
        return new.lines().collect();
    }
    let prev: HashSet<&str> = old.lines().collect();
    new.lines().filter(|line| !prev.contains(line)).collect()
}

fn function_reply(rest: &str) -> String {
    let rest = rest.trim();
    if rest.is_empty() {
        return "FUNCTION ER Function is required\r\n".into();
    }
    let (name, query) = match rest.split_once(char::is_whitespace) {
        Some((name, query)) => (name, query.trim()),
        None => (rest, ""),
    };
    let params = crate::vmix_api::parse_query(query);
    match crate::vmix_api::dispatch_function(name, &params) {
        Ok(()) => {
            crate::diag::http_info(&format!("tcp FUNCTION {name} OK"));
            "FUNCTION OK Completed\r\n".into()
        }
        Err(crate::vmix_api::DispatchError::Unknown(message))
        | Err(crate::vmix_api::DispatchError::BadRequest(message))
        | Err(crate::vmix_api::DispatchError::Failed(message)) => {
            crate::diag::http_warn(&format!("tcp FUNCTION {name} ER {message}"));
            format!("FUNCTION ER {message}\r\n")
        }
    }
}

fn encode_xml(xml: &str) -> Vec<u8> {
    let mut body = xml.as_bytes().to_vec();
    if !body.ends_with(b"\r\n") {
        body.extend_from_slice(b"\r\n");
    }
    let header = format!("XML {}\r\n", body.len());
    let mut out = header.into_bytes();
    out.extend_from_slice(&body);
    out
}

fn xmltext_reply(xpath: &str) -> String {
    let xpath = xpath.trim();
    if xpath.is_empty() {
        return "XMLTEXT ER XPATH is required\r\n".into();
    }
    match crate::vmix_api::current_xml() {
        Ok(xml) => match xml_text(xpath, &xml) {
            Ok(value) => {
                if value.contains('\n') {
                    let mut body = value;
                    if !body.ends_with("\r\n") {
                        body.push_str("\r\n");
                    }
                    format!("XMLTEXT {}\r\n{body}", body.len())
                } else {
                    format!("XMLTEXT OK {value}\r\n")
                }
            }
            Err(error) => format!("XMLTEXT ER {error}\r\n"),
        },
        Err(error) => format!("XMLTEXT ER {error}\r\n"),
    }
}

fn xml_text(xpath: &str, xml: &str) -> Result<String, String> {
    let path = xpath.trim().trim_start_matches('/');
    if let Some(tag) = path.strip_prefix("vmix/") {
        if !tag.contains('/') && !tag.contains('@') && !tag.contains('[') {
            return tag_text(xml, tag);
        }
    }
    if let Some((index, attr)) = parse_input_attr_path(path) {
        return input_attr(xml, index, attr);
    }
    Err(format!("unsupported XPATH {xpath}"))
}

fn parse_input_attr_path(path: &str) -> Option<(usize, &str)> {
    let rest = path.strip_prefix("vmix/inputs/input[")?;
    let (index, rest) = rest.split_once(']')?;
    let index = index.parse::<usize>().ok()?;
    let attr = rest.strip_prefix("/@")?;
    if index == 0 {
        return None;
    }
    Some((index, attr))
}

fn tag_text(xml: &str, tag: &str) -> Result<String, String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open).ok_or_else(|| format!("missing <{tag}>"))? + open.len();
    let end = xml[start..]
        .find(&close)
        .ok_or_else(|| format!("missing </{tag}>"))?
        + start;
    Ok(xml[start..end].to_string())
}

fn input_attr(xml: &str, index: usize, attr: &str) -> Result<String, String> {
    let mut remaining = xml;
    let mut found = 0usize;
    while let Some(pos) = remaining.find("<input ") {
        remaining = &remaining[pos + 7..];
        found += 1;
        if found != index {
            continue;
        }
        let end = remaining
            .find('>')
            .ok_or_else(|| "bad input tag".to_string())?;
        let attrs = &remaining[..end];
        let key = format!("{attr}=\"");
        let start = attrs.find(&key).ok_or_else(|| format!("missing @{attr}"))? + key.len();
        let stop = attrs[start..]
            .find('"')
            .ok_or_else(|| format!("bad @{attr}"))?
            + start;
        return Ok(attrs[start..stop].to_string());
    }
    Err(format!("no input[{index}]"))
}

fn tally_string() -> String {
    let Some((flat, lives)) = session_tally() else {
        return String::new();
    };
    tally_from(&flat, &lives)
}

fn tally_from(flat: &FlatMap, lives: &[UnitLive]) -> String {
    let mut digits = vec![b'0'; flat.inputs.len()];
    for live in lives {
        mark_tally(flat, &mut digits, live.preview_source, b'2');
        mark_tally(flat, &mut digits, live.program_source, b'1');
    }
    String::from_utf8(digits).unwrap_or_default()
}

fn mark_tally(flat: &FlatMap, digits: &mut [u8], source: u64, value: u8) {
    if let Some(input) = flat.by_source(source) {
        let index = input.number.saturating_sub(1) as usize;
        if index < digits.len() && digits[index] != b'1' {
            digits[index] = value;
        }
    }
}

fn acts_reply(rest: &str) -> String {
    let rest = rest.trim();
    let mut parts = rest.split_whitespace();
    let Some(name) = parts.next() else {
        return "ACTS ER ActivatorName is required\r\n".into();
    };
    let input = parts.next();
    match acts_value(name, input) {
        Ok(line) => format!("ACTS OK {line}\r\n"),
        Err(error) => format!("ACTS ER {error}\r\n"),
    }
}

fn acts_snapshot() -> String {
    let Some((flat, lives)) = session_tally() else {
        return String::new();
    };
    let mut lines = Vec::new();
    for (index, live) in lives.iter().enumerate() {
        let program = flat.by_source(live.program_source).map(|item| item.number);
        let preview = flat.by_source(live.preview_source).map(|item| item.number);
        for input in &flat.inputs {
            let on_pgm = program == Some(input.number);
            let on_pvw = preview == Some(input.number);
            if index == 0 {
                lines.push(format!(
                    "ACTS OK Input {} {}",
                    input.number,
                    u8::from(on_pgm)
                ));
                lines.push(format!(
                    "ACTS OK InputPreview {} {}",
                    input.number,
                    u8::from(on_pvw)
                ));
            } else {
                let mix = index + 1;
                lines.push(format!(
                    "ACTS OK InputMix{mix} {} {}",
                    input.number,
                    u8::from(on_pgm)
                ));
                lines.push(format!(
                    "ACTS OK InputPreviewMix{mix} {} {}",
                    input.number,
                    u8::from(on_pvw)
                ));
            }
        }
        if index == 0 {
            for slot in 1..=8 {
                let source = live.overlay_sources.get(slot - 1).copied().unwrap_or(0);
                let number = flat.by_source(source).map(|item| item.number).unwrap_or(0);
                let active = u8::from(source != 0);
                lines.push(format!("ACTS OK Overlay{slot} {number} {active}"));
            }
        }
    }
    lines.join("\n")
}

fn acts_value(name: &str, input: Option<&str>) -> Result<String, String> {
    if is_bool_stub(name) {
        return Ok(format!("{name} 0"));
    }
    if is_input_bool_stub(name) {
        let number = input.unwrap_or("0");
        return Ok(format!("{name} {number} 0"));
    }
    let Some((flat, lives)) = session_tally() else {
        return Err("session not published".into());
    };
    if input.is_none() && (name == "Input" || name == "InputPreview" || name.starts_with("Overlay"))
    {
        return assigned_input(name, &flat, &lives);
    }
    let number = match input {
        Some(raw) => raw
            .parse::<u32>()
            .map_err(|_| format!("bad InputNumber {raw}"))?,
        None => {
            return Err("No Input".into());
        }
    };
    let (mix_index, preview) = mix_from_activator(name)?;
    let live = lives
        .get(mix_index)
        .ok_or_else(|| "unknown Mix".to_string())?;
    if name.starts_with("Overlay") {
        let slot = name
            .trim_start_matches("Overlay")
            .parse::<usize>()
            .map_err(|_| "unknown activator".to_string())?;
        if !(1..=8).contains(&slot) {
            return Err("unknown activator".into());
        }
        let source = live.overlay_sources.get(slot - 1).copied().unwrap_or(0);
        let assigned = flat.by_source(source).map(|item| item.number).unwrap_or(0);
        let active = u8::from(assigned == number && source != 0);
        return Ok(format!("{name} {number} {active}"));
    }
    let source = if preview {
        live.preview_source
    } else {
        live.program_source
    };
    let on = flat
        .by_source(source)
        .is_some_and(|item| item.number == number);
    Ok(format!("{name} {number} {}", u8::from(on)))
}

fn assigned_input(name: &str, flat: &FlatMap, lives: &[UnitLive]) -> Result<String, String> {
    if name.starts_with("Overlay") {
        let slot = name
            .trim_start_matches("Overlay")
            .parse::<usize>()
            .map_err(|_| "unknown activator".to_string())?;
        let live = lives.first().ok_or_else(|| "No Input".to_string())?;
        let source = live.overlay_sources.get(slot - 1).copied().unwrap_or(0);
        let number = flat
            .by_source(source)
            .map(|item| item.number)
            .ok_or_else(|| "No Input".to_string())?;
        return Ok(format!("{name} {number} 1"));
    }
    let live = lives.first().ok_or_else(|| "No Input".to_string())?;
    let source = if name == "InputPreview" {
        live.preview_source
    } else {
        live.program_source
    };
    let number = flat
        .by_source(source)
        .map(|item| item.number)
        .ok_or_else(|| "No Input".to_string())?;
    Ok(format!("{name} {number} 1"))
}

fn is_bool_stub(name: &str) -> bool {
    matches!(
        name,
        "FadeToBlack"
            | "Recording"
            | "Streaming"
            | "External"
            | "Fullscreen"
            | "ReplayPlaying"
            | "MasterAudio"
            | "BusAAudio"
            | "BusBAudio"
            | "BusCAudio"
            | "BusDAudio"
            | "BusEAudio"
            | "BusFAudio"
            | "BusGAudio"
            | "BusASolo"
            | "BusBSolo"
            | "BusCSolo"
            | "BusDSolo"
            | "BusESolo"
            | "BusFSolo"
            | "BusGSolo"
    )
}

fn is_input_bool_stub(name: &str) -> bool {
    matches!(
        name,
        "InputPlaying"
            | "InputAudio"
            | "InputSolo"
            | "InputBusAAudio"
            | "InputBusBAudio"
            | "InputBusCAudio"
            | "InputBusDAudio"
            | "InputBusEAudio"
            | "InputBusFAudio"
            | "InputBusGAudio"
            | "InputMasterAudio"
    )
}

fn mix_from_activator(name: &str) -> Result<(usize, bool), String> {
    match name {
        "Input" => Ok((0, false)),
        "InputPreview" => Ok((0, true)),
        other if other.starts_with("InputPreviewMix") => {
            let n = other
                .trim_start_matches("InputPreviewMix")
                .parse::<usize>()
                .map_err(|_| "unknown activator".to_string())?;
            if n < 2 {
                return Err("unknown activator".into());
            }
            Ok((n - 1, true))
        }
        other if other.starts_with("InputMix") => {
            let n = other
                .trim_start_matches("InputMix")
                .parse::<usize>()
                .map_err(|_| "unknown activator".to_string())?;
            if n < 2 {
                return Err("unknown activator".into());
            }
            Ok((n - 1, false))
        }
        other if other.starts_with("Overlay") => Ok((0, false)),
        _ => Err("unknown activator".into()),
    }
}

fn session_tally() -> Option<(FlatMap, Vec<UnitLive>)> {
    let doc = crate::vmix_api::published_document()?;
    let live = crate::live_snapshot();
    let flat = FlatMap::build(&doc);
    let lives = doc
        .units
        .iter()
        .map(|unit| {
            live.units.get(&unit.id).cloned().unwrap_or(UnitLive {
                program_source: 0,
                preview_source: 0,
                overlay_sources: Vec::new(),
            })
        })
        .collect();
    Some((flat, lives))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_frame_includes_crlf_in_length() {
        let encoded = encode_xml("<vmix></vmix>");
        let text = String::from_utf8(encoded).unwrap();
        assert!(text.starts_with("XML 15\r\n"));
        assert!(text.ends_with("</vmix>\r\n"));
    }

    #[test]
    fn xmltext_reads_version_and_input_title() {
        let xml = r#"<vmix><version>0.2.1-beta.1</version><inputs><input key="a" number="1" title="Scene 1"></input></inputs></vmix>"#;
        assert_eq!(xml_text("vmix/version", xml).unwrap(), "0.2.1-beta.1");
        assert_eq!(
            xml_text("vmix/inputs/input[1]/@title", xml).unwrap(),
            "Scene 1"
        );
    }

    #[test]
    fn function_without_name_is_er() {
        assert_eq!(
            dispatch_line("FUNCTION"),
            Reply::Line("FUNCTION ER Function is required\r\n".into())
        );
    }

    #[test]
    fn subscribe_and_quit_are_recognized() {
        assert_eq!(dispatch_line("SUBSCRIBE TALLY"), Reply::SubscribeTally);
        assert_eq!(dispatch_line("SUBSCRIBE ACTS"), Reply::SubscribeActs);
        assert_eq!(dispatch_line("UNSUBSCRIBE TALLY"), Reply::UnsubscribeTally);
        assert_eq!(dispatch_line("QUIT"), Reply::Quit);
    }

    #[test]
    fn input_attr_path_is_one_based() {
        assert_eq!(
            parse_input_attr_path("vmix/inputs/input[2]/@title"),
            Some((2, "title"))
        );
        assert_eq!(parse_input_attr_path("vmix/inputs/input[0]/@title"), None);
    }

    #[test]
    fn commands_are_case_insensitive() {
        assert_eq!(dispatch_line("tally"), dispatch_line("TALLY"));
        assert_eq!(dispatch_line("version"), dispatch_line("VERSION"));
        assert_eq!(dispatch_line("subscribe tally"), Reply::SubscribeTally);
    }

    #[test]
    fn acts_stubs_do_not_need_a_session() {
        assert_eq!(
            dispatch_line("ACTS FadeToBlack"),
            Reply::Line("ACTS OK FadeToBlack 0\r\n".into())
        );
        assert_eq!(
            dispatch_line("ACTS InputPlaying 1"),
            Reply::Line("ACTS OK InputPlaying 1 0\r\n".into())
        );
    }

    #[test]
    fn program_tally_wins_over_preview() {
        let flat = FlatMap {
            inputs: vec![
                crate::vmix_xml::FlatInput {
                    number: 1,
                    key: "a".into(),
                    title: "A".into(),
                    input_type: "Blank".into(),
                    source_id: 10,
                    is_scene: true,
                    overlays: Vec::new(),
                },
                crate::vmix_xml::FlatInput {
                    number: 2,
                    key: "b".into(),
                    title: "B".into(),
                    input_type: "Blank".into(),
                    source_id: 20,
                    is_scene: true,
                    overlays: Vec::new(),
                },
            ],
        };
        let lives = vec![UnitLive {
            program_source: 10,
            preview_source: 20,
            overlay_sources: Vec::new(),
        }];
        assert_eq!(tally_from(&flat, &lives), "12");
        let both_program = vec![UnitLive {
            program_source: 10,
            preview_source: 10,
            overlay_sources: Vec::new(),
        }];
        assert_eq!(tally_from(&flat, &both_program), "10");
    }

    #[test]
    fn acts_subscribe_sends_only_new_lines() {
        let old = "ACTS OK Input 1 1\nACTS OK Input 2 0";
        let new = "ACTS OK Input 1 0\nACTS OK Input 2 0";
        assert_eq!(changed_lines(old, new), vec!["ACTS OK Input 1 0"]);
        assert_eq!(
            changed_lines("", new),
            vec!["ACTS OK Input 1 0", "ACTS OK Input 2 0"]
        );
    }
}
