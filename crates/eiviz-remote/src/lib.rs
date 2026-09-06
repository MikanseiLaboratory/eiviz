//! Persistent Protobuf client as its own cdylib so hosts can load GPU mixer
//! and remote control independently.
//!
//! # Safety
//! Pointer arguments follow the mixer C ABI: UTF-8 C strings may be null (treated
//! as empty), output buffers must be writable for `cap` bytes, and JSON blobs must
//! cover `len` bytes.

#![allow(clippy::missing_safety_doc)]

mod abi;

use std::collections::HashMap;
use std::ffi::{CStr, c_char};
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use eiviz_api::ControlClient;
use eiviz_api::client::{ControlSession, SessionView};

use crate::abi::{ERR_INVALID_ARGUMENT, ERR_IO, ERR_NOT_CREATED, OK};

struct RemoteSlot {
    handle: tokio::runtime::Handle,
    session: Arc<ControlSession>,
    stop: tokio::sync::watch::Sender<bool>,
    join: Option<JoinHandle<()>>,
}

fn slots() -> &'static Mutex<HashMap<i32, RemoteSlot>> {
    static SLOTS: OnceLock<Mutex<HashMap<i32, RemoteSlot>>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_id() -> i32 {
    static NEXT: AtomicI32 = AtomicI32::new(1);
    NEXT.fetch_add(1, Ordering::SeqCst)
}

pub unsafe fn open(url: *const c_char, token: *const c_char) -> i32 {
    let url = unsafe { read_cstr(url) }.unwrap_or_default();
    let token = unsafe { read_cstr(token) }.unwrap_or_default();
    if url.is_empty() {
        return -ERR_INVALID_ARGUMENT;
    }
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let join = match thread::Builder::new()
        .name("eiviz-remote".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .thread_name("eiviz-remote-rt")
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = ready_tx.send(Err(error.to_string()));
                    return;
                }
            };
            let handle = runtime.handle().clone();
            runtime.block_on(async move {
                let client = ControlClient::websocket(url, token);
                let session = match client.connect().await {
                    Ok(session) => session,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                if let Err(error) = session.subscribe(0).await {
                    let _ = ready_tx.send(Err(error.to_string()));
                    return;
                }
                let _ = session.snapshot().await;
                let _ = ready_tx.send(Ok((handle, Arc::new(session))));
                let _ = stop_rx.changed().await;
            });
        }) {
        Ok(join) => join,
        Err(_) => return -ERR_IO,
    };
    match ready_rx.recv_timeout(Duration::from_secs(15)) {
        Ok(Ok((handle, session))) => {
            let id = next_id();
            if let Ok(mut map) = slots().lock() {
                map.insert(
                    id,
                    RemoteSlot {
                        handle,
                        session,
                        stop: stop_tx,
                        join: Some(join),
                    },
                );
            }
            id
        }
        _ => {
            let _ = stop_tx.send(true);
            let _ = join.join();
            -ERR_IO
        }
    }
}

pub fn close(handle: i32) -> i32 {
    let Some(mut slot) = slots().lock().ok().and_then(|mut map| map.remove(&handle)) else {
        return ERR_NOT_CREATED;
    };
    let _ = slot.stop.send(true);
    if let Some(join) = slot.join.take() {
        join_timeout(join, Duration::from_secs(2));
    }
    OK
}

fn with_slot<T>(handle: i32, f: impl FnOnce(&RemoteSlot) -> T) -> Option<T> {
    let map = slots().lock().ok()?;
    map.get(&handle).map(f)
}

fn view(handle: i32) -> Option<SessionView> {
    with_slot(handle, |slot| slot.session.view())
}

pub unsafe fn copy_snapshot(handle: i32, out: *mut u8, cap: usize) -> i32 {
    copy_bytes(handle, out, cap, |view| view.document_json.clone())
}

pub unsafe fn copy_live(handle: i32, out: *mut u8, cap: usize) -> i32 {
    copy_bytes(handle, out, cap, |view| view.live_json.clone())
}

pub unsafe fn copy_status(handle: i32, out: *mut u8, cap: usize) -> i32 {
    copy_bytes(handle, out, cap, |view| {
        serde_json::to_vec(&serde_json::json!({
            "connected": view.connected,
            "epoch": view.epoch,
            "revision": view.revision,
            "sequence": view.sequence,
            "error": view.error,
            "lag": view.lag,
        }))
        .unwrap_or_default()
    })
}

fn copy_bytes(handle: i32, out: *mut u8, cap: usize, f: impl Fn(&SessionView) -> Vec<u8>) -> i32 {
    if out.is_null() || cap == 0 {
        return -ERR_INVALID_ARGUMENT;
    }
    let bytes = match view(handle) {
        Some(view) => f(&view),
        None => return -ERR_NOT_CREATED,
    };
    if bytes.len() > cap {
        return -1;
    }
    if !bytes.is_empty() {
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len()) };
    }
    bytes.len() as i32
}

