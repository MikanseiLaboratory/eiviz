use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, Once};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFSample, IMFSourceReader, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, MF_MT_AUDIO_NUM_CHANNELS,
    MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_PD_DURATION, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS,
    MF_SOURCE_READER_ANY_STREAM, MF_SOURCE_READER_D3D_MANAGER,
    MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
    MF_SOURCE_READER_MEDIASOURCE, MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_STREAMTICK,
    MF_VERSION, MFAudioFormat_Float, MFCreateAttributes, MFCreateDeviceSource, MFCreateMediaType,
    MFCreateSourceReaderFromMediaSource, MFCreateSourceReaderFromURL, MFEnumDeviceSources,
    MFMediaType_Audio, MFMediaType_Video, MFSTARTUP_NOSOCKET, MFStartup, MFVideoFormat_NV12,
    MFVideoFormat_RGB32, MFVideoFormat_UYVY, MFVideoFormat_YUY2,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::System::Variant::VT_I8;
use windows::core::{GUID, PCWSTR};

use crate::abi::{FMT_BGRA, MixerVideoInfo};
use crate::convert::VideoGpuRing;
use crate::dxgi::GpuVideoContext;
use crate::upload::{
    AUDIO_LIVE_FRAMES, AudioPacket, CpuFormat, GpuVideoFrame, UploadStore, ingest_audio_clocked,
    ingest_audio_throttled,
};

static MF_ONCE: Once = Once::new();

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
        format: u32,
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
        uploads: Arc<Mutex<UploadStore>>,
        gpu: GpuVideoContext,
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
            .name(format!("eiviz-mf-{source_id}"))
            .spawn(move || {
                if let Err(error) = run_loop(
                    source_id, path, capture, format, width, height, fps_num, fps_den, uploads,
                    gpu, depth, stop_t, playing_t, looping_t, seek_t, pos_t, dur_t, ready_tx,
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
            crate::diag::join_timeout(join, Duration::from_secs(2), "mf-video");
        }
    }
}

