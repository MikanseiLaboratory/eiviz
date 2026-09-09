use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

enum NdiSendCmd {
    Video {
        packed: Arc<[u8]>,
        width: i32,
        height: i32,
        stride: i32,
        pts: i64,
        fps_num: i32,
        fps_den: i32,
    },
    Audio(AudioPacket),
    Pump,
    Shutdown,
}

use grafton_ndi::{
    AudioFrame, BorrowedVideoFrame, Finder, FinderOptions, LineStrideOrSize, NDI, PixelFormat,
    Receiver, ReceiverBandwidth, ReceiverColorFormat, ReceiverOptions, ScanType, Sender,
    SenderOptions, Source, SourceAddress, VideoFrame,
};

use crate::abi::FMT_BGRA;
use crate::upload::{
    AudioPacket, CpuFormat, GpuIngest, GpuUploadRing, GpuVideoFrame, UploadStore,
    ingest_audio_throttled, write_slot,
};

static RUNTIME: OnceLock<Result<NDI, String>> = OnceLock::new();
static FINDER: OnceLock<Result<Finder, String>> = OnceLock::new();
/// The NDI SDK invalidates `NDIlib_find_get_current_sources` on the next call
/// to the same instance. grafton-ndi copies immediately but does not serialize
/// overlapping calls, so we lock around Finder use.
static FINDER_OP: Mutex<()> = Mutex::new(());

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
        Finder::new(
            ndi,
            &FinderOptions::builder().show_local_sources(true).build(),
        )
        .map_err(|error| error.to_string())
    }) {
        Ok(finder) => Ok(finder),
        Err(error) => Err(error.clone()),
    }
}

fn with_finder<T>(f: impl FnOnce(&Finder) -> Result<T, String>) -> Result<T, String> {
    let finder = finder()?;
    let _guard = FINDER_OP
        .lock()
        .map_err(|_| "ndi finder lock".to_string())?;
    f(finder)
}

pub fn warm_finder() {
    let _ = with_finder(|finder| {
        let _ = finder.wait_for_sources(Duration::from_secs(2));
        Ok(())
    });
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
        gpu: Option<GpuIngest>,
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
                let mut gpu_ring = GpuUploadRing::new();
                #[cfg(windows)]
                let mut ingest_ring: Option<crate::rebar::FrameIngestRing> = None;
                let mut gpu_warned = false;
                while !stop_thread.load(Ordering::Relaxed) {
                    match receiver.video().try_capture(Duration::from_millis(4)) {
                        Ok(Some(frame)) => ingest_video(
                            &uploads,
                            gpu.as_ref(),
                            &mut gpu_ring,
                            #[cfg(windows)]
                            &mut ingest_ring,
                            &mut gpu_warned,
                            source_id,
                            depth,
                            &frame,
                        ),
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
            crate::diag::join_timeout(join, Duration::from_secs(2), "ndi-recv");
        }
    }
}

pub struct NdiSender {
    cmd_tx: Option<SyncSender<NdiSendCmd>>,
    worker: Option<JoinHandle<()>>,
    pack_ms: f32,
    sdk_ms: f32,
}

