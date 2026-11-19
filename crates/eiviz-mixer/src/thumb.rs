use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use crate::compose::Composer;
use crate::device::GpuDevice;

pub const MAX_WIDTH: u32 = 960;
pub const MAX_HEIGHT: u32 = 540;
/// One readback holds its slot from the copy until the harvest one `advance`
/// later. At one capture per frame two slots keep the rotation fed, and the two
/// spare ones absorb a GPU that falls a couple of frames behind before a
/// capture has to be dropped. A slot's readback buffer is created the first
/// time the slot is used, so an idle subscription pays for the slots it
/// actually rotates through, not for all four.
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

/// What a slot is holding, kept apart from the GPU buffer so the rotation can
/// be exercised without a device.
#[derive(Clone, Copy, Default)]
struct SlotState {
    /// A copy was queued into this slot and its readback is not harvested yet.
    pending: bool,
    /// `map_async` was issued and the slot is waiting for the GPU.
    waiting: bool,
}

impl SlotState {
    fn idle(self) -> bool {
        !self.pending && !self.waiting
    }
}

/// The readback rotation: which slot holds a picture and where the next capture
/// starts looking. `capture` fills a slot and `advance` maps then harvests it.
struct Rotation {
    slots: [SlotState; SLOTS],
    /// Where `idle_slot` starts looking, one past the last slot filled.
    write: usize,
}

impl Rotation {
    fn new() -> Self {
        Self {
            slots: [SlotState::default(); SLOTS],
            write: 0,
        }
    }

    fn busy(&self) -> bool {
        self.slots.iter().any(|slot| slot.pending || slot.waiting)
    }

    fn idle_slot(&self) -> Option<usize> {
        (0..SLOTS)
            .map(|step| (self.write + step) % SLOTS)
            .find(|&slot| self.slots[slot].idle())
    }

    /// The slot holds a copy that has not been harvested. The search starts one
    /// past it, so the next capture takes a different slot.
    fn fill(&mut self, slot: usize) {
        self.slots[slot].pending = true;
        self.write = (slot + 1) % SLOTS;
    }

    /// The copy is on the queue, so its readback can be mapped now.
    fn map(&mut self, slot: usize) {
        self.slots[slot].waiting = true;
    }

    /// The readback is harvested and its buffer unmapped: the slot is free.
    fn harvest(&mut self, slot: usize) {
        self.slots[slot] = SlotState::default();
    }
}

struct Slot {
    /// Created the first time the slot takes a copy; an idle slot holds none.
    buffer: Option<wgpu::Buffer>,
    ready: Option<Receiver<Result<(), wgpu::BufferAsyncError>>>,
}

