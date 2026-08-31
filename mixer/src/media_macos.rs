//! File / UVC ingest on macOS. The host passes a path or device id;
//! AVFoundation lives in the mixer, matching Windows Media Foundation.

use std::ffi::{c_char, CString};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::abi::MixerVideoInfo;
use crate::upload::{
    ingest_audio_clocked, ingest_audio_throttled, ingest_cpu_frame, AudioPacket, CpuFormat,
    GpuIngest, GpuUploadRing, UploadStore,
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

unsafe extern "C" {
    fn eiviz_av_open_file(path: *const c_char, start_hns: i64) -> *mut AvPump;
    fn eiviz_av_open_capture(device_id: *const c_char) -> *mut AvPump;
    fn eiviz_av_close(pump: *mut AvPump);
    fn eiviz_av_duration_hns(pump: *const AvPump) -> i64;
    fn eiviz_av_next(pump: *mut AvPump, out: *mut AvSample) -> i32;
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

    fn open_capture(device_id: &str) -> Result<Self, String> {
        let c_id = CString::new(device_id).map_err(|_| "invalid capture id".to_string())?;
        let ptr = unsafe { eiviz_av_open_capture(c_id.as_ptr()) };
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
        uploads: Arc<Mutex<UploadStore>>,
        ingest: GpuIngest,
    ) -> Result<Self, String> {
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
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name(format!("eiviz-av-{source_id}"))
            .spawn(move || {
                if let Err(error) = run_loop(
                    source_id, path, capture, uploads, ingest, stop_t, playing_t, looping_t, seek_t,
                    pos_t, dur_t, ready_tx,
                ) {
                    eprintln!("eiviz video: {error}");
                }
            })
            .map_err(|error| error.to_string())?;
        match ready_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(())) => Ok(Self {
                stop,
                playing,
                looping,
                seek_hns,
                position_hns,
                duration_hns,
                is_file: !capture,
                join: Some(join),
            }),
            Ok(Err(error)) => {
                stop.store(true, Ordering::Relaxed);
                let _ = join.join();
                Err(error)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                stop.store(true, Ordering::Relaxed);
                Err("timed out opening the video source".into())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                stop.store(true, Ordering::Relaxed);
                let _ = join.join();
                Err("video thread exited before the source opened".into())
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

impl Drop for VideoPump {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.playing.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn run_loop(
    source_id: u64,
    path: String,
    capture: bool,
    uploads: Arc<Mutex<UploadStore>>,
    ingest: GpuIngest,
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
            NativePump::open_capture(&path)
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
        let mut clock_pts = -1i64;
        let mut seek_base = 0i64;
        let mut clock_start = Instant::now();
        let mut need_frame = false;
        let mut was_playing = false;
        let mut gpu_ring = GpuUploadRing::new();
        let mut gpu_warned = false;
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
                uploads.lock().expect("uploads").flush_audio(source_id);
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
            if !is_playing && !need_frame {
                thread::sleep(Duration::from_millis(16));
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
                    if is_playing && !preview && !capture {
                        let wait = Duration::from_nanos((pts - clock_pts).max(0) as u64 * 100)
                            .saturating_sub(clock_start.elapsed());
                        if wait > Duration::ZERO && wait < Duration::from_secs(2) {
                            thread::sleep(wait.min(Duration::from_millis(40)));
                        }
                    }
                    if let Err(error) =
                        push_video(&uploads, &ingest, &mut gpu_ring, &mut gpu_warned, source_id, &sample)
                    {
                        eprintln!("eiviz video cpu: {error}");
                    }
                    need_frame = false;
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

fn push_video(
    uploads: &Mutex<UploadStore>,
    ingest: &GpuIngest,
    gpu_ring: &mut GpuUploadRing,
    gpu_warned: &mut bool,
    source_id: u64,
    sample: &AvSample,
) -> Result<(), String> {
    if sample.data.is_null() || sample.width <= 0 || sample.height <= 0 || sample.stride <= 0 {
        return Err("empty video sample".into());
    }
    let height = sample.height as u32;
    let stride = sample.stride as usize;
    let src = unsafe { std::slice::from_raw_parts(sample.data, stride * height as usize) };
    let width = (sample.width as u32).max(2);
    if ingest.video_gpu.load(Ordering::Relaxed) {
        ingest_cpu_frame(
            uploads,
            Some(ingest),
            true,
            gpu_ring,
            gpu_warned,
            source_id,
            1,
            src,
            stride,
            width,
            height.max(2),
            CpuFormat::Bgra,
            sample.pts_hns,
            false,
            "video",
        );
        return Ok(());
    }
    let mut store = uploads.lock().expect("uploads");
    store.ensure(
        source_id,
        width,
        height.max(2),
        CpuFormat::Bgra,
    );
    store.push(source_id, src, stride, sample.pts_hns)
}
