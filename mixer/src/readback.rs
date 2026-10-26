use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};

use std::collections::VecDeque;

use crate::device::GpuDevice;

/// Slots a preview readback keeps. Enough to overlap one map with the copy being
/// written.
pub const READBACK_SLOTS: usize = 3;
/// Slots a recording readback keeps. A map takes a few frames to come back, so
/// the recorder needs a deeper ring than a preview: with only three, the fourth
/// frame of latency is a lost one. The copy runs on the pool now, so a slot is also held
/// while that copy is made rather than only until it is mapped; eight left the ring dry at
/// eight angles and lost frames to it.
pub const READBACK_RECORD_SLOTS: usize = 16;

/// Slots for a recording, given how many angles are being read back at once.
///
/// Each slot holds a full picture, so the ring is bounded by memory rather than by a constant:
/// sixteen slots are right for eight 1080p angles and far too many for thirty-two, where they
/// would pin a gigabyte of buffers. The deep ring is what keeps the recorder fed, so it only
/// gives way when the angle count makes it expensive.
pub fn record_slots(angles: usize) -> usize {
    match angles {
        0..=8 => READBACK_RECORD_SLOTS,
        9..=16 => 10,
        _ => 6,
    }
}

struct Slot {
    buffer: wgpu::Buffer,
    pending: bool,
    waiting: bool,
    /// Set while the copy pool owns this slot. The pool clears it when it has finished with
    /// the buffer, so the ring learns slot by slot rather than guessing from a count.
    offloaded: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
    /// Copy on the pool instead of here. Set for the stores a recording reads, where the
    /// copy is the difference between a 60 fps render loop and a 48 fps one.
    offload: bool,
    /// Angle the finished pictures belong to, for the offloaded path.
    angle_id: u32,
    /// 0 is UYVY, anything else is BGRA.
    layout_code: u32,
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
                offloaded: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
            offload: false,
            angle_id: 0,
            layout_code: 0,
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
            // Only hand over while the pool is keeping up. If it is behind, the copy is made
            // here instead: the render thread must never depend on the pool making progress,
            // or a pool that falls behind stalls the whole recorder.
            let room = IN_FLIGHT.load(Ordering::Relaxed) < MAX_IN_FLIGHT;
            if result.is_ok() && self.offload && room {
                // The map has completed; the copy has not been made. Reading a mapped buffer
                // is about three times slower than reading RAM, and a recording does it once
                // per angle per frame, so the pool takes it from here.
                let busy = self.slots[i].offloaded.clone();
                busy.store(true, Ordering::Release);
                IN_FLIGHT.fetch_add(1, Ordering::Relaxed);
                let job = CopyJob {
                    buffer: self.slots[i].buffer.clone(),
                    stride: self.stride as usize,
                    height: self.height as usize,
                    pts: self.slots[i].pts,
                    frame: self.slots[i].frame,
                    angle_id: self.angle_id,
                    layout: self.layout_code,
                    busy,
                    store: std::ptr::null(),
                };
                if pool().send(job).is_err() {
                    IN_FLIGHT.fetch_sub(1, Ordering::Relaxed);
                }
                self.mapped_frames = self.mapped_frames.saturating_add(1);
                self.slots[i].pending = false;
                self.slots[i].waiting = false;
                self.slots[i].ready = None;
                continue;
            }
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
            if !self.slots[i].pending
                || self.slots[i].waiting
                || self.slots[i].offloaded.load(Ordering::Acquire)
            {
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

    /// Copies this store's frames on the pool instead of on the render thread. Set for the
    /// stores a recording reads; preview readbacks are only a frame deep and do not need it.
    pub fn offload_to(&mut self, angle_id: u32, layout_code: u32) {
        self.offload = true;
        self.angle_id = angle_id;
        self.layout_code = layout_code;
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

/// One finished readback on its way to the copy pool.
struct CopyJob {
    buffer: wgpu::Buffer,
    stride: usize,
    height: usize,
    pts: i64,
    frame: u64,
    angle_id: u32,
    /// 0 is UYVY, anything else is BGRA.
    layout: u32,
    /// Cleared by the pool when the slot may be reused.
    busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The store this slot belongs to, so the pool can clear `offloaded`.
    store: *const (),
}

// SAFETY: every field is `Send`. The raw store pointer is only used to hand a counter back,
// and is never dereferenced by the pool.
unsafe impl Send for CopyJob {}

/// Slots handed to the pool and not yet returned.
/// Angles up to which the readback copy runs on the pool rather than on the render thread.
///
/// Above this the pool's calls back into the engine cost the deck more than the copy costs
/// the render thread: see the note in `emit_replay`.
pub const OFFLOAD_MAX_ANGLES: usize = 8;

/// Jobs the pool may hold at once. Beyond this the copy is made on the render thread, so the
/// render thread never depends on the pool catching up.
const MAX_IN_FLIGHT: usize = 24;

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// Slots the pool is working on, so callers can tell a busy pool from an idle one.
pub fn in_flight() -> usize {
    IN_FLIGHT.load(Ordering::Relaxed)
}

fn pool() -> &'static mpsc::Sender<CopyJob> {
    use std::sync::OnceLock;
    static POOL: OnceLock<mpsc::Sender<CopyJob>> = OnceLock::new();
    POOL.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<CopyJob>();
        // One lane per core up to twelve. The copy is a straight memory read of a mapped
        // buffer, so it scales with threads; four lanes were chosen while the pool also had
        // to contend for the engine mutex, and once that contention was removed four became
        // the limit instead of the lock.
        let lanes = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(4, 12);
        let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
        for lane in 0..lanes {
            let rx = std::sync::Arc::clone(&rx);
            let _ = std::thread::Builder::new()
                .name(format!("eiviz-readback-{lane}"))
                .spawn(move || {
                    loop {
                        let job = {
                            let guard = rx.lock().unwrap_or_else(|e| e.into_inner());
                            guard.recv()
                        };
                        let Ok(job) = job else { return };
                        // The render thread already started the map and saw it complete, so the
                        // range is readable now; mapping again here would fail, because a buffer
                        // cannot be mapped twice. The pool only does the copy and the release.
                        let bytes = job.stride.saturating_mul(job.height);
                        let slice = job.buffer.slice(..);
                        let mut packed = None;
                        if let Ok(view) = slice.get_mapped_range() {
                            if let Some(src) = view.get(..bytes) {
                                packed = Some(copy_out(src));
                            }
                            drop(view);
                        }
                        job.buffer.unmap();
                        job.busy.store(false, Ordering::Release);
                        IN_FLIGHT.fetch_sub(1, Ordering::Relaxed);
                        let Some(packed) = packed else { continue };
                        let layout = if job.layout >= 1 {
                            eiviz_pro_api::PixelLayout::Bgra
                        } else {
                            eiviz_pro_api::PixelLayout::Uyvy
                        };
                        crate::replay::engine().offer_frame(
                            job.angle_id,
                            job.frame,
                            job.pts,
                            packed,
                            job.stride,
                            layout,
                        );
                        let _ = job.store;
                    }
                });
        }
        tx
    })
}

