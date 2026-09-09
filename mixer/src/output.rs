use super::*;

pub(crate) fn spawn_output_worker(
    output_id: u64,
    handle: OutputHandle,
    video_sub: Arc<AtomicBool>,
    omt_gpu: OmtGpu,
    stop: Arc<AtomicBool>,
) -> OutputWorker {
    let (tx, rx) = mpsc::channel();
    let join = thread::Builder::new()
        .name(format!("eiviz-send-{output_id}"))
        .spawn(move || send_worker(output_id, rx, handle, video_sub, omt_gpu, stop))
        .expect("send worker");
    OutputWorker { tx, join }
}

pub(crate) fn shutdown_output_worker(worker: OutputWorker) {
    let _ = worker.tx.send(SendCmd::Shutdown);
    crate::diag::join_timeout(worker.join, Duration::from_secs(2), "send");
}

pub(crate) fn send_worker(
    output_id: u64,
    rx: mpsc::Receiver<SendCmd>,
    mut sender: OutputHandle,
    video_sub: Arc<AtomicBool>,
    omt_gpu: OmtGpu,
    stop: Arc<AtomicBool>,
) {
    let mut batch = Vec::new();
    let mut video_n = 0u32;
    let mut drop_n = 0u32;
    let mut pack_sum = 0.0f32;
    let mut sdk_sum = 0.0f32;
    let mut timed_n = 0u32;
    loop {
        if stop.load(Ordering::Relaxed) || crate::diag::is_fatal() {
            drain_release_gpu(&rx, std::mem::take(&mut batch));
            return;
        }
        loop {
            match rx.try_recv() {
                Ok(SendCmd::Shutdown) => {
                    drain_release_gpu(&rx, std::mem::take(&mut batch));
                    return;
                }
                Ok(cmd) => {
                    if stop.load(Ordering::Relaxed) {
                        release_send_cmd(cmd);
                        continue;
                    }
                    batch.push(cmd);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = dispatch_worker_batch(
                        &mut sender,
                        &mut batch,
                        &omt_gpu,
                        output_id,
                        &mut video_n,
                        &mut drop_n,
                        &mut pack_sum,
                        &mut sdk_sum,
                        &mut timed_n,
                    );
                    drain_release_gpu(&rx, std::mem::take(&mut batch));
                    return;
                }
            }
        }
        if !dispatch_worker_batch(
            &mut sender,
            &mut batch,
            &omt_gpu,
            output_id,
            &mut video_n,
            &mut drop_n,
            &mut pack_sum,
            &mut sdk_sum,
            &mut timed_n,
        ) {
            drain_release_gpu(&rx, std::mem::take(&mut batch));
            return;
        }
        if !pump_one(&mut sender, &video_sub) {
            drain_release_gpu(&rx, std::mem::take(&mut batch));
            return;
        }
        match rx.recv_timeout(Duration::from_millis(2)) {
            Ok(SendCmd::Shutdown) => {
                drain_release_gpu(&rx, std::mem::take(&mut batch));
                return;
            }
            Ok(cmd) => {
                if stop.load(Ordering::Relaxed) {
                    release_send_cmd(cmd);
                    continue;
                }
                batch.push(cmd);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                drain_release_gpu(&rx, std::mem::take(&mut batch));
                return;
            }
        }
    }
}

pub(crate) fn drain_release_gpu(rx: &mpsc::Receiver<SendCmd>, extra: Vec<SendCmd>) {
    for cmd in extra {
        release_send_cmd(cmd);
    }
    while let Ok(cmd) = rx.try_recv() {
        release_send_cmd(cmd);
    }
}

pub(crate) fn release_send_cmd(cmd: SendCmd) {
    if let SendCmd::GpuVideo { busy, .. } = cmd {
        busy.store(false, Ordering::Release);
    }
}

pub(crate) fn dispatch_worker_batch(
    sender: &mut OutputHandle,
    batch: &mut Vec<SendCmd>,
    omt_gpu: &OmtGpu,
    output_id: u64,
    video_n: &mut u32,
    drop_n: &mut u32,
    pack_sum: &mut f32,
    sdk_sum: &mut f32,
    timed_n: &mut u32,
) -> bool {
    if batch.is_empty() {
        return true;
    }
    let in_video = batch
        .iter()
        .filter(|cmd| matches!(cmd, SendCmd::Video { .. } | SendCmd::GpuVideo { .. }))
        .count() as u32;
    let ordered = coalesce_latest_video(std::mem::take(batch));
    let ordered = if sender.ndi_audio_first() {
        take_audio_first(ordered)
    } else {
        ordered
    };
    for cmd in ordered {
        let is_video = matches!(cmd, SendCmd::Video { .. } | SendCmd::GpuVideo { .. });
        let heavy = matches!(cmd, SendCmd::GpuVideo { .. } | SendCmd::Video { .. });
        if heavy && !pump_accept_one(sender) {
            release_send_cmd(cmd);
            return false;
        }
        apply_send_cmd(sender, cmd, omt_gpu);
        if is_video {
            *video_n += 1;
            #[cfg(any(windows, target_os = "macos"))]
            if crate::diag::profile_send()
                && let Some((pack, sdk)) = sender.last_ndi_send_ms()
            {
                *pack_sum += pack;
                *sdk_sum += sdk;
                *timed_n += 1;
            }
            if crate::diag::profile_send() && *timed_n > 0 && *video_n % 60 == 0 {
                let pack = if *timed_n > 0 {
                    *pack_sum / *timed_n as f32
                } else {
                    0.0
                };
                let sdk = if *timed_n > 0 {
                    *sdk_sum / *timed_n as f32
                } else {
                    0.0
                };
                crate::diag::info(&format!(
                    "profile send={output_id} video={} drop={} pack={pack:.2}ms sdk={sdk:.2}ms",
                    *video_n, *drop_n
                ));
                *pack_sum = 0.0;
                *sdk_sum = 0.0;
                *timed_n = 0;
                *drop_n = 0;
            }
        }
        if heavy && !pump_accept_one(sender) {
            return false;
        }
    }
    *drop_n += in_video.saturating_sub(1);
    // coalesce keeps at most one video; extras are drops
    true
}

