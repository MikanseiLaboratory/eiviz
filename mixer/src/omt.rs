use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use openmediatransport::{
    AudioIngress, Codec, DecodedAudioFrame, Discovery, FrameType, GpuVideoContext, MediaFrame,
    Quality, ReceiverConfig, ReceiverSession, Sender, Tally, VideoTextureMeta,
};

use crate::abi::FMT_BGRA;
use crate::device::GpuDevice;
use crate::save::debounce_want_full;
use crate::upload::{AudioPacket, CpuFormat, GpuVideoFrame, UploadStore, ingest_audio_throttled};

pub type OmtGpu = GpuVideoContext;

pub fn omt_gpu_from_device(device: &GpuDevice) -> OmtGpu {
    GpuVideoContext {
        device: Arc::new(device.device.clone()),
        queue: Arc::new(device.queue.clone()),
        gpu_lock: Some(crate::device::gpu_queue_lock_handle()),
    }
}

/// Send-side VMX encode must not hold the compose `Queue::submit` lock across
/// GPU readback. Receive still uses [`omt_gpu_from_device`] so decode cannot
/// race `Surface::configure`.
pub fn omt_gpu_for_send(device: &GpuDevice) -> OmtGpu {
    GpuVideoContext {
        device: Arc::new(device.device.clone()),
        queue: Arc::new(device.queue.clone()),
        gpu_lock: None,
    }
}

pub struct OmtReceiver {
    stop: Arc<AtomicBool>,
    want_full: Arc<AtomicBool>,
    on_program: Arc<AtomicBool>,
    on_preview: Arc<AtomicBool>,
    quality: Arc<AtomicU32>,
    session_live: Arc<AtomicBool>,
    has_video: Arc<AtomicBool>,
    last_error: Arc<Mutex<String>>,
    join: Option<JoinHandle<()>>,
}

