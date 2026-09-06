//! Protobuf WebSocket control API (`eiviz.protobuf.v1`). Loopback, default port 9400.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use eiviz_api::auth::AuthConfig;
use eiviz_api::server::{ServerConfig, listen};

use crate::MixerFacade;
use crate::abi::{ERR_INVALID_ARGUMENT, ERR_IO, OK};

struct WsState {
    stop: Option<tokio::sync::watch::Sender<bool>>,
    join: Option<JoinHandle<()>>,
    listen_owner: Option<String>,
}

fn ws_slot() -> &'static Mutex<WsState> {
    static SLOT: OnceLock<Mutex<WsState>> = OnceLock::new();
    SLOT.get_or_init(|| {
        Mutex::new(WsState {
            stop: None,
            join: None,
            listen_owner: None,
        })
    })
}

pub fn configure(enabled: bool, port: u32) -> i32 {
    stop_worker();
    if !enabled {
        crate::diag::http_info("ws disabled");
        return OK;
    }
    if port == 0 || port > u32::from(u16::MAX) {
        return ERR_INVALID_ARGUMENT;
    }
    let port = port as u16;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("eiviz-ws-rt")
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            crate::diag::http_error(&format!("ws runtime: {error}"));
            return ERR_IO;
        }
    };
    let config = ServerConfig {
        bind: addr,
        auth: AuthConfig::from_env(),
        idle_timeout: Duration::from_secs(60),
        max_clients: 32,
    };
    let control = Arc::new(MixerFacade);
    let started = runtime.block_on(listen(config, control));
    let (bind, task) = match started {
        Ok(pair) => pair,
        Err(error) => {
            let owner = crate::tcp_listen_owner::name(port);
            match owner.as_deref() {
                Some(name) => {
                    crate::diag::http_error(&format!("ws listen {addr}: {error} ({name})"))
                }
                None => crate::diag::http_error(&format!("ws listen {addr}: {error}")),
            }
            if let Ok(mut slot) = ws_slot().lock() {
                slot.listen_owner = owner;
            }
            return ERR_IO;
        }
    };
    crate::diag::http_info(&format!("ws listen {}", bind.ws_addr));
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    let Ok(mut slot) = ws_slot().lock() else {
        task.abort();
        return ERR_INVALID_ARGUMENT;
    };
    slot.listen_owner = None;
    match thread::Builder::new()
        .name("eiviz-native-ws".into())
        .spawn(move || {
            runtime.block_on(async move {
                tokio::select! {
                    result = task => {
                        if let Err(error) = result {
                            crate::diag::http_error(&format!("ws accept: {error}"));
                        }
                    }
                    _ = stop_rx.changed() => {}
                }
            });
            runtime.shutdown_background();
        }) {
        Ok(join) => {
            slot.stop = Some(stop_tx);
            slot.join = Some(join);
            OK
        }
        Err(error) => {
            crate::diag::http_error(&format!("ws spawn: {error}"));
            ERR_IO
        }
    }
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
    if let Some(stop) = slot.stop.take() {
        let _ = stop.send(true);
    }
    if let Some(join) = slot.join.take() {
        crate::diag::join_timeout(join, Duration::from_secs(2), "native-ws");
    }
    slot.listen_owner = None;
}
