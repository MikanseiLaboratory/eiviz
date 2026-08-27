use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows::Win32::Media::MediaFoundation::{
    IMFSample, IMFSourceReader, MFAudioFormat_Float, MFCreateAttributes,
    MFCreateDeviceSource, MFCreateMediaType, MFCreateSourceReaderFromMediaSource,
    MFCreateSourceReaderFromURL, MFMediaType_Audio, MFMediaType_Video, MFStartup, MFVideoFormat_NV12,
    MFVideoFormat_RGB32, MFSTARTUP_NOSOCKET,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, MF_MT_AUDIO_NUM_CHANNELS,
    MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_PD_DURATION, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SOURCE_READERF_ENDOFSTREAM,
    MF_SOURCE_READERF_STREAMTICK, MF_SOURCE_READER_ANY_STREAM, MF_SOURCE_READER_D3D_MANAGER,
    MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READER_MEDIASOURCE,
    MF_VERSION,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::VT_I8;
use windows::core::{GUID, PCWSTR};

use crate::dxgi::GpuVideoContext;
use crate::upload::{AudioPacket, UploadStore};

static MF_ONCE: Once = Once::new();

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MixerVideoInfo {
    pub playing: u32,
    pub is_file: u32,
    pub position_hns: i64,
    pub duration_hns: i64,
}

pub struct VideoPump {
    stop: Arc<AtomicBool>,
    playing: Arc<AtomicBool>,
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
        let seek_hns = Arc::new(AtomicI64::new(-1));
        let position_hns = Arc::new(AtomicI64::new(0));
        let duration_hns = Arc::new(AtomicI64::new(0));
        let stop_t = Arc::clone(&stop);
        let playing_t = Arc::clone(&playing);
        let seek_t = Arc::clone(&seek_hns);
        let pos_t = Arc::clone(&position_hns);
        let dur_t = Arc::clone(&duration_hns);
        let join = thread::Builder::new()
            .name(format!("eiviz-mf-{source_id}"))
            .spawn(move || {
                if let Err(error) = run_loop(
                    source_id,
                    path,
                    capture,
                    format,
                    uploads,
                    gpu,
                    stop_t,
                    playing_t,
                    seek_t,
                    pos_t,
                    dur_t,
                ) {
                    eprintln!("eiviz video: {error}");
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stop,
            playing,
            seek_hns,
            position_hns,
            duration_hns,
            is_file: !capture,
            join: Some(join),
        })
    }

    pub fn set_playing(&self, playing: bool) {
        self.playing.store(playing, Ordering::Relaxed);
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
    seek_hns: Arc<AtomicI64>,
    position_hns: Arc<AtomicI64>,
    duration_hns: Arc<AtomicI64>,
) -> Result<(), String> {
    startup()?;
    let _ = format;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let reader = open_reader(&path, capture, &gpu)?;
        configure_video(&reader)?;
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
        let mut clock_pts = -1i64;
        let mut seek_base = 0i64;
        let clock = Instant::now();
        let mut clock_start = clock;
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
            }
            if !playing.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(16));
                continue;
            }
            let sample = match read_sample(&reader, audio.as_ref()) {
                Ok(Some(sample)) => sample,
                Ok(None) => continue,
                Err(end) if end => break,
                Err(_) => continue,
            };
            match sample {
                Decoded::Audio { pts, packet } => {
                    if clock_pts >= 0 || capture {
                        uploads.lock().expect("uploads").ingest_audio(source_id, packet);
                    }
                    let _ = pts;
                }
                Decoded::Video { pts, sample } => {
                    if clock_pts < 0 {
                        clock_pts = pts;
                        clock_start = Instant::now();
                    }
                    position_hns.store(seek_base + (pts - clock_pts).max(0), Ordering::Relaxed);
                    if !capture {
                        let wait = Duration::from_nanos((pts - clock_pts).max(0) as u64 * 100)
                            .saturating_sub(clock_start.elapsed());
                        if wait > Duration::ZERO && wait < Duration::from_secs(2) {
                            thread::sleep(wait);
                        }
                    }
                    match gpu.dxgi.import_sample(&gpu, &sample, pts) {
                        Ok(frame) => {
                            let _ = uploads.lock().expect("uploads").push_gpu(source_id, frame);
                        }
                        Err(error) => eprintln!("eiviz video gpu: {error}"),
                    }
                }
            }
        }
        if capture {
            thread::sleep(Duration::from_millis(40));
        }
    }
}

enum Decoded {
    Video {
        pts: i64,
        sample: IMFSample,
    },
    Audio {
        pts: i64,
        packet: AudioPacket,
    },
}

fn startup() -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    MF_ONCE.call_once(|| {
        unsafe {
            let _ = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);
        }
    });
    Ok(())
}

fn stream(value: windows::Win32::Media::MediaFoundation::MF_SOURCE_READER_CONSTANTS) -> u32 {
    value.0 as u32
}

fn open_reader(path: &str, capture: bool, gpu: &GpuVideoContext) -> Result<IMFSourceReader, String> {
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 6).map_err(|e| e.to_string())?;
        let attrs = attrs.ok_or("MF attributes")?;
        let _ = attrs.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, &gpu.dxgi.manager);
        let _ = attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);
        let _ = attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1);
        let _ = attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1);
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
        let url = file_url(path);
        let wide = wide(&url);
        MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), &attrs).map_err(|e| e.to_string())
    }
}

fn configure_video(reader: &IMFSourceReader) -> Result<(), String> {
    unsafe {
        let _ = reader.SetStreamSelection(stream(MF_SOURCE_READER_ANY_STREAM), false);
        reader
            .SetStreamSelection(stream(MF_SOURCE_READER_FIRST_VIDEO_STREAM), true)
            .map_err(|e| e.to_string())?;
        let _ = reader.SetStreamSelection(stream(MF_SOURCE_READER_FIRST_AUDIO_STREAM), true);
        if set_video_subtype(reader, MFVideoFormat_NV12).is_ok() {
            return Ok(());
        }
        if set_video_subtype(reader, MFVideoFormat_RGB32).is_ok() {
            return Ok(());
        }
        Err("Media Foundation could not provide a GPU-backed NV12 or RGB32 type".into())
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
        let rate = current.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(48_000) as i32;
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

fn read_sample(reader: &IMFSourceReader, audio: Option<&AudioLayout>) -> Result<Option<Decoded>, bool> {
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
            return Ok(decode_audio(&sample, pts, layout).map(|packet| Decoded::Audio { pts, packet }));
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

fn file_url(path: &str) -> String {
    if path.starts_with("file:") || path.starts_with("omt:") {
        path.to_string()
    } else {
        Path::new(path)
            .canonicalize()
            .ok()
            .and_then(|p| url::from_path(&p))
            .unwrap_or_else(|| path.to_string())
    }
}

fn wide(text: &str) -> Vec<u16> {
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(Some(0))
        .collect()
}

mod url {
    use std::path::Path;

    pub fn from_path(path: &Path) -> Option<String> {
        let raw = path.to_str()?;
        Some(format!("file:///{}", raw.replace('\\', "/")))
    }
}