impl OmtReceiver {
    pub fn start(
        source_id: u64,
        address: String,
        uploads: Arc<Mutex<UploadStore>>,
        gpu: Option<OmtGpu>,
        frame_buffer_frames: u32,
        quality: u32,
    ) -> Result<Self, String> {
        let depth = frame_buffer_frames.clamp(1, 8);
        let use_gpu = gpu.is_some();
        let quality = quality_from_abi(quality);
        let config = ReceiverConfig {
            frame_types: FrameType::VIDEO | FrameType::AUDIO,
            connect_timeout: Duration::from_secs(5),
            gpu: gpu.clone(),
            quality,
            ..ReceiverConfig::default()
        };
        let stop = Arc::new(AtomicBool::new(false));
        let want_full = Arc::new(AtomicBool::new(true));
        let on_program = Arc::new(AtomicBool::new(false));
        let on_preview = Arc::new(AtomicBool::new(false));
        let quality_atom = Arc::new(AtomicU32::new(quality_to_abi(quality)));
        let session_live = Arc::new(AtomicBool::new(false));
        let has_video = Arc::new(AtomicBool::new(false));
        let last_error = Arc::new(Mutex::new(String::new()));
        let stop_thread = Arc::clone(&stop);
        let want_full_thread = Arc::clone(&want_full);
        let on_program_thread = Arc::clone(&on_program);
        let on_preview_thread = Arc::clone(&on_preview);
        let quality_thread = Arc::clone(&quality_atom);
        let session_live_thread = Arc::clone(&session_live);
        let has_video_thread = Arc::clone(&has_video);
        let last_error_thread = Arc::clone(&last_error);
        let join = thread::Builder::new()
            .name(format!("eiviz-omt-{source_id}"))
            .spawn(move || {
                let format = if use_gpu {
                    CpuFormat::GpuRgba
                } else {
                    CpuFormat::from_abi(FMT_BGRA).expect("BGRA")
                };
                loop {
                    if stop_thread.load(Ordering::Relaxed) || crate::diag::is_fatal() {
                        return;
                    }
                    let session = match connect_receiver(&address, config.clone()) {
                        Ok(session) => {
                            session_live_thread.store(true, Ordering::Relaxed);
                            if let Ok(mut slot) = last_error_thread.lock() {
                                slot.clear();
                            }
                            session
                        }
                        Err(error) => {
                            session_live_thread.store(false, Ordering::Relaxed);
                            let message = format!("omt_connect id={source_id}: {error}");
                            crate::diag::error(&message);
                            if let Ok(mut slot) = last_error_thread.lock() {
                                *slot = message;
                            }
                            wait_stop(&stop_thread, Duration::from_secs(1));
                            continue;
                        }
                    };
                    if stop_thread.load(Ordering::Relaxed) {
                        session.disconnect();
                        return;
                    }
                    {
                        let mut store = uploads.lock().expect("uploads lock");
                        store.ensure_playout(source_id, 16, 16, format, depth);
                    }
                    let mut sent: Option<(bool, bool, bool, u32)> = None;
                    let mut drop_full_at: Option<Instant> = None;
                    let mut pts_seq = 0i64;
                    let live_since = Instant::now();
                    let mut missing_video_logged = false;
                    let run = panic::catch_unwind(AssertUnwindSafe(|| {
                        while !stop_thread.load(Ordering::Relaxed) && !crate::diag::is_fatal() {
                            let full = debounce_want_full(
                                want_full_thread.load(Ordering::Relaxed),
                                &mut drop_full_at,
                            );
                            apply_omt_save(
                                &session,
                                full,
                                on_program_thread.load(Ordering::Relaxed),
                                on_preview_thread.load(Ordering::Relaxed),
                                quality_from_abi(quality_thread.load(Ordering::Relaxed)),
                                &mut sent,
                            );
                            if let Some(error) = session.last_error() {
                                store_omt_error(&last_error_thread, source_id, error);
                            }
                            if use_gpu {
                                if let Some(frame) =
                                    session.recv_video_gpu_timeout(Duration::from_millis(4))
                                {
                                    let width = frame.width.max(2);
                                    let height = frame.height.max(2);
                                    let pts = next_pts(&mut pts_seq, frame.timestamp);
                                    let gpu_frame = if depth > 1 {
                                        if let Some(ctx) = gpu.as_ref() {
                                            copy_gpu_frame(ctx, &frame.texture, width, height, pts)
                                        } else {
                                            gpu_frame_from_omt(frame, pts)
                                        }
                                    } else {
                                        gpu_frame_from_omt(frame, pts)
                                    };
                                    let mut store = uploads.lock().expect("uploads lock");
                                    store.ensure_playout(
                                        source_id,
                                        gpu_frame.width,
                                        gpu_frame.height,
                                        CpuFormat::GpuRgba,
                                        depth,
                                    );
                                    match store.push_playout_gpu(source_id, gpu_frame) {
                                        Ok(()) => has_video_thread.store(true, Ordering::Relaxed),
                                        Err(error) => {
                                            let message =
                                                format!("omt_push_gpu id={source_id}: {error}");
                                            crate::diag::error(&message);
                                            if let Ok(mut slot) = last_error_thread.lock() {
                                                *slot = message;
                                            }
                                        }
                                    }
                                }
                            } else if let Some(frame) =
                                session.recv_video_timeout(Duration::from_millis(4))
                            {
                                let mut store = uploads.lock().expect("uploads lock");
                                store.ensure_playout(
                                    source_id,
                                    frame.width.max(2),
                                    frame.height.max(2),
                                    CpuFormat::from_abi(FMT_BGRA).expect("BGRA"),
                                    depth,
                                );
                                let stride = frame.stride.max(frame.width * 4) as usize;
                                let pts = next_pts(&mut pts_seq, frame.timestamp);
                                store
                                    .push_playout_cpu(source_id, &frame.pixels, stride, pts)
                                    .map(|()| has_video_thread.store(true, Ordering::Relaxed))
                                    .unwrap_or_else(|error| {
                                        let message =
                                            format!("omt_push_cpu id={source_id}: {error}");
                                        crate::diag::error(&message);
                                        if let Ok(mut slot) = last_error_thread.lock() {
                                            *slot = message;
                                        }
                                    });
                            }
                            if !missing_video_logged
                                && !has_video_thread.load(Ordering::Relaxed)
                                && live_since.elapsed() >= Duration::from_secs(2)
                            {
                                missing_video_logged = true;
                                let kind = if use_gpu { "GPU" } else { "CPU" };
                                let detail = session.last_error().unwrap_or_else(|| {
                                    format!("connected, no {kind} video frames")
                                });
                                store_omt_error(&last_error_thread, source_id, detail);
                            }
                            while let Some(audio) = session.try_recv_audio() {
                                ingest_audio_throttled(&uploads, source_id, to_audio(audio));
                            }
                        }
                        session.disconnect();
                        session_live_thread.store(false, Ordering::Relaxed);
                    }));
                    if run.is_err() {
                        crate::diag::mark_fatal(format!("omt recv panicked id={source_id}"));
                        return;
                    }
                    if stop_thread.load(Ordering::Relaxed) || crate::diag::is_fatal() {
                        return;
                    }
                    wait_stop(&stop_thread, Duration::from_millis(500));
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stop,
            want_full,
            on_program,
            on_preview,
            quality: quality_atom,
            session_live,
            has_video,
            last_error,
            join: Some(join),
        })
    }

    pub fn apply_save(&self, full: bool, on_program: bool, on_preview: bool) {
        self.want_full.store(full, Ordering::Relaxed);
        self.on_program.store(on_program, Ordering::Relaxed);
        self.on_preview.store(on_preview, Ordering::Relaxed);
    }

    pub fn set_quality(&self, quality: u32) {
        self.quality
            .store(quality_to_abi(quality_from_abi(quality)), Ordering::Relaxed);
    }

    pub fn session_live(&self) -> bool {
        self.session_live.load(Ordering::Relaxed)
    }

    pub fn has_video(&self) -> bool {
        self.has_video.load(Ordering::Relaxed)
    }

    pub fn last_error(&self) -> String {
        self.last_error
            .lock()
            .map(|slot| slot.clone())
            .unwrap_or_default()
    }
}

impl Drop for OmtReceiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            crate::diag::join_timeout(join, Duration::from_secs(2), "omt-recv");
        }
    }
}

