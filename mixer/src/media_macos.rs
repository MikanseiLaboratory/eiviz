//! File / UVC ingest on macOS. The host passes a path or device id;
//! AVFoundation lives in the mixer, matching Windows Media Foundation.

use std::ffi::{CString, c_char};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::abi::MixerVideoInfo;
use crate::upload::{
    AudioPacket, CpuFormat, UploadStore, ingest_audio_clocked, ingest_audio_throttled,
};

const KIND_VIDEO: i32 = 1;
const KIND_AUDIO: i32 = 2;
const NEXT_RETRY: i32 = 0;
const NEXT_OK: i32 = 1;
const NEXT_EOF: i32 = -1;

#[repr(C)]
struct AvSample {
    kind: i32,
    width: i32,
    height: i32,
    stride: i32,
    sample_rate: i32,
    channels: i32,
    frames: i32,
    pts_hns: i64,
    data: *const u8,
    bytes: u32,
}

enum AvPump {}

#[repr(C)]
#[derive(Clone, Copy)]
struct AvCaptureInfo {
    id: [u8; 512],
    name: [u8; 256],
}

unsafe extern "C" {
    fn eiviz_av_open_file(path: *const c_char, start_hns: i64) -> *mut AvPump;
    fn eiviz_av_open_capture(
        device_id: *const c_char,
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
    ) -> *mut AvPump;
    fn eiviz_av_enum_captures(out: *mut AvCaptureInfo, cap: u32) -> i32;
    fn eiviz_av_enum_capture_modes(
        device_id: *const c_char,
        out: *mut crate::abi::VideoCaptureMode,
        cap: u32,
    ) -> i32;
    fn eiviz_av_close(pump: *mut AvPump);
    fn eiviz_av_duration_hns(pump: *const AvPump) -> i64;
    fn eiviz_av_next(pump: *mut AvPump, out: *mut AvSample) -> i32;
}

fn cstr_field(bytes: &[u8]) -> Option<String> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    if end == 0 {
        None
    } else {
        Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }
}

pub fn enumerate_video_captures() -> Vec<(String, String)> {
    let mut buf = [AvCaptureInfo {
        id: [0; 512],
        name: [0; 256],
    }; 64];
    let n = unsafe { eiviz_av_enum_captures(buf.as_mut_ptr(), buf.len() as u32) };
    if n <= 0 {
        return Vec::new();
    }
    buf.into_iter()
        .take(n as usize)
        .filter_map(|item| {
            let name = cstr_field(&item.name)?;
            let id = cstr_field(&item.id)?;
            if name.is_empty() || id.is_empty() {
                None
            } else {
                Some((name, id))
            }
        })
        .collect()
}

pub fn enumerate_capture_modes(device_id: &str) -> Vec<crate::abi::VideoCaptureMode> {
    let Ok(c_id) = CString::new(device_id) else {
        return Vec::new();
    };
    let mut buf = [crate::abi::VideoCaptureMode::default(); 64];
    let n =
        unsafe { eiviz_av_enum_capture_modes(c_id.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
    if n <= 0 {
        return Vec::new();
    }
    buf.into_iter().take(n as usize).collect()
}

struct NativePump {
    ptr: *mut AvPump,
}

unsafe impl Send for NativePump {}

impl NativePump {
    fn open_file(path: &str, start_hns: i64) -> Result<Self, String> {
        let c_path = CString::new(path).map_err(|_| "invalid video path".to_string())?;
        let ptr = unsafe { eiviz_av_open_file(c_path.as_ptr(), start_hns) };
        if ptr.is_null() {
            return Err(format!("could not open video file: {path}"));
        }
        Ok(Self { ptr })
    }

    fn open_capture(
        device_id: &str,
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
    ) -> Result<Self, String> {
        let c_id = CString::new(device_id).map_err(|_| "invalid capture id".to_string())?;
        let ptr = unsafe { eiviz_av_open_capture(c_id.as_ptr(), width, height, fps_num, fps_den) };
        if ptr.is_null() {
            return Err("could not open video capture device".into());
        }
        Ok(Self { ptr })
    }

    fn duration_hns(&self) -> i64 {
        unsafe { eiviz_av_duration_hns(self.ptr) }
    }

    fn next(&mut self) -> Result<Option<AvSample>, bool> {
        let mut sample = AvSample {
            kind: 0,
            width: 0,
            height: 0,
            stride: 0,
            sample_rate: 0,
            channels: 0,
            frames: 0,
            pts_hns: 0,
            data: std::ptr::null(),
            bytes: 0,
        };
        let code = unsafe { eiviz_av_next(self.ptr, &mut sample) };
        match code {
            NEXT_OK => Ok(Some(sample)),
            NEXT_RETRY => Ok(None),
            NEXT_EOF => Err(true),
            _ => Err(false),
        }
    }
}

impl Drop for NativePump {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { eiviz_av_close(self.ptr) };
            self.ptr = std::ptr::null_mut();
        }
    }
}

