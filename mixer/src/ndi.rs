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
use crate::upload::{
    ingest_audio_throttled, write_slot, AudioPacket, CpuFormat, GpuIngest, GpuUploadRing,
    GpuVideoFrame, UploadStore,
};

static RUNTIME: OnceLock<Result<NDI, String>> = OnceLock::new();
static FINDER: OnceLock<Result<Finder, String>> = OnceLock::new();
/// NDI forbids overlapping `NDIlib_find_get_current_sources` on one instance.
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
        // extra_ips replaces NDI's registry/config list and wants machine IPs
        // ("12.0.0.8,13.0.12.8"), not a CIDR of our own NIC. Passing
        // "192.168.3.0/24" hid LAN mDNS sources. Leave it unset.
        if let Some(ips) = lan_extra_ips() {
            eprintln!("eiviz ndi finder lan={ips} (mDNS/registry; extra_ips unset)");
        }
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

/// Hint NDI at each LAN NIC /24. A machine often has Wi-Fi as the default
/// route plus a dedicated LAN NIC; mDNS on the wrong adapter sees nothing.
/// Only adapter IPv4s are used — subnet masks / gateways / DNS look like
/// IPv4 in `ipconfig` and would replace NDI's registry extra-IP list.
fn lan_extra_ips() -> Option<String> {
    let mut nets = Vec::new();
    push_slash24(&mut nets, default_route_v4());
    if let Ok(output) = std::process::Command::new("ipconfig").output() {
        collect_ipconfig_adapter_v4s(&String::from_utf8_lossy(&output.stdout), &mut nets);
    }
    if nets.is_empty() {
        None
    } else {
        Some(nets.join(","))
    }
}

fn default_route_v4() -> Option<std::net::Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(v4) if usable_lan_v4(v4) => Some(v4),
        _ => None,
    }
}

fn collect_ipconfig_adapter_v4s(text: &str, nets: &mut Vec<String>) {
    for line in text.lines() {
        if !line.contains("IPv4") {
            continue;
        }
        let Some(token) = line.split_whitespace().last() else {
            continue;
        };
        if let Ok(v4) = token.parse::<std::net::Ipv4Addr>() {
            push_slash24(nets, Some(v4));
        }
    }
}

fn usable_lan_v4(v4: std::net::Ipv4Addr) -> bool {
    if v4.is_loopback() || v4.is_link_local() || v4.is_unspecified() || v4.is_multicast() || v4.is_broadcast()
    {
        return false;
    }
    v4.octets()[0] < 224
}

fn push_slash24(nets: &mut Vec<String>, ip: Option<std::net::Ipv4Addr>) {
    let Some(v4) = ip else {
        return;
    };
    if !usable_lan_v4(v4) {
        return;
    }
    let oct = v4.octets();
    let cidr = format!("{}.{}.{}.0/24", oct[0], oct[1], oct[2]);
    if !nets.contains(&cidr) {
        nets.push(cidr);
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
                let mut rebar_ring: Option<crate::rebar::RebarIngestRing> = None;
                let mut gpu_warned = false;
                while !stop_thread.load(Ordering::Relaxed) {
                    match receiver.video().try_capture(Duration::from_millis(4)) {
                        Ok(Some(frame)) => ingest_video(
                            &uploads,
                            gpu.as_ref(),
                            &mut gpu_ring,
                            #[cfg(windows)]
                            &mut rebar_ring,
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
        let snapshot = finder.current_sources().map_err(|error| error.to_string())?;
        if !snapshot.is_empty() {
            return Ok(snapshot);
        }
        // mDNS / unicast responders trickle in. A single wait_for_sources
        // returns on the first change and can miss the rest.
        finder
            .find_sources(Duration::from_secs(5))
            .map_err(|error| error.to_string())
    })?;
    Ok(sources.into_iter().map(|source| source.to_string()).collect())
}

fn resolve_source(query: &str) -> Result<Source, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("NDI source name is empty".into());
    }
    if let Ok(sources) = with_finder(|finder| {
        let _ = finder.wait_for_sources(Duration::from_millis(200));
        finder.current_sources().map_err(|error| error.to_string())
    }) {
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
    gpu: Option<&GpuIngest>,
    gpu_ring: &mut GpuUploadRing,
    #[cfg(windows)] rebar_ring: &mut Option<crate::rebar::RebarIngestRing>,
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
            if rebar_ring.is_none() {
                *rebar_ring = crate::rebar::RebarIngestRing::new(&gpu.device, &gpu.queue);
            }
            if let Some(ring) = rebar_ring.as_mut().filter(|ring| ring.is_live()) {
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
                        finish_gpu_frame(uploads, gpu_warned, source_id, depth, width, height, pixel_format, stride, frame, uploaded, "rebar");
                        return;
                    }
                    Err(error) => {
                        eprintln!("eiviz ndi rebar upload: {error}; falling back to write_texture");
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
                finish_gpu_frame(uploads, gpu_warned, source_id, depth, width, height, pixel_format, stride, frame, uploaded, "queue");
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
    write_slot(&mut pixels, frame.data(), stride, width, height, format, opaque_x);
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
    let _ = store.push_playout_gpu(source_id, uploaded);
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

    #[test]
    fn extra_ips_ignores_masks_gateways_and_apipa() {
        let text = "\
Windows IP Configuration

Ethernet adapter LAN:
   IPv4 Address. . . . . . . . . . . : 192.168.3.34
   Subnet Mask . . . . . . . . . . . : 255.255.255.0
   Default Gateway . . . . . . . . . : 192.168.3.1

Ethernet adapter Bluetooth:
   IPv4 Address. . . . . . . . . . . : 169.254.61.129
   Subnet Mask . . . . . . . . . . . : 255.255.0.0
";
        let mut nets = Vec::new();
        super::collect_ipconfig_adapter_v4s(text, &mut nets);
        assert_eq!(nets, vec!["192.168.3.0/24".to_string()]);
    }

    #[test]
    fn usable_lan_v4_rejects_reserved() {
        assert!(super::usable_lan_v4("192.168.3.34".parse().unwrap()));
        assert!(!super::usable_lan_v4("255.255.255.0".parse().unwrap()));
        assert!(!super::usable_lan_v4("169.254.1.1".parse().unwrap()));
        assert!(!super::usable_lan_v4("127.0.0.1".parse().unwrap()));
        assert!(!super::usable_lan_v4("224.0.0.251".parse().unwrap()));
    }
}