pub struct ProgramSender {
    sender: Sender,
    encoder: VmxEncoder,
    audio_ingress: AudioIngress,
    name: String,
    discovery: Option<Discovery>,
}

impl ProgramSender {
    pub fn start(name: &str) -> Result<Self, String> {
        let sender = Sender::create(name, FrameType::VIDEO | FrameType::AUDIO)
            .map_err(|error| error.to_string())?;
        let port = sender.port();
        let advertised = name.to_string();
        let discovery = panic::catch_unwind(AssertUnwindSafe(|| {
            Discovery::new().ok().and_then(|mut discovery| {
                discovery.register(&advertised, port).ok()?;
                Some(discovery)
            })
        }))
        .ok()
        .flatten();
        let audio_ingress = sender.audio_ingress();
        Ok(Self {
            sender,
            encoder: VmxEncoder::new(),
            audio_ingress,
            name: name.to_string(),
            discovery,
        })
    }

    pub fn audio_ingress(&self) -> AudioIngress {
        self.audio_ingress.clone()
    }

    /// Non-blocking accept only. Safe around GPU encode (no peer-lock reads).
    pub fn pump_accept(&mut self) -> Result<(), String> {
        self.sender.poll_accept().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn pump(&mut self) -> Result<(), String> {
        self.sender.poll_accept().map_err(|e| e.to_string())?;
        self.sender
            .poll_peer_metadata()
            .map_err(|e| e.to_string())?;
        self.sender.enable_audio_output();
        Ok(())
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
        let bitstream = self.encoder.encode_uyvy(&pixels, width, height, stride)?;
        self.send_video_vmx1(width, height, pts, bitstream.into(), fps_num, fps_den)
    }

    pub fn send_video_vmx1(
        &mut self,
        width: u32,
        height: u32,
        pts: i64,
        bitstream: Arc<[u8]>,
        fps_num: u32,
        fps_den: u32,
    ) -> Result<(), String> {
        let frame = MediaFrame {
            frame_type: FrameType::VIDEO,
            timestamp: pts,
            codec: Codec::Vmx1 as i32,
            width: width as i32,
            height: height as i32,
            stride: 0,
            frame_rate_n: fps_num as i32,
            frame_rate_d: fps_den as i32,
            aspect_ratio: width as f32 / height.max(1) as f32,
            data: bitstream.to_vec(),
            ..Default::default()
        };
        self.sender.send_video(frame).map_err(|e| e.to_string())
    }

    pub fn send_video_texture(
        &mut self,
        ctx: &OmtGpu,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
        pts: i64,
        fps_num: u32,
        fps_den: u32,
    ) -> Result<(), String> {
        let meta = VideoTextureMeta {
            width,
            height,
            timestamp: pts,
            frame_rate_n: fps_num as i32,
            frame_rate_d: fps_den as i32,
            ..Default::default()
        };
        self.sender
            .send_video_texture(ctx, texture, meta)
            .map_err(|e| e.to_string())
    }

    pub fn video_subscribed(&self) -> bool {
        self.sender.video_subscribed()
    }

    pub fn send_audio(&mut self, audio: &AudioPacket) -> Result<(), String> {
        send_audio_via_ingress(&self.audio_ingress, audio)
    }
}

pub(crate) fn audio_packet_to_frame(audio: &AudioPacket) -> Option<MediaFrame> {
    if audio.samples_per_channel <= 0 || audio.pcm_planar_f32.is_empty() {
        return None;
    }
    Some(MediaFrame {
        frame_type: FrameType::AUDIO,
        timestamp: audio.timestamp,
        codec: Codec::Fpa1 as i32,
        sample_rate: audio.sample_rate,
        channels: audio.channels,
        samples_per_channel: audio.samples_per_channel,
        data: audio
            .pcm_planar_f32
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect(),
        ..Default::default()
    })
}

pub(crate) fn send_audio_via_ingress(
    ingress: &AudioIngress,
    audio: &AudioPacket,
) -> Result<(), String> {
    let Some(frame) = audio_packet_to_frame(audio) else {
        return Ok(());
    };
    ingress.send(frame).map_err(|e| e.to_string())
}

pub(crate) fn omt_audio_send(ingress: AudioIngress) -> Arc<dyn Fn(AudioPacket) + Send + Sync> {
    Arc::new(move |packet| {
        let _ = send_audio_via_ingress(&ingress, &packet);
    })
}

impl Drop for ProgramSender {
    fn drop(&mut self) {
        if let Some(discovery) = self.discovery.as_mut() {
            let _ = discovery.deregister(&self.name);
        }
    }
}

const SEND_SLOTS: usize = 3;

struct GpuSendSlot {
    texture: wgpu::Texture,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    busy: Arc<AtomicBool>,
}

struct GpuSendRing {
    slots: Vec<GpuSendSlot>,
    next: usize,
}

#[derive(Default)]
pub struct GpuSendStore {
    rings: HashMap<u64, GpuSendRing>,
}

impl GpuSendStore {
    pub fn copy(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        output_id: u64,
        src: &wgpu::Texture,
    ) -> Option<(wgpu::Texture, u32, u32, Arc<AtomicBool>)> {
        if !src.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return None;
        }
        let size = src.size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        if width < 16 || height < 16 {
            return None;
        }
        let format = src.format();
        let ring = self
            .rings
            .entry(output_id)
            .or_insert_with(|| GpuSendRing::new(device, width, height, format));
        for i in 0..SEND_SLOTS {
            let idx = (ring.next + i) % ring.slots.len();
            let slot = &mut ring.slots[idx];
            if slot.busy.load(Ordering::Acquire) {
                continue;
            }
            if slot.width != width || slot.height != height || slot.format != format {
                *slot = GpuSendSlot::new(device, width, height, format);
            }
            encoder.copy_texture_to_texture(
                src.as_image_copy(),
                slot.texture.as_image_copy(),
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            slot.busy.store(true, Ordering::Release);
            let texture = slot.texture.clone();
            let busy = Arc::clone(&slot.busy);
            let slot_count = ring.slots.len();
            ring.next = (idx + 1) % slot_count;
            return Some((texture, width, height, busy));
        }
        None
    }