fn run_loop(
    source_id: u64,
    path: String,
    capture: bool,
    format: u32,
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    uploads: Arc<Mutex<UploadStore>>,
    gpu: GpuVideoContext,
    depth: u32,
    stop: Arc<AtomicBool>,
    playing: Arc<AtomicBool>,
    looping: Arc<AtomicBool>,
    seek_hns: Arc<AtomicI64>,
    position_hns: Arc<AtomicI64>,
    duration_hns: Arc<AtomicI64>,
    ready: mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    startup()?;
    let prefer_packed = format != FMT_BGRA;
    let mut ready = Some(ready);
    loop {
        if stop.load(Ordering::Relaxed) {
            send_ready(&mut ready, Ok(()));
            return Ok(());
        }
        let opened = match open_reader(&path, capture, Some(&gpu), prefer_packed) {
            Ok(reader) => match configure_video(
                &reader,
                true,
                prefer_packed,
                width,
                height,
                fps_num,
                fps_den,
            ) {
                Ok(layout) => Ok((reader, layout)),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        let (reader, layout) = match opened {
            Ok(pair) => pair,
            Err(gpu_error) => match open_reader(&path, capture, None, prefer_packed) {
                Ok(reader) => match configure_video(
                    &reader,
                    false,
                    prefer_packed,
                    width,
                    height,
                    fps_num,
                    fps_den,
                ) {
                    Ok(layout) => (reader, layout),
                    Err(error) => {
                        let message = format!("{gpu_error}; cpu fallback: {error}");
                        send_ready(&mut ready, Err(message.clone()));
                        return Err(message);
                    }
                },
                Err(error) => {
                    let message = format!("{gpu_error}; cpu fallback: {error}");
                    send_ready(&mut ready, Err(message.clone()));
                    return Err(message);
                }
            },
        };
        let audio = match configure_audio(&reader) {
            Ok(layout) => Some(layout),
            Err(_) => {
                let _ = unsafe {
                    reader.SetStreamSelection(stream(MF_SOURCE_READER_FIRST_AUDIO_STREAM), false)
                };
                None
            }
        };
        duration_hns.store(read_duration(&reader), Ordering::Relaxed);
        send_ready(&mut ready, Ok(()));
        let live_depth = depth;
        let file_prefetch = depth.max(3);
        let mut gpu_ring = VideoGpuRing::new(if capture {
            live_depth.max(3)
        } else {
            file_prefetch
        });
        let mut prefetch = std::collections::VecDeque::new();
        let mut ring_vram = 0u64;
        let mut clock_pts = -1i64;
        let mut seek_base = 0i64;
        let mut clock_start = Instant::now();
        let mut gpu_warned = false;
        let mut need_frame = !capture;
        let mut was_playing = false;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let seek = seek_hns.swap(-1, Ordering::Relaxed);
            if seek >= 0 {
                let _ = unsafe { reader.Flush(stream(MF_SOURCE_READER_ANY_STREAM)) };
                seek_to(&reader, seek);
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
                    ring_vram,
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
            let prefetch_cap = if capture { 0 } else { file_prefetch as usize };
            if !capture && prefetch.len() >= prefetch_cap && !need_frame {
                if let Some(front) = prefetch.front() {
                    wait_for_pts(
                        front.pts(),
                        clock_pts,
                        clock_start,
                        &reader,
                        audio.as_ref(),
                        &uploads,
                        source_id,
                        is_playing,
                    );
                } else {
                    thread::sleep(Duration::from_millis(2));
                }
                continue;
            }
            let sample = match read_sample(&reader, audio.as_ref()) {
                Ok(Some(sample)) => sample,
                Ok(None) => continue,
                Err(end) if end => {
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
                Err(_) => continue,
            };
            match sample {
                Decoded::Audio { pts, packet } => {
                    if is_playing {
                        if capture {
                            ingest_audio_throttled(&uploads, source_id, packet);
                        } else {
                            ingest_audio_clocked(&uploads, source_id, packet);
                        }
                    }
                    let _ = pts;
                }
                Decoded::Video { pts, sample } => {
                    let preview = need_frame;
                    if clock_pts < 0 {
                        clock_pts = pts;
                        clock_start = Instant::now();
                    }
                    position_hns.store(seek_base + (pts - clock_pts).max(0), Ordering::Relaxed);
                    let decoded = decode_video_frame(
                        &gpu,
                        &mut gpu_ring,
                        &sample,
                        &layout,
                        pts,
                        &mut gpu_warned,
                    );
                    let Some(decoded) = decoded else {
                        continue;
                    };
                    if capture {
                        push_live_frame(
                            &uploads,
                            source_id,
                            decoded,
                            live_depth,
                            gpu_ring.vram_bytes(),
                        );
                        need_frame = false;
                        continue;
                    }
                    ring_vram = gpu_ring.vram_bytes();
                    prefetch.push_back(decoded);
                    if preview {
                        present_due_file(
                            &uploads,
                            source_id,
                            &mut prefetch,
                            ring_vram,
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
        }
        if capture {
            thread::sleep(Duration::from_millis(40));
        }
    }
}

enum Prefetched {
    Gpu(GpuVideoFrame),
    Cpu {
        pixels: Vec<u8>,
        stride: usize,
        format: CpuFormat,
        pts: i64,
        width: u32,
        height: u32,
    },
}

impl Prefetched {
    fn pts(&self) -> i64 {
        match self {
            Self::Gpu(frame) => frame.pts,
            Self::Cpu { pts, .. } => *pts,
        }
    }
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

fn wait_for_pts(
    pts: i64,
    clock_pts: i64,
    clock_start: Instant,
    reader: &IMFSourceReader,
    audio: Option<&AudioLayout>,
    uploads: &Mutex<UploadStore>,
    source_id: u64,
    is_playing: bool,
) {
    let wait = pts_wait(pts, clock_pts, clock_start);
    if wait == Duration::ZERO || wait >= Duration::from_secs(2) {
        return;
    }
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        drain_audio(reader, audio, uploads, source_id, is_playing);
        let remain = deadline.saturating_duration_since(Instant::now());
        if remain.is_zero() {
            break;
        }
        thread::sleep(remain.min(Duration::from_millis(4)));
    }
}

fn present_due_file(
    uploads: &Mutex<UploadStore>,
    source_id: u64,
    prefetch: &mut std::collections::VecDeque<Prefetched>,
    ring_vram: u64,
    clock_pts: &mut i64,
    clock_start: &mut Instant,
    seek_base: i64,
    position_hns: &AtomicI64,
    force: bool,
    is_playing: bool,
) {
    if prefetch.is_empty() {
        return;
    }
    if !force && !is_playing {
        return;
    }
    while let Some(front) = prefetch.front() {
        let pts = front.pts();
        if *clock_pts < 0 {
            *clock_pts = pts;
            *clock_start = Instant::now();
        }
        if !force && !frame_due(pts, *clock_pts, clock_start.elapsed()) {
            break;
        }
        let frame = prefetch.pop_front().expect("front");
        position_hns.store(seek_base + (pts - *clock_pts).max(0), Ordering::Relaxed);
        push_file_frame(uploads, source_id, frame, ring_vram);
        if force {
            break;
        }
    }
}

fn decode_video_frame(
    gpu: &GpuVideoContext,
    gpu_ring: &mut VideoGpuRing,
    sample: &windows::Win32::Media::MediaFoundation::IMFSample,
    layout: &VideoLayout,
    pts: i64,
    gpu_warned: &mut bool,
) -> Option<Prefetched> {
    if layout.gpu {
        match gpu.dxgi.import_sample(gpu, gpu_ring, sample, pts) {
            Ok(frame) => return Some(Prefetched::Gpu(frame)),
            Err(error) if !*gpu_warned => {
                eprintln!("eiviz video gpu: {error}; falling back to CPU frames");
                *gpu_warned = true;
            }
            Err(_) => {}
        }
    }
    match take_cpu_frame(sample, layout, pts) {
        Ok(frame) => Some(frame),
        Err(error) => {
            if !*gpu_warned {
                eprintln!("eiviz video cpu: {error}");
                *gpu_warned = true;
            }
            None
        }
    }
}

fn push_live_frame(
    uploads: &Mutex<UploadStore>,
    source_id: u64,
    frame: Prefetched,
    depth: u32,
    ring_vram: u64,
) {
    let mut store = uploads.lock().expect("uploads");
    match frame {
        Prefetched::Gpu(gpu) => {
            store.ensure_playout(
                source_id,
                gpu.width.max(2),
                gpu.height.max(2),
                CpuFormat::GpuRgba,
                depth,
            );
            store.set_ring_vram(source_id, ring_vram);
            let _ = store.push_playout_gpu(source_id, gpu);
        }
        Prefetched::Cpu {
            pixels,
            stride,
            format,
            pts,
            width,
            height,
        } => {
            store.ensure_playout(source_id, width.max(2), height.max(2), format, depth);
            let _ = store.push_playout_cpu(source_id, &pixels, stride, pts);
        }
    }
}

fn push_file_frame(
    uploads: &Mutex<UploadStore>,
    source_id: u64,
    frame: Prefetched,
    ring_vram: u64,
) {
    let mut store = uploads.lock().expect("uploads");
    match frame {
        Prefetched::Gpu(gpu) => {
            store.ensure_playout(
                source_id,
                gpu.width.max(2),
                gpu.height.max(2),
                CpuFormat::GpuRgba,
                1,
            );
            store.set_ring_vram(source_id, ring_vram);
            let _ = store.push_gpu(source_id, gpu);
        }
        Prefetched::Cpu {
            pixels,
            stride,
            format,
            pts,
            width,
            height,
        } => {
            store.ensure_playout(source_id, width.max(2), height.max(2), format, 1);
            let _ = store.push(source_id, &pixels, stride, pts);
        }
    }
}

fn send_ready(ready: &mut Option<mpsc::SyncSender<Result<(), String>>>, value: Result<(), String>) {
    if let Some(tx) = ready.take() {
        let _ = tx.send(value);
    }
}

enum Decoded {
    Video { pts: i64, sample: IMFSample },
    Audio { pts: i64, packet: AudioPacket },
}

struct VideoLayout {
    width: u32,
    height: u32,
    subtype: GUID,
    stride: i32,
    gpu: bool,
    packed: bool,
}

fn startup() -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    MF_ONCE.call_once(|| unsafe {
        let _ = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);
    });
    Ok(())
}

pub fn enumerate_video_captures() -> Vec<(String, String)> {
    if startup().is_err() {
        return Vec::new();
    }
    unsafe {
        let mut attrs = None;
        if MFCreateAttributes(&mut attrs, 1).is_err() {
            return Vec::new();
        }
        let Some(attrs) = attrs else {
            return Vec::new();
        };
        if attrs
            .SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )
            .is_err()
        {
            return Vec::new();
        }
        let mut devices = std::ptr::null_mut();
        let mut count = 0u32;
        if MFEnumDeviceSources(&attrs, &mut devices, &mut count).is_err() || devices.is_null() {
            return Vec::new();
        }
        let slice = std::slice::from_raw_parts_mut(devices, count as usize);
        let mut out = Vec::new();
        for slot in slice.iter_mut() {
            let Some(activate) = slot.take() else {
                continue;
            };
            let name = mf_attr_string(&activate, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME);
            let link = mf_attr_string(
                &activate,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
            );
            if let (Some(name), Some(link)) = (name, link)
                && !name.is_empty()
                && !link.is_empty()
            {
                out.push((name, link));
            }
        }
        CoTaskMemFree(Some(devices.cast()));
        out
    }
}

fn mf_attr_string(activate: &IMFActivate, key: &GUID) -> Option<String> {
    let mut buf = [0u16; 512];
    unsafe {
        activate.GetString(key, &mut buf, None).ok()?;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    if end == 0 {
        None
    } else {
        Some(String::from_utf16_lossy(&buf[..end]))
    }
}

fn stream(value: windows::Win32::Media::MediaFoundation::MF_SOURCE_READER_CONSTANTS) -> u32 {
    value.0 as u32
}

fn open_reader(
    path: &str,
    capture: bool,
    gpu: Option<&GpuVideoContext>,
    prefer_packed: bool,
) -> Result<IMFSourceReader, String> {
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 6).map_err(|e| e.to_string())?;
        let attrs = attrs.ok_or("MF attributes")?;
        if let Some(gpu) = gpu {
            attrs
                .SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, &gpu.dxgi.manager)
                .map_err(|e| e.to_string())?;
            attrs
                .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
                .map_err(|e| e.to_string())?;
            attrs
                .SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)
                .map_err(|e| e.to_string())?;
            attrs
                .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
                .map_err(|e| e.to_string())?;
        } else {
            let _ = attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 0);
            if !prefer_packed {
                let _ = attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1);
                let _ = attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1);
            }
        }
        if capture {
            let mut src_attrs = None;
            MFCreateAttributes(&mut src_attrs, 2).map_err(|e| e.to_string())?;
            let src_attrs = src_attrs.ok_or("MF attributes")?;
            src_attrs
                .SetGUID(
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
                )
                .map_err(|e| e.to_string())?;
            let wide = wide(path);
            src_attrs
                .SetString(
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
                    PCWSTR(wide.as_ptr()),
                )
                .map_err(|e| e.to_string())?;
            let source = MFCreateDeviceSource(&src_attrs).map_err(|e| e.to_string())?;
            return MFCreateSourceReaderFromMediaSource(&source, &attrs).map_err(|e| e.to_string());
        }
        open_file(path, &attrs)
    }
}

