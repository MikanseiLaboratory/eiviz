use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::upload::AudioInputStore;

use super::graph::{DEVICE_ASIO, DEVICE_COREAUDIO, DEVICE_WASAPI};
use super::info::CAPTURE_MODE_PROCESS_LOOPBACK;

#[derive(Clone, Debug)]
pub struct AudioCaptureSpec {
    pub id: u64,
    pub kind: u32,
    pub device_id: String,
    pub mode: u32,
    pub map_left: i32,
    pub map_right: i32,
    pub process_exe: String,
    pub process_aumid: String,
}

struct CaptureHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            crate::diag::join_timeout(join, Duration::from_secs(2), "audio-cap");
        }
    }
}

#[derive(Default)]
pub struct AudioCaptureStore {
    captures: HashMap<u64, CaptureHandle>,
}

impl AudioCaptureStore {
    pub fn start(
        &mut self,
        spec: AudioCaptureSpec,
        uploads: Arc<Mutex<AudioInputStore>>,
    ) -> Result<(), String> {
        self.stop(spec.id);
        #[cfg(windows)]
        if spec.kind == DEVICE_ASIO {
            super::asio::start_capture(&spec, uploads)?;
            self.captures.insert(
                spec.id,
                CaptureHandle {
                    stop: Arc::new(AtomicBool::new(false)),
                    join: None,
                },
            );
            return Ok(());
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop_t = Arc::clone(&stop);
        let id = spec.id;
        let (ready_tx, ready_rx) = mpsc::channel();
        let join = thread::Builder::new()
            .name(format!("eiviz-acap-{id}"))
            .spawn(move || run_capture(spec, uploads, stop_t, Some(ready_tx)))
            .map_err(|error| error.to_string())?;
        match ready_rx.recv_timeout(Duration::from_secs(8)) {
            Ok(Ok(())) => {
                self.captures.insert(
                    id,
                    CaptureHandle {
                        stop,
                        join: Some(join),
                    },
                );
                Ok(())
            }
            Ok(Err(error)) => {
                stop.store(true, Ordering::Relaxed);
                crate::diag::join_timeout(join, Duration::from_secs(2), "audio-cap");
                Err(error)
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                crate::diag::join_timeout(join, Duration::from_secs(2), "audio-cap");
                Err("audio capture start timed out".into())
            }
        }
    }

    pub fn stop(&mut self, id: u64) {
        #[cfg(windows)]
        super::asio::stop_capture(id);
        self.captures.remove(&id);
    }

    pub fn stop_all(&mut self) {
        let ids: Vec<u64> = self.captures.keys().copied().collect();
        for id in ids {
            self.stop(id);
        }
        self.captures.clear();
    }
}

fn run_capture(
    spec: AudioCaptureSpec,
    uploads: Arc<Mutex<AudioInputStore>>,
    stop: Arc<AtomicBool>,
    ready: Option<mpsc::Sender<Result<(), String>>>,
) {
    let signaled = AtomicBool::new(false);
    let result = run_capture_inner(&spec, &uploads, &stop, ready.as_ref(), &signaled);
    if !signaled.swap(true, Ordering::Relaxed) {
        if let Some(tx) = ready {
            let _ = tx.send(result.clone());
        }
    }
    if let Err(error) = result {
        crate::diag::error(&format!("audio capture {}: {error}", spec.id));
    }
}

pub(super) fn send_ready(
    ready: Option<&mpsc::Sender<Result<(), String>>>,
    signaled: &AtomicBool,
    result: Result<(), String>,
) -> Result<(), String> {
    if !signaled.swap(true, Ordering::Relaxed) {
        if let Some(tx) = ready {
            let _ = tx.send(result.clone());
        }
    }
    result
}

fn run_capture_inner(
    spec: &AudioCaptureSpec,
    uploads: &Arc<Mutex<AudioInputStore>>,
    stop: &AtomicBool,
    ready: Option<&mpsc::Sender<Result<(), String>>>,
    signaled: &AtomicBool,
) -> Result<(), String> {
    match spec.kind {
        0 | DEVICE_WASAPI => {
            #[cfg(windows)]
            {
                return wasapi_capture(spec, uploads, stop, ready, signaled);
            }
            #[cfg(not(windows))]
            {
                return Err("WASAPI capture is only available on Windows".into());
            }
        }
        DEVICE_ASIO => Err("ASIO capture is not implemented".into()),
        DEVICE_COREAUDIO => {
            #[cfg(target_os = "macos")]
            {
                return coreaudio_capture(spec, uploads, stop, ready, signaled);
            }
            #[cfg(not(target_os = "macos"))]
            {
                Err("Core Audio capture is only available on macOS".into())
            }
        }
        other => Err(format!("unknown audio capture backend {other}")),
    }
}

#[cfg(windows)]
fn wasapi_capture(
    spec: &AudioCaptureSpec,
    uploads: &Arc<Mutex<AudioInputStore>>,
    stop: &AtomicBool,
    ready: Option<&mpsc::Sender<Result<(), String>>>,
    signaled: &AtomicBool,
) -> Result<(), String> {
    if spec.mode == CAPTURE_MODE_PROCESS_LOOPBACK {
        return super::rsac_process::run(spec, uploads, stop, ready, signaled);
    }
    super::cpal_io::run_capture(spec, uploads, stop, ready, signaled)
}

#[cfg(target_os = "macos")]
fn coreaudio_capture(
    spec: &AudioCaptureSpec,
    uploads: &Arc<Mutex<AudioInputStore>>,
    stop: &AtomicBool,
    ready: Option<&mpsc::Sender<Result<(), String>>>,
    signaled: &AtomicBool,
) -> Result<(), String> {
    if spec.mode == CAPTURE_MODE_PROCESS_LOOPBACK {
        return Err("Core Audio process loopback is not implemented".into());
    }
    super::cpal_io::run_capture(spec, uploads, stop, ready, signaled)
}