impl NdiSender {
    pub fn start(name: &str) -> Result<Self, String> {
        let ndi = runtime()?;
        // Video stays clocked so the source stays visible when no audio is
        // sent (None / silent Master). Audio is not clocked here: grafton-ndi
        // requires one clock, and clock_audio on the send path blocked after
        // GPU encode (issue 141). One worker owns Sender so video/audio never
        // alias a Rust &mut across threads.
        let options = SenderOptions::builder(name)
            .clock_video(true)
            .clock_audio(false)
            .build();
        let mut sender = Sender::new(ndi, &options).map_err(|error| error.to_string())?;
        let (cmd_tx, cmd_rx) = sync_channel::<NdiSendCmd>(16);
        let worker = thread::Builder::new()
            .name("eiviz-ndi-send".into())
            .spawn(move || {
                let mut inflight: Option<Arc<[u8]>> = None;
                while let Ok(cmd) = cmd_rx.recv() {
                    match cmd {
                        NdiSendCmd::Shutdown => break,
                        NdiSendCmd::Pump => {
                            let _ = sender.connection_count(Duration::ZERO);
                        }
                        NdiSendCmd::Audio(audio) => {
                            if let Ok(frame) = build_audio_frame(&audio) {
                                sender.send_audio(&frame);
                            }
                        }
                        NdiSendCmd::Video {
                            packed,
                            width,
                            height,
                            stride,
                            pts,
                            fps_num,
                            fps_den,
                        } => {
                            // SAFETY: packed is tightly packed UYVY at width*2.
                            // The slice lives through send_video_async, then
                            // stays in inflight until the next submit or drop.
                            let borrowed = unsafe {
                                BorrowedVideoFrame::from_parts_unchecked(
                                    packed.as_ref(),
                                    width,
                                    height,
                                    PixelFormat::UYVY.into(),
                                    fps_num,
                                    fps_den,
                                    width as f32 / height.max(1) as f32,
                                    ScanType::Progressive,
                                    pts,
                                    LineStrideOrSize::LineStrideBytes(stride),
                                    None,
                                    pts,
                                )
                            };
                            let token = sender.send_video_async(&borrowed);
                            // Forget the token so Drop does not flush/wait.
                            // The next async submit waits for this buffer.
                            std::mem::forget(token);
                            inflight = Some(packed);
                        }
                    }
                }
                drop(inflight);
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            cmd_tx: Some(cmd_tx),
            worker: Some(worker),
            pack_ms: 0.0,
            sdk_ms: 0.0,
        })
    }

    fn cmds(&self) -> Result<&SyncSender<NdiSendCmd>, String> {
        self.cmd_tx
            .as_ref()
            .ok_or_else(|| "ndi sender stopped".into())
    }

    pub fn pump(&mut self) -> Result<bool, String> {
        // Always encode. connection_count can stay 0 on macOS until a receiver
        // has already seen a source, so gating on it hides the sender entirely.
        match self.cmds()?.try_send(NdiSendCmd::Pump) {
            Ok(()) | Err(TrySendError::Full(_)) => Ok(true),
            Err(TrySendError::Disconnected(_)) => Err("ndi sender stopped".into()),
        }
    }

    pub fn send_video_uyvy(
        &mut self,
        width: u32,
        height: u32,
        stride: u32,
        pts: i64,
        pixels: Arc<[u8]>,
        fps_num: u32,
        fps_den: u32,
    ) -> Result<(), String> {
        let timed = crate::diag::profile_send();
        let t0 = timed.then(std::time::Instant::now);
        let packed = packed_uyvy(width, height, stride, pixels);
        if let Some(t0) = t0 {
            self.pack_ms = t0.elapsed().as_secs_f32() * 1000.0;
        }
        let width_i = width.max(1) as i32;
        let height_i = height.max(1) as i32;
        let packed_stride = width_i.saturating_mul(2);
        let t1 = timed.then(std::time::Instant::now);
        self.cmds()?
            .send(NdiSendCmd::Video {
                packed,
                width: width_i,
                height: height_i,
                stride: packed_stride,
                pts,
                fps_num: fps_num.max(1) as i32,
                fps_den: fps_den.max(1) as i32,
            })
            .map_err(|_| "ndi sender stopped".to_string())?;
        if let Some(t1) = t1 {
            self.sdk_ms = t1.elapsed().as_secs_f32() * 1000.0;
        }
        Ok(())
    }

    pub fn last_send_ms(&self) -> (f32, f32) {
        (self.pack_ms, self.sdk_ms)
    }