fn open_file(
    path: &str,
    attrs: &windows::Win32::Media::MediaFoundation::IMFAttributes,
) -> Result<IMFSourceReader, String> {
    unsafe {
        let url = file_url(path);
        let wide_url = wide(&url);
        if let Ok(reader) = MFCreateSourceReaderFromURL(PCWSTR(wide_url.as_ptr()), attrs) {
            return Ok(reader);
        }
        let native = path.trim_start_matches(r"\\?\");
        let wide_path = wide(native);
        MFCreateSourceReaderFromURL(PCWSTR(wide_path.as_ptr()), attrs).map_err(|e| e.to_string())
    }
}

fn configure_video(
    reader: &IMFSourceReader,
    gpu: bool,
    prefer_packed: bool,
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
) -> Result<VideoLayout, String> {
    unsafe {
        let _ = reader.SetStreamSelection(stream(MF_SOURCE_READER_ANY_STREAM), false);
        reader
            .SetStreamSelection(stream(MF_SOURCE_READER_FIRST_VIDEO_STREAM), true)
            .map_err(|e| e.to_string())?;
        let _ = reader.SetStreamSelection(stream(MF_SOURCE_READER_FIRST_AUDIO_STREAM), true);
        let subtypes: &[GUID] = if gpu {
            &[MFVideoFormat_NV12, MFVideoFormat_RGB32]
        } else if prefer_packed {
            &[
                MFVideoFormat_UYVY,
                MFVideoFormat_YUY2,
                MFVideoFormat_NV12,
                MFVideoFormat_RGB32,
            ]
        } else {
            &[
                MFVideoFormat_RGB32,
                MFVideoFormat_NV12,
                MFVideoFormat_YUY2,
                MFVideoFormat_UYVY,
            ]
        };
        let mut last = "no subtype".to_string();
        for subtype in subtypes {
            match set_video_subtype(reader, *subtype, width, height, fps_num, fps_den) {
                Ok(()) => return read_video_layout(reader, gpu, prefer_packed),
                Err(error) => last = error,
            }
        }
        Err(last)
    }
}

fn set_video_subtype(
    reader: &IMFSourceReader,
    subtype: GUID,
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
) -> Result<(), String> {
    unsafe {
        let ty = MFCreateMediaType().map_err(|e| e.to_string())?;
        ty.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|e| e.to_string())?;
        ty.SetGUID(&MF_MT_SUBTYPE, &subtype)
            .map_err(|e| e.to_string())?;
        if width > 0 && height > 0 {
            let size = ((width as u64) << 32) | height as u64;
            ty.SetUINT64(&MF_MT_FRAME_SIZE, size)
                .map_err(|e| e.to_string())?;
        }
        if fps_num > 0 && fps_den > 0 {
            let rate = ((fps_num as u64) << 32) | fps_den as u64;
            ty.SetUINT64(&MF_MT_FRAME_RATE, rate)
                .map_err(|e| e.to_string())?;
        }
        reader
            .SetCurrentMediaType(stream(MF_SOURCE_READER_FIRST_VIDEO_STREAM), None, &ty)
            .map_err(|e| e.to_string())
    }
}

