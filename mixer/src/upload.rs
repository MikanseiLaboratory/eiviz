use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::abi::{FMT_BGRA, FMT_RGBA, FMT_UYVA, FMT_UYVY};

const SLOTS: usize = 3;
pub const AUDIO_RATE: i32 = 48_000;
/// Output / burst cap. Live devices may jitter; keep this well under half a second
/// so a file decoder that runs ahead cannot park 500 ms of audio behind picture.
pub const AUDIO_FIFO_FRAMES: usize = AUDIO_RATE as usize / 12;
const AUDIO_FIFO_HIGH_FRAMES: usize = AUDIO_FIFO_FRAMES * 4 / 5;
/// Target source latency (~40 ms). File pumps must not ingest past this.
pub const AUDIO_LIVE_FRAMES: usize = AUDIO_RATE as usize / 25;
const AUDIO_PRIME_FRAMES: usize = AUDIO_RATE as usize / 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuFormat {
    Uyvy,
    Bgra,
    Rgba,
    Uyva,
    GpuRgba,
}

impl CpuFormat {
    pub fn from_abi(value: u32) -> Option<Self> {
        match value {
            FMT_UYVY => Some(Self::Uyvy),
            FMT_BGRA => Some(Self::Bgra),
            FMT_UYVA => Some(Self::Uyva),
            FMT_RGBA => Some(Self::Rgba),
            _ => None,
        }
    }
}

struct CpuQueuedFrame {
    pts: i64,
    pixels: Vec<u8>,
}

pub struct SourceRing {
    pub width: u32,
    pub height: u32,
    pub format: CpuFormat,
    slots: [Arc<Vec<u8>>; SLOTS],
    write: AtomicUsize,
    pub last_pts: i64,
    pub has_frame: bool,
    pub audio: Option<AudioPacket>,
    pub gpu: Option<GpuVideoFrame>,
    playout_depth: usize,
    cpu_fifo: VecDeque<CpuQueuedFrame>,
    gpu_fifo: VecDeque<GpuVideoFrame>,
    free: Vec<Vec<u8>>,
    fifo: VecDeque<f32>,
    last_peak: (f32, f32),
    last_hold: (f32, f32),
    fifo_primed: bool,
}

/// CPU frame borrowed for GPU upload after the ingest lock is dropped.
#[derive(Clone)]
pub struct CpuFrameSnap {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub format: CpuFormat,
    pub last_pts: i64,
    pub has_frame: bool,
    pub pixels: Arc<Vec<u8>>,
    pub gpu: Option<GpuVideoFrame>,
}

#[derive(Clone, Debug)]
pub struct GpuVideoFrame {
    pub pts: i64,
    pub width: u32,
    pub height: u32,
    pub packed: bool,
    pub bgra: bool,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

/// wgpu device/queue handle for ingest threads. Render only binds the result.
#[derive(Clone)]
pub struct GpuIngest {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

struct GpuUploadSlot {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

/// Triple-buffered textures written on the NDI/OMT thread.
pub struct GpuUploadRing {
    slots: Vec<GpuUploadSlot>,
    next: usize,
}

impl GpuUploadRing {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            next: 0,
        }
    }

