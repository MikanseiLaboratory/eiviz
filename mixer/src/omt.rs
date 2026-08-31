use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use openmediatransport::{
    Codec, DecodedAudioFrame, Discovery, FrameType, GpuVideoContext, MediaFrame, Quality,
    ReceiverConfig, ReceiverSession, Sender, Tally, VideoTextureMeta,
};

use crate::abi::FMT_BGRA;
use crate::device::GpuDevice;
use crate::save::debounce_want_full;
use crate::upload::{
    ingest_audio_throttled, ingest_cpu_frame, AudioPacket, CpuFormat, GpuIngest, GpuUploadRing,
    GpuVideoFrame, UploadStore,
};

pub type OmtGpu = GpuVideoContext;

pub fn omt_gpu_from_device(device: &GpuDevice) -> OmtGpu {
    GpuVideoContext {
        device: Arc::new(device.device.clone()),
        queue: Arc::new(device.queue.clone()),
    }
}

pub struct OmtReceiver {
    stop: Arc<AtomicBool>,
    want_full: Arc<AtomicBool>,
    on_program: Arc<AtomicBool>,
    on_preview: Arc<AtomicBool>,
    quality: Arc<AtomicU32>,
    join: Option<JoinHandle<()>>,
}

impl OmtReceiver {
    pub fn start(
        source_id: u64,
        address: String,
        uploads: Arc<Mutex<UploadStore>>,
        gpu: Option<OmtGpu>,
        ingest: GpuIngest,
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
        let session = connect_receiver(&address, config)?;
        let stop = Arc::new(AtomicBool::new(false));
        let want_full = Arc::new(AtomicBool::new(true));
        let on_program = Arc::new(AtomicBool::new(false));
        let on_preview = Arc::new(AtomicBool::new(false));
        let quality_atom = Arc::new(AtomicU32::new(quality_to_abi(quality)));
        let stop_thread = Arc::clone(&stop);
        let want_full_thread = Arc::clone(&want_full);
        let on_program_thread = Arc::clone(&on_program);
        let on_preview_thread = Arc::clone(&on_preview);
        let quality_thread = Arc::clone(&quality_atom);
        let join = thread::Builder::new()
            .name(format!("eiviz-omt-{source_id}"))
            .spawn(move || {
                {
                    let mut store = uploads.lock().expect("uploads lock");
                    let format = if use_gpu {
                        CpuFormat::GpuRgba
                    } else {
                        CpuFormat::from_abi(FMT_BGRA).expect("BGRA")
                    };
                    store.ensure_playout(source_id, 16, 16, format, depth);
                }
                let mut sent: Option<(bool, bool, bool, u32)> = None;
                let mut drop_full_at: Option<Instant> = None;
                let mut gpu_ring = GpuUploadRing::new();
                #[cfg(windows)]
                let mut rebar_ring: Option<crate::rebar::RebarIngestRing> = None;
                let mut gpu_warned = false;
                while !stop_thread.load(Ordering::Relaxed) {
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
                    if use_gpu {
                        if let Some(frame) =
                            session.recv_video_gpu_timeout(Duration::from_millis(4))
                        {
                            let width = frame.width.max(2);
                            let height = frame.height.max(2);
                            let skip_copy = ingest.omt_skip_jitter.load(Ordering::Relaxed);
                            let gpu_frame = if !skip_copy && depth > 1 {
                                if let Some(ctx) = gpu.as_ref() {
                                    copy_gpu_frame(
                                        ctx,
                                        &frame.texture,
                                        width,
                                        height,
                                        frame.timestamp,
                                    )
                                } else {
                                    gpu_frame_from_omt(frame)
                                }
                            } else {
                                gpu_frame_from_omt(frame)
                            };
                            let mut store = uploads.lock().expect("uploads lock");
                            store.ensure_playout(
                                source_id,
                                gpu_frame.width,
                                gpu_frame.height,
                                CpuFormat::GpuRgba,
                                depth,
                            );
                            store.push_playout_gpu(source_id, gpu_frame).ok();
                        }
                    } else if let Some(frame) = session.recv_video_timeout(Duration::from_millis(4))
                    {
                        let width = frame.width.max(2);
                        let height = frame.height.max(2);
                        let stride = frame.stride.max(frame.width * 4) as usize;
                        let cpu_ingest = ingest.omt_cpu_ingest.load(Ordering::Relaxed);
                        if cpu_ingest {
                            ingest_cpu_frame(
                                &uploads,
                                Some(&ingest),
                                true,
                                &mut gpu_ring,
                                #[cfg(windows)]
                                &mut rebar_ring,
                                &mut gpu_warned,
                                source_id,
                                depth,
                                &frame.pixels,
                                stride,
                                width,
                                height,
                                CpuFormat::from_abi(FMT_BGRA).expect("BGRA"),
                                frame.timestamp,
                                false,
                                "omt",
                            );
                        } else {
                            let mut store = uploads.lock().expect("uploads lock");
                            store.ensure_playout(
                                source_id,
                                width,
                                height,
                                CpuFormat::from_abi(FMT_BGRA).expect("BGRA"),
                                depth,
                            );
                            store
                                .push_playout_cpu(source_id, &frame.pixels, stride, frame.timestamp)
                                .ok();
                        }
                    }
                    while let Some(audio) = session.try_recv_audio() {
                        ingest_audio_throttled(&uploads, source_id, to_audio(audio));
                    }
                }
                session.disconnect();
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stop,
            want_full,
            on_program,
            on_preview,
            quality: quality_atom,
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
}

impl Drop for OmtReceiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub struct ProgramSender {
    pub sender: Sender,
    pub unit_id: u64,
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
        Ok(Self {
            sender,
            unit_id: 0,
            name: name.to_string(),
            discovery,
        })
    }

    pub fn pump(&mut self) -> Result<(), String> {
        self.sender.poll_accept().map_err(|e| e.to_string())?;
        self.sender
            .poll_peer_metadata()
            .map_err(|e| e.to_string())?;
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
        let data = pixels.to_vec();
        let frame = MediaFrame {
            frame_type: FrameType::VIDEO,
            timestamp: pts,
            codec: Codec::Uyvy as i32,
            width: width as i32,
            height: height as i32,
            stride: stride as i32,
            frame_rate_n: fps_num as i32,
            frame_rate_d: fps_den as i32,
            aspect_ratio: width as f32 / height.max(1) as f32,
            data,
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
        let frame = MediaFrame {
            frame_type: FrameType::AUDIO,
            timestamp: audio.timestamp,
            codec: Codec::Fpa1 as i32,
            sample_rate: audio.sample_rate,
            channels: audio.channels,
            samples_per_channel: audio.samples_per_channel,
            data: audio.pcm_planar_f32.clone(),
            ..Default::default()
        };
        self.sender.send_audio(frame).map_err(|e| e.to_string())
    }
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
        && let Some(source) = discovery.sources().iter().find(|source| {
            source.to_url() == trimmed
                || source.to_string() == trimmed
                || source.instance_name() == trimmed
        })
    {
        return ReceiverSession::connect_from_address(source, config).map_err(|e| e.to_string());
    }
    if trimmed.starts_with("omt://") {
        return ReceiverSession::connect(trimmed, config).map_err(|e| e.to_string());
    }
    Err(format!("OMT source not found: {trimmed}"))
}

fn gpu_frame_from_omt(frame: openmediatransport::DecodedVideoGpuFrame) -> GpuVideoFrame {
    GpuVideoFrame {
        pts: frame.timestamp,
        width: frame.width.max(2),
        height: frame.height.max(2),
        packed: false,
        bgra: false,
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
    crate::device::submit_ingest(&ctx.queue, Some(encoder.finish()));
    GpuVideoFrame {
        pts,
        width,
        height,
        packed: false,
        bgra: false,
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
    AudioPacket {
        timestamp: frame.timestamp,
        sample_rate: frame.sample_rate,
        channels: frame.channels,
        samples_per_channel: samples,
        pcm_planar_f32: frame.pcm_planar_f32.to_vec(),
    }
}