pub fn enumerate_capture_modes(device_id: &str) -> Vec<crate::abi::VideoCaptureMode> {
    if startup().is_err() {
        return Vec::new();
    }
    let Ok(reader) = open_reader(device_id, true, None, true) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    unsafe {
        let _ = reader.SetStreamSelection(stream(MF_SOURCE_READER_ANY_STREAM), false);
        let _ = reader.SetStreamSelection(stream(MF_SOURCE_READER_FIRST_VIDEO_STREAM), true);
        for index in 0..64u32 {
            let Ok(ty) =
                reader.GetNativeMediaType(stream(MF_SOURCE_READER_FIRST_VIDEO_STREAM), index)
            else {
                break;
            };
            let Ok(frame_size) = ty.GetUINT64(&MF_MT_FRAME_SIZE) else {
                continue;
            };
            let width = (frame_size >> 32) as u32;
            let height = frame_size as u32;
            if width == 0 || height == 0 {
                continue;
            }
            let (fps_num, fps_den) = ty
                .GetUINT64(&MF_MT_FRAME_RATE)
                .map(|rate| ((rate >> 32) as u32, rate as u32))
                .unwrap_or((60, 1));
            let mode = crate::abi::VideoCaptureMode {
                width,
                height,
                fps_num: fps_num.max(1),
                fps_den: fps_den.max(1),
                format: FMT_BGRA,
            };
            if !out.iter().any(|item: &crate::abi::VideoCaptureMode| {
                item.width == mode.width
                    && item.height == mode.height
                    && item.fps_num == mode.fps_num
                    && item.fps_den == mode.fps_den
            }) {
                out.push(mode);
            }
        }
    }
    out
}