struct ThumbGpu {
    width: u32,
    height: u32,
    gpu_stride: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    slots: [Slot; SLOTS],
    rotation: Rotation,
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
            let Some(slot) = gpu.rotation.idle_slot() else {
                continue;
            };
            if !composer.blit_source_to(device, encoder, *id, &gpu.view, gpu.width, gpu.height) {
                continue;
            }
            gpu.ensure_buffer(device, slot);
            gpu.copy_slot(encoder, slot);
            gpu.rotation.fill(slot);
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
            if !gpu.rotation.busy() {
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
            buffer: None,
            ready: None,
        });
        Self {
            width: sub.width,
            height: sub.height,
            gpu_stride,
            texture,
            view,
            slots,
            rotation: Rotation::new(),
        }
    }

    /// Allocates the slot's readback the first time the slot takes a copy.
    fn ensure_buffer(&mut self, device: &GpuDevice, slot: usize) {
        if self.slots[slot].buffer.is_some() {
            return;
        }
        let size = u64::from(self.gpu_stride * self.height);
        self.slots[slot].buffer = Some(device.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("eiviz thumb readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        }));
    }

    fn copy_slot(&self, encoder: &mut wgpu::CommandEncoder, slot: usize) {
        let buffer = self.slots[slot]
            .buffer
            .as_ref()
            .expect("a slot's buffer is created before its copy is queued");
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer,
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
        for slot in 0..SLOTS {
            if !self.rotation.slots[slot].waiting {
                continue;
            }
            let Some(result) = self.slots[slot]
                .ready
                .as_ref()
                .and_then(|rx| rx.try_recv().ok())
            else {
                continue;
            };
            let buffer = self.slots[slot]
                .buffer
                .as_ref()
                .expect("a waiting slot's buffer is never dropped");
            if result.is_ok() {
                let slice = buffer.slice(..);
                if let Ok(view) = slice.get_mapped_range() {
                    mapped.push(pack_bgra(&view, self.width, self.height, self.gpu_stride));
                    drop(view);
                }
                buffer.unmap();
            }
            self.slots[slot].ready = None;
            self.rotation.harvest(slot);
        }
        for slot in 0..SLOTS {
            let state = self.rotation.slots[slot];
            if !state.pending || state.waiting {
                continue;
            }
            let (tx, rx) = mpsc::channel();
            {
                let buffer = self.slots[slot]
                    .buffer
                    .as_ref()
                    .expect("a pending slot's buffer is created with its copy");
                let slice = buffer.slice(..);
                slice.map_async(wgpu::MapMode::Read, move |result| {
                    let _ = tx.send(result);
                });
            }
            self.slots[slot].ready = Some(rx);
            self.rotation.map(slot);
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
    use super::{MAX_HEIGHT, MAX_WIDTH, Rotation, SLOTS, ThumbSub, aligned_stride, pack_bgra};

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

    /// Replays `capture` and `advance` for one subscription against a GPU that
    /// finishes a readback `latency` frames after the copy is submitted. The
    /// picture a slot holds is the frame that captured it, so a slot reused
    /// while a readback is in flight shows up as a lost or doubled delivery.
    struct Sim {
        rotation: Rotation,
        ready_at: [u64; SLOTS],
        held: [Option<u64>; SLOTS],
        frame: u64,
        captured: Vec<u64>,
        delivered: Vec<u64>,
        dropped: u64,
    }

    impl Sim {
        fn new() -> Self {
            Self {
                rotation: Rotation::new(),
                ready_at: [0; SLOTS],
                held: [None; SLOTS],
                frame: 0,
                captured: Vec::new(),
                delivered: Vec::new(),
                dropped: 0,
            }
        }

        /// One master frame: capture when the subscription is due, then the
        /// frame's `advance` (harvest what finished, map what was just queued).
        fn step(&mut self, due: bool, latency: u64) {
            if due {
                match self.rotation.idle_slot() {
                    Some(slot) => {
                        self.rotation.fill(slot);
                        self.held[slot] = Some(self.frame);
                        self.ready_at[slot] = self.frame + latency;
                        self.captured.push(self.frame);
                    }
                    None => self.dropped += 1,
                }
            }
            for slot in 0..SLOTS {
                if self.rotation.slots[slot].waiting && self.ready_at[slot] <= self.frame {
                    let captured = self.held[slot].take().expect("a waiting slot holds a copy");
                    self.delivered.push(captured);
                    self.rotation.harvest(slot);
                }
            }
            for slot in 0..SLOTS {
                let state = self.rotation.slots[slot];
                if state.pending && !state.waiting {
                    self.rotation.map(slot);
                }
            }
            self.frame += 1;
        }
    }

    #[test]
    fn every_captured_frame_is_delivered_once_at_the_configured_interval() {
        // The bug this guards: the old schedule mapped a slot only after the
        // write pointer had passed it, so every other capture was overwritten
        // before it was read and delivery stalled at half the configured rate.
        for interval in 1..=4u64 {
            let captures = 60u64;
            let frames = captures * interval;
            let mut sim = Sim::new();
            for frame in 0..frames + 4 {
                sim.step(frame < frames && frame % interval == 0, 1);
            }
            assert_eq!(sim.dropped, 0, "interval {interval} dropped captures");
            assert_eq!(sim.captured.len() as u64, captures, "interval {interval}");
            assert_eq!(
                sim.delivered, sim.captured,
                "interval {interval} must deliver every capture once, in order"
            );
        }
    }

    #[test]
    fn a_capture_is_mapped_by_the_advance_of_the_frame_that_made_it() {
        // Waiting for the write pointer to pass the slot put the map_async one
        // capture too late; the copy is submitted by then, so it can be mapped
        // in the same advance.
        let mut sim = Sim::new();
        sim.step(true, 1);
        let slot = sim
            .held
            .iter()
            .position(|held| held.is_some())
            .expect("the subscription captured");
        assert!(sim.rotation.slots[slot].waiting);
        assert!(sim.delivered.is_empty());
    }

    #[test]
    fn a_capture_reaches_the_cpu_one_frame_later() {
        let mut sim = Sim::new();
        for _ in 0..10 {
            sim.step(true, 1);
        }
        assert_eq!(sim.delivered, (0u64..9).collect::<Vec<_>>());
    }

    #[test]
    fn a_stalled_gpu_drops_captures_instead_of_overwriting_them() {
        // The GPU never keeps up: each slot takes exactly one capture, so no
        // picture in flight is replaced, and the rest are dropped.
        let mut sim = Sim::new();
        for _ in 0..20 {
            sim.step(true, 10_000);
        }
        assert_eq!(sim.captured.len(), SLOTS);
        assert_eq!(sim.dropped, 20 - SLOTS as u64);
        assert!(sim.delivered.is_empty());
    }

    #[test]
    fn delivery_resumes_after_a_stall_without_losing_an_accepted_capture() {
        let mut sim = Sim::new();
        // The GPU runs six frames behind: the pipeline fills and captures drop.
        for _ in 0..12 {
            sim.step(true, 6);
        }
        assert!(sim.dropped > 0, "the pipeline should have filled up");
        // It catches up. Every capture that was accepted still arrives exactly
        // once, and the pipeline drains to a single in-flight copy (one
        // delivery per capture) instead of leaving a slot stuck.
        for _ in 0..30 {
            sim.step(true, 1);
        }
        assert_eq!(
            sim.held.iter().flatten().copied().collect::<Vec<_>>(),
            vec![*sim.captured.last().expect("a capture")],
            "the pipeline drains to the newest copy in flight"
        );
        let mut delivered = sim.delivered.clone();
        delivered.sort_unstable();
        assert_eq!(
            delivered,
            sim.captured[..sim.captured.len() - 1],
            "every accepted capture is delivered once"
        );
    }
}
