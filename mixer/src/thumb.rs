use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use crate::compose::Composer;
use crate::device::GpuDevice;

pub const MAX_WIDTH: u32 = 960;
pub const MAX_HEIGHT: u32 = 540;
const SLOTS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThumbSub {
    pub width: u32,
    pub height: u32,
    pub interval: u32,
}

impl ThumbSub {
    pub fn clamp(width: u32, height: u32, interval: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        Some(Self {
            width: width.clamp(2, MAX_WIDTH),
            height: height.clamp(2, MAX_HEIGHT),
            interval: interval.clamp(1, 8),
        })
    }
}

#[derive(Clone)]
pub struct ThumbPixels {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub data: Vec<u8>,
}

struct Slot {
    buffer: wgpu::Buffer,
    pending: bool,
    waiting: bool,
    ready: Option<Receiver<()>>,
}

struct ThumbGpu {
    width: u32,
    height: u32,
    gpu_stride: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    slots: [Slot; SLOTS],
    write: usize,
    copied: bool,
}

pub struct ThumbStore {
    gpu: HashMap<u64, ThumbGpu>,
    pixels: Arc<Mutex<HashMap<u64, ThumbPixels>>>,
}

impl ThumbStore {
    pub fn new(pixels: Arc<Mutex<HashMap<u64, ThumbPixels>>>) -> Self {
        Self {
            gpu: HashMap::new(),
            pixels,
        }
    }

    pub fn capture(
        &mut self,
        device: &GpuDevice,
        composer: &mut Composer,
        encoder: &mut wgpu::CommandEncoder,
        frame_i: u64,
        subs: &HashMap<u64, ThumbSub>,
    ) {
        self.gpu.retain(|id, _| subs.contains_key(id));
        if let Ok(mut pixels) = self.pixels.lock() {
            pixels.retain(|id, _| subs.contains_key(id));
        }
        for (id, sub) in subs {
            if frame_i % u64::from(sub.interval) != 0 {
                continue;
            }
            let gpu = self
                .gpu
                .entry(*id)
                .or_insert_with(|| ThumbGpu::new(device, *sub));
            if gpu.width != sub.width || gpu.height != sub.height {
                *gpu = ThumbGpu::new(device, *sub);
            }
            if gpu.slots[gpu.write].waiting {
                continue;
            }
            if !composer.blit_source_to(device, encoder, *id, &gpu.view) {
                continue;
            }
            gpu.copy_slot(encoder);
            gpu.copied = true;
        }
    }

    pub fn advance(&mut self, device: &GpuDevice) {
        for (id, gpu) in &mut self.gpu {
            if !gpu.copied {
                continue;
            }
            gpu.copied = false;
            if let Some(frame) = gpu.advance(device) {
                if let Ok(mut pixels) = self.pixels.lock() {
                    pixels.insert(*id, frame);
                }
            }
        }
    }
}

impl ThumbGpu {
    fn new(device: &GpuDevice, sub: ThumbSub) -> Self {
        let gpu_stride = aligned_stride(sub.width);
        let size = u64::from(gpu_stride * sub.height);
        let texture = device.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("eiviz thumb"),
            size: wgpu::Extent3d {
                width: sub.width,
                height: sub.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let slots = std::array::from_fn(|_| Slot {
            buffer: device.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("eiviz thumb readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            pending: false,
            waiting: false,
            ready: None,
        });
        Self {
            width: sub.width,
            height: sub.height,
            gpu_stride,
            texture,
            view,
            slots,
            write: 0,
            copied: false,
        }
    }

    fn copy_slot(&self, encoder: &mut wgpu::CommandEncoder) {
        let slot = &self.slots[self.write];
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &slot.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.gpu_stride),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }

    fn advance(&mut self, device: &GpuDevice) -> Option<ThumbPixels> {
        self.slots[self.write].pending = true;
        let _ = device.device.poll(wgpu::PollType::Poll);
        let mut mapped = None;
        for slot in &mut self.slots {
            if !slot.waiting {
                continue;
            }
            let done = slot.ready.as_ref().is_some_and(|rx| rx.try_recv().is_ok());
            if !done {
                continue;
            }
            let slice = slot.buffer.slice(..);
            if let Ok(view) = slice.get_mapped_range() {
                mapped = Some(pack_bgra(&view, self.width, self.height, self.gpu_stride));
                drop(view);
            }
            slot.buffer.unmap();
            slot.pending = false;
            slot.waiting = false;
            slot.ready = None;
        }
        let read = (self.write + 1) % SLOTS;
        if self.slots[read].pending && !self.slots[read].waiting {
            let slice = self.slots[read].buffer.slice(..);
            let (tx, rx) = mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |_| {
                let _ = tx.send(());
            });
            self.slots[read].waiting = true;
            self.slots[read].ready = Some(rx);
        }
        let next = (self.write + 1) % SLOTS;
        if !self.slots[next].waiting {
            self.write = next;
        }
        mapped
    }
}

fn aligned_stride(width: u32) -> u32 {
    ((width * 4 + 255) / 256) * 256
}

fn pack_bgra(src: &[u8], width: u32, height: u32, gpu_stride: u32) -> ThumbPixels {
    let stride = width * 4;
    let mut data = vec![0u8; (stride * height) as usize];
    for y in 0..height as usize {
        let row = y * gpu_stride as usize;
        let dest = y * stride as usize;
        for x in 0..width as usize {
            let s = row + x * 4;
            let d = dest + x * 4;
            if s + 3 >= src.len() || d + 3 >= data.len() {
                break;
            }
            data[d] = src[s + 2];
            data[d + 1] = src[s + 1];
            data[d + 2] = src[s];
            data[d + 3] = src[s + 3];
        }
    }
    ThumbPixels {
        width,
        height,
        stride,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::{aligned_stride, pack_bgra, ThumbSub, MAX_HEIGHT, MAX_WIDTH};

    #[test]
    fn unsubscribe_when_size_is_zero() {
        assert!(ThumbSub::clamp(0, 90, 3).is_none());
        assert!(ThumbSub::clamp(170, 0, 3).is_none());
    }

    #[test]
    fn clamps_size_and_interval() {
        let sub = ThumbSub::clamp(2000, 2000, 0).expect("sub");
        assert_eq!(sub.width, MAX_WIDTH);
        assert_eq!(sub.height, MAX_HEIGHT);
        assert_eq!(sub.interval, 1);
        let sub = ThumbSub::clamp(170, 90, 3).expect("sub");
        assert_eq!(sub.width, 170);
        assert_eq!(sub.height, 90);
        assert_eq!(sub.interval, 3);
    }

    #[test]
    fn gpu_stride_is_256_aligned() {
        assert_eq!(aligned_stride(170) % 256, 0);
        assert!(aligned_stride(170) >= 170 * 4);
    }

    #[test]
    fn pack_swizzles_rgba_to_bgra() {
        let src = [10u8, 20, 30, 40];
        let frame = pack_bgra(&src, 1, 1, 4);
        assert_eq!(frame.data, vec![30, 20, 10, 40]);
        assert_eq!(frame.stride, 4);
    }
}