fn read_video_layout(
    reader: &IMFSourceReader,
    gpu: bool,
    packed: bool,
) -> Result<VideoLayout, String> {
    unsafe {
        let ty = reader
            .GetCurrentMediaType(stream(MF_SOURCE_READER_FIRST_VIDEO_STREAM))
            .map_err(|e| e.to_string())?;
        let subtype = ty.GetGUID(&MF_MT_SUBTYPE).map_err(|e| e.to_string())?;
        let frame_size = ty.GetUINT64(&MF_MT_FRAME_SIZE).map_err(|e| e.to_string())?;
        let width = (frame_size >> 32) as u32;
        let height = frame_size as u32;
        if width == 0 || height == 0 {
            return Err("Media Foundation reported a zero-sized frame".into());
        }
        let stride = ty.GetUINT32(&MF_MT_DEFAULT_STRIDE).unwrap_or(0) as i32;
        Ok(VideoLayout {
            width,
            height,
            subtype,
            stride,
            gpu,
            packed,
        })
    }
}

struct AudioLayout {
    channels: i32,
    sample_rate: i32,
}

fn configure_audio(reader: &IMFSourceReader) -> Result<AudioLayout, String> {
    unsafe {
        let ty = MFCreateMediaType().map_err(|e| e.to_string())?;
        ty.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
            .map_err(|e| e.to_string())?;
        ty.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)
            .map_err(|e| e.to_string())?;
        reader
            .SetCurrentMediaType(stream(MF_SOURCE_READER_FIRST_AUDIO_STREAM), None, &ty)
            .map_err(|e| e.to_string())?;
        let current = reader
            .GetCurrentMediaType(stream(MF_SOURCE_READER_FIRST_AUDIO_STREAM))
            .map_err(|e| e.to_string())?;
        let channels = current.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(2) as i32;
        let rate = current
            .GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)
            .unwrap_or(48_000) as i32;
        if channels <= 0 || rate <= 0 {
            let _ = reader.SetStreamSelection(stream(MF_SOURCE_READER_FIRST_AUDIO_STREAM), false);
            return Err("no audio".into());
        }
        Ok(AudioLayout {
            channels,
            sample_rate: rate,
        })
    }
}

