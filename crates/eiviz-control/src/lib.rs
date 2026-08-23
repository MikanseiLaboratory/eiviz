//! Control adapters convert wire/HID events into CommandEnvelope only.

use eiviz_command::{Command, CommandEnvelope};
use eiviz_engine::Engine;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use tiny_http::{Header, Method, Response, Server};

#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    #[error("{0}")]
    Other(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireCommand {
    pub token: Option<String>,
    pub command: Command,
}

pub struct ControlConfig {
    pub bind: String,
    pub require_token: Option<String>,
}

impl Default for ControlConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:0".into(),
            require_token: None,
        }
    }
}

pub fn spawn_http(
    engine: Arc<Engine>,
    cfg: ControlConfig,
    stop: Arc<AtomicBool>,
) -> std::io::Result<u16> {
    let server = Server::http(&cfg.bind).map_err(|e| std::io::Error::other(e.to_string()))?;
    let port = server.server_addr().to_ip().unwrap().port();
    thread::spawn(move || {
        for mut req in server.incoming_requests() {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let url = req.url().to_string();
            let method = req.method().clone();
            let mut body = String::new();
            let _ = req.as_reader().read_to_string(&mut body);
            let resp = handle(&engine, &cfg, &method, &url, &body);
            let _ = req.respond(resp);
        }
    });
    Ok(port)
}

fn handle(
    engine: &Engine,
    cfg: &ControlConfig,
    method: &Method,
    url: &str,
    body: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let cors = Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap();
    if url == "/v1/health" {
        return Response::from_string("ok").with_header(cors);
    }
    if url == "/v1/project" && *method == Method::Get {
        let json = serde_json::to_string(&engine.snapshot()).unwrap_or_else(|_| "{}".into());
        return Response::from_string(json).with_header(cors);
    }
    if url == "/v1/command" && *method == Method::Post {
        let parsed: Result<WireCommand, _> = serde_json::from_str(body);
        match parsed {
            Ok(wire) => {
                if let Some(need) = &cfg.require_token {
                    if wire.token.as_deref() != Some(need.as_str()) {
                        return Response::from_string("unauthorized")
                            .with_status_code(401)
                            .with_header(cors);
                    }
                }
                match engine.submit(CommandEnvelope::new(engine.client(), wire.command)) {
                    Ok(ack) => {
                        let json = serde_json::to_string(&ack).unwrap();
                        Response::from_string(json).with_header(cors)
                    }
                    Err(e) => Response::from_string(e.to_string())
                        .with_status_code(400)
                        .with_header(cors),
                }
            }
            Err(e) => Response::from_string(e.to_string())
                .with_status_code(400)
                .with_header(cors),
        }
    } else {
        Response::from_string("not found")
            .with_status_code(404)
            .with_header(cors)
    }
}

pub fn spawn_tcp(engine: Arc<Engine>, bind: &str, stop: Arc<AtomicBool>) -> std::io::Result<u16> {
    let listener = TcpListener::bind(bind)?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    thread::spawn(move || {
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    let engine = engine.clone();
                    thread::spawn(move || {
                        let _ = serve_tcp(engine, stream);
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });
    Ok(port)
}

fn serve_tcp(engine: Arc<Engine>, mut stream: TcpStream) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let n = match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        buf.extend_from_slice(&tmp[..n]);
        while let Some(idx) = buf.iter().position(|b| *b == b'\n') {
            let line = buf.drain(..=idx).collect::<Vec<_>>();
            let line = String::from_utf8_lossy(&line);
            if let Ok(cmd) = serde_json::from_str::<Command>(line.trim()) {
                let _ = engine.submit(CommandEnvelope::new(engine.client(), cmd));
                let hash = engine.state_hash();
                let _ = stream.write_all(format!("{hash}\n").as_bytes());
            }
        }
    }
    Ok(())
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
    use std::io::Write;

    #[test]
    fn tcp_json_line_applies_command() {
        let engine = Engine::new("ctl").shared();
        let stop = Arc::new(AtomicBool::new(false));
        let port = spawn_tcp(engine.clone(), "127.0.0.1:0", stop.clone()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let body = serde_json::to_string(&Command::SetName {
            name: "renamed".into(),
        })
        .unwrap();
        s.write_all(body.as_bytes()).unwrap();
        s.write_all(b"\n").unwrap();
        s.flush().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(80));
        assert_eq!(engine.snapshot().name, "renamed");
        stop.store(true, Ordering::Relaxed);
    }
}