/// Keep audio in order; keep only the last CPU/GPU video in a drain so a slow
/// output does not send stale frames. Dropped GPU frames free the send ring.
pub(crate) fn coalesce_latest_video(cmds: Vec<SendCmd>) -> Vec<SendCmd> {
    let mut last_video = None;
    let mut out = Vec::with_capacity(cmds.len());
    for cmd in cmds {
        match cmd {
            SendCmd::Audio { .. } => out.push(cmd),
            SendCmd::Shutdown => {
                if let Some(prev) = last_video.take() {
                    release_send_cmd(prev);
                }
                out.push(SendCmd::Shutdown);
            }
            SendCmd::Video { .. } | SendCmd::GpuVideo { .. } => {
                if let Some(prev) = last_video.replace(cmd) {
                    release_send_cmd(prev);
                }
            }
        }
    }
    if let Some(video) = last_video {
        out.push(video);
    }
    out
}

/// Send audio before video in a drain. OMT PCM uses AudioIngress; NDI audio
/// still leaves this worker before SpeedHQ.
pub(crate) fn take_audio_first(cmds: Vec<SendCmd>) -> Vec<SendCmd> {
    let mut out = Vec::with_capacity(cmds.len());
    let mut pending_video = Vec::new();
    for cmd in cmds {
        match &cmd {
            SendCmd::Shutdown => {
                out.append(&mut pending_video);
                out.push(cmd);
            }
            SendCmd::Audio { .. } => out.push(cmd),
            SendCmd::Video { .. } | SendCmd::GpuVideo { .. } => pending_video.push(cmd),
        }
    }
    out.append(&mut pending_video);
    out
}

pub(crate) fn pump_accept_one(sender: &mut OutputHandle) -> bool {
    // Accept only: metadata `read` shares the peer lock with writers and
    // used to stall GPU encode when a socket was left blocking.
    match panic::catch_unwind(AssertUnwindSafe(|| sender.pump_accept())) {
        Ok(Ok(())) => true,
        Ok(Err(_)) | Err(_) => false,
    }
}

pub(crate) fn pump_one(sender: &mut OutputHandle, video_sub: &AtomicBool) -> bool {
    match panic::catch_unwind(AssertUnwindSafe(|| sender.pump())) {
        Ok(Ok(subscribed)) => {
            video_sub.store(subscribed, Ordering::Relaxed);
            true
        }
        Ok(Err(_)) | Err(_) => false,
    }
}

pub(crate) fn apply_send_cmd(sender: &mut OutputHandle, cmd: SendCmd, omt_gpu: &OmtGpu) {
    match cmd {
        SendCmd::Video {
            width,
            height,
            stride,
            pts,
            data,
            fps_n,
            fps_d,
        } => {
            match panic::catch_unwind(AssertUnwindSafe(|| {
                sender.send_video_uyvy(width, height, stride, pts, data, fps_n, fps_d)
            })) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => crate::diag::error(&format!("omt send video: {error}")),
                Err(_) => crate::diag::mark_fatal("omt send video panicked"),
            }
        }
        SendCmd::GpuVideo {
            texture,
            width,
            height,
            pts,
            fps_n,
            fps_d,
            busy,
        } => {
            match panic::catch_unwind(AssertUnwindSafe(|| {
                sender.send_video_texture(omt_gpu, &texture, width, height, pts, fps_n, fps_d)
            })) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => crate::diag::mark_fatal(format!("omt send texture: {error}")),
                Err(_) => crate::diag::mark_fatal("omt send texture panicked"),
            }
            busy.store(false, Ordering::Release);
        }
        SendCmd::Audio { packet } => {
            match panic::catch_unwind(AssertUnwindSafe(|| sender.send_audio(&packet))) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => crate::diag::error(&format!("send audio: {error}")),
                Err(_) => crate::diag::mark_fatal("send audio panicked"),
            }
        }
        SendCmd::Shutdown => {}
    }
}