fn map_result(result: eiviz_control::ControlResult<()>) -> i32 {
    match result {
        Ok(()) => OK,
        Err(error) => -error.to_abi(),
    }
}

fn run<T>(handle: i32, fut: impl std::future::Future<Output = T>) -> Option<T> {
    let rt = with_slot(handle, |slot| slot.handle.clone())?;
    Some(rt.block_on(fut))
}

pub fn cut(handle: i32, unit_id: u64, swap: u32) -> i32 {
    let Some(session) = with_slot(handle, |slot| Arc::clone(&slot.session)) else {
        return ERR_NOT_CREATED;
    };
    run(handle, async move {
        map_result(session.cut(unit_id, swap != 0).await)
    })
    .unwrap_or(ERR_NOT_CREATED)
}

pub fn preview(handle: i32, unit_id: u64, scene_id: u64) -> i32 {
    let Some(session) = with_slot(handle, |slot| Arc::clone(&slot.session)) else {
        return ERR_NOT_CREATED;
    };
    run(handle, async move {
        map_result(session.preview(unit_id, scene_id).await)
    })
    .unwrap_or(ERR_NOT_CREATED)
}

#[allow(clippy::too_many_arguments)]
pub fn auto(
    handle: i32,
    unit_id: u64,
    kind: u32,
    duration_ms: u32,
    swap: u32,
    keep_preview: u32,
    easing: u32,
    direction: u32,
    dip_r: f32,
    dip_g: f32,
    dip_b: f32,
    dip_a: f32,
    softness: f32,
    param: f32,
) -> i32 {
    let Some(session) = with_slot(handle, |slot| Arc::clone(&slot.session)) else {
        return ERR_NOT_CREATED;
    };
    run(handle, async move {
        map_result(
            session
                .auto_full(
                    unit_id,
                    kind,
                    duration_ms,
                    swap != 0,
                    keep_preview != 0,
                    easing,
                    direction,
                    dip_r,
                    dip_g,
                    dip_b,
                    dip_a,
                    softness,
                    param,
                )
                .await,
        )
    })
    .unwrap_or(ERR_NOT_CREATED)
}

pub fn set_mix(handle: i32, unit_id: u64, value: f32) -> i32 {
    let Some(session) = with_slot(handle, |slot| Arc::clone(&slot.session)) else {
        return ERR_NOT_CREATED;
    };
    run(handle, async move {
        map_result(session.set_mix(unit_id, value).await)
    })
    .unwrap_or(ERR_NOT_CREATED)
}

pub fn overlay_auto(handle: i32, unit_id: u64, index: u32, duration_ms: u32, to_on: u32) -> i32 {
    let Some(session) = with_slot(handle, |slot| Arc::clone(&slot.session)) else {
        return ERR_NOT_CREATED;
    };
    run(handle, async move {
        map_result(
            session
                .overlay_auto(unit_id, index, duration_ms, to_on != 0)
                .await,
        )
    })
    .unwrap_or(ERR_NOT_CREATED)
}

pub unsafe fn mutate(handle: i32, json: *const u8, len: usize, expected_revision: u64) -> i32 {
    if json.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let bytes = unsafe { std::slice::from_raw_parts(json, len) }.to_vec();
    let Some(session) = with_slot(handle, |slot| Arc::clone(&slot.session)) else {
        return ERR_NOT_CREATED;
    };
    run(handle, async move {
        map_result(session.mutate_session(bytes, expected_revision).await)
    })
    .unwrap_or(ERR_NOT_CREATED)
}

pub unsafe fn replace(handle: i32, json: *const u8, len: usize, expected_revision: u64) -> i32 {
    if json.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let bytes = unsafe { std::slice::from_raw_parts(json, len) }.to_vec();
    let Some(session) = with_slot(handle, |slot| Arc::clone(&slot.session)) else {
        return ERR_NOT_CREATED;
    };
    run(handle, async move {
        map_result(session.replace_session(bytes, expected_revision).await)
    })
    .unwrap_or(ERR_NOT_CREATED)
}

pub fn video_play(handle: i32, input_id: u64, playing: u32) -> i32 {
    let Some(session) = with_slot(handle, |slot| Arc::clone(&slot.session)) else {
        return ERR_NOT_CREATED;
    };
    run(handle, async move {
        map_result(session.video_play(input_id, playing != 0).await)
    })
    .unwrap_or(ERR_NOT_CREATED)
}

pub fn video_loop(handle: i32, input_id: u64, looping: u32) -> i32 {
    let Some(session) = with_slot(handle, |slot| Arc::clone(&slot.session)) else {
        return ERR_NOT_CREATED;
    };
    run(handle, async move {
        map_result(session.video_loop(input_id, looping != 0).await)
    })
    .unwrap_or(ERR_NOT_CREATED)
}

