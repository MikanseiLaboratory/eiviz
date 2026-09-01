use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, Once};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFSample, IMFSourceReader, MFEnumDeviceSources,
    MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, MF_MT_AUDIO_NUM_CHANNELS,
    MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE,
    MF_MT_SUBTYPE, MF_PD_DURATION, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS,
    MF_SOURCE_READER_ANY_STREAM, MF_SOURCE_READER_D3D_MANAGER,
    MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
    MF_SOURCE_READER_MEDIASOURCE, MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_STREAMTICK,
    MF_VERSION, MFAudioFormat_Float, MFCreateAttributes, MFCreateDeviceSource, MFCreateMediaType,
    MFCreateSourceReaderFromMediaSource, MFCreateSourceReaderFromURL, MFMediaType_Audio,
    MFMediaType_Video, MFSTARTUP_NOSOCKET, MFStartup, MFVideoFormat_NV12, MFVideoFormat_RGB32,
    MFVideoFormat_UYVY, MFVideoFormat_YUY2,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::System::Variant::VT_I8;
use windows::core::{GUID, PCWSTR};

use crate::abi::{FMT_BGRA, MixerVideoInfo};
use crate::dxgi::GpuVideoContext;
use crate::upload::{
    AUDIO_LIVE_FRAMES, AUDIO_RATE, AudioPacket, CpuFormat, UploadStore, ingest_audio_clocked,
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
        uploads: Arc<Mutex<UploadStore>>,
        gpu: GpuVideoContext,
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
            .name(format!("eiviz-mf-{source_id}"))
            .spawn(move || {
                if let Err(error) = run_loop(
                    source_id, path, capture, format, uploads, gpu, stop_t, playing_t, looping_t,
                    seek_t, pos_t, dur_t, ready_tx,
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
    format: u32,
    uploads: Arc<Mutex<UploadStore>>,
    gpu: GpuVideoContext,
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
            Ok(reader) => match configure_video(&reader, true, prefer_packed) {
                Ok(layout) => Ok((reader, layout)),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        let (reader, layout) = match opened {
            Ok(pair) => pair,
            Err(gpu_error) => match open_reader(&path, capture, None, prefer_packed) {
                Ok(reader) => match configure_video(&reader, false, prefer_packed) {
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
        let mut clock_pts = -1i64;
        let mut seek_base = 0i64;
        let mut clock_start = Instant::now();
        let mut gpu_warned = false;
        let mut need_frame = false;
        let mut was_playing = false;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let seek = seek_hns.swap(-1, Ordering::Relaxed);
            if seek >= 0 {
                let _ = unsafe { reader.Flush(stream(MF_SOURCE_READER_ANY_STREAM)) };
                seek_to(&reader, seek);
                {
                    let mut store = uploads.lock().expect("uploads");
                    store.flush_audio(source_id);
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
            if !is_playing && !need_frame {
                thread::sleep(Duration::from_millis(16));
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
                    if is_playing && !preview && !capture {
                        drain_audio(&reader, audio.as_ref(), &uploads, source_id, is_playing);
                        let wait = Duration::from_nanos((pts - clock_pts).max(0) as u64 * 100)
                            .saturating_sub(clock_start.elapsed());
                        if wait > Duration::ZERO && wait < Duration::from_secs(2) {
                            let deadline = Instant::now() + wait;
                            while Instant::now() < deadline {
                                drain_audio(
                                    &reader,
                                    audio.as_ref(),
                                    &uploads,
                                    source_id,
                                    is_playing,
                                );
                                let remain = deadline.saturating_duration_since(Instant::now());
                                if remain.is_zero() {
                                    break;
                                }
                                thread::sleep(remain.min(Duration::from_millis(4)));
                            }
                        }
                    }
                    if layout.gpu {
                        match gpu.dxgi.import_sample(&gpu, &sample, pts) {
                            Ok(frame) => {
                                let _ = uploads.lock().expect("uploads").push_gpu(source_id, frame);
                                need_frame = false;
                                continue;
                            }
                            Err(error) if !gpu_warned => {
                                eprintln!("eiviz video gpu: {error}; falling back to CPU frames");
                                gpu_warned = true;
                            }
                            Err(_) => {}
                        }
                    }
                    if let Err(error) = push_cpu_frame(&uploads, source_id, &sample, &layout, pts) {
                        if !gpu_warned {
                            eprintln!("eiviz video cpu: {error}");
                            gpu_warned = true;
                        }
                    }
                    need_frame = false;
                }
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
            match set_video_subtype(reader, *subtype) {
                Ok(()) => return read_video_layout(reader, gpu, prefer_packed),
                Err(error) => last = error,
            }
        }
        Err(last)
    }
}

fn set_video_subtype(reader: &IMFSourceReader, subtype: GUID) -> Result<(), String> {
    unsafe {
        let ty = MFCreateMediaType().map_err(|e| e.to_string())?;
        ty.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|e| e.to_string())?;
        ty.SetGUID(&MF_MT_SUBTYPE, &subtype)
            .map_err(|e| e.to_string())?;
        reader
            .SetCurrentMediaType(stream(MF_SOURCE_READER_FIRST_VIDEO_STREAM), None, &ty)
            .map_err(|e| e.to_string())
    }
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
        let mut planar = Vec::with_capacity(frames * channels * 4);
        for ch in 0..channels {
            for i in 0..frames {
                let o = (i * channels + ch) * 4;
                if o + 4 <= bytes.len() {
                    planar.extend_from_slice(&bytes[o..o + 4]);
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

fn push_cpu_frame(
    uploads: &Mutex<UploadStore>,
    source_id: u64,
    sample: &IMFSample,
    layout: &VideoLayout,
    pts: i64,
) -> Result<(), String> {
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
        let mut store = uploads.lock().expect("uploads");
        store.ensure(source_id, layout.width.max(2), layout.height.max(2), format);
        store.push(source_id, &pixels, stride, pts)
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
    for y in 0..h {
        let s = y * stride;
        let d = y * dst_stride;
        for x in (0..w).step_by(2) {
            let i = s + x * 2;
            let o = d + x * 2;
            if i + 3 >= src.len() {
                break;
            }
            dst[o] = src[i + 1];
            dst[o + 1] = src[i];
            dst[o + 2] = src[i + 3];
            dst[o + 3] = src[i + 2];
        }
    }
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
    for y in 0..h {
        let s = y * stride;
        for x in (0..w).step_by(2) {
            let i = s + x * 2;
            if i + 3 >= src.len() {
                break;
            }
            let (u, y0, v, y1) = if uyvy {
                (
                    src[i] as f32,
                    src[i + 1] as f32,
                    src[i + 2] as f32,
                    src[i + 3] as f32,
                )
            } else {
                (
                    src[i + 1] as f32,
                    src[i] as f32,
                    src[i + 3] as f32,
                    src[i + 2] as f32,
                )
            };
            write_yuv_bgra(&mut dst, y * dst_stride + x * 4, y0, u - 128.0, v - 128.0);
            write_yuv_bgra(
                &mut dst,
                y * dst_stride + (x + 1) * 4,
                y1,
                u - 128.0,
                v - 128.0,
            );
        }
    }
    (dst, dst_stride, CpuFormat::Bgra)
}

fn write_yuv_bgra(dst: &mut [u8], o: usize, luma: f32, u: f32, v: f32) {
    let yv = (luma - 16.0) * (255.0 / 219.0);
    dst[o] = (yv + 1.8556 * u).clamp(0.0, 255.0) as u8;
    dst[o + 1] = (yv - 0.1873 * u - 0.4681 * v).clamp(0.0, 255.0) as u8;
    dst[o + 2] = (yv + 1.5748 * v).clamp(0.0, 255.0) as u8;
    dst[o + 3] = 255;
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
    for y in 0..height as usize {
        let s = y * stride;
        if s + row > src.len() {
            break;
        }
        dst[y * row..y * row + row].copy_from_slice(&src[s..s + row]);
    }
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
        for px in dst[y * row..y * row + row].chunks_exact_mut(4) {
            px[3] = 255;
        }
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