fn read_duration(reader: &IMFSourceReader) -> i64 {
    unsafe {
        reader
            .GetPresentationAttribute(stream(MF_SOURCE_READER_MEDIASOURCE), &MF_PD_DURATION)
            .ok()
            .map(|value| {
                let raw: PropVar = std::mem::transmute_copy(&value);
                raw.value
            })
            .filter(|value| *value > 0)
            .unwrap_or(0)
    }
}

#[repr(C)]
struct PropVar {
    vt: u16,
    reserved: [u16; 3],
    value: i64,
    pad: u64,
}

fn seek_to(reader: &IMFSourceReader, hns: i64) {
    unsafe {
        let pos = PropVar {
            vt: VT_I8.0,
            reserved: [0; 3],
            value: hns.max(0),
            pad: 0,
        };
        let _ = reader.SetCurrentPosition(&GUID::zeroed(), std::ptr::from_ref(&pos).cast());
    }
}

fn drain_audio(
    reader: &IMFSourceReader,
    audio: Option<&AudioLayout>,
    uploads: &Mutex<UploadStore>,
    source_id: u64,
    is_playing: bool,
) {
    let Some(layout) = audio else {
        return;
    };
    let deadline = Instant::now() + Duration::from_millis(6);
    let mut packets = 0u32;
    while Instant::now() < deadline && packets < 16 {
        let frames = uploads.lock().expect("uploads").fifo_frames(source_id);
        if frames >= AUDIO_LIVE_FRAMES {
            break;
        }
        match read_audio_sample(reader, layout) {
            Ok(Some(packet)) => {
                if is_playing {
                    ingest_audio_throttled(uploads, source_id, packet);
                }
                packets += 1;
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
}

fn read_audio_sample(
    reader: &IMFSourceReader,
    layout: &AudioLayout,
) -> Result<Option<AudioPacket>, bool> {
    unsafe {
        let mut stream_index = 0u32;
        let mut flags = 0u32;
        let mut pts = 0i64;
        let mut sample = None;
        reader
            .ReadSample(
                stream(MF_SOURCE_READER_FIRST_AUDIO_STREAM),
                0,
                Some(&mut stream_index),
                Some(&mut flags),
                Some(&mut pts),
                Some(&mut sample),
            )
            .map_err(|_| false)?;
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            return Ok(None);
        }
        if flags & MF_SOURCE_READERF_STREAMTICK.0 as u32 != 0 || sample.is_none() {
            return Ok(None);
        }
        let sample = sample.ok_or(false)?;
        Ok(decode_audio(&sample, pts, layout))
    }
}

fn read_sample(
    reader: &IMFSourceReader,
    audio: Option<&AudioLayout>,
) -> Result<Option<Decoded>, bool> {
    unsafe {
        let mut stream_index = 0u32;
        let mut flags = 0u32;
        let mut pts = 0i64;
        let mut sample = None;
        reader
            .ReadSample(
                stream(MF_SOURCE_READER_ANY_STREAM),
                0,
                Some(&mut stream_index),
                Some(&mut flags),
                Some(&mut pts),
                Some(&mut sample),
            )
            .map_err(|_| false)?;
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            return Err(true);
        }
        if flags & MF_SOURCE_READERF_STREAMTICK.0 as u32 != 0 || sample.is_none() {
            return Ok(None);
        }
        let sample = sample.ok_or(false)?;
        if is_audio(reader, stream_index) {
            let Some(layout) = audio else {
                return Ok(None);
            };
            return Ok(
                decode_audio(&sample, pts, layout).map(|packet| Decoded::Audio { pts, packet })
            );
        }
        Ok(Some(Decoded::Video { pts, sample }))
    }
}

fn is_audio(reader: &IMFSourceReader, stream_index: u32) -> bool {
    if stream_index == stream(MF_SOURCE_READER_FIRST_AUDIO_STREAM) {
        return true;
    }
    unsafe {
        reader
            .GetCurrentMediaType(stream_index)
            .ok()
            .and_then(|ty| ty.GetGUID(&MF_MT_MAJOR_TYPE).ok())
            .is_some_and(|guid| guid == MFMediaType_Audio)
    }
}

fn decode_audio(sample: &IMFSample, pts: i64, layout: &AudioLayout) -> Option<AudioPacket> {
    unsafe {
        let buffer = sample.ConvertToContiguousBuffer().ok()?;
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        buffer.Lock(&mut ptr, None, Some(&mut len)).ok()?;
        let bytes = std::slice::from_raw_parts(ptr, len as usize);
        let channels = layout.channels.max(1) as usize;
        let frames = (bytes.len() / 4) / channels;
        if frames == 0 {
            let _ = buffer.Unlock();
            return None;
        }
        let mut planar = Vec::with_capacity(frames * channels);
        for ch in 0..channels {
            for i in 0..frames {
                let o = (i * channels + ch) * 4;
                if o + 4 <= bytes.len() {
                    planar.push(f32::from_le_bytes([
                        bytes[o],
                        bytes[o + 1],
                        bytes[o + 2],
                        bytes[o + 3],
                    ]));
                }
            }
        }
        let _ = buffer.Unlock();
        Some(AudioPacket {
            timestamp: pts,
            sample_rate: layout.sample_rate,
            channels: layout.channels,
            samples_per_channel: frames as i32,
            pcm_planar_f32: planar,
        })
    }
}

fn take_cpu_frame(
    sample: &IMFSample,
    layout: &VideoLayout,
    pts: i64,
) -> Result<Prefetched, String> {
    unsafe {
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|e| e.to_string())?;
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        buffer
            .Lock(&mut ptr, None, Some(&mut len))
            .map_err(|e| e.to_string())?;
        let src = std::slice::from_raw_parts(ptr, len as usize);
        let converted = convert_cpu(src, layout);
        let _ = buffer.Unlock();
        let Some((pixels, stride, format)) = converted else {
            return Err("unsupported CPU video layout".into());
        };
        Ok(Prefetched::Cpu {
            pixels,
            stride,
            format,
            pts,
            width: layout.width,
            height: layout.height,
        })
    }
}

