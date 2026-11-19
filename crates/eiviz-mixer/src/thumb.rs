use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use crate::compose::Composer;
use crate::device::GpuDevice;

pub const MAX_WIDTH: u32 = 960;
pub const MAX_HEIGHT: u32 = 540;
/// One readback holds its slot from the copy until the harvest one `advance`
/// later. At one capture per frame three slots keep the rotation fed, and the
/// fourth absorbs a slow GPU frame without dropping the capture.
const SLOTS: usize = 4;

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
    /// A copy was queued into this slot and its readback is not harvested yet.
    pending: bool,
    /// `map_async` was issued and the slot is waiting for the GPU.
    waiting: bool,
    ready: Option<Receiver<Result<(), wgpu::BufferAsyncError>>>,
}

struct ThumbGpu {
    width: u32,
    height: u32,
    gpu_stride: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    slots: [Slot; SLOTS],
    /// Where `idle_slot` starts looking, one past the last slot filled.
    write: usize,
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
            // Every slot still holds an unharvested readback: the GPU is behind
            // the subscription, so this capture is dropped rather than
            // overwriting a picture that is still in flight.
            let Some(slot) = gpu.idle_slot() else {
                continue;
            };
            if !composer.blit_source_to(device, encoder, *id, &gpu.view, gpu.width, gpu.height) {
                continue;
            }
            gpu.copy_slot(encoder, slot);
            gpu.slots[slot].pending = true;
            gpu.write = (slot + 1) % SLOTS;
        }
    }

    pub fn advance(&mut self, device: &GpuDevice) {
        // One poll for every thumbnail, then the per-thumb work. A poll walks
        // the submission queue, so polling per thumb paid that walk once per
        // subscribed unit per frame. This runs after the frame's submit, so a
        // slot filled by this frame's capture is mapped here rather than a
        // capture later.
        let _ = device.device.poll(wgpu::PollType::Poll);
        for (id, gpu) in &mut self.gpu {
            if !gpu.busy() {
                continue;
            }
            for frame in gpu.advance_polled() {
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
        }
    }

    fn busy(&self) -> bool {
        self.slots.iter().any(|slot| slot.pending || slot.waiting)
    }

    fn idle_slot(&self) -> Option<usize> {
        (0..SLOTS)
            .map(|step| (self.write + step) % SLOTS)
            .find(|&slot| !self.slots[slot].pending && !self.slots[slot].waiting)
    }

    fn copy_slot(&self, encoder: &mut wgpu::CommandEncoder, slot: usize) {
        let slot = &self.slots[slot];
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

    /// The per-thumb work, for a caller that has already polled the device.
    /// Returns every readback the GPU finished since the last call, then maps
    /// every copy the frame's submit has already landed on the GPU queue. The
    /// old schedule mapped a slot only once the write pointer had passed it,
    /// which let the pointer lap unmapped slots and overwrite their pictures:
    /// every subscription delivered at half its configured rate with several
    /// frames of extra latency.
    fn advance_polled(&mut self) -> Vec<ThumbPixels> {
        let mut mapped = Vec::new();
        for slot in &mut self.slots {
            if !slot.waiting {
                continue;
            }
            let Some(result) = slot.ready.as_ref().and_then(|rx| rx.try_recv().ok()) else {
                continue;
            };
            if result.is_ok() {
                let slice = slot.buffer.slice(..);
                if let Ok(view) = slice.get_mapped_range() {
                    mapped.push(pack_bgra(&view, self.width, self.height, self.gpu_stride));
                    drop(view);
                }
                slot.buffer.unmap();
            }
            slot.pending = false;
            slot.waiting = false;
            slot.ready = None;
        }
        for slot in &mut self.slots {
            if !slot.pending || slot.waiting {
                continue;
            }
            let slice = slot.buffer.slice(..);
            let (tx, rx) = mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
            slot.waiting = true;
            slot.ready = Some(rx);
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
    // The GPU readback rows are padded to 256 bytes, so the repack is a
    // stride-copy plus a per-pixel red/blue swap. Both are a byte permute;
    // the SIMD path is byte-identical to the scalar loop.
    crate::simd::bgra_to_rgba_rows(
        src,
        gpu_stride as usize,
        &mut data,
        stride as usize,
        width,
        height,
    );
    ThumbPixels {
        width,
        height,
        stride,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_HEIGHT, MAX_WIDTH, ThumbSub, aligned_stride, pack_bgra};

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