    pub fn vram_bytes(&self) -> u64 {
        self.rings
            .values()
            .flat_map(|ring| ring.slots.iter())
            .map(|slot| crate::upload::texture_bytes(&slot.texture))
            .sum()
    }
}

impl GpuSendRing {
    fn new(device: &GpuDevice, width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self {
            slots: (0..SEND_SLOTS)
                .map(|_| GpuSendSlot::new(device, width, height, format))
                .collect(),
            next: 0,
        }
    }
}

impl GpuSendSlot {
    fn new(device: &GpuDevice, width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        let texture = device.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("eiviz omt gpu send"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Self {
            texture,
            width,
            height,
            format,
            busy: Arc::new(AtomicBool::new(false)),
        }
    }
}

fn quality_from_abi(value: u32) -> Quality {
    match value {
        1 => Quality::Low,
        50 => Quality::Medium,
        100 => Quality::High,
        _ => Quality::Default,
    }
}

fn quality_to_abi(quality: Quality) -> u32 {
    match quality {
        Quality::Low => 1,
        Quality::Medium => 50,
        Quality::High => 100,
        Quality::Default => 0,
    }
}

/// Ask the crate to switch preview / quality / tally. Preview and quality are
/// sent as separate protocol tokens inside `set_preview` / `set_suggested_quality`.
fn apply_omt_save(
    session: &ReceiverSession,
    full: bool,
    on_program: bool,
    on_preview: bool,
    quality: Quality,
    sent: &mut Option<(bool, bool, bool, u32)>,
) {
    let next = (full, on_program, on_preview, quality_to_abi(quality));
    if *sent == Some(next) {
        return;
    }
    *sent = Some(next);
    let _ = session.set_preview(!full);
    let _ = session.set_suggested_quality(quality);
    let _ = session.set_tally(Tally::new(i32::from(on_preview), i32::from(on_program)));
}

pub fn discover_addresses() -> Result<Vec<String>, String> {
    let mut discovery = Discovery::new().map_err(|e| e.to_string())?;
    discovery.refresh().map_err(|e| e.to_string())?;
    Ok(discovery
        .sources()
        .into_iter()
        .map(|source| source.to_url())
        .collect())
}

fn connect_receiver(address: &str, config: ReceiverConfig) -> Result<ReceiverSession, String> {
    let trimmed = address.trim();
    if let Ok(mut discovery) = Discovery::new()
        && discovery.refresh().is_ok()
        && let Some(source) = pick_omt_source(discovery.sources(), trimmed)
    {
        return ReceiverSession::connect_from_address(source, config).map_err(|e| e.to_string());
    }
    if trimmed.starts_with("omt://") || trimmed.contains("://") {
        return ReceiverSession::connect(trimmed, config).map_err(|e| e.to_string());
    }
    Err(format!("OMT source not found: {trimmed}"))
}

fn pick_omt_source<'a>(
    sources: &'a [openmediatransport::OmtAddress],
    query: &str,
) -> Option<&'a openmediatransport::OmtAddress> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    sources
        .iter()
        .find(|source| {
            omt_query_matches(
                query,
                &source.to_url(),
                &source.instance_name(),
                &source.name,
            )
        })
        .or_else(|| {
            let needle = query.to_ascii_lowercase();
            let mut matches = sources.iter().filter(|source| {
                source.to_url().to_ascii_lowercase().contains(&needle)
                    || source
                        .instance_name()
                        .to_ascii_lowercase()
                        .contains(&needle)
                    || source.name.to_ascii_lowercase().contains(&needle)
            });
            match (matches.next(), matches.next()) {
                (Some(only), None) => Some(only),
                _ => None,
            }
        })
}

