//! Protobuf WebSocket control API (`eiviz.protobuf.v1`). Bind address, token,
//! max role, and media directory are host-owned. `ws://` is for a trusted LAN/VPN.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use eiviz_api::auth::{AuthConfig, Role};
use eiviz_api::server::{ServerConfig, listen};

use crate::MixerFacade;
use crate::abi::{ERR_INVALID_ARGUMENT, ERR_IO, OK};

/// How often a bind refused with "address in use" is tried again, and the wait between
/// tries. A listener that is still closing - this process starting a second time, or a
/// port changed in the same process - gives the address back within a moment. Waiting
/// here costs a fraction of a second; making the operator restart the app to recover an
/// API that was never broken costs a lot more.
const LISTEN_ATTEMPTS: u32 = 4;
const LISTEN_RETRY: Duration = Duration::from_millis(150);

/// Everything the running worker was started with. Two calls that ask for the same
/// listener describe the same server, so the second one has nothing to do.
#[derive(Clone, PartialEq, Eq)]
struct WsConfig {
    host: String,
    port: u16,
    token: String,
    max_role: String,
    media_directory: String,
}

struct WsState {
    stop: Option<tokio::sync::watch::Sender<bool>>,
    join: Option<JoinHandle<()>>,
    listen_owner: Option<String>,
    /// The configuration of the worker in `join`. `None` while nothing is listening.
    running: Option<WsConfig>,
}

fn ws_slot() -> &'static Mutex<WsState> {
    static SLOT: OnceLock<Mutex<WsState>> = OnceLock::new();
    SLOT.get_or_init(|| {
        Mutex::new(WsState {
            stop: None,
            join: None,
            listen_owner: None,
            running: None,
        })
    })
}

pub fn configure(enabled: bool, port: u32) -> i32 {
    configure_bind(enabled, "127.0.0.1", port)
}

pub fn configure_bind(enabled: bool, host: &str, port: u32) -> i32 {
    configure_owned(enabled, host, port, "", "", "")
}

pub fn configure_owned(
    enabled: bool,
    host: &str,
    port: u32,
    token: &str,
    max_role: &str,
    media_directory: &str,
) -> i32 {
    if !enabled {
        return disable();
    }
    if port == 0 || port > u32::from(u16::MAX) {
        return ERR_INVALID_ARGUMENT;
    }
    let ip: IpAddr = match host.parse() {
        Ok(ip) => ip,
        Err(_) => return ERR_INVALID_ARGUMENT,
    };
    let port = port as u16;
    let config = WsConfig {
        host: host.to_string(),
        port,
        token: token.to_string(),
        max_role: max_role.to_string(),
        media_directory: media_directory.to_string(),
    };
    // Every check runs before the listener is touched, so a request the core refuses
    // cannot take down an API that is serving clients right now.
    match ws_slot().lock() {
        Ok(slot) if slot.running.as_ref() == Some(&config) => return OK,
        Ok(_) => {}
        Err(_) => return ERR_INVALID_ARGUMENT,
    }
    let mut auth = AuthConfig::from_env();
    if !token.is_empty() {
        auth.token = token.to_string();
        auth.require_auth = true;
    }
    if !max_role.is_empty() {
        auth.max_role = Role::from_name(max_role);
    }
    if !ip.is_loopback() && !auth.require_auth {
        crate::diag::http_error("ws remote bind requires authentication");
        return ERR_INVALID_ARGUMENT;
    }
    stop_worker();
    let addr = SocketAddr::new(ip, port);
    let media_path = eiviz_api::resolve_media_directory(media_directory);
    let media = eiviz_api::FileMediaStorage::new(eiviz_api::MediaStorageConfig::new(media_path))
        .ok()
        .map(|store| Arc::new(store) as Arc<dyn eiviz_api::MediaStorage>);
    let control = Arc::new(MixerFacade);
    let mut attempt = 0;
    while attempt < LISTEN_ATTEMPTS {
        attempt += 1;
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        // Build the Tokio runtime on this worker. `eiviz-headless run` already
        // sits inside a runtime, and nested `Runtime::new` panics.
        let join = match thread::Builder::new()
            .name("eiviz-native-ws".into())
            .spawn({
                let config = ServerConfig::new(addr, auth.clone(), media.clone());
                let control = Arc::clone(&control);
                move || ws_worker(config, control, ready_tx, stop_rx)
            }) {
            Ok(join) => join,
            Err(error) => {
                crate::diag::http_error(&format!("ws spawn: {error}"));
                return ERR_IO;
            }
        };
        match ready_rx.recv() {
            Ok(Ok(bound)) => {
                crate::diag::http_info(&format!("ws listen {bound}"));
                let Ok(mut slot) = ws_slot().lock() else {
                    let _ = stop_tx.send(true);
                    crate::diag::join_timeout(join, Duration::from_secs(2), "native-ws");
                    return ERR_INVALID_ARGUMENT;
                };
                slot.listen_owner = None;
                slot.stop = Some(stop_tx);
                slot.join = Some(join);
                slot.running = Some(config);
                return OK;
            }
            Ok(Err((kind, error))) => {
                crate::diag::join_timeout(join, Duration::from_secs(2), "native-ws");
                if kind == std::io::ErrorKind::AddrInUse && attempt < LISTEN_ATTEMPTS {
                    thread::sleep(LISTEN_RETRY);
                    continue;
                }
                report_listen_error(addr, port, &error);
                return ERR_IO;
            }
            Err(_) => {
                crate::diag::join_timeout(join, Duration::from_secs(2), "native-ws");
                crate::diag::http_error(&format!("ws listen {addr}: worker exited"));
                return ERR_IO;
            }
        }
    }
    ERR_IO
}