    pub fn send_audio(&mut self, audio: &AudioPacket) -> Result<(), String> {
        if audio.samples_per_channel <= 0 || audio.pcm_planar_f32.is_empty() {
            return Ok(());
        }
        match self.cmds()?.try_send(NdiSendCmd::Audio(audio.clone())) {
            Ok(()) | Err(TrySendError::Full(_)) => Ok(()),
            Err(TrySendError::Disconnected(_)) => Err("ndi sender stopped".into()),
        }
    }
}

impl Drop for NdiSender {
    fn drop(&mut self) {
        if let Some(tx) = self.cmd_tx.take() {
            let _ = tx.send(NdiSendCmd::Shutdown);
        }
        if let Some(handle) = self.worker.take() {
            crate::diag::join_timeout(handle, Duration::from_secs(2), "ndi-send");
        }
    }
}

fn build_audio_frame(audio: &AudioPacket) -> Result<AudioFrame, String> {
    let channels = audio.channels.max(1);
    let samples = audio.samples_per_channel.max(1);
    let floats = &audio.pcm_planar_f32;
    if floats.is_empty() {
        return Err("empty audio".into());
    }
    let expected = channels as usize * samples as usize;
    let data = if floats.len() == expected {
        floats.clone()
    } else {
        let n = floats.len().min(expected);
        let mut trimmed = vec![0.0f32; expected];
        trimmed[..n].copy_from_slice(&floats[..n]);
        trimmed
    };
    AudioFrame::builder()
        .sample_rate(audio.sample_rate.max(1))
        .channels(channels)
        .samples(samples)
        .timestamp(audio.timestamp)
        .timecode(audio.timestamp)
        .data(data)
        .build()
        .map_err(|error| error.to_string())
}

fn open_receiver(
    ndi: &NDI,
    source: &Source,
    source_id: u64,
    low_bandwidth: bool,
) -> Result<Receiver, String> {
    let options = ReceiverOptions::builder(source.clone())
        .color(ReceiverColorFormat::UYVY_BGRA)
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
    let sources = with_finder(|finder| {
        let snapshot = finder
            .current_sources()
            .map_err(|error| error.to_string())?;
        if !snapshot.is_empty() {
            return Ok(snapshot);
        }
        // mDNS / unicast responders trickle in. A single wait_for_sources
        // returns on the first change and can miss the rest.
        finder
            .find_sources(Duration::from_secs(5))
            .map_err(|error| error.to_string())
    })?;
    Ok(names_of(sources))
}

/// Snapshot only. Control-plane discover must not block live ops for 5s.
pub fn current_source_names() -> Result<Vec<String>, String> {
    with_finder(|finder| {
        finder
            .current_sources()
            .map_err(|error| error.to_string())
            .map(names_of)
    })
}

fn names_of(sources: Vec<Source>) -> Vec<String> {
    sources
        .into_iter()
        .map(|source| source.to_string())
        .collect()
}