fn omt_path_name(value: &str) -> &str {
    value
        .rsplit('/')
        .next()
        .filter(|tail| !tail.is_empty())
        .unwrap_or(value)
}

fn omt_query_matches(query: &str, url: &str, instance: &str, name: &str) -> bool {
    let query = query.trim();
    url.eq_ignore_ascii_case(query)
        || instance.eq_ignore_ascii_case(query)
        || name.eq_ignore_ascii_case(query)
        || name.eq_ignore_ascii_case(omt_path_name(query))
        || omt_path_name(url).eq_ignore_ascii_case(omt_path_name(query))
}

fn wait_stop(stop: &AtomicBool, dur: Duration) {
    let deadline = Instant::now() + dur;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) || crate::diag::is_fatal() {
            return;
        }
        let remain = deadline.saturating_duration_since(Instant::now());
        thread::sleep(remain.min(Duration::from_millis(50)));
    }
}

fn next_pts(seq: &mut i64, timestamp: i64) -> i64 {
    let pts = if timestamp > *seq {
        timestamp
    } else {
        seq.saturating_add(1)
    };
    *seq = pts;
    pts
}

fn store_omt_error(slot: &Mutex<String>, source_id: u64, error: String) {
    if error.is_empty() {
        return;
    }
    let message = if error.contains("omt ") || error.starts_with("omt_") {
        error
    } else {
        format!("omt id={source_id}: {error}")
    };
    if let Ok(mut guard) = slot.lock() {
        if *guard == message {
            return;
        }
        *guard = message.clone();
    }
    crate::diag::error(&message);
}