    pub fn upload(
        &mut self,
        gpu: &GpuIngest,
        data: &[u8],
        stride: usize,
        width: u32,
        height: u32,
        format: CpuFormat,
        pts: i64,
    ) -> Result<GpuVideoFrame, String> {
        let packed = matches!(format, CpuFormat::Uyvy | CpuFormat::Uyva);
        let bgra = format == CpuFormat::Bgra;
        let bpp = if packed { 2usize } else { 4 };
        let row_bytes = (width as usize).saturating_mul(bpp);
        let tex_w = if packed {
            (width / 2).max(1)
        } else {
            width.max(1)
        };
        let tex_h = height.max(1);
        let tex_format = if packed {
            wgpu::TextureFormat::Rgba8Unorm
        } else if bgra {
            wgpu::TextureFormat::Bgra8Unorm
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        let slot_i = self.ensure(gpu, tex_w, tex_h, tex_format);
        let slot = &self.slots[slot_i];
        write_queue_texture(
            &gpu.queue,
            &slot.texture,
            data,
            stride,
            row_bytes,
            tex_h,
            tex_w,
        );
        self.next = (slot_i + 1) % 3;
        Ok(GpuVideoFrame {
            pts,
            width,
            height,
            packed,
            bgra,
            texture: slot.texture.clone(),
            view: slot.view.clone(),
        })
    }

    fn ensure(
        &mut self,
        gpu: &GpuIngest,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> usize {
        if self.slots.get(self.next).is_some_and(|slot| {
            slot.width == width && slot.height == height && slot.format == format
        }) {
            return self.next;
        }
        if self.slots.len() == 3
            && self
                .slots
                .iter()
                .any(|slot| slot.width != width || slot.height != height || slot.format != format)
        {
            self.slots.clear();
            self.next = 0;
        }
        while self.slots.len() < 3 {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("eiviz ndi gpu"),
                size: wgpu::Extent3d {
                    width,
                    height,
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
            let view = texture.create_view(&Default::default());
            self.slots.push(GpuUploadSlot {
                texture,
                view,
                width,
                height,
                format,
            });
        }
        self.next
    }
}

pub(crate) fn write_queue_texture(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    data: &[u8],
    stride: usize,
    row_bytes: usize,
    height: u32,
    tex_width: u32,
) {
    if row_bytes == 0 || height == 0 {
        return;
    }
    let tight = stride == row_bytes && stride % 256 == 0;
    let (bytes, pitch): (Cow<[u8]>, u32) = if tight {
        let n = row_bytes.saturating_mul(height as usize).min(data.len());
        (Cow::Borrowed(&data[..n]), stride as u32)
    } else if stride >= row_bytes && stride % 256 == 0 {
        let n = stride
            .saturating_mul(height.saturating_sub(1) as usize)
            .saturating_add(row_bytes)
            .min(data.len());
        (Cow::Borrowed(&data[..n]), stride as u32)
    } else {
        let aligned = row_bytes.div_ceil(256) * 256;
        let mut padded = vec![0u8; aligned.saturating_mul(height as usize)];
        for y in 0..height as usize {
            let src = y * stride;
            let dst = y * aligned;
            if src + row_bytes <= data.len() {
                padded[dst..dst + row_bytes].copy_from_slice(&data[src..src + row_bytes]);
            }
        }
        (Cow::Owned(padded), aligned as u32)
    };
    queue.write_texture(
        texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(pitch),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width: tex_width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
    );
}

#[derive(Clone, Debug)]
pub struct AudioPacket {
    pub timestamp: i64,
    pub sample_rate: i32,
    pub channels: i32,
    pub samples_per_channel: i32,
    pub pcm_planar_f32: Vec<u8>,
}

impl SourceRing {
    pub fn new(width: u32, height: u32, format: CpuFormat) -> Self {
        let bytes = slot_bytes(width, height, format);
        Self {
            width,
            height,
            format,
            slots: [
                Arc::new(vec![0u8; bytes]),
                Arc::new(vec![0u8; bytes]),
                Arc::new(vec![0u8; bytes]),
            ],
            write: AtomicUsize::new(0),
            last_pts: 0,
            has_frame: false,
            audio: None,
            gpu: None,
            playout_depth: 1,
            cpu_fifo: VecDeque::new(),
            gpu_fifo: VecDeque::new(),
            free: Vec::new(),
            fifo: VecDeque::new(),
            last_peak: (0.0, 0.0),
            last_hold: (0.0, 0.0),
            fifo_primed: false,
        }
    }

    pub fn push(&mut self, src: &[u8], stride: usize, pts: i64) {
        self.push_opaque(src, stride, pts, false);
    }

    pub fn push_opaque(&mut self, src: &[u8], stride: usize, pts: i64, opaque_x: bool) {
        let idx = (self.write.load(Ordering::Relaxed) + 1) % SLOTS;
        let bytes = slot_bytes(self.width, self.height, self.format);
        {
            let slot = Arc::make_mut(&mut self.slots[idx]);
            if slot.len() != bytes {
                slot.resize(bytes, 0);
            }
            write_slot(
                slot,
                src,
                stride,
                self.width,
                self.height,
                self.format,
                opaque_x,
            );
        }
        self.write.store(idx, Ordering::Release);
        self.last_pts = pts;
        self.has_frame = true;
        self.gpu = None;
    }

    pub fn push_gpu(&mut self, frame: GpuVideoFrame) {
        self.last_pts = frame.pts;
        self.has_frame = true;
        self.gpu = Some(frame);
    }

    pub fn set_playout_depth(&mut self, depth: u32) {
        self.playout_depth = depth.clamp(1, 8) as usize;
        while self.cpu_fifo.len() > self.playout_depth {
            self.cpu_fifo.pop_front();
        }
        while self.gpu_fifo.len() > self.playout_depth {
            self.gpu_fifo.pop_front();
        }
    }

    pub fn push_playout_cpu(&mut self, src: &[u8], stride: usize, pts: i64) {
        self.push_playout_cpu_opaque(src, stride, pts, false);
    }

    pub fn push_playout_cpu_opaque(&mut self, src: &[u8], stride: usize, pts: i64, opaque_x: bool) {
        if self.playout_depth <= 1 {
            self.push_opaque(src, stride, pts, opaque_x);
            return;
        }
        let bytes = slot_bytes(self.width, self.height, self.format);
        let mut pixels = self.take_free(bytes);
        write_slot(
            &mut pixels,
            src,
            stride,
            self.width,
            self.height,
            self.format,
            opaque_x,
        );
        self.finish_playout_cpu(pixels, pts);
    }

    pub fn take_playout_buf(&mut self) -> Vec<u8> {
        let bytes = slot_bytes(self.width, self.height, self.format);
        self.take_free(bytes)
    }

    pub fn finish_playout_cpu(&mut self, pixels: Vec<u8>, pts: i64) {
        if self.playout_depth <= 1 {
            let idx = (self.write.load(Ordering::Relaxed) + 1) % SLOTS;
            let old = std::mem::replace(&mut self.slots[idx], Arc::new(pixels));
            if let Ok(buf) = Arc::try_unwrap(old) {
                self.recycle(buf);
            }
            self.write.store(idx, Ordering::Release);
            self.last_pts = pts;
            self.has_frame = true;
            self.gpu = None;
            return;
        }
        self.cpu_fifo.push_back(CpuQueuedFrame { pts, pixels });
        while self.cpu_fifo.len() > self.playout_depth {
            if let Some(old) = self.cpu_fifo.pop_front() {
                self.recycle(old.pixels);
            }
        }
    }

    fn take_free(&mut self, bytes: usize) -> Vec<u8> {
        match self.free.pop() {
            Some(buf) if buf.len() == bytes => buf,
            Some(mut buf) => {
                buf.resize(bytes, 0);
                buf
            }
            None => vec![0u8; bytes],
        }
    }

    fn recycle(&mut self, buf: Vec<u8>) {
        if buf.is_empty() {
            return;
        }
        if self.free.len() < 4 {
            self.free.push(buf);
        }
    }

    pub fn push_playout_gpu(&mut self, frame: GpuVideoFrame) {
        if self.playout_depth <= 1 {
            self.push_gpu(frame);
            return;
        }
        self.gpu_fifo.push_back(frame);
        while self.gpu_fifo.len() > self.playout_depth {
            self.gpu_fifo.pop_front();
        }
    }

    pub fn advance_playout(&mut self) {
        if self.playout_depth <= 1 {
            return;
        }
        if self.format == CpuFormat::GpuRgba {
            if let Some(frame) = self.gpu_fifo.pop_front() {
                self.push_gpu(frame);
            }
            return;
        }
        if let Some(queued) = self.cpu_fifo.pop_front() {
            let idx = (self.write.load(Ordering::Relaxed) + 1) % SLOTS;
            let old = std::mem::replace(&mut self.slots[idx], Arc::new(queued.pixels));
            if let Ok(buf) = Arc::try_unwrap(old) {
                self.recycle(buf);
            }
            self.write.store(idx, Ordering::Release);
            self.last_pts = queued.pts;
            self.has_frame = true;
            self.gpu = None;
        }
    }

    pub fn peak(&self) -> (f32, f32) {
        self.last_peak
    }

    pub fn ingest_audio(&mut self, packet: AudioPacket) {
        self.last_peak = peak_planar(&packet);
        let stereo = resample_to_stereo_48k(&packet);
        self.fifo.extend(stereo);
        let cap = AUDIO_FIFO_FRAMES * 2;
        while self.fifo.len() > cap {
            self.fifo.pop_front();
        }
        if self.fifo.len() >= AUDIO_PRIME_FRAMES * 2 {
            self.fifo_primed = true;
        }
        self.audio = Some(packet);
    }

    pub fn clear_audio(&mut self) {
        self.fifo.clear();
        self.audio = None;
        self.last_peak = (0.0, 0.0);
        self.last_hold = (0.0, 0.0);
        self.fifo_primed = false;
    }

    fn pop_stereo(&mut self) -> (f32, f32) {
        if !self.fifo_primed {
            return (0.0, 0.0);
        }
        self.trim_to_live();
        if self.fifo.len() >= 2 {
            let left = self.fifo.pop_front().unwrap_or(self.last_hold.0);
            let right = self.fifo.pop_front().unwrap_or(left);
            self.last_hold = (left, right);
            (left, right)
        } else {
            self.last_hold
        }
    }

    pub fn skip_audio_frames(&mut self, frames: usize) {
        let n = frames.saturating_mul(2).min(self.fifo.len());
        if n > 0 {
            self.fifo.drain(..n);
        }
    }

    /// Drop old samples when a producer ran ahead of picture (typical for files).
    fn trim_to_live(&mut self) {
        let live = AUDIO_LIVE_FRAMES.saturating_mul(2);
        let max = AUDIO_LIVE_FRAMES.saturating_mul(2).saturating_mul(2);
        if self.fifo.len() > max {
            let drop = self.fifo.len().saturating_sub(live);
            if drop > 0 {
                self.fifo.drain(..drop);
            }
        }
    }

    pub fn latest_rgba_or_packed(&self) -> &[u8] {
        self.slots[self.write.load(Ordering::Acquire)].as_slice()
    }

    pub fn latest_pixels(&self) -> Arc<Vec<u8>> {
        Arc::clone(&self.slots[self.write.load(Ordering::Acquire)])
    }

    pub fn ram_bytes(&self) -> u64 {
        self.slots.iter().map(|slot| slot.len() as u64).sum::<u64>()
            + self
                .cpu_fifo
                .iter()
                .map(|frame| frame.pixels.len() as u64)
                .sum::<u64>()
            + self.free.iter().map(|buf| buf.len() as u64).sum::<u64>()
    }

    pub fn vram_bytes(&self) -> u64 {
        let bpp = match self.format {
            CpuFormat::Uyvy | CpuFormat::Uyva => 2,
            CpuFormat::Bgra | CpuFormat::Rgba | CpuFormat::GpuRgba => 4,
        };
        u64::from(self.width) * u64::from(self.height) * bpp
    }
}

#[derive(Default)]
pub struct UploadStore {
    sources: HashMap<u64, SourceRing>,
}

impl UploadStore {
    pub fn register(&mut self, id: u64, width: u32, height: u32, format: CpuFormat) {
        let prev = self.sources.remove(&id);
        let mut ring = SourceRing::new(width, height, format);
        if let Some(old) = prev {
            ring.audio = old.audio;
            ring.fifo = old.fifo;
            ring.last_peak = old.last_peak;
            ring.last_hold = old.last_hold;
            ring.fifo_primed = old.fifo_primed;
            ring.playout_depth = old.playout_depth;
            if old.width == width && old.height == height && old.format == format {
                ring.gpu = old.gpu;
                ring.cpu_fifo = old.cpu_fifo;
                ring.gpu_fifo = old.gpu_fifo;
                ring.free = old.free;
                ring.has_frame = old.has_frame;
                ring.last_pts = old.last_pts;
            }
        }
        self.sources.insert(id, ring);
    }

    pub fn unregister(&mut self, id: u64) {
        self.sources.remove(&id);
    }

    pub fn ensure(&mut self, id: u64, width: u32, height: u32, format: CpuFormat) {
        match self.sources.get(&id) {
            Some(ring) if ring.width == width && ring.height == height && ring.format == format => {}
            _ => self.register(id, width, height, format),
        }
    }

    pub fn ensure_playout(
        &mut self,
        id: u64,
        width: u32,
        height: u32,
        format: CpuFormat,
        depth: u32,
    ) {
        self.ensure(id, width, height, format);
        if let Some(ring) = self.sources.get_mut(&id) {
            ring.set_playout_depth(depth);
        }
    }

    pub fn push_playout_cpu(
        &mut self,
        id: u64,
        src: &[u8],
        stride: usize,
        pts: i64,
    ) -> Result<(), String> {
        self.push_playout_cpu_opaque(id, src, stride, pts, false)
    }

    pub fn push_playout_cpu_opaque(
        &mut self,
        id: u64,
        src: &[u8],
        stride: usize,
        pts: i64,
        opaque_x: bool,
    ) -> Result<(), String> {
        let ring = self
            .sources
            .get_mut(&id)
            .ok_or_else(|| format!("unknown source {id}"))?;
        ring.push_playout_cpu_opaque(src, stride, pts, opaque_x);
        Ok(())
    }

    pub fn take_playout_buf(
        &mut self,
        id: u64,
        width: u32,
        height: u32,
        format: CpuFormat,
        depth: u32,
    ) -> Option<(Vec<u8>, CpuFormat, u32, u32)> {
        self.ensure_playout(id, width, height, format, depth);
        let ring = self.sources.get_mut(&id)?;
        Some((
            ring.take_playout_buf(),
            ring.format,
            ring.width,
            ring.height,
        ))
    }

    pub fn finish_playout_cpu(&mut self, id: u64, pixels: Vec<u8>, pts: i64) {
        if let Some(ring) = self.sources.get_mut(&id) {
            ring.finish_playout_cpu(pixels, pts);
        }
    }

    pub fn snapshot(&self, needed: &HashSet<u64>) -> Vec<CpuFrameSnap> {
        let mut out = Vec::with_capacity(needed.len());
        for id in needed {
            let Some(ring) = self.sources.get(id) else {
                continue;
            };
            out.push(CpuFrameSnap {
                id: *id,
                width: ring.width,
                height: ring.height,
                format: ring.format,
                last_pts: ring.last_pts,
                has_frame: ring.has_frame,
                pixels: ring.latest_pixels(),
                gpu: ring.gpu.clone(),
            });
        }
        out
    }

    pub fn push_playout_gpu(&mut self, id: u64, frame: GpuVideoFrame) -> Result<(), String> {
        self.ensure(
            id,
            frame.width.max(2),
            frame.height.max(2),
            CpuFormat::GpuRgba,
        );
        let ring = self
            .sources
            .get_mut(&id)
            .ok_or_else(|| format!("unknown source {id}"))?;
        ring.push_playout_gpu(frame);
        Ok(())
    }

    pub fn advance_playout(&mut self, needed: &HashSet<u64>) {
        for id in needed {
            if let Some(ring) = self.sources.get_mut(id) {
                ring.advance_playout();
            }
        }
    }

    pub fn push_gpu(&mut self, id: u64, frame: GpuVideoFrame) -> Result<(), String> {
        self.ensure(id, frame.width.max(2), frame.height.max(2), CpuFormat::GpuRgba);
        let ring = self
            .sources
            .get_mut(&id)
            .ok_or_else(|| format!("unknown source {id}"))?;
        ring.push_gpu(frame);
        Ok(())
    }

    pub fn push(&mut self, id: u64, src: &[u8], stride: usize, pts: i64) -> Result<(), String> {
        let ring = self
            .sources
            .get_mut(&id)
            .ok_or_else(|| format!("unknown source {id}"))?;
        ring.push(src, stride, pts);
        Ok(())
    }

    pub fn push_audio(
        &mut self,
        id: u64,
        sample_rate: i32,
        channels: i32,
        frames: u32,
        pts: i64,
        planar: &[f32],
    ) {
        let Some(ring) = self.sources.get_mut(&id) else {
            return;
        };
        let mut pcm = Vec::with_capacity(planar.len() * 4);
        for sample in planar {
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
        ring.ingest_audio(AudioPacket {
            timestamp: pts,
            sample_rate,
            channels,
            samples_per_channel: frames as i32,
            pcm_planar_f32: pcm,
        });
    }

    pub fn ingest_audio(&mut self, id: u64, packet: AudioPacket) {
        if self.sources.get(&id).is_none() {
            self.register(id, 16, 16, CpuFormat::Bgra);
        }
        if let Some(ring) = self.sources.get_mut(&id) {
            ring.ingest_audio(packet);
        }
    }

    pub fn fifo_over_high_water(&self, id: u64) -> bool {
        self.fifo_frames(id) >= AUDIO_FIFO_HIGH_FRAMES
    }

    pub fn primed_ids(&self) -> Vec<u64> {
        self.sources
            .iter()
            .filter(|(_, ring)| ring.fifo_primed)
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn pop_frames(&mut self, id: u64, frames: usize) -> Vec<(f32, f32)> {
        let Some(ring) = self.sources.get_mut(&id) else {
            return vec![(0.0, 0.0); frames];
        };
        (0..frames).map(|_| ring.pop_stereo()).collect()
    }

    pub fn skip_audio_frames(&mut self, frames: usize) {
        if frames == 0 {
            return;
        }
        for ring in self.sources.values_mut() {
            ring.skip_audio_frames(frames);
        }
    }

    pub fn flush_audio(&mut self, id: u64) {
        if let Some(ring) = self.sources.get_mut(&id) {
            ring.clear_audio();
        }
    }

    pub fn mix_follow(&mut self, gains: &[(u64, f32)], frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames.saturating_mul(2)];
        if frames == 0 || gains.is_empty() {
            return out;
        }
        for &(id, gain) in gains {
            if gain.abs() < 1e-6 {
                continue;
            }
            let Some(ring) = self.sources.get_mut(&id) else {
                continue;
            };
            for i in 0..frames {
                let (left, right) = ring.pop_stereo();
                out[i * 2] += left * gain;
                out[i * 2 + 1] += right * gain;
            }
        }
        out
    }

    pub fn fifo_frames(&self, id: u64) -> usize {
        self.sources.get(&id).map(|ring| ring.fifo.len() / 2).unwrap_or(0)
    }

    pub fn get(&self, id: u64) -> Option<&SourceRing> {
        self.sources.get(&id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut SourceRing> {
        self.sources.get_mut(&id)
    }

    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.sources.keys().copied()
    }
}

pub fn ingest_audio_throttled(uploads: &Mutex<UploadStore>, id: u64, packet: AudioPacket) {
    wait_fifo_below(uploads, id, AUDIO_FIFO_HIGH_FRAMES, 8);
    uploads.lock().expect("uploads").ingest_audio(id, packet);
}

/// File pumps are clocked by video PTS. Keep only a short audio lead so the
/// mix does not play 400–500 ms of already-decoded sound behind the current frame.
pub fn ingest_audio_clocked(uploads: &Mutex<UploadStore>, id: u64, packet: AudioPacket) {
    wait_fifo_below(uploads, id, AUDIO_LIVE_FRAMES, 32);
    uploads.lock().expect("uploads").ingest_audio(id, packet);
}

fn wait_fifo_below(uploads: &Mutex<UploadStore>, id: u64, limit: usize, tries: u32) {
    for _ in 0..tries {
        let frames = uploads.lock().expect("uploads").fifo_frames(id);
        if frames < limit {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn resample_to_stereo_48k(packet: &AudioPacket) -> Vec<f32> {
    let channels = packet.channels.max(1) as usize;
    let src_rate = packet.sample_rate.max(1) as usize;
    let values: Vec<f32> = packet
        .pcm_planar_f32
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    if values.is_empty() {
        return Vec::new();
    }
    let src_frames = if packet.samples_per_channel > 0 {
        packet.samples_per_channel as usize
    } else {
        values.len() / channels
    }
    .max(1)
    .min(values.len() / channels.max(1));
    if src_rate == AUDIO_RATE as usize {
        let mut out = Vec::with_capacity(src_frames * 2);
        for i in 0..src_frames {
            let left = values[i];
            let right = if channels > 1 {
                values.get(src_frames + i).copied().unwrap_or(left)
            } else {
                left
            };
            out.push(left);
            out.push(right);
        }
        return out;
    }
    let dst_frames = (src_frames * AUDIO_RATE as usize + src_rate / 2) / src_rate;
    let mut out = Vec::with_capacity(dst_frames * 2);
    let last = src_frames.saturating_sub(1);
    for i in 0..dst_frames {
        let src = i as f64 * src_rate as f64 / AUDIO_RATE as f64;
        let idx = (src.floor() as usize).min(last);
        let frac = (src - idx as f64) as f32;
        let nxt = (idx + 1).min(last);
        let left = values[idx] * (1.0 - frac) + values[nxt] * frac;
        let right = if channels > 1 {
            let a = values.get(src_frames + idx).copied().unwrap_or(left);
            let b = values.get(src_frames + nxt).copied().unwrap_or(a);
            a * (1.0 - frac) + b * frac
        } else {
            left
        };
        out.push(left);
        out.push(right);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn linear_resample_48k_passthrough() {
        let mut pcm = Vec::new();
        for sample in [0.0f32, 0.5, 1.0] {
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
        let packet = AudioPacket {
            timestamp: 0,
            sample_rate: 48_000,
            channels: 1,
            samples_per_channel: 3,
            pcm_planar_f32: pcm,
        };
        let out = resample_to_stereo_48k(&packet);
        assert_eq!(out.len(), 6);
        assert!((out[0] - 0.0).abs() < 1e-6);
        assert!((out[2] - 0.5).abs() < 1e-6);
        assert!((out[4] - 1.0).abs() < 1e-6);
    }

    fn tone_packet(frames: usize) -> AudioPacket {
        let mut pcm = Vec::with_capacity(frames * 4);
        for i in 0..frames {
            pcm.extend_from_slice(&(i as f32 * 0.001).to_le_bytes());
        }
        AudioPacket {
            timestamp: 0,
            sample_rate: AUDIO_RATE,
            channels: 1,
            samples_per_channel: frames as i32,
            pcm_planar_f32: pcm,
        }
    }

    #[test]
    fn source_fifo_cannot_hold_half_a_second() {
        let mut store = UploadStore::default();
        store.ingest_audio(1, tone_packet(AUDIO_RATE as usize / 2));
        assert!(
            store.fifo_frames(1) <= AUDIO_FIFO_FRAMES,
            "fifo held {} frames",
            store.fifo_frames(1)
        );
    }

    #[test]
    fn playout_fill_outside_lock_swaps_into_fifo() {
        let mut store = UploadStore::default();
        let (mut buf, format, width, height) =
            store.take_playout_buf(7, 4, 2, CpuFormat::Bgra, 3).unwrap();
        assert_eq!((format, width, height), (CpuFormat::Bgra, 4, 2));
        buf.fill(9);
        store.finish_playout_cpu(7, buf, 11);
        store.advance_playout(&HashSet::from([7]));
        let ring = store.get(7).unwrap();
        assert_eq!(ring.last_pts, 11);
        assert!(ring.latest_rgba_or_packed().iter().all(|&b| b == 9));
    }

    #[test]
    fn playout_reuses_buffers_instead_of_copying_into_slots() {
        let mut ring = SourceRing::new(4, 2, CpuFormat::Bgra);
        ring.set_playout_depth(3);
        let frame_a = vec![1u8; 32];
        let frame_b = vec![2u8; 32];
        ring.push_playout_cpu(&frame_a, 16, 1);
        ring.push_playout_cpu(&frame_b, 16, 2);
        ring.advance_playout();
        assert_eq!(ring.last_pts, 1);
        assert_eq!(ring.latest_rgba_or_packed()[..4], [1, 1, 1, 1]);
        ring.advance_playout();
        assert_eq!(ring.last_pts, 2);
        assert_eq!(ring.latest_rgba_or_packed()[..4], [2, 2, 2, 2]);
    }

    #[test]
    fn mix_trims_file_audio_that_ran_ahead() {
        let mut store = UploadStore::default();
        store.ingest_audio(1, tone_packet(AUDIO_FIFO_FRAMES));
        let before = store.fifo_frames(1);
        assert!(before > AUDIO_LIVE_FRAMES);
        let _ = store.pop_frames(1, 8);
        assert!(
            store.fifo_frames(1) <= AUDIO_LIVE_FRAMES + 8,
            "fifo stayed at {} frames after mix",
            store.fifo_frames(1)
        );
    }
}

fn peak_planar(audio: &AudioPacket) -> (f32, f32) {
    let channels = audio.channels.max(1) as usize;
    let byte_samples = audio.pcm_planar_f32.len() / 4;
    if byte_samples == 0 {
        return (0.0, 0.0);
    }
    let samples = if audio.samples_per_channel > 0 {
        audio.samples_per_channel as usize
    } else {
        (byte_samples / channels).max(1)
    };
    let plane_bytes = samples * 4;
    let left = peak_bytes(&audio.pcm_planar_f32[..plane_bytes.min(audio.pcm_planar_f32.len())]);
    let right = if channels > 1 {
        let start = plane_bytes.min(audio.pcm_planar_f32.len());
        let end = (start + plane_bytes).min(audio.pcm_planar_f32.len());
        peak_bytes(&audio.pcm_planar_f32[start..end])
    } else {
        left
    };
    (left.min(1.0), right.min(1.0))
}

fn peak_bytes(bytes: &[u8]) -> f32 {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]).abs())
        .fold(0.0f32, f32::max)
}

fn slot_bytes(width: u32, height: u32, format: CpuFormat) -> usize {
    match format {
        CpuFormat::Uyvy | CpuFormat::Uyva => (width as usize) * (height as usize) * 2,
        CpuFormat::Bgra | CpuFormat::Rgba => (width as usize) * (height as usize) * 4,
        CpuFormat::GpuRgba => 0,
    }
}

pub(crate) fn write_slot(
    dst: &mut [u8],
    src: &[u8],
    stride: usize,
    width: u32,
    height: u32,
    format: CpuFormat,
    opaque_x: bool,
) {
    let bpp = match format {
        CpuFormat::Bgra | CpuFormat::Rgba => 4usize,
        CpuFormat::Uyvy | CpuFormat::Uyva => 2,
        CpuFormat::GpuRgba => return,
    };
    let row_bytes = width as usize * bpp;
    if stride < row_bytes {
        return;
    }
    let needed_src = match height {
        0 => 0,
        h => (h as usize - 1).saturating_mul(stride).saturating_add(row_bytes),
    };
    let needed_dst = slot_bytes(width, height, format);
    if src.len() < needed_src || dst.len() < needed_dst {
        return;
    }
    if stride == row_bytes {
        dst[..needed_dst].copy_from_slice(&src[..needed_dst]);
    } else {
        for y in 0..height as usize {
            let src_row = &src[y * stride..y * stride + row_bytes];
            dst[y * row_bytes..y * row_bytes + row_bytes].copy_from_slice(src_row);
        }
    }
    if opaque_x && matches!(format, CpuFormat::Bgra | CpuFormat::Rgba) {
        let words = needed_dst / 4;
        let ptr = dst.as_mut_ptr();
        unsafe {
            for i in 0..words {
                *ptr.add(i * 4).cast::<u32>() |= 0xFF00_0000;
            }
        }
    }
}