/// Copies the mapped range out into a fresh buffer.
///
/// The destination is allocated without its zero-fill: `vec![0u8; n]` writes every byte of
/// the buffer and then the copy writes them all again, which is a whole extra pass over 4.1 MB
/// per angle per frame. Nothing reads the zeros — every byte is replaced — so the buffer is
/// handed out uninitialised and filled by `copy_nonoverlapping` before anyone can look at it.
///
/// # Safety
/// The buffer is never read before the copy fills it, and the two ranges cannot overlap
/// because one is a fresh allocation.
fn copy_out(src: &[u8]) -> Vec<u8> {
    let mut buffer = recycle(src.len());
    unsafe {
        std::ptr::copy_nonoverlapping(src.as_ptr(), buffer.as_mut_ptr(), src.len());
        buffer.set_len(src.len());
    }
    buffer
}

/// Buffers waiting to be filled again, keyed by size.
///
/// A frame lives only until the encoder has read it, and at most a few are in flight, so a
/// short list covers the steady state and the allocator stops being touched at all.
fn spare() -> &'static std::sync::Mutex<Vec<Vec<u8>>> {
    use std::sync::OnceLock;
    static SPARE: OnceLock<std::sync::Mutex<Vec<Vec<u8>>>> = OnceLock::new();
    SPARE.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// A buffer of `len` bytes with its length set to zero, ready to be written into.
fn recycle(len: usize) -> Vec<u8> {
    let mut pool = spare().lock().unwrap_or_else(|e| e.into_inner());
    while let Some(buffer) = pool.pop() {
        if buffer.capacity() >= len {
            let mut buffer = buffer;
            buffer.clear();
            return buffer;
        }
    }
    drop(pool);
    Vec::with_capacity(len)
}