pub struct VideoPump {
    stop: Arc<AtomicBool>,
    playing: Arc<AtomicBool>,
    looping: Arc<AtomicBool>,
    seek_hns: Arc<AtomicI64>,
    position_hns: Arc<AtomicI64>,
    duration_hns: Arc<AtomicI64>,
    is_file: bool,
    join: Option<JoinHandle<()>>,
}

impl VideoPump {
    pub fn start(
        source_id: u64,
        path: String,
        capture: bool,
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
        uploads: Arc<Mutex<UploadStore>>,
        frame_buffer_frames: u32,
    ) -> Result<Self, String> {
        if !capture && !std::path::Path::new(&path).is_file() {
            return Err(format!("video file not found: {path}"));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let playing = Arc::new(AtomicBool::new(true));
        let looping = Arc::new(AtomicBool::new(true));
        let seek_hns = Arc::new(AtomicI64::new(-1));
        let position_hns = Arc::new(AtomicI64::new(0));
        let duration_hns = Arc::new(AtomicI64::new(0));
        let stop_t = Arc::clone(&stop);
        let playing_t = Arc::clone(&playing);
        let looping_t = Arc::clone(&looping);
        let seek_t = Arc::clone(&seek_hns);
        let pos_t = Arc::clone(&position_hns);
        let dur_t = Arc::clone(&duration_hns);
        let depth = frame_buffer_frames.clamp(1, 8);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name(format!("eiviz-av-{source_id}"))
            .spawn(move || {
                if let Err(error) = run_loop(
                    source_id, path, capture, width, height, fps_num, fps_den, uploads, depth,
                    stop_t, playing_t, looping_t, seek_t, pos_t, dur_t, ready_tx,
                ) {
                    eprintln!("eiviz video: {error}");
                }
            })
            .map_err(|error| error.to_string())?;
        match wait_ready(ready_rx) {
            Ok(()) => Ok(Self {
                stop,
                playing,
                looping,
                seek_hns,
                position_hns,
                duration_hns,
                is_file: !capture,
                join: Some(join),
            }),
            Err(error) => {
                stop.store(true, Ordering::Relaxed);
                let _ = join.join();
                Err(error)
            }
        }
    }

    pub fn set_playing(&self, playing: bool) {
        self.playing.store(playing, Ordering::Relaxed);
    }

    pub fn set_looping(&self, looping: bool) {
        self.looping.store(looping, Ordering::Relaxed);
    }

    pub fn seek(&self, hns: i64) {
        self.seek_hns.store(hns.max(0), Ordering::Relaxed);
        self.position_hns.store(hns.max(0), Ordering::Relaxed);
    }

    pub fn info(&self) -> MixerVideoInfo {
        MixerVideoInfo {
            playing: u32::from(self.playing.load(Ordering::Relaxed)),
            is_file: u32::from(self.is_file),
            position_hns: self.position_hns.load(Ordering::Relaxed),
            duration_hns: self.duration_hns.load(Ordering::Relaxed),
        }
    }
}

fn wait_ready(ready_rx: mpsc::Receiver<Result<(), String>>) -> Result<(), String> {
    let result = ready_rx.recv_timeout(Duration::from_millis(400));
    match result {
        Ok(Ok(())) | Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
        Ok(Err(error)) => Err(error),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("video thread exited before the source opened".into())
        }
    }
}

impl Drop for VideoPump {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.playing.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            crate::diag::join_timeout(join, Duration::from_secs(2), "av-video");
        }
    }
}