fn ws_worker(
    config: ServerConfig,
    control: Arc<MixerFacade>,
    ready_tx: std::sync::mpsc::SyncSender<Result<SocketAddr, (std::io::ErrorKind, String)>>,
    mut stop_rx: tokio::sync::watch::Receiver<bool>,
) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("eiviz-ws-rt")
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready_tx.send(Err((
                std::io::ErrorKind::Other,
                format!("ws runtime: {error}"),
            )));
            return;
        }
    };
    let started = runtime.block_on(listen(config, control));
    let handle = match started {
        Ok(handle) => handle,
        Err(error) => {
            let _ = ready_tx.send(Err((error.kind(), error.to_string())));
            runtime.shutdown_background();
            return;
        }
    };
    if ready_tx.send(Ok(handle.bind.ws_addr)).is_err() {
        runtime.block_on(handle.shutdown()).ok();
        runtime.shutdown_background();
        return;
    }
    runtime.block_on(async move {
        let _ = stop_rx.changed().await;
        if let Err(error) = handle.shutdown().await {
            crate::diag::http_error(&format!("ws accept: {error}"));
        }
    });
    runtime.shutdown_background();
}

fn report_listen_error(addr: SocketAddr, port: u16, error: &str) {
    let owner = crate::tcp_listen_owner::name(port);
    match owner.as_deref() {
        Some(name) => crate::diag::http_error(&format!("ws listen {addr}: {error} ({name})")),
        None => crate::diag::http_error(&format!("ws listen {addr}: {error}")),
    }
    if let Ok(mut slot) = ws_slot().lock() {
        slot.listen_owner = owner;
    }
}

/// Turn the listener off. Asking for an API that is already off is not an error and,
/// more importantly, not a restart either.
fn disable() -> i32 {
    let running = ws_slot()
        .lock()
        .map(|slot| slot.running.is_some())
        .unwrap_or(false);
    if !running {
        return OK;
    }
    stop_worker();
    crate::diag::http_info("ws disabled");
    OK
}

pub fn listen_owner() -> Option<String> {
    ws_slot()
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
    let Ok(mut slot) = ws_slot().lock() else {
        return;
    };
    slot.running = None;
    if let Some(stop) = slot.stop.take() {
        let _ = stop.send(true);
    }
    if let Some(join) = slot.join.take() {
        crate::diag::join_timeout(join, Duration::from_secs(2), "native-ws");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_api::client::{ControlClient, ControlSession};
    use eiviz_control::ControlError;
    use serial_test::serial;
    use std::net::TcpListener;

    fn free_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("ephemeral port");
        listener.local_addr().expect("bound address").port()
    }

    fn snapshot_error(
        runtime: &tokio::runtime::Runtime,
        session: &mut ControlSession,
    ) -> Option<ControlError> {
        runtime.block_on(async {
            match tokio::time::timeout(Duration::from_secs(5), session.snapshot()).await {
                Ok(result) => result.err(),
                Err(_) => Some(ControlError::io("snapshot timed out")),
            }
        })
    }

    /// Applying the configuration that is already running must leave the listener -
    /// and the clients on it - alone.
    #[test]
    #[serial]
    fn repeated_configuration_keeps_the_open_session() {
        let port = free_port();
        let url = format!("ws://127.0.0.1:{port}");
        assert_eq!(
            configure_owned(true, "127.0.0.1", u32::from(port), "secret", "admin", ""),
            OK
        );
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let client = ControlClient::websocket(&url, "secret");
        let mut session = runtime.block_on(client.connect()).expect("connect");
        // Without a mixer the service answers "not ready". Only the fact that an answer
        // arrives over the connection the server already accepted matters here.
        let before = snapshot_error(&runtime, &mut session);
        assert!(
            before.is_some(),
            "the stand-in server must answer without a mixer"
        );
        assert_eq!(
            configure_owned(true, "127.0.0.1", u32::from(port), "secret", "admin", ""),
            OK
        );
        let after = snapshot_error(&runtime, &mut session);
        assert_eq!(
            after, before,
            "re-applying the same WebSocket configuration dropped the open session"
        );
        assert_eq!(configure_owned(false, "127.0.0.1", 0, "", "", ""), OK);
    }

    /// A request the core refuses must not disturb the listener that is running.
    #[test]
    #[serial]
    fn refused_configuration_keeps_the_listener_bound() {
        let port = free_port();
        assert_eq!(
            configure_owned(true, "127.0.0.1", u32::from(port), "", "admin", ""),
            OK
        );
        assert_eq!(
            configure_owned(true, "127.0.0.1", 0, "", "admin", ""),
            ERR_INVALID_ARGUMENT
        );
        assert!(
            TcpListener::bind(("127.0.0.1", port)).is_err(),
            "a refused configuration released the port that was serving"
        );
        assert_eq!(configure_owned(false, "127.0.0.1", 0, "", "", ""), OK);
        assert!(
            TcpListener::bind(("127.0.0.1", port)).is_ok(),
            "disabling the API left the port bound"
        );
    }
}
