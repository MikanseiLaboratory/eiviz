use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};

use crate::device::GpuDevice;

const SLOTS: usize = 3;

struct Slot {
    buffer: wgpu::Buffer,
    pending: bool,
    waiting: bool,
    ready: Option<Receiver<()>>,
}

pub struct UnitReadback {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    slots: [Slot; SLOTS],
    write: usize,
    mapped: Option<Vec<u8>>,
}

impl UnitReadback {
    pub fn new(device: &GpuDevice, width: u32, height: u32) -> Self {
        let stride = ((width * 2 + 255) / 256) * 256;
        let size = u64::from(stride * height);
        let slots = std::array::from_fn(|_| Slot {
            buffer: device.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("eiviz uyvy readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            pending: false,
            waiting: false,
            ready: None,
        });
        Self {
            width,
            height,
            stride,
            slots,
            write: 0,
            mapped: None,
        }
    }

    pub fn copy_from(&mut self, encoder: &mut wgpu::CommandEncoder, packed: &wgpu::Texture) {
        if self.slots[self.write].waiting {
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
                width: (self.width / 2).max(1),
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
            let done = self.slots[i]
                .ready
                .as_ref()
                .is_some_and(|rx| rx.try_recv().is_ok());
            if !done {
                continue;
            }
            let slice = self.slots[i].buffer.slice(..);
            if let Ok(view) = slice.get_mapped_range() {
                let mut packed = vec![0u8; (self.width * 2 * self.height) as usize];
                for y in 0..self.height as usize {
                    let src = y * self.stride as usize;
                    let dst = y * self.width as usize * 2;
                    packed[dst..dst + self.width as usize * 2]
                        .copy_from_slice(&view[src..src + self.width as usize * 2]);
                }
                drop(view);
                self.slots[i].buffer.unmap();
                self.mapped = Some(packed);
            }
            self.slots[i].pending = false;
            self.slots[i].waiting = false;
            self.slots[i].ready = None;
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
    }

    pub fn latest(&self) -> Option<&[u8]> {
        self.mapped.as_deref()
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
    ) -> &mut UnitReadback {
        if let Some(existing) = self.units.get(&id) {
            if existing.width != width || existing.height != height {
                self.units.remove(&id);
            }
        }
        self.units
            .entry(id)
            .or_insert_with(|| UnitReadback::new(device, width, height))
    }

    pub fn get(&self, unit_id: u64) -> Option<&UnitReadback> {
        self.units.get(&unit_id)
    }

    pub fn get_mut(&mut self, unit_id: u64) -> Option<&mut UnitReadback> {
        self.units.get_mut(&unit_id)
    }
}
