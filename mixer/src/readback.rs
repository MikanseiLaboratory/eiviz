use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};

use crate::device::GpuDevice;

const SLOTS: usize = 3;

struct Slot {
    buffer: wgpu::Buffer,
    pending: bool,
    waiting: bool,
    ready: Option<Receiver<Result<(), wgpu::BufferAsyncError>>>,
    pts: i64,
}

pub struct UnitReadback {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub bytes_per_pixel: u32,
    slots: [Slot; SLOTS],
    write: usize,
    mapped: Option<(Vec<u8>, i64)>,
}

impl UnitReadback {
    pub fn new(device: &GpuDevice, width: u32, height: u32, bytes_per_pixel: u32) -> Self {
        let bytes_per_pixel = if bytes_per_pixel >= 4 { 4 } else { 2 };
        let stride = ((width * bytes_per_pixel + 255) / 256) * 256;
        let size = u64::from(stride * height);
        let slots = std::array::from_fn(|_| Slot {
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
        });
        Self {
            width,
            height,
            stride,
            bytes_per_pixel,
            slots,
            write: 0,
            mapped: None,
        }
    }

    pub fn copy_from(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        packed: &wgpu::Texture,
        pts: i64,
    ) {
        if self.slots[self.write].waiting {
            return;
        }
        self.slots[self.write].pts = pts;
        let size = packed.size();
        let texels_w = if self.bytes_per_pixel >= 4 {
            self.width
        } else {
            (self.width / 2).max(1)
        };
        if size.width != texels_w || size.height != self.height {
            return;
        }
        let slot = &self.slots[self.write];
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
    }

    pub fn advance(&mut self, device: &GpuDevice) {
        self.slots[self.write].pending = true;
        let _ = device.device.poll(wgpu::PollType::Poll);
        for i in 0..SLOTS {
            if !self.slots[i].waiting {
                continue;
            }
            let Some(result) = self.slots[i]
                .ready
                .as_ref()
                .and_then(|rx| rx.try_recv().ok())
            else {
                continue;
            };
            if result.is_ok() {
                let slice = self.slots[i].buffer.slice(..);
                if let Ok(view) = slice.get_mapped_range() {
                    let row = self.width as usize * self.bytes_per_pixel as usize;
                    let mut packed = vec![0u8; row * self.height as usize];
                    for y in 0..self.height as usize {
                        let src = y * self.stride as usize;
                        let dst = y * row;
                        if let Some(line) = view.get(src..src + row) {
                            packed[dst..dst + row].copy_from_slice(line);
                        }
                    }
                    if self.bytes_per_pixel >= 4 {
                        for pixel in packed.chunks_exact_mut(4) {
                            pixel.swap(0, 2);
                        }
                    }
                    drop(view);
                    self.mapped = Some((packed, self.slots[i].pts));
                }
                self.slots[i].buffer.unmap();
            }
            self.slots[i].pending = false;
            self.slots[i].waiting = false;
            self.slots[i].ready = None;
        }
        let read = (self.write + 1) % SLOTS;
        if self.slots[read].pending && !self.slots[read].waiting {
            let slice = self.slots[read].buffer.slice(..);
            let (tx, rx) = mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
            self.slots[read].waiting = true;
            self.slots[read].ready = Some(rx);
        }
        let next = (self.write + 1) % SLOTS;
        if !self.slots[next].waiting {
            self.write = next;
        }
    }

    pub fn latest(&self) -> Option<(&[u8], i64)> {
        self.mapped
            .as_ref()
            .map(|(data, pts)| (data.as_slice(), *pts))
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
    ) -> &mut UnitReadback {
        let bytes_per_pixel = if bytes_per_pixel >= 4 { 4 } else { 2 };
        if let Some(existing) = self.units.get(&id) {
            if existing.width != width
                || existing.height != height
                || existing.bytes_per_pixel != bytes_per_pixel
            {
                self.units.remove(&id);
            }
        }
        self.units
            .entry(id)
            .or_insert_with(|| UnitReadback::new(device, width, height, bytes_per_pixel))
    }

    pub fn retain(&mut self, live: &std::collections::HashSet<u64>) {
        self.units.retain(|id, _| live.contains(id));
    }

    pub fn get_mut(&mut self, unit_id: u64) -> Option<&mut UnitReadback> {
        self.units.get_mut(&unit_id)
    }
}
