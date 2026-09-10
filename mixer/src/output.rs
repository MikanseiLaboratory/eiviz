use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::clock::SharedMediaClock;
use crate::frame_hub::{
    AudioMailbox, OutputPace, OutputTx, VideoMailbox, clone_video_cmd, cursor_for_pace,
    retimestamp, sleep_until_deadline,
};
use crate::*;

pub(crate) fn spawn_output_worker(
    output_id: u64,
    handle: OutputHandle,
    video_sub: Arc<AtomicBool>,
    connections: Arc<AtomicU32>,
    omt_gpu: OmtGpu,
    stop: Arc<AtomicBool>,
    clock: SharedMediaClock,
    fps_n: u32,
    fps_d: u32,
) -> OutputWorker {
    let video = Arc::new(VideoMailbox::new());
    let audio = Arc::new(AudioMailbox::new());
    let pace = Arc::new(OutputPace::new(fps_n, fps_d));
    let (ctrl_tx, ctrl_rx) = mpsc::channel();
    let tx = OutputTx::new(
        Arc::clone(&video),
        Arc::clone(&audio),
        Arc::clone(&pace),
        ctrl_tx,
    );
    let join = thread::Builder::new()
        .name(format!("eiviz-send-{output_id}"))
        .spawn(move || {
            send_worker(
                output_id,
                ctrl_rx,
                handle,
                video_sub,
                connections,
                omt_gpu,
                stop,
                clock,
                video,
                audio,
                pace,
            )
        })
        .expect("send worker");
    OutputWorker { tx, join }
}

pub(crate) fn shutdown_output_worker(worker: OutputWorker) {
    let _ = worker.tx.send(SendCmd::Shutdown);
    crate::diag::join_timeout(worker.join, Duration::from_secs(2), "send");
}

fn send_worker(
    output_id: u64,
    rx: mpsc::Receiver<SendCmd>,
    mut sender: OutputHandle,
    video_sub: Arc<AtomicBool>,
    connections: Arc<AtomicU32>,
    omt_gpu: OmtGpu,
    stop: Arc<AtomicBool>,
    clock: SharedMediaClock,
    video: Arc<VideoMailbox>,
    audio: Arc<AudioMailbox>,
    pace: Arc<OutputPace>,
) {
    let mut cursor = cursor_for_pace(&pace);
    let mut last_cpu: Option<SendCmd> = None;
    let mut video_n = 0u32;
    let mut repeat_n = 0u32;
    let mut drop_n = 0u32;
    let mut pack_sum = 0.0f32;
    let mut sdk_sum = 0.0f32;
    let mut timed_n = 0u32;
    loop {
        if stop.load(Ordering::Relaxed) || crate::diag::is_fatal() {
            drain_release_gpu(&rx, video.take());
            return;
        }
        if !drain_ctrl(&rx, &mut last_cpu) {
            drain_release_gpu(&rx, video.take());
            return;
        }
        if !pump_one(&mut sender, &video_sub, &connections) {
            drain_release_gpu(&rx, video.take());
            return;
        }
        for packet in audio.drain() {
            apply_send_cmd(&mut sender, SendCmd::Audio { packet }, &omt_gpu);
        }
        pace.record_audio_drop(audio.drops());
        cursor.set_rate(clock, Instant::now(), pace.rate());
        let now = Instant::now();
        if let Some(pts) = cursor.due(clock, now) {
            if !dispatch_video_slot(
                &mut sender,
                &omt_gpu,
                &video,
                &mut last_cpu,
                &pace,
                pace.rate(),
                pts,
                &mut video_n,
                &mut repeat_n,
                &mut drop_n,
                &mut pack_sum,
                &mut sdk_sum,
                &mut timed_n,
                output_id,
            ) {
                drain_release_gpu(&rx, video.take());
                return;
            }
            continue;
        }
        let wait = cursor.next_deadline(clock);
        sleep_until_deadline(wait, stop.as_ref());
    }
}

fn drain_ctrl(rx: &mpsc::Receiver<SendCmd>, last_cpu: &mut Option<SendCmd>) -> bool {
    loop {
        match rx.try_recv() {
            Ok(SendCmd::Shutdown) => return false,
            Ok(cmd) => {
                if matches!(cmd, SendCmd::Video { .. }) {
                    *last_cpu = Some(cmd);
                } else {
                    release_send_cmd(cmd);
                }
            }
            Err(mpsc::TryRecvError::Empty) => return true,
            Err(mpsc::TryRecvError::Disconnected) => return false,
        }
    }
}

fn dispatch_video_slot(
    sender: &mut OutputHandle,
    omt_gpu: &OmtGpu,
    video: &VideoMailbox,
    last_cpu: &mut Option<SendCmd>,
    pace: &OutputPace,
    rate: crate::clock::Rate,
    pts: i64,
    video_n: &mut u32,
    repeat_n: &mut u32,
    drop_n: &mut u32,
    pack_sum: &mut f32,
    sdk_sum: &mut f32,
    timed_n: &mut u32,
    output_id: u64,
) -> bool {
    if !pump_accept_one(sender) {
        return false;
    }
    let incoming = video.take();
    let repeating = incoming.is_none();
    let cmd = incoming.or_else(|| last_cpu.as_ref().and_then(clone_video_cmd));
    let Some(cmd) = cmd else {
        return true;
    };
    if let Some(clone) = clone_video_cmd(&cmd) {
        *last_cpu = Some(clone);
    }
    if repeating {
        *repeat_n += 1;
    }
    apply_send_cmd(sender, retimestamp(cmd, pts, rate.num, rate.den), omt_gpu);
    *video_n += 1;
    pace.record_video(
        u64::from(*video_n),
        u64::from(*repeat_n),
        video.drops().saturating_add(u64::from(*drop_n)),
    );
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
            "profile send={output_id} video={} repeat={} drop={} pack={pack:.2}ms sdk={sdk:.2}ms",
            *video_n, *repeat_n, *drop_n
        ));
        *pack_sum = 0.0;
        *sdk_sum = 0.0;
        *timed_n = 0;
        *drop_n = 0;
        *repeat_n = 0;
    }
    pump_accept_one(sender)
}

pub(crate) fn drain_release_gpu(rx: &mpsc::Receiver<SendCmd>, extra: Option<SendCmd>) {
    if let Some(cmd) = extra {
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
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sender.pump_accept())) {
        Ok(Ok(())) => true,
        Ok(Err(_)) | Err(_) => false,
    }
}

pub(crate) fn pump_one(
    sender: &mut OutputHandle,
    video_sub: &AtomicBool,
    connections: &AtomicU32,
) -> bool {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sender.pump())) {
        Ok(Ok(subscribed)) => {
            video_sub.store(subscribed, Ordering::Relaxed);
            if let Some(count) = sender.omt_video_subscribers() {
                connections.store(count, Ordering::Relaxed);
            }
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
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                sender.send_video_texture(omt_gpu, &texture, width, height, pts, fps_n, fps_d)
            })) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => crate::diag::mark_fatal(format!("omt send texture: {error}")),
                Err(_) => crate::diag::mark_fatal("omt send texture panicked"),
            }
            busy.store(false, Ordering::Release);
        }
        SendCmd::Audio { packet } => {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                sender.send_audio(&packet)
            })) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => crate::diag::error(&format!("send audio: {error}")),
                Err(_) => crate::diag::mark_fatal("send audio panicked"),
            }
        }
        SendCmd::Shutdown => {}
    }
}