fn resolve_source(query: &str) -> Result<Source, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("NDI source name is empty".into());
    }
    if let Ok(sources) =
        with_finder(|finder| finder.current_sources().map_err(|error| error.to_string()))
    {
        if let Some(source) = sources.iter().find(|source| {
            source.to_string().eq_ignore_ascii_case(trimmed)
                || source.name.eq_ignore_ascii_case(trimmed)
        }) {
            return Ok(source.clone());
        }
        let needle = trimmed.to_ascii_lowercase();
        let mut matches = sources.into_iter().filter(|source| {
            source.to_string().to_ascii_lowercase().contains(&needle)
                || source.name.to_ascii_lowercase().contains(&needle)
        });
        if let (Some(only), None) = (matches.next(), matches.next()) {
            return Ok(only);
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

fn ingest_video(
    uploads: &Mutex<UploadStore>,
    gpu: Option<&GpuIngest>,
    gpu_ring: &mut GpuUploadRing,
    #[cfg(windows)] ingest_ring: &mut Option<crate::rebar::FrameIngestRing>,
    gpu_warned: &mut bool,
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
    if let Some(gpu) = gpu.filter(|gpu| gpu.ndi_gpu.load(Ordering::Relaxed)) {
        #[cfg(windows)]
        if gpu.use_rebar.load(Ordering::Relaxed) && gpu.rebar_available {
            if ingest_ring.is_none() {
                *ingest_ring = crate::rebar::FrameIngestRing::new(&gpu.device, &gpu.queue);
            }
            if let Some(ring) = ingest_ring.as_mut().filter(|ring| ring.is_live()) {
                let packed = matches!(format, CpuFormat::Uyvy | CpuFormat::Uyva);
                let bgra = format == CpuFormat::Bgra;
                let tex_format = if packed {
                    wgpu::TextureFormat::Rgba8Unorm
                } else if bgra {
                    wgpu::TextureFormat::Bgra8Unorm
                } else {
                    wgpu::TextureFormat::Rgba8Unorm
                };
                match ring.upload(
                    frame.data(),
                    stride,
                    width as usize * bpp,
                    width,
                    height,
                    packed,
                    bgra,
                    tex_format,
                    frame.timestamp(),
                ) {
                    Ok(uploaded) => {
                        finish_gpu_frame(
                            uploads,
                            gpu_warned,
                            source_id,
                            depth,
                            width,
                            height,
                            pixel_format,
                            stride,
                            frame,
                            uploaded,
                            ring.vram_bytes(),
                            "host",
                        );
                        return;
                    }
                    Err(error) => {
                        eprintln!(
                            "eiviz ndi host-visible upload: {error}; falling back to write_texture"
                        );
                    }
                }
            }
        }
        match gpu_ring.upload(
            gpu,
            frame.data(),
            stride,
            width,
            height,
            format,
            frame.timestamp(),
        ) {
            Ok(uploaded) => {
                finish_gpu_frame(
                    uploads,
                    gpu_warned,
                    source_id,
                    depth,
                    width,
                    height,
                    pixel_format,
                    stride,
                    frame,
                    uploaded,
                    gpu_ring.vram_bytes(),
                    "queue",
                );
                return;
            }
            Err(error) if !*gpu_warned => {
                eprintln!("eiviz ndi gpu upload: {error}; falling back to CPU frames");
                *gpu_warned = true;
            }
            Err(_) => {}
        }
    }
    let opaque_x = matches!(pixel_format, PixelFormat::BGRX | PixelFormat::RGBX);
    let (mut pixels, format, width, height) = {
        let mut store = uploads.lock().expect("uploads lock");
        match store.take_playout_buf(source_id, width, height, format, depth) {
            Some(ready) => ready,
            None => return,
        }
    };
    write_slot(
        &mut pixels,
        frame.data(),
        stride,
        width,
        height,
        format,
        opaque_x,
    );
    let mut store = uploads.lock().expect("uploads lock");
    store.finish_playout_cpu(source_id, pixels, frame.timestamp());
}

fn finish_gpu_frame(
    uploads: &Mutex<UploadStore>,
    gpu_warned: &mut bool,
    source_id: u64,
    depth: u32,
    width: u32,
    height: u32,
    pixel_format: PixelFormat,
    stride: usize,
    frame: &VideoFrame,
    uploaded: GpuVideoFrame,
    ring_vram: u64,
    path: &str,
) {
    if !*gpu_warned {
        eprintln!(
            "ndi gpu {source_id} {width}x{height} {pixel_format:?} stride={stride} bytes={} path={path}",
            frame.data().len()
        );
        *gpu_warned = true;
    }
    let mut store = uploads.lock().expect("uploads lock");
    store.ensure_playout(source_id, width, height, CpuFormat::GpuRgba, depth);
    store.set_ring_vram(source_id, ring_vram);
    let _ = store.push_playout_gpu(source_id, uploaded);
}

fn to_audio(frame: &AudioFrame) -> AudioPacket {
    let channels = frame.num_channels().max(1);
    let samples = frame.num_samples().max(1);
    let pcm = frame.data().to_vec();
    AudioPacket {
        timestamp: frame.timestamp(),
        sample_rate: frame.sample_rate().max(1),
        channels,
        samples_per_channel: samples,
        pcm_planar_f32: pcm,
    }
}

fn packed_stride_bytes(width: u32) -> usize {
    width.max(1) as usize * 2
}

fn uyvy_is_packed(width: u32, height: u32, stride: u32, pixels: &[u8]) -> bool {
    let packed_stride = packed_stride_bytes(width);
    let src_stride = stride.max(packed_stride as u32) as usize;
    src_stride == packed_stride && pixels.len() >= packed_stride * height.max(1) as usize
}

/// Packed UYVY (stride = width*2) is sent as-is. Padded rows are copied once.
fn packed_uyvy(width: u32, height: u32, stride: u32, pixels: Arc<[u8]>) -> Arc<[u8]> {
    if uyvy_is_packed(width, height, stride, &pixels) {
        pixels
    } else {
        Arc::from(pack_uyvy(width, height, stride, &pixels))
    }
}

fn pack_uyvy(width: u32, height: u32, stride: u32, pixels: &[u8]) -> Vec<u8> {
    let width = width.max(1);
    let height = height.max(1);
    let packed_stride = packed_stride_bytes(width);
    let src_stride = stride.max(packed_stride as u32) as usize;
    let mut packed = vec![0u8; packed_stride * height as usize];
    crate::simd::copy_rows(
        pixels,
        src_stride,
        &mut packed,
        packed_stride,
        packed_stride,
        height as usize,
    );
    packed
}

#[cfg(test)]
mod tests {
    use super::{pack_uyvy, packed_uyvy};
    use std::sync::Arc;

    #[test]
    fn packed_uyvy_reuses_arc_when_already_packed() {
        let width = 4u32;
        let height = 2u32;
        let stride = 8u32;
        let src: Arc<[u8]> = vec![1u8; (stride * height) as usize].into();
        let packed = packed_uyvy(width, height, stride, Arc::clone(&src));
        assert!(Arc::ptr_eq(&src, &packed));
    }

    #[test]
    fn packed_uyvy_copies_padded_stride() {
        let width = 4u32;
        let height = 2u32;
        let stride = 16u32;
        let mut src = vec![0u8; (stride * height) as usize];
        src[0..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        src[16..24].copy_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]);
        let src: Arc<[u8]> = src.into();
        let packed = packed_uyvy(width, height, stride, Arc::clone(&src));
        assert!(!Arc::ptr_eq(&src, &packed));
        assert_eq!(
            packed.as_ref(),
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
    }

    #[test]
    fn pack_uyvy_strips_padded_stride() {
        let width = 4u32;
        let height = 2u32;
        let stride = 16u32;
        let mut src = vec![0u8; (stride * height) as usize];
        src[0..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        src[16..24].copy_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]);
        let packed = pack_uyvy(width, height, stride, &src);
        assert_eq!(
            packed,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
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

    #[test]
    fn send_audio_does_not_block_on_sample_clock() {
        use std::time::{Duration, Instant};

        let name = format!("eiviz-ndi-audio-clock-{}", std::process::id());
        let mut sender = super::NdiSender::start(&name).expect("ndi sender");
        let samples = 48_000i32;
        let packet = crate::upload::AudioPacket {
            timestamp: 0,
            sample_rate: 48_000,
            channels: 2,
            samples_per_channel: samples,
            pcm_planar_f32: vec![0.0; samples as usize * 2],
        };
        let start = Instant::now();
        sender.send_audio(&packet).expect("send 1");
        sender.send_audio(&packet).expect("send 2");
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(400),
            "NDI send_audio must not clock to sample rate (took {elapsed:?})"
        );
    }
}
