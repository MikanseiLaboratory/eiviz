use std::collections::HashMap;

use crate::abi::{OUTPUT_PREVIEW, mixing_unit_bus, mixing_unit_from_source};
use crate::compose::{Composer, UnitTargets};
use crate::device::GpuDevice;
use crate::upload::texture_bytes;

struct DelaySlot {
    mixed: wgpu::Texture,
    mixed_view: wgpu::TextureView,
    preview: wgpu::Texture,
    preview_view: wgpu::TextureView,
    packed: Option<wgpu::Texture>,
    packed_prv: Option<wgpu::Texture>,
}

struct UnitRing {
    slots: Vec<DelaySlot>,
    write: usize,
    read: usize,
    queued: usize,
    filled: usize,
    width: u32,
    height: u32,
    depth: usize,
}

struct SceneSlot {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct SceneRing {
    slots: Vec<SceneSlot>,
    write: usize,
    filled: usize,
    width: u32,
    height: u32,
    depth: usize,
}

pub struct FrameDelay {
    depth: usize,
    units: HashMap<u64, UnitRing>,
    scenes: HashMap<u64, SceneRing>,
    epoch: u64,
}

impl FrameDelay {
    pub fn new(depth: u32) -> Self {
        Self {
            depth: depth.clamp(1, 8) as usize,
            units: HashMap::new(),
            scenes: HashMap::new(),
            epoch: 1,
        }
    }