fn gpu_frame_from_omt(frame: openmediatransport::DecodedVideoGpuFrame, pts: i64) -> GpuVideoFrame {
    GpuVideoFrame {
        pts,
        width: frame.width.max(2),
        height: frame.height.max(2),
        packed: false,
        bgra: matches!(
            frame.texture.format(),
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        ),
        view: frame.texture.create_view(&Default::default()),
        texture: frame.texture,
    }
}

fn copy_gpu_frame(
    ctx: &OmtGpu,
    src: &wgpu::Texture,
    width: u32,
    height: u32,
    pts: i64,
) -> GpuVideoFrame {
    let width = width.max(1);
    let height = height.max(1);
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("eiviz omt jitter"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: src.format(),
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("eiviz omt jitter copy"),
        });
    encoder.copy_texture_to_texture(
        src.as_image_copy(),
        texture.as_image_copy(),
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let _guard = crate::device::lock_gpu_queue();
    ctx.queue.submit(Some(encoder.finish()));
    GpuVideoFrame {
        pts,
        width,
        height,
        packed: false,
        bgra: matches!(
            texture.format(),
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        ),
        view: texture.create_view(&Default::default()),
        texture,
    }
}

fn to_audio(frame: DecodedAudioFrame) -> AudioPacket {
    let channels = frame.channels.max(1);
    let samples = if frame.samples_per_channel > 0 {
        frame.samples_per_channel
    } else {
        (frame.pcm_planar_f32.len() as i32 / 4 / channels).max(1)
    };
    let pcm = frame
        .pcm_planar_f32
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    AudioPacket {
        timestamp: frame.timestamp,
        sample_rate: frame.sample_rate,
        channels: frame.channels,
        samples_per_channel: samples,
        pcm_planar_f32: pcm,
    }
}

/// Per-output VMX1 encoder. Lives on the send thread so PCM hold/flush
/// stays on the same call as the video that pairs with it.
struct VmxEncoder {
    codec: Option<vmx::Codec>,
    width: i32,
    height: i32,
    buf: Vec<u8>,
}

impl VmxEncoder {
    fn new() -> Self {
        Self {
            codec: None,
            width: 0,
            height: 0,
            buf: Vec::new(),
        }
    }

    fn encode_uyvy(
        &mut self,
        pixels: &[u8],
        width: u32,
        height: u32,
        stride: u32,
    ) -> Result<Vec<u8>, String> {
        let width = width.max(16);
        let height = height.max(16);
        let packed = width.saturating_mul(2);
        let stride = stride.max(packed) as usize;
        let need = stride.saturating_mul(height as usize);
        if pixels.len() < need {
            return Err(format!(
                "UYVY frame too short: need {need}, have {}",
                pixels.len()
            ));
        }
        self.ensure(width as i32, height as i32)?;
        let vmx = self.codec.as_mut().expect("codec");
        vmx.encode_uyvy(pixels, stride).map_err(|e| e.to_string())?;
        self.save()
    }

    fn ensure(&mut self, width: i32, height: i32) -> Result<(), String> {
        if self.codec.is_some() && self.width == width && self.height == height {
            return Ok(());
        }
        let codec = vmx::Codec::new(vmx::Config {
            width,
            height,
            profile: vmx::Profile::Default,
            color_space: vmx::ColorSpace::Undefined,
        })
        .map_err(|e| e.to_string())?;
        self.codec = Some(codec);
        self.width = width;
        self.height = height;
        Ok(())
    }

