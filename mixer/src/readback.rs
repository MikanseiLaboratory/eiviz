use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};

use std::collections::VecDeque;

use crate::device::GpuDevice;

/// Slots a preview readback keeps. Enough to overlap one map with the copy being
/// written.
pub const READBACK_SLOTS: usize = 3;
/// Slots a recording readback keeps. A map takes a few frames to come back, so
/// the recorder needs a deeper ring than a preview: with only three, the fourth
/// frame of latency is a lost one.
pub const READBACK_RECORD_SLOTS: usize = 8;

struct Slot {
    buffer: wgpu::Buffer,
    pending: bool,
    waiting: bool,
    ready: Option<Receiver<Result<(), wgpu::BufferAsyncError>>>,
    pts: i64,
    /// Render frame the copy came from. The picture must be labelled with the
    /// frame it actually holds, not with the frame that happens to be rendering
    /// when the readback finally completes.
    frame: u64,
}

pub struct UnitReadback {
    /// Copies refused because every slot was still busy. A ring that is too shallow is
    /// the difference between a recording that keeps up and one that does not.
    pub exhausted: u64,
    /// Frames whose map completed, i.e. what the recorder actually had to work with.
    pub mapped_frames: u64,
    /// Slots examined by `advance`, and how many of those were still pending.
    pub inspected: u64,
    pub pending_seen: u64,
    /// `try_recv` outcomes while a map was in flight: ready and not yet ready.
    pub callbacks_ready: u64,
    pub callbacks_waiting: u64,
    /// Frames handed to the recorder by `take_latest`.
    pub taken: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub bytes_per_pixel: u32,
    slots: Vec<Slot>,
    write: usize,
    /// Finished frames waiting in order for the outputs.
    mapped: VecDeque<(Vec<u8>, i64, u64)>,
    /// Frames that finished and were dropped because the queue was full.
    replaced: u64,
}

impl UnitReadback {
    pub fn new(
        device: &GpuDevice,
        width: u32,
        height: u32,
        bytes_per_pixel: u32,
        depth: usize,
    ) -> Self {
        let bytes_per_pixel = if bytes_per_pixel >= 4 { 4 } else { 2 };
        let stride = ((width * bytes_per_pixel + 255) / 256) * 256;
        let size = u64::from(stride * height);
        let depth = depth.max(2);
        let slots: Vec<Slot> = (0..depth)
            .map(|_| Slot {
                buffer: device.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("eiviz frame readback"),
                    size,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                pending: false,
                waiting: false,
                ready: None,
                pts: 0,
                frame: 0,
            })
            .collect();
        Self {
            width,
            height,
            stride,
            bytes_per_pixel,
            slots,
            write: 0,
            mapped: VecDeque::new(),
            exhausted: 0,
            mapped_frames: 0,
            inspected: 0,
            pending_seen: 0,
            callbacks_ready: 0,
            callbacks_waiting: 0,
            taken: 0,
            replaced: 0,
        }
    }

    /// True when this readback holds exactly this frame size, so a copy would
    /// land in the buffer rather than being rejected.
    pub fn matches(&self, width: u32, height: u32) -> bool {
        self.width == width && self.height == height
    }

    pub fn copy_from(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        packed: &wgpu::Texture,
        pts: i64,
    ) {
        self.copy_from_frame(encoder, packed, pts, 0);
    }

    pub fn copy_from_frame(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        packed: &wgpu::Texture,
        pts: i64,
        frame: u64,
    ) {
        // Take a slot that holds nothing yet. Writing into a slot whose copy is
        // still queued overwrites the frame that copy belongs to, which lost
        // recordings a third of their frames without counting anything.
        let len = self.slots.len();
        let start = (self.write + 1) % len;
        let mut chosen = None;
        for step in 0..len {
            let index = (start + step) % len;
            if !self.slots[index].waiting && !self.slots[index].pending {
                chosen = Some(index);
                break;
            }
        }
        let Some(index) = chosen else {
            // Every slot is busy, so this frame cannot be read back. Counted apart from the
            // size-mismatch case below: one says the ring is too shallow, the other that the
            // picture changed shape.
            self.exhausted = self.exhausted.saturating_add(1);
            self.replaced = self.replaced.saturating_add(1);
            return;
        };
        self.write = index;
        self.slots[index].pts = pts;
        self.slots[index].frame = frame;
        let size = packed.size();
        let texels_w = if self.bytes_per_pixel >= 4 {
            self.width
        } else {
            (self.width / 2).max(1)
        };
        if size.width != texels_w || size.height != self.height {
            self.replaced = self.replaced.saturating_add(1);
            return;
        }
        let slot = &self.slots[index];
        encoder.copy_texture_to_buffer(
            packed.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &slot.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.stride),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: texels_w,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        // Marked where the copy is queued, so a second `advance` in the same frame
        // cannot mark a slot that holds nothing new.
        self.slots[index].pending = true;
    }

