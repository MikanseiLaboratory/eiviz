use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use grafton_ndi::{
    AudioFrame, Finder, FinderOptions, LineStrideOrSize, PixelFormat, Receiver, ReceiverBandwidth,
    ReceiverColorFormat, ReceiverOptions, Sender, SenderOptions, Source, SourceAddress, VideoFrame,
    NDI,
};

use crate::abi::FMT_BGRA;
use crate::upload::{ingest_audio_throttled, AudioPacket, CpuFormat, UploadStore};

static RUNTIME: OnceLock<Result<NDI, String>> = OnceLock::new();
static FINDER: OnceLock<Result<Finder, String>> = OnceLock::new();

fn runtime() -> Result<&'static NDI, String> {
    match RUNTIME.get_or_init(|| {
        preload_ndi_dylib();
        NDI::new().map_err(|error| format!("NDI runtime: {error}"))
    }) {
        Ok(ndi) => Ok(ndi),
        Err(error) => Err(error.clone()),
    }
}

fn preload_ndi_dylib() {
    #[cfg(target_os = "macos")]
    {
        let mut dirs = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                dirs.push(dir.to_path_buf());
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            dirs.push(cwd);
        }
        for dir in dirs {
            for name in ["libndi.dylib", "libndi.6.dylib"] {
                let path = dir.join(name);
                if !path.is_file() {
                    continue;
                }
                if let Ok(c_path) = std::ffi::CString::new(path.to_string_lossy().as_bytes()) {
                    unsafe {
                        let _ = macos::dlopen(c_path.as_ptr(), 1);
                    }
                    return;
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    unsafe extern "C" {
        pub fn dlopen(path: *const std::ffi::c_char, mode: i32) -> *mut std::ffi::c_void;
    }
}

fn finder() -> Result<&'static Finder, String> {
    match FINDER.get_or_init(|| {
        let ndi = runtime()?;
        Finder::new(ndi, &FinderOptions::builder().show_local_sources(true).build())
            .map_err(|error| error.to_string())
    }) {
        Ok(finder) => Ok(finder),
        Err(error) => Err(error.clone()),
    }
}

pub fn warm_finder() {
    if let Ok(finder) = finder() {
        let _ = finder.wait_for_sources(Duration::from_secs(2));
    }
}

pub struct NdiReceiver {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl NdiReceiver {
    pub fn start(
        source_id: u64,
        address: String,
        uploads: Arc<Mutex<UploadStore>>,
        frame_buffer_frames: u32,
        low_bandwidth: u32,
    ) -> Result<Self, String> {
        let depth = frame_buffer_frames.clamp(1, 8);
        let ndi = runtime()?;
        let source = resolve_source(&address)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        // Bandwidth is chosen at create time. Dynamic Highest/Lowest switching
        // needs Advanced SDK 6.1 `NDIlib_recv_set_bandwidth` (Vendor ID). Implement
        // that path when Advanced SDK is available.
        let receiver = open_receiver(ndi, &source, source_id, low_bandwidth != 0)?;
        let join = thread::Builder::new()
            .name(format!("eiviz-ndi-{source_id}"))
            .spawn(move || {
                {
                    let mut store = uploads.lock().expect("uploads lock");
                    store.ensure_playout(
                        source_id,
                        16,
                        16,
                        CpuFormat::from_abi(FMT_BGRA).expect("BGRA"),
                        depth,
                    );
                }
                while !stop_thread.load(Ordering::Relaxed) {
                    match receiver.video().try_capture(Duration::from_millis(4)) {
                        Ok(Some(frame)) => ingest_video(&uploads, source_id, depth, &frame),
                        Ok(None) => {}
                        Err(_) => {}
                    }
                    loop {
                        match receiver.audio().try_capture(Duration::ZERO) {
                            Ok(Some(audio)) => {
                                ingest_audio_throttled(&uploads, source_id, to_audio(&audio));
                            }
                            _ => break,
                        }
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stop,
            join: Some(join),
        })
    }
}

impl Drop for NdiReceiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub struct NdiSender {
    sender: Sender,
}

impl NdiSender {
    pub fn start(name: &str) -> Result<Self, String> {
        let ndi = runtime()?;
        let options = SenderOptions::builder(name)
            .clock_video(true)
            .clock_audio(true)
            .build();
        let sender = Sender::new(ndi, &options).map_err(|error| error.to_string())?;
        Ok(Self { sender })
    }

    pub fn pump(&mut self) -> Result<bool, String> {
        // Always encode. connection_count can stay 0 on macOS until a receiver
        // has already seen a source, so gating on it hides the sender entirely.
        let _ = self.sender.connection_count(Duration::ZERO);
        Ok(true)
    }

    pub fn send_video_uyvy(
        &mut self,
        width: u32,
        height: u32,
        stride: u32,
        pts: i64,
        pixels: &[u8],
        fps_num: u32,
        fps_den: u32,
    ) -> Result<(), String> {
        let packed = pack_uyvy(width, height, stride, pixels);
        let mut frame = VideoFrame::builder()
            .resolution(width.max(1) as i32, height.max(1) as i32)
            .pixel_format(PixelFormat::UYVY)
            .frame_rate(fps_num.max(1) as i32, fps_den.max(1) as i32)
            .aspect_ratio(width as f32 / height.max(1) as f32)
            .timestamp(pts)
            .timecode(pts)
            .build()
            .map_err(|error| error.to_string())?;
        frame
            .replace_data(packed)
            .map_err(|error| error.to_string())?;
        self.sender.send_video(&frame);
        Ok(())
    }

    pub fn send_audio(&mut self, audio: &AudioPacket) -> Result<(), String> {
        let channels = audio.channels.max(1);
        let samples = audio.samples_per_channel.max(1);
        let floats: Vec<f32> = audio
            .pcm_planar_f32
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();
        if floats.is_empty() {
            return Ok(());
        }
        let expected = channels as usize * samples as usize;
        let data = if floats.len() == expected {
            floats
        } else {
            let n = floats.len().min(expected);
            let mut trimmed = vec![0.0f32; expected];
            trimmed[..n].copy_from_slice(&floats[..n]);
            trimmed
        };
        let frame = AudioFrame::builder()
            .sample_rate(audio.sample_rate.max(1))
            .channels(channels)
            .samples(samples)
            .timestamp(audio.timestamp)
            .timecode(audio.timestamp)
            .data(data)
            .build()
            .map_err(|error| error.to_string())?;
        self.sender.send_audio(&frame);
        Ok(())
    }
}

fn open_receiver(
    ndi: &NDI,
    source: &Source,
    source_id: u64,
    low_bandwidth: bool,
) -> Result<Receiver, String> {
    let options = ReceiverOptions::builder(source.clone())
        .color(ReceiverColorFormat::BGRX_BGRA)
        .bandwidth(if low_bandwidth {
            ReceiverBandwidth::Lowest
        } else {
            ReceiverBandwidth::Highest
        })
        .allow_video_fields(false)
        .name(format!("eiviz-ndi-{source_id}"))
        .build();
    Receiver::new(ndi, &options).map_err(|error| error.to_string())
}

pub fn discover_sources() -> Result<Vec<String>, String> {
    let finder = finder()?;
    let mut sources = finder.current_sources().unwrap_or_default();
    if sources.is_empty() {
        let _ = finder.wait_for_sources(Duration::from_millis(1500));
        sources = finder
            .current_sources()
            .or_else(|_| finder.find_sources(Duration::from_millis(400)))
            .map_err(|error| error.to_string())?;
    }
    Ok(sources.into_iter().map(|source| source.to_string()).collect())
}

fn resolve_source(query: &str) -> Result<Source, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("NDI source name is empty".into());
    }
    if let Ok(finder) = finder() {
        let _ = finder.wait_for_sources(Duration::from_millis(200));
        if let Ok(sources) = finder
            .current_sources()
            .or_else(|_| finder.find_sources(Duration::from_millis(400)))
        {
            if let Some(source) = sources
                .iter()
                .find(|source| source_key(source).eq_ignore_ascii_case(trimmed))
            {
                return Ok(source.clone());
            }
            if let Some(source) = sources
                .iter()
                .find(|source| source.name.eq_ignore_ascii_case(trimmed))
            {
                return Ok(source.clone());
            }
            let matches: Vec<_> = sources
                .iter()
                .filter(|source| {
                    let needle = trimmed.to_ascii_lowercase();
                    source_key(source).to_ascii_lowercase().contains(&needle)
                        || source.name.to_ascii_lowercase().contains(&needle)
                })
                .cloned()
                .collect();
            if matches.len() == 1 {
                return Ok(matches.into_iter().next().expect("len 1"));
            }
        }
    }
    Ok(source_from_query(trimmed))
}

pub(crate) fn source_from_query(query: &str) -> Source {
    let trimmed = query.trim();
    if let Some((name, addr)) = trimmed.rsplit_once('@')
        && !name.is_empty()
        && !addr.is_empty()
    {
        let address = if addr.contains("://") {
            SourceAddress::Url(addr.to_string())
        } else {
            SourceAddress::Ip(addr.to_string())
        };
        return Source {
            name: name.to_string(),
            address,
        };
    }
    Source {
        name: trimmed.to_string(),
        address: SourceAddress::None,
    }
}

fn source_key(source: &Source) -> String {
    source.to_string()
}

fn ingest_video(
    uploads: &Mutex<UploadStore>,
    source_id: u64,
    depth: u32,
    frame: &VideoFrame,
) {
    let width = frame.width().max(2) as u32;
    let height = frame.height().max(2) as u32;
    let pixel_format = frame.pixel_format();
    let format = match pixel_format {
        PixelFormat::UYVY => CpuFormat::Uyvy,
        PixelFormat::RGBA | PixelFormat::RGBX => CpuFormat::Rgba,
        _ => CpuFormat::Bgra,
    };
    let bpp = match format {
        CpuFormat::Uyvy => 2usize,
        _ => 4usize,
    };
    let stride = match frame.line_stride_or_size() {
        LineStrideOrSize::LineStrideBytes(stride) if stride > 0 => stride as usize,
        _ => width as usize * bpp,
    };
    let mut opaque = Vec::new();
    let pixels = if matches!(pixel_format, PixelFormat::BGRX | PixelFormat::RGBX) {
        opaque = frame.data().to_vec();
        for chunk in opaque.chunks_exact_mut(4) {
            if chunk.len() == 4 {
                chunk[3] = 255;
            }
        }
        opaque.as_slice()
    } else {
        frame.data()
    };
    let mut store = uploads.lock().expect("uploads lock");
    store.ensure_playout(source_id, width, height, format, depth);
    let _ = store.push_playout_cpu(source_id, pixels, stride, frame.timestamp());
}

fn to_audio(frame: &AudioFrame) -> AudioPacket {
    let channels = frame.num_channels().max(1);
    let samples = frame.num_samples().max(1);
    let mut pcm = Vec::with_capacity(frame.data().len() * 4);
    for sample in frame.data() {
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    AudioPacket {
        timestamp: frame.timestamp(),
        sample_rate: frame.sample_rate().max(1),
        channels,
        samples_per_channel: samples,
        pcm_planar_f32: pcm,
    }
}

fn pack_uyvy(width: u32, height: u32, stride: u32, pixels: &[u8]) -> Vec<u8> {
    let width = width.max(1);
    let height = height.max(1);
    let packed_stride = width.saturating_mul(2) as usize;
    let src_stride = stride.max(packed_stride as u32) as usize;
    let mut packed = vec![0u8; packed_stride * height as usize];
    for y in 0..height as usize {
        let src = y * src_stride;
        let dst = y * packed_stride;
        let end = src.saturating_add(packed_stride);
        if end <= pixels.len() {
            packed[dst..dst + packed_stride].copy_from_slice(&pixels[src..end]);
        }
    }
    packed
}

#[cfg(test)]
mod tests {
    use super::pack_uyvy;

    #[test]
    fn pack_uyvy_strips_padded_stride() {
        let width = 4u32;
        let height = 2u32;
        let stride = 16u32;
        let mut src = vec![0u8; (stride * height) as usize];
        src[0..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        src[16..24].copy_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]);
        let packed = pack_uyvy(width, height, stride, &src);
        assert_eq!(packed, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    }

    #[test]
    fn source_from_query_keeps_name_and_url() {
        let named = super::source_from_query("CAM 1");
        assert_eq!(named.name, "CAM 1");
        let with_ip = super::source_from_query("DESKTOP (CAM)@192.168.0.10:5960");
        assert_eq!(with_ip.name, "DESKTOP (CAM)");
        match with_ip.address {
            grafton_ndi::SourceAddress::Ip(ip) => assert!(ip.starts_with("192.168.0.10")),
            other => panic!("{other:?}"),
        }
    }
}
