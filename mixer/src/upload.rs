use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use crate::abi::{FMT_BGRA, FMT_RGBA, FMT_UYVA, FMT_UYVY};

const SLOTS: usize = 3;
pub const AUDIO_RATE: i32 = 48_000;
/// ~500 ms of stereo frames at 48 kHz.
pub const AUDIO_FIFO_FRAMES: usize = AUDIO_RATE as usize / 2;
const AUDIO_FIFO_HIGH_FRAMES: usize = AUDIO_FIFO_FRAMES * 4 / 5;
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

pub struct SourceRing {
    pub width: u32,
    pub height: u32,
    pub format: CpuFormat,
    slots: [Vec<u8>; SLOTS],
    write: AtomicUsize,
    pub last_pts: i64,
    pub has_frame: bool,
    pub audio: Option<AudioPacket>,
    pub gpu: Option<GpuVideoFrame>,
    fifo: VecDeque<f32>,
    last_peak: (f32, f32),
    last_hold: (f32, f32),
    fifo_primed: bool,
}

#[derive(Clone, Debug)]
pub struct GpuVideoFrame {
    pub pts: i64,
    pub width: u32,
    pub height: u32,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
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
            slots: [vec![0u8; bytes], vec![0u8; bytes], vec![0u8; bytes]],
            write: AtomicUsize::new(0),
            last_pts: 0,
            has_frame: false,
            audio: None,
            gpu: None,
            fifo: VecDeque::new(),
            last_peak: (0.0, 0.0),
            last_hold: (0.0, 0.0),
            fifo_primed: false,
        }
    }

    pub fn push(&mut self, src: &[u8], stride: usize, pts: i64) {
        let idx = (self.write.load(Ordering::Relaxed) + 1) % SLOTS;
        write_slot(
            &mut self.slots[idx],
            src,
            stride,
            self.width,
            self.height,
            self.format,
        );
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

    pub fn latest_rgba_or_packed(&self) -> &[u8] {
        &self.slots[self.write.load(Ordering::Acquire)]
    }

    pub fn ram_bytes(&self) -> u64 {
        self.slots.iter().map(|slot| slot.len() as u64).sum()
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
            ring.gpu = old.gpu;
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
    for _ in 0..8 {
        let over = uploads
            .lock()
            .expect("uploads")
            .fifo_over_high_water(id);
        if !over {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    uploads.lock().expect("uploads").ingest_audio(id, packet);
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

fn write_slot(dst: &mut [u8], src: &[u8], stride: usize, width: u32, height: u32, format: CpuFormat) {
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
    match format {
        CpuFormat::Bgra | CpuFormat::Rgba | CpuFormat::Uyvy | CpuFormat::Uyva => {
            for y in 0..height as usize {
                let src_row = &src[y * stride..y * stride + row_bytes];
                dst[y * row_bytes..y * row_bytes + row_bytes].copy_from_slice(src_row);
            }
        }
        CpuFormat::GpuRgba => {}
    }
}