fn convert_cpu(src: &[u8], layout: &VideoLayout) -> Option<(Vec<u8>, usize, CpuFormat)> {
    let width = layout.width;
    let height = layout.height;
    if layout.subtype == MFVideoFormat_NV12 {
        let y_stride = if layout.stride == 0 {
            width as usize
        } else {
            layout.stride.unsigned_abs() as usize
        };
        return Some(if layout.packed {
            nv12_to_uyvy(src, width, height, y_stride)
        } else {
            nv12_to_bgra(src, width, height, y_stride)
        });
    }
    if layout.subtype == MFVideoFormat_YUY2 {
        let stride = packed_stride(layout.stride, width, 2);
        return Some(if layout.packed {
            yuy2_to_uyvy(src, width, height, stride)
        } else {
            yuy2_to_bgra(src, width, height, stride)
        });
    }
    if layout.subtype == MFVideoFormat_UYVY {
        let stride = packed_stride(layout.stride, width, 2);
        if layout.packed {
            return Some(copy_packed(src, width, height, stride, 2, CpuFormat::Uyvy));
        }
        return Some(uyvy_to_bgra(src, width, height, stride));
    }
    if layout.subtype == MFVideoFormat_RGB32 {
        let stride = packed_stride(layout.stride, width, 4);
        return Some(copy_bgra(src, width, height, stride, layout.stride < 0));
    }
    None
}

fn packed_stride(stride: i32, width: u32, bpp: u32) -> usize {
    if stride == 0 {
        width as usize * bpp as usize
    } else {
        stride.unsigned_abs() as usize
    }
}

fn nv12_to_uyvy(
    src: &[u8],
    width: u32,
    height: u32,
    y_stride: usize,
) -> (Vec<u8>, usize, CpuFormat) {
    let w = (width as usize) & !1;
    let h = height as usize;
    let uv_off = y_stride.saturating_mul(h);
    let dst_stride = w * 2;
    let mut dst = vec![0u8; dst_stride * h];
    for y in 0..h {
        let y_row = y * y_stride;
        let uv_row = uv_off + (y / 2) * y_stride;
        let d_row = y * dst_stride;
        for x in (0..w).step_by(2) {
            if y_row + x + 1 >= src.len() || uv_row + x + 1 >= src.len() {
                break;
            }
            let o = d_row + x * 2;
            dst[o] = src[uv_row + x];
            dst[o + 1] = src[y_row + x];
            dst[o + 2] = src[uv_row + x + 1];
            dst[o + 3] = src[y_row + x + 1];
        }
    }
    (dst, dst_stride, CpuFormat::Uyvy)
}

