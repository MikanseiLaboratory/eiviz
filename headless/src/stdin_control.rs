//! Headless stdin uses the same Clap surface as eivizctl over a loopback WebSocket.

use std::io::{BufRead, BufReader};
use std::net::SocketAddr;

use eiviz_api::client::ControlSession;
use tokio::sync::mpsc;

use crate::ctl::{self, parse_line};

pub fn loopback_ws_url(bind: SocketAddr) -> String {
    let port = bind.port();
    if bind.is_ipv6() {
        let host = if bind.ip().is_unspecified() {
            "::1".to_string()
        } else {
            bind.ip().to_string()
        };
        format!("ws://[{host}]:{port}")
    } else {
        let host = if bind.ip().is_unspecified() {
            "127.0.0.1".to_string()
        } else {
            bind.ip().to_string()
        };
        format!("ws://{host}:{port}")
    }
}

/// Dedicated OS thread so Tokio shutdown is never blocked on stdin.
pub fn spawn_lines() -> mpsc::UnboundedReceiver<String> {
    let (tx, rx) = mpsc::unbounded_channel();
    let _ = std::thread::Builder::new()
        .name("eiviz-stdin".into())
        .spawn(move || {
            let reader = BufReader::new(std::io::stdin());
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    rx
}

pub async fn handle_line(session: &ControlSession, line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    match parse_line(line) {
        Ok(cmd) => {
            if let Some(reason) = ctl::stdin_unsupported(&cmd) {
                eprintln!("eiviz-headless error=stdin {reason}");
                return;
            }
            if let Err(error) = ctl::run_cmd(session, cmd, false).await {
                eprintln!("eiviz-headless error=stdin {error}");
            }
        }
        Err(error) => eprintln!("eiviz-headless error=stdin {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    #[test]
    fn unspecified_v4_uses_loopback() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 9400);
        assert_eq!(loopback_ws_url(addr), "ws://127.0.0.1:9400");
    }

    #[test]
    fn unspecified_v6_uses_loopback() {
        let addr = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 9400);
        assert_eq!(loopback_ws_url(addr), "ws://[::1]:9400");
    }
}