pub fn video_seek(handle: i32, input_id: u64, position_hns: i64) -> i32 {
    let Some(session) = with_slot(handle, |slot| Arc::clone(&slot.session)) else {
        return ERR_NOT_CREATED;
    };
    run(handle, async move {
        map_result(session.video_seek(input_id, position_hns).await)
    })
    .unwrap_or(ERR_NOT_CREATED)
}

pub unsafe fn upload(
    handle: i32,
    path: *const c_char,
    kind: *const c_char,
    name: *const c_char,
    video_loop: u32,
    expected_revision: u64,
) -> i32 {
    let path = unsafe { read_cstr(path) }.unwrap_or_default();
    let kind = unsafe { read_cstr(kind) }.unwrap_or_default();
    let name = unsafe { read_cstr(name) }.unwrap_or_default();
    if path.is_empty() || kind.is_empty() {
        return ERR_INVALID_ARGUMENT;
    }
    let Some(session) = with_slot(handle, |slot| Arc::clone(&slot.session)) else {
        return ERR_NOT_CREATED;
    };
    run(handle, async move {
        map_result(
            session
                .upload_file(
                    Path::new(&path),
                    &kind,
                    &name,
                    video_loop != 0,
                    expected_revision,
                )
                .await,
        )
    })
    .unwrap_or(ERR_NOT_CREATED)
}

unsafe fn read_cstr(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return Some(String::new());
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .map(|value| value.to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_remote_open(url: *const c_char, token: *const c_char) -> i32 {
    unsafe { open(url, token) }
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_remote_close(handle: i32) -> i32 {
    close(handle)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_remote_copy_snapshot(handle: i32, out: *mut u8, cap: usize) -> i32 {
    unsafe { copy_snapshot(handle, out, cap) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_remote_copy_live(handle: i32, out: *mut u8, cap: usize) -> i32 {
    unsafe { copy_live(handle, out, cap) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_remote_copy_status(handle: i32, out: *mut u8, cap: usize) -> i32 {
    unsafe { copy_status(handle, out, cap) }
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_remote_cut(handle: i32, unit_id: u64, swap: u32) -> i32 {
    cut(handle, unit_id, swap)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_remote_preview(handle: i32, unit_id: u64, scene_id: u64) -> i32 {
    preview(handle, unit_id, scene_id)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_remote_auto(
    handle: i32,
    unit_id: u64,
    kind: u32,
    duration_ms: u32,
    swap: u32,
    keep_preview: u32,
    easing: u32,
    direction: u32,
    dip_r: f32,
    dip_g: f32,
    dip_b: f32,
    dip_a: f32,
    softness: f32,
    param: f32,
) -> i32 {
    auto(
        handle,
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
    )
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_remote_set_mix(handle: i32, unit_id: u64, value: f32) -> i32 {
    set_mix(handle, unit_id, value)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_remote_overlay_auto(
    handle: i32,
    unit_id: u64,
    index: u32,
    duration_ms: u32,
    to_on: u32,
) -> i32 {
    overlay_auto(handle, unit_id, index, duration_ms, to_on)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_remote_mutate(
    handle: i32,
    json: *const u8,
    len: usize,
    expected_revision: u64,
) -> i32 {
    unsafe { mutate(handle, json, len, expected_revision) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_remote_replace(
    handle: i32,
    json: *const u8,
    len: usize,
    expected_revision: u64,
) -> i32 {
    unsafe { replace(handle, json, len, expected_revision) }
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_remote_video_play(handle: i32, input_id: u64, playing: u32) -> i32 {
    video_play(handle, input_id, playing)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_remote_video_loop(handle: i32, input_id: u64, looping: u32) -> i32 {
    video_loop(handle, input_id, looping)
}

#[unsafe(no_mangle)]
pub extern "C" fn mixer_remote_video_seek(handle: i32, input_id: u64, position_hns: i64) -> i32 {
    video_seek(handle, input_id, position_hns)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_remote_upload(
    handle: i32,
    path: *const c_char,
    kind: *const c_char,
    name: *const c_char,
    video_loop: u32,
    expected_revision: u64,
) -> i32 {
    unsafe { upload(handle, path, kind, name, video_loop, expected_revision) }
}

fn join_timeout(handle: JoinHandle<()>, timeout: Duration) {
    let (tx, rx) = mpsc::channel();
    if thread::Builder::new()
        .name("eiviz-remote-join".into())
        .spawn(move || {
            let _ = handle.join();
            let _ = tx.send(());
        })
        .is_err()
    {
        return;
    }
    let _ = rx.recv_timeout(timeout);
}