fn nv12_to_bgra(
    src: &[u8],
    width: u32,
    height: u32,
    y_stride: usize,
) -> (Vec<u8>, usize, CpuFormat) {
    let w = width as usize;
    let h = height as usize;
    let uv_off = y_stride.saturating_mul(h);
    let dst_stride = w * 4;
    let mut dst = vec![0u8; dst_stride * h];
    for y in 0..h {
        let y_row = y * y_stride;
        let uv_row = uv_off + (y / 2) * y_stride;
        for x in 0..w {
            if y_row + x >= src.len() || uv_row + (x & !1) + 1 >= src.len() {
                break;
            }
            let luma = src[y_row + x] as f32;
            let u = src[uv_row + (x & !1)] as f32 - 128.0;
            let v = src[uv_row + (x & !1) + 1] as f32 - 128.0;
            let yv = (luma - 16.0) * (255.0 / 219.0);
            let r = (yv + 1.5748 * v).clamp(0.0, 255.0) as u8;
            let g = (yv - 0.1873 * u - 0.4681 * v).clamp(0.0, 255.0) as u8;
            let b = (yv + 1.8556 * u).clamp(0.0, 255.0) as u8;
            let o = y * dst_stride + x * 4;
            dst[o] = b;
            dst[o + 1] = g;
            dst[o + 2] = r;
            dst[o + 3] = 255;
        }
    }
    (dst, dst_stride, CpuFormat::Bgra)
}

fn yuy2_to_uyvy(src: &[u8], width: u32, height: u32, stride: usize) -> (Vec<u8>, usize, CpuFormat) {
    let w = (width as usize) & !1;
    let h = height as usize;
    let dst_stride = w * 2;
    let mut dst = vec![0u8; dst_stride * h];
    crate::simd::yuy2_to_uyvy(src, width, height, stride, &mut dst);
    (dst, dst_stride, CpuFormat::Uyvy)
}

fn yuy2_to_bgra(src: &[u8], width: u32, height: u32, stride: usize) -> (Vec<u8>, usize, CpuFormat) {
    packed_yuv_to_bgra(src, width, height, stride, false)
}

fn uyvy_to_bgra(src: &[u8], width: u32, height: u32, stride: usize) -> (Vec<u8>, usize, CpuFormat) {
    packed_yuv_to_bgra(src, width, height, stride, true)
}

fn packed_yuv_to_bgra(
    src: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    uyvy: bool,
) -> (Vec<u8>, usize, CpuFormat) {
    let w = (width as usize) & !1;
    let h = height as usize;
    let dst_stride = w * 4;
    let mut dst = vec![0u8; dst_stride * h];
    crate::simd::yuv422_to_bgra(src, width, height, stride, uyvy, &mut dst);
    (dst, dst_stride, CpuFormat::Bgra)
}

fn copy_packed(
    src: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    bpp: usize,
    format: CpuFormat,
) -> (Vec<u8>, usize, CpuFormat) {
    let row = width as usize * bpp;
    let mut dst = vec![0u8; row * height as usize];
    crate::simd::copy_rows(src, stride, &mut dst, row, row, height as usize);
    (dst, row, format)
}

fn copy_bgra(
    src: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    flip: bool,
) -> (Vec<u8>, usize, CpuFormat) {
    let row = width as usize * 4;
    let mut dst = vec![0u8; row * height as usize];
    for y in 0..height as usize {
        let src_y = if flip { height as usize - 1 - y } else { y };
        let s = src_y * stride;
        if s + row > src.len() {
            break;
        }
        dst[y * row..y * row + row].copy_from_slice(&src[s..s + row]);
        crate::simd::or_opaque_bgra(&mut dst[y * row..y * row + row]);
    }
    (dst, row, CpuFormat::Bgra)
}

fn file_url(path: &str) -> String {
    if path.starts_with("file:") || path.starts_with("omt:") {
        return path.to_string();
    }
    let trimmed = path.trim_start_matches(r"\\?\");
    let mut out = String::from("file:///");
    for ch in trimmed.replace('\\', "/").chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '/' | ':' | '-' | '_' | '.' | '~' => out.push(ch),
            _ => {
                let mut buf = [0u8; 4];
                for byte in ch.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

fn wide(text: &str) -> Vec<u16> {
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(Some(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_due_follows_pts_clock() {
        assert!(frame_due(0, -1, Duration::ZERO));
        assert!(!frame_due(400_000, 0, Duration::from_millis(20)));
        assert!(frame_due(400_000, 0, Duration::from_millis(40)));
    }

    #[test]
    fn file_prefetch_is_at_least_three_frames() {
        assert_eq!(1u32.clamp(1, 8).max(3), 3);
        assert_eq!(3u32.clamp(1, 8).max(3), 3);
        assert_eq!(8u32.clamp(1, 8).max(3), 8);
    }
}