    pub fn set_depth(&mut self, depth: u32) {
        let depth = depth.clamp(1, 8) as usize;
        if depth == self.depth {
            return;
        }
        self.depth = depth;
        self.units.clear();
        self.scenes.clear();
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn vram_bytes(&self) -> u64 {
        let units = self
            .units
            .values()
            .flat_map(|ring| ring.slots.iter())
            .map(DelaySlot::vram_bytes)
            .sum::<u64>();
        let scenes = self
            .scenes
            .values()
            .flat_map(|ring| ring.slots.iter())
            .map(|slot| texture_bytes(&slot.texture))
            .sum::<u64>();
        units + scenes
    }

    /// Drop queued display frames so present falls back to the current compose
    /// until new captures fill the ring. Avoids NEW→old-delay rollback flicker.
    pub fn discard(&mut self, unit_ids: impl IntoIterator<Item = u64>) {
        let mut changed = false;
        for unit_id in unit_ids {
            let Some(ring) = self.units.get_mut(&unit_id) else {
                continue;
            };
            if ring.queued == 0 {
                continue;
            }
            ring.write = 0;
            ring.read = 0;
            ring.queued = 0;
            ring.filled = 0;
            changed = true;
        }
        if changed {
            self.epoch = self.epoch.wrapping_add(1);
        }
    }

    pub fn capture(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        composer: &Composer,
        unit_ids: impl IntoIterator<Item = u64>,
    ) {
        for unit_id in unit_ids {
            let Some(src) = composer.unit(unit_id) else {
                continue;
            };
            self.ensure_unit(device, unit_id, src);
            let Some(ring) = self.units.get_mut(&unit_id) else {
                continue;
            };
            let cap = ring.slots.len();
            if ring.queued >= cap.saturating_sub(1) {
                continue;
            }
            let slot = &mut ring.slots[ring.write];
            copy_tex(encoder, &src.mixed, &slot.mixed);
            copy_tex(encoder, &src.preview, &slot.preview);
            copy_optional(encoder, src.packed.as_ref(), slot.packed.as_ref());
            copy_optional(encoder, src.packed_prv.as_ref(), slot.packed_prv.as_ref());
            ring.write = (ring.write + 1) % cap;
            ring.queued = ring.queued.saturating_add(1).min(cap.saturating_sub(1));
            ring.filled = ring.filled.saturating_add(1).min(cap);
            self.epoch = self.epoch.wrapping_add(1);
        }
    }

    pub fn capture_scenes(
        &mut self,
        device: &GpuDevice,
        encoder: &mut wgpu::CommandEncoder,
        composer: &Composer,
        scene_ids: impl IntoIterator<Item = u64>,
    ) {
        for scene_id in scene_ids {
            let Some(src) = composer.scene_texture(scene_id) else {
                continue;
            };
            let width = src.size().width;
            let height = src.size().height;
            self.ensure_scene(device, scene_id, width, height);
            let Some(ring) = self.scenes.get_mut(&scene_id) else {
                continue;
            };
            let cap = ring.slots.len();
            let slot = &mut ring.slots[ring.write];
            copy_tex(encoder, src, &slot.texture);
            ring.write = (ring.write + 1) % cap;
            ring.filled = ring.filled.saturating_add(1).min(cap);
            self.epoch = self.epoch.wrapping_add(1);
        }
    }

    pub fn consume_display(&mut self, drain: bool) {
        for ring in self.units.values_mut() {
            if ring.queued == 0 {
                continue;
            }
            let catch_up = drain && ring.queued > 1;
            if ring.queued >= ring.depth || catch_up {
                ring.read = (ring.read + 1) % ring.slots.len();
                ring.queued = ring.queued.saturating_sub(1);
            }
        }
    }

    pub fn view(&self, unit_id: u64, kind: u32) -> Option<wgpu::TextureView> {
        let ring = self.units.get(&unit_id)?;
        let slot = ring.display_slot()?;
        Some(match kind {
            OUTPUT_PREVIEW => slot.preview_view.clone(),
            _ => slot.mixed_view.clone(),
        })
    }

    pub fn view_at(&self, unit_id: u64, kind: u32, offset: u32) -> Option<wgpu::TextureView> {
        let ring = self.units.get(&unit_id)?;
        let slot = ring.slot_at(offset)?;
        Some(match kind {
            OUTPUT_PREVIEW => slot.preview_view.clone(),
            _ => slot.mixed_view.clone(),
        })
    }

    pub fn scene_view_at(&self, scene_id: u64, offset: u32) -> Option<wgpu::TextureView> {
        let ring = self.scenes.get(&scene_id)?;
        Some(ring.slot_at(offset)?.view.clone())
    }

    pub fn packed(&self, unit_id: u64, kind: u32) -> Option<&wgpu::Texture> {
        let ring = self.units.get(&unit_id)?;
        let slot = ring.display_slot()?;
        match kind {
            OUTPUT_PREVIEW => slot.packed_prv.as_ref(),
            _ => slot.packed.as_ref(),
        }
    }

    pub fn rgba(&self, unit_id: u64, kind: u32) -> Option<&wgpu::Texture> {
        let ring = self.units.get(&unit_id)?;
        let slot = ring.display_slot()?;
        match kind {
            OUTPUT_PREVIEW => Some(&slot.preview),
            _ => Some(&slot.mixed),
        }
    }

    pub fn rgba_at(&self, unit_id: u64, kind: u32, offset: u32) -> Option<&wgpu::Texture> {
        let ring = self.units.get(&unit_id)?;
        let slot = ring.slot_at(offset)?;
        match kind {
            OUTPUT_PREVIEW => Some(&slot.preview),
            _ => Some(&slot.mixed),
        }
    }

    pub fn scene_rgba_at(&self, scene_id: u64, offset: u32) -> Option<&wgpu::Texture> {
        let ring = self.scenes.get(&scene_id)?;
        Some(&ring.slot_at(offset)?.texture)
    }

    pub fn view_for_source(&self, source_id: u64) -> Option<wgpu::TextureView> {
        let unit_id = mixing_unit_from_source(source_id)?;
        self.view(unit_id, mixing_unit_bus(source_id))
    }

    fn ensure_unit(&mut self, device: &GpuDevice, unit_id: u64, src: &UnitTargets) {
        let depth = self.depth;
        let cap = depth + 1;
        let recreate = self.units.get(&unit_id).is_none_or(|ring| {
            ring.width != src.width
                || ring.height != src.height
                || ring.depth != depth
                || ring.slots.len() != cap
        });
        if !recreate {
            let ring = self.units.get_mut(&unit_id).expect("ring");
            for slot in &mut ring.slots {
                ensure_optional(
                    device,
                    src.packed.as_ref(),
                    &mut slot.packed,
                    src.width / 2,
                    src.height,
                    true,
                );
                ensure_optional(
                    device,
                    src.packed_prv.as_ref(),
                    &mut slot.packed_prv,
                    src.width / 2,
                    src.height,
                    true,
                );
            }
            return;
        }
        let slots = (0..cap)
            .map(|_| DelaySlot::from_unit(device, src))
            .collect();
        self.units.insert(
            unit_id,
            UnitRing {
                slots,
                write: 0,
                read: 0,
                queued: 0,
                filled: 0,
                width: src.width,
                height: src.height,
                depth,
            },
        );
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn ensure_scene(&mut self, device: &GpuDevice, scene_id: u64, width: u32, height: u32) {
        let depth = self.depth;
        let cap = depth + 1;
        let recreate = self.scenes.get(&scene_id).is_none_or(|ring| {
            ring.width != width || ring.height != height || ring.depth != depth || ring.slots.len() != cap
        });
        if !recreate {
            return;
        }
        self.scenes.insert(
            scene_id,
            SceneRing {
                slots: (0..cap)
                    .map(|_| {
                        let texture = make_delay_texture(device, width, height, false);
                        let view = texture.create_view(&Default::default());
                        SceneSlot { texture, view }
                    })
                    .collect(),
                write: 0,
                filled: 0,
                width,
                height,
                depth,
            },
        );
        self.epoch = self.epoch.wrapping_add(1);
    }
}

impl UnitRing {
    fn display_slot(&self) -> Option<&DelaySlot> {
        if self.queued == 0 {
            return None;
        }
        self.slots.get(self.read)
    }

    fn slot_at(&self, offset: u32) -> Option<&DelaySlot> {
        let offset = offset.max(1) as usize;
        if self.filled == 0 {
            return None;
        }
        let back = offset.min(self.filled);
        let idx = (self.write + self.slots.len() - back) % self.slots.len();
        self.slots.get(idx)
    }
}

impl SceneRing {
    fn slot_at(&self, offset: u32) -> Option<&SceneSlot> {
        let offset = offset.max(1) as usize;
        if self.filled == 0 {
            return None;
        }
        let back = offset.min(self.filled);
        let idx = (self.write + self.slots.len() - back) % self.slots.len();
        self.slots.get(idx)
    }
}

impl DelaySlot {
    fn from_unit(device: &GpuDevice, src: &UnitTargets) -> Self {
        let mixed = make_delay_texture(device, src.width, src.height, false);
        let preview = make_delay_texture(device, src.width, src.height, false);
        let mut slot = Self {
            mixed_view: mixed.create_view(&Default::default()),
            preview_view: preview.create_view(&Default::default()),
            mixed,
            preview,
            packed: None,
            packed_prv: None,
        };
        if src.packed.is_some() {
            slot.packed = Some(make_delay_texture(device, src.width / 2, src.height, true));
        }
        if src.packed_prv.is_some() {
            slot.packed_prv = Some(make_delay_texture(device, src.width / 2, src.height, true));
        }
        slot
    }

    fn vram_bytes(&self) -> u64 {
        let mut total = texture_bytes(&self.mixed) + texture_bytes(&self.preview);
        if let Some(tex) = &self.packed {
            total += texture_bytes(tex);
        }
        if let Some(tex) = &self.packed_prv {
            total += texture_bytes(tex);
        }
        total
    }
}

fn make_delay_texture(device: &GpuDevice, width: u32, height: u32, packed: bool) -> wgpu::Texture {
    let _ = packed;
    device.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("eiviz delay"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn ensure_optional(
    device: &GpuDevice,
    src: Option<&wgpu::Texture>,
    dest: &mut Option<wgpu::Texture>,
    width: u32,
    height: u32,
    packed: bool,
) {
    if src.is_none() || dest.is_some() {
        return;
    }
    *dest = Some(make_delay_texture(device, width, height, packed));
}

fn copy_tex(encoder: &mut wgpu::CommandEncoder, src: &wgpu::Texture, dst: &wgpu::Texture) {
    let size = src.size();
    if size != dst.size() {
        return;
    }
    encoder.copy_texture_to_texture(
        src.as_image_copy(),
        dst.as_image_copy(),
        wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
    );
}

fn copy_optional(
    encoder: &mut wgpu::CommandEncoder,
    src: Option<&wgpu::Texture>,
    dst: Option<&wgpu::Texture>,
) {
    if let (Some(src), Some(dst)) = (src, dst) {
        copy_tex(encoder, src, dst);
    }
}