fn run_loop(
    source_id: u64,
    path: String,
    capture: bool,
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    uploads: Arc<Mutex<UploadStore>>,
    depth: u32,
    stop: Arc<AtomicBool>,
    playing: Arc<AtomicBool>,
    looping: Arc<AtomicBool>,
    seek_hns: Arc<AtomicI64>,
    position_hns: Arc<AtomicI64>,
    duration_hns: Arc<AtomicI64>,
    ready: mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    let mut ready = Some(ready);
    loop {
        if stop.load(Ordering::Relaxed) {
            send_ready(&mut ready, Ok(()));
            return Ok(());
        }
        let opened = if capture {
            NativePump::open_capture(&path, width, height, fps_num, fps_den)
        } else {
            NativePump::open_file(&path, 0)
        };
        let mut native = match opened {
            Ok(pump) => pump,
            Err(error) => {
                send_ready(&mut ready, Err(error.clone()));
                return Err(error);
            }
        };
        duration_hns.store(native.duration_hns(), Ordering::Relaxed);
        send_ready(&mut ready, Ok(()));
        let live_depth = depth;
        let file_prefetch = depth.max(3) as usize;
        let mut prefetch = std::collections::VecDeque::new();
        let mut clock_pts = -1i64;
        let mut seek_base = 0i64;
        let mut clock_start = Instant::now();
        let mut need_frame = false;
        let mut was_playing = false;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let seek = seek_hns.swap(-1, Ordering::Relaxed);
            if seek >= 0 && !capture {
                native = match NativePump::open_file(&path, seek) {
                    Ok(pump) => pump,
                    Err(error) => return Err(error),
                };
                duration_hns.store(native.duration_hns(), Ordering::Relaxed);
                prefetch.clear();
                {
                    let mut store = uploads.lock().expect("uploads");
                    store.flush_audio(source_id);
                    store.flush_video(source_id);
                }
                clock_pts = -1;
                seek_base = seek;
                position_hns.store(seek, Ordering::Relaxed);
                clock_start = Instant::now();
                need_frame = true;
            }
            let is_playing = playing.load(Ordering::Relaxed);
            if is_playing && !was_playing {
                clock_pts = -1;
                seek_base = position_hns.load(Ordering::Relaxed);
                clock_start = Instant::now();
            }
            was_playing = is_playing;
            if !capture {
                present_due_file(
                    &uploads,
                    source_id,
                    &mut prefetch,
                    &mut clock_pts,
                    &mut clock_start,
                    seek_base,
                    &position_hns,
                    need_frame,
                    is_playing,
                );
                if need_frame && uploads.lock().expect("uploads").has_video_frame(source_id) {
                    need_frame = false;
                }
            }
            if !is_playing && !need_frame {
                thread::sleep(Duration::from_millis(16));
                continue;
            }
            if !capture && prefetch.len() >= file_prefetch && !need_frame {
                if let Some(front) = prefetch.front() {
                    let wait = pts_wait(front.pts, clock_pts, clock_start);
                    if wait > Duration::ZERO && wait < Duration::from_secs(2) {
                        thread::sleep(wait.min(Duration::from_millis(8)));
                    }
                } else {
                    thread::sleep(Duration::from_millis(2));
                }
                continue;
            }
            let sample = match native.next() {
                Ok(Some(sample)) => sample,
                Ok(None) => {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(true) => {
                    if capture {
                        break;
                    }
                    if looping.load(Ordering::Relaxed) {
                        position_hns.store(0, Ordering::Relaxed);
                        seek_hns.store(0, Ordering::Relaxed);
                    } else {
                        playing.store(false, Ordering::Relaxed);
                    }
                    continue;
                }
                Err(false) => continue,
            };
            match sample.kind {
                KIND_AUDIO => {
                    if is_playing {
                        if let Some(packet) = audio_packet(&sample) {
                            if capture {
                                ingest_audio_throttled(&uploads, source_id, packet);
                            } else {
                                ingest_audio_clocked(&uploads, source_id, packet);
                            }
                        }
                    }
                }
                KIND_VIDEO => {
                    let preview = need_frame;
                    let pts = sample.pts_hns;
                    if clock_pts < 0 {
                        clock_pts = pts;
                        clock_start = Instant::now();
                    }
                    position_hns.store(seek_base + (pts - clock_pts).max(0), Ordering::Relaxed);
                    match copy_video(&sample) {
                        Ok(frame) => {
                            if capture {
                                push_live_cpu(&uploads, source_id, &frame, live_depth);
                                need_frame = false;
                            } else {
                                prefetch.push_back(frame);
                                if preview {
                                    present_due_file(
                                        &uploads,
                                        source_id,
                                        &mut prefetch,
                                        &mut clock_pts,
                                        &mut clock_start,
                                        seek_base,
                                        &position_hns,
                                        true,
                                        is_playing,
                                    );
                                    need_frame = false;
                                }
                            }
                        }
                        Err(error) => eprintln!("eiviz video cpu: {error}"),
                    }
                }
                _ => {}
            }
        }
        if capture {
            thread::sleep(Duration::from_millis(40));
        }
    }
}

