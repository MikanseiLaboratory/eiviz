use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::abi::{FMT_BGRA, FMT_RGBA, FMT_UYVA, FMT_UYVY};

const SLOTS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuFormat {
    Uyvy,
    Bgra,
    Rgba,
    Uyva,
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
    }

    pub fn peak(&self) -> (f32, f32) {
        let Some(audio) = self.audio.as_ref() else {
            return (0.0, 0.0);
        };
        peak_planar(audio)
    }

    pub fn latest_rgba_or_packed(&self) -> &[u8] {
        &self.slots[self.write.load(Ordering::Acquire)]
    }
}

#[derive(Default)]
pub struct UploadStore {
    sources: HashMap<u64, SourceRing>,
}

impl UploadStore {
    pub fn register(&mut self, id: u64, width: u32, height: u32, format: CpuFormat) {
        self.sources.insert(id, SourceRing::new(width, height, format));
    }

    pub fn push(&mut self, id: u64, src: &[u8], stride: usize, pts: i64) -> Result<(), String> {
        let ring = self
            .sources
            .get_mut(&id)
            .ok_or_else(|| format!("unknown source {id}"))?;
        ring.push(src, stride, pts);
        Ok(())
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

fn peak_planar(audio: &AudioPacket) -> (f32, f32) {
    let channels = audio.channels.max(1) as usize;
    let samples = audio.samples_per_channel.max(0) as usize;
    if samples == 0 || audio.pcm_planar_f32.len() < 4 {
        return (0.0, 0.0);
    }
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
    }
}

fn write_slot(dst: &mut [u8], src: &[u8], stride: usize, width: u32, height: u32, format: CpuFormat) {
    match format {
        CpuFormat::Bgra => {
            for y in 0..height as usize {
                let row = &src[y * stride..];
                for x in 0..width as usize {
                    let i = y * width as usize * 4 + x * 4;
                    dst[i] = row[x * 4 + 2];
                    dst[i + 1] = row[x * 4 + 1];
                    dst[i + 2] = row[x * 4];
                    dst[i + 3] = row[x * 4 + 3];
                }
            }
        }
        CpuFormat::Rgba => {
            let row_bytes = width as usize * 4;
            for y in 0..height as usize {
                let src_row = &src[y * stride..y * stride + row_bytes];
                dst[y * row_bytes..y * row_bytes + row_bytes].copy_from_slice(src_row);
            }
        }
        CpuFormat::Uyvy | CpuFormat::Uyva => {
            let row_bytes = width as usize * 2;
            for y in 0..height as usize {
                let src_row = &src[y * stride..y * stride + row_bytes];
                dst[y * row_bytes..y * row_bytes + row_bytes].copy_from_slice(src_row);
            }
        }
    }
}