    /// Polls the device for completed readbacks.
    ///
    /// The poll is the expensive part and it belongs to the device, not to one store, so the
    /// caller runs [`poll_device`] once and then [`Self::advance_polled`] for each store. Doing
    /// the poll inside every store meant one device poll per angle per frame; with eight
    /// replay angles that was sixteen milliseconds of a sixteen-millisecond frame.
    pub fn advance(&mut self, device: &GpuDevice) {
        let _ = device.device.poll(wgpu::PollType::Poll);
        self.advance_polled();
    }

    /// The per-store work, for a caller that has already polled the device.
    pub fn advance_polled(&mut self) {
        for i in 0..self.slots.len() {
            self.inspected = self.inspected.saturating_add(1);
            if self.slots[i].pending {
                self.pending_seen = self.pending_seen.saturating_add(1);
            }
            if !self.slots[i].waiting {
                continue;
            }
            let Some(result) = self.slots[i]
                .ready
                .as_ref()
                .and_then(|rx| rx.try_recv().ok())
            else {
                self.callbacks_waiting = self.callbacks_waiting.saturating_add(1);
                continue;
            };
            self.callbacks_ready = self.callbacks_ready.saturating_add(1);
            if result.is_ok() {
                let slice = self.slots[i].buffer.slice(..);
                if let Ok(view) = slice.get_mapped_range() {
                    let copy_started = std::time::Instant::now();
                    // One copy of the mapped rows, stride and all. The repack to tightly
                    // packed rows and the channel order the recorder wants are both handled
                    // by the encoder, which reads the stride it is given; doing them here put
                    // a full-frame pass per angle on the render thread, and with eight replay
                    // angles that was 13 ms of a 16.67 ms frame.
                    let bytes = self.stride as usize * self.height as usize;
                    let mut packed = vec![0u8; bytes];
                    if let Some(src) = view.get(..bytes) {
                        packed.copy_from_slice(src);
                    }
                    drop(view);
                    crate::replay::profile::readback(copy_started.elapsed());
                    // Finished frames wait in order. Pushing on the back and
                    // taking from the front keeps the outputs chronological; only
                    // a queue that overflows loses one, and that is counted.
                    if self.mapped.len() >= 2 {
                        self.mapped.pop_front();
                        self.replaced = self.replaced.saturating_add(1);
                    }
                    self.mapped_frames = self.mapped_frames.saturating_add(1);
                    self.mapped
                        .push_back((packed, self.slots[i].pts, self.slots[i].frame));
                }
                self.slots[i].buffer.unmap();
            }
            self.slots[i].pending = false;
            self.slots[i].waiting = false;
            self.slots[i].ready = None;
        }
        // Every finished slot is mapped, not one per frame. Waiting for a map to
        // finish before starting the next one capped the outputs at one frame per
        // map latency, which cost recordings most of their frames.
        for i in 0..self.slots.len() {
            if !self.slots[i].pending || self.slots[i].waiting {
                continue;
            }
            // Leave the slot being written this frame alone: it is the copy that
            // was just queued.
            if i == self.write {
                continue;
            }
            let slice = self.slots[i].buffer.slice(..);
            let (tx, rx) = mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
            self.slots[i].waiting = true;
            self.slots[i].ready = Some(rx);
        }
    }

    pub fn latest(&self) -> Option<(&[u8], i64)> {
        self.mapped
            .back()
            .map(|(data, pts, _frame)| (data.as_slice(), *pts))
    }

    /// Oldest finished frame, so a consumer that writes them out keeps order.
    /// Row stride of the mapped rows this store hands over.
    pub fn stride(&self) -> usize {
        self.stride as usize
    }

    pub fn take_latest(&mut self) -> Option<(Vec<u8>, i64, u64)> {
        let frame = self.mapped.pop_front();
        if frame.is_some() {
            self.taken = self.taken.saturating_add(1);
        }
        frame
    }

    /// Frames that finished and were replaced before anyone read them, since the
    /// previous call.
    pub fn take_replaced(&mut self) -> u64 {
        std::mem::take(&mut self.replaced)
    }
}

#[derive(Default)]
pub struct ReadbackStore {
    units: HashMap<u64, UnitReadback>,
}

impl ReadbackStore {
    pub fn ensure(
        &mut self,
        device: &GpuDevice,
        id: u64,
        width: u32,
        height: u32,
        bytes_per_pixel: u32,
        depth: usize,
    ) -> &mut UnitReadback {
        let bytes_per_pixel = if bytes_per_pixel >= 4 { 4 } else { 2 };
        let depth = depth.max(2);
        if let Some(existing) = self.units.get(&id) {
            if existing.width != width
                || existing.height != height
                || existing.bytes_per_pixel != bytes_per_pixel
                || existing.slots.len() != depth
            {
                self.units.remove(&id);
            }
        }
        self.units
            .entry(id)
            .or_insert_with(|| UnitReadback::new(device, width, height, bytes_per_pixel, depth))
    }

    pub fn retain(&mut self, live: &std::collections::HashSet<u64>) {
        self.units.retain(|id, _| live.contains(id));
    }

    pub fn get_mut(&mut self, unit_id: u64) -> Option<&mut UnitReadback> {
        self.units.get_mut(&unit_id)
    }
}