fn send_ready(ready: &mut Option<mpsc::SyncSender<Result<(), String>>>, value: Result<(), String>) {
    if let Some(tx) = ready.take() {
        let _ = tx.send(value);
    }
}

fn audio_packet(sample: &AvSample) -> Option<AudioPacket> {
    if sample.data.is_null() || sample.frames <= 0 || sample.channels <= 0 {
        return None;
    }
    let count = sample.channels as usize * sample.frames as usize;
    let bytes = count * 4;
    if sample.bytes as usize != bytes {
        return None;
    }
    let src = unsafe { std::slice::from_raw_parts(sample.data, bytes) };
    Some(AudioPacket {
        timestamp: sample.pts_hns,
        sample_rate: sample.sample_rate,
        channels: sample.channels,
        samples_per_channel: sample.frames,
        pcm_planar_f32: src.to_vec(),
    })
}

struct CpuFrame {
    pixels: Vec<u8>,
    stride: usize,
    width: u32,
    height: u32,
    pts: i64,
}

fn frame_due(pts: i64, clock_pts: i64, elapsed: Duration) -> bool {
    if clock_pts < 0 {
        return true;
    }
    let due = Duration::from_nanos((pts - clock_pts).max(0) as u64 * 100);
    elapsed >= due
}

fn pts_wait(pts: i64, clock_pts: i64, clock_start: Instant) -> Duration {
    Duration::from_nanos((pts - clock_pts).max(0) as u64 * 100)
        .saturating_sub(clock_start.elapsed())
}

fn present_due_file(
    uploads: &Mutex<UploadStore>,
    source_id: u64,
    prefetch: &mut std::collections::VecDeque<CpuFrame>,
    clock_pts: &mut i64,
    clock_start: &mut Instant,
    seek_base: i64,
    position_hns: &AtomicI64,
    force: bool,
    is_playing: bool,
) {
    if prefetch.is_empty() || (!force && !is_playing) {
        return;
    }
    while let Some(front) = prefetch.front() {
        let pts = front.pts;
        if *clock_pts < 0 {
            *clock_pts = pts;
            *clock_start = Instant::now();
        }
        if !force && !frame_due(pts, *clock_pts, clock_start.elapsed()) {
            break;
        }
        let frame = prefetch.pop_front().expect("front");
        position_hns.store(seek_base + (pts - *clock_pts).max(0), Ordering::Relaxed);
        push_file_cpu(uploads, source_id, &frame);
        if force {
            break;
        }
    }
}

fn copy_video(sample: &AvSample) -> Result<CpuFrame, String> {
    if sample.data.is_null() || sample.width <= 0 || sample.height <= 0 || sample.stride <= 0 {
        return Err("empty video sample".into());
    }
    let height = sample.height as u32;
    let stride = sample.stride as usize;
    let src = unsafe { std::slice::from_raw_parts(sample.data, stride * height as usize) };
    Ok(CpuFrame {
        pixels: src.to_vec(),
        stride,
        width: (sample.width as u32).max(2),
        height: height.max(2),
        pts: sample.pts_hns,
    })
}

fn push_live_cpu(uploads: &Mutex<UploadStore>, source_id: u64, frame: &CpuFrame, depth: u32) {
    let mut store = uploads.lock().expect("uploads");
    store.ensure_playout(source_id, frame.width, frame.height, CpuFormat::Bgra, depth);
    let _ = store.push_playout_cpu(source_id, &frame.pixels, frame.stride, frame.pts);
}

fn push_file_cpu(uploads: &Mutex<UploadStore>, source_id: u64, frame: &CpuFrame) {
    let mut store = uploads.lock().expect("uploads");
    store.ensure_playout(source_id, frame.width, frame.height, CpuFormat::Bgra, 1);
    let _ = store.push(source_id, &frame.pixels, frame.stride, frame.pts);
}