    fn save(&mut self) -> Result<Vec<u8>, String> {
        let codec = self.codec.as_mut().expect("codec after encode");
        let pixels = (self.width.max(0) as usize).saturating_mul(self.height.max(0) as usize);
        let min = pixels.saturating_mul(2).max(1 << 20);
        if self.buf.len() < min {
            self.buf.resize(min, 0);
        }
        loop {
            match codec.save_to(&mut self.buf) {
                Ok(n) => return Ok(self.buf[..n].to_vec()),
                Err(vmx::VmxError::OutputTooSmall { need, .. }) => {
                    self.buf
                        .resize(need.max(self.buf.len().saturating_mul(2)), 0);
                }
                Err(vmx::VmxError::BufferOverflow) => {
                    let next = self
                        .buf
                        .len()
                        .saturating_mul(2)
                        .max(self.buf.len().saturating_add(1 << 20));
                    if next <= self.buf.len() {
                        return Err("vmx save buffer overflow".into());
                    }
                    self.buf.resize(next, 0);
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ProgramSender, audio_packet_to_frame, omt_query_matches, send_audio_via_ingress};
    use crate::upload::AudioPacket;
    use openmediatransport::Codec;
    use std::time::{Duration, Instant};

    fn pkt(ts: i64) -> AudioPacket {
        AudioPacket {
            timestamp: ts,
            sample_rate: 48_000,
            channels: 2,
            samples_per_channel: 1,
            pcm_planar_f32: vec![0.0, 0.0],
        }
    }

    #[test]
    fn audio_packet_to_frame_encodes_planar_f32() {
        let packet = AudioPacket {
            timestamp: 42,
            sample_rate: 48_000,
            channels: 2,
            samples_per_channel: 2,
            pcm_planar_f32: vec![0.5, -0.5, 0.25, -0.25],
        };
        let frame = audio_packet_to_frame(&packet).expect("frame");
        assert_eq!(frame.timestamp, 42);
        assert_eq!(frame.codec, Codec::Fpa1 as i32);
        assert_eq!(frame.sample_rate, 48_000);
        assert_eq!(frame.channels, 2);
        assert_eq!(frame.samples_per_channel, 2);
        assert_eq!(frame.data.len(), 16);
        assert!(audio_packet_to_frame(&pkt(0)).is_some());
        assert!(
            audio_packet_to_frame(&AudioPacket {
                samples_per_channel: 0,
                pcm_planar_f32: vec![],
                ..pkt(1)
            })
            .is_none()
        );
    }

    #[test]
    fn omt_audio_ingress_keeps_cadence_while_sender_is_busy() {
        let mut sender =
            ProgramSender::start(&format!("eiviz-audio-ingress-{}", std::process::id()))
                .expect("omt sender");
        let ingress = sender.audio_ingress();
        let _ = sender.pump();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let join = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            let _ = sender.pump_accept();
            let _ = done_tx.send(());
        });
        let start = Instant::now();
        for i in 0..20 {
            send_audio_via_ingress(&ingress, &pkt(i)).expect("ingress send");
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(100),
            "delayed video must not stall PCM ingress, elapsed={elapsed:?}"
        );
        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("busy sender finished");
        join.join().expect("busy sender thread");
    }

    #[test]
    fn quality_from_abi_keeps_protocol_values() {
        use openmediatransport::Quality;
        assert!(matches!(super::quality_from_abi(0), Quality::Default));
        assert!(matches!(super::quality_from_abi(1), Quality::Low));
        assert!(matches!(super::quality_from_abi(50), Quality::Medium));
        assert!(matches!(super::quality_from_abi(100), Quality::High));
        assert!(matches!(super::quality_from_abi(2), Quality::Default));
        assert!(matches!(super::quality_from_abi(3), Quality::Default));
        assert_eq!(super::quality_to_abi(Quality::Default), 0);
        assert_eq!(super::quality_to_abi(Quality::Low), 1);
        assert_eq!(super::quality_to_abi(Quality::Medium), 50);
        assert_eq!(super::quality_to_abi(Quality::High), 100);
    }

    #[test]
    fn omt_query_matches_output_name() {
        assert!(omt_query_matches(
            "eiviz-pgm",
            "omt://studio/eiviz-pgm",
            "STUDIO (eiviz-pgm)",
            "eiviz-pgm"
        ));
        assert!(omt_query_matches(
            "omt://studio/eiviz-pgm",
            "omt://studio/eiviz-pgm",
            "STUDIO (eiviz-pgm)",
            "eiviz-pgm"
        ));
        assert!(!omt_query_matches(
            "eiviz-prv",
            "omt://studio/eiviz-pgm",
            "STUDIO (eiviz-pgm)",
            "eiviz-pgm"
        ));
        assert!(omt_query_matches(
            "omt://studio/eiviz-pgm",
            "omt://192.168.1.10/eiviz-pgm",
            "STUDIO (eiviz-pgm)",
            "eiviz-pgm"
        ));
    }
}
