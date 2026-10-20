//! Encoding a replay angle straight from its scene texture, on a worker per angle.
//!
//! The CPU path copies the picture back to memory and encodes it there. At 1080p that is
//! eight megabytes per angle per frame, and the copy — not the encoder — is what capped
//! the recorder at a handful of frames a second. Encoding from the texture keeps the
//! picture on the GPU from the scene that produced it to the packet that reaches disk.
//!
//! The encode itself runs on a worker thread per angle. Doing it inline on the render
//! thread made the angles serial: at four angles the renderer spent its whole budget
//! waiting for one encode after another, and the frame rate fell from 60 to 42. The
//! renderer now only copies the scene into a ring texture — one GPU copy — and hands it
//! over; the worker encodes and writes the packet, and the picture never comes back.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};

use vmx::{Codec, ColorSpace, Config, Profile};

/// Textures kept per angle, so a frame can be copied while the previous one encodes.
const RING: usize = 3;

struct Job {
    texture: wgpu::Texture,
    frame: u64,
    pts: i64,
}

struct Worker {
    tx: SyncSender<Job>,
    free: Arc<Mutex<Vec<wgpu::Texture>>>,
    width: u32,
    height: u32,
    profile: u32,
    /// Frames the worker could not take because its queue was full.
    dropped: Arc<AtomicU32>,
}

static WORKERS: Mutex<Option<HashMap<u32, Worker>>> = Mutex::new(None);
static DEVICE: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();

/// The replay format's profile code (0 high, 1 standard, 2 low) as the codec's preset.
fn profile_of(code: u32) -> Profile {
    match code {
        0 => Profile::Hq,
        2 => Profile::Lq,
        _ => Profile::Sq,
    }
}

fn make_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("eiviz-replay-encode-ring"),
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

/// One frame handed to a worker: a ring texture holding the picture, and its place on
/// the timeline.
pub struct CopyJob {
    pub texture: wgpu::Texture,
    pub frame: u64,
    pub pts: i64,
}

/// Prepares an encode for one angle.
///
/// Returns a free ring texture to copy the scene into, or `None` when the worker could
/// not be started. The caller copies the picture into it and later calls [`dispatch`].
pub fn prepare(
    angle_id: u32,
    width: u32,
    height: u32,
    profile: u32,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Option<wgpu::Texture> {
    if width == 0 || height == 0 {
        return None;
    }
    let _ = DEVICE.set((device.clone(), queue.clone()));
    let mut guard = WORKERS.lock().unwrap_or_else(|error| error.into_inner());
    let workers = guard.get_or_insert_with(HashMap::new);
    if let Some(existing) = workers.get(&angle_id) {
        // A format or profile change invalidates the worker; it is rebuilt below.
        if existing.width != width || existing.height != height || existing.profile != profile {
            workers.remove(&angle_id);
        }
    }
    if !workers.contains_key(&angle_id) {
        let codec = match Codec::new(Config {
            width: width as i32,
            height: height as i32,
            profile: profile_of(profile),
            color_space: ColorSpace::Bt709,
        }) {
            Ok(codec) => codec,
            Err(error) => {
                crate::diag::error(&format!(
                    "replay GPU encoder could not be built angle={angle_id} {width}x{height} profile={profile}: {error}"
                ));
                crate::replay::engine().note_encoder_unavailable();
                return None;
            }
        };
        let free = Arc::new(Mutex::new(
            (0..RING)
                .map(|_| make_texture(device, width, height))
                .collect::<Vec<_>>(),
        ));
        let dropped = Arc::new(AtomicU32::new(0));
        // Deep enough for a few frames of jitter, shallow enough that a stuck worker is
        // reported as drops instead of as unbounded memory.
        let (tx, rx) = sync_channel::<Job>(8);
        spawn_worker(
            angle_id,
            codec,
            rx,
            Arc::clone(&free),
            Arc::clone(&dropped),
            width,
            height,
        );
        workers.insert(
            angle_id,
            Worker {
                tx,
                free,
                width,
                height,
                profile,
                dropped,
            },
        );
    }
    let worker = workers.get(&angle_id)?;
    // Reuse a returned texture; make one if the ring is still out with the worker.
    let texture = {
        let mut free = worker
            .free
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        free.pop()
    };
    Some(texture.unwrap_or_else(|| make_texture(device, width, height)))
}

fn spawn_worker(
    angle_id: u32,
    mut codec: Codec,
    rx: Receiver<Job>,
    free: Arc<Mutex<Vec<wgpu::Texture>>>,
    dropped: Arc<AtomicU32>,
    width: u32,
    height: u32,
) {
    let Some((device, queue)) = DEVICE.get().cloned() else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name(format!("eiviz-replay-enc-{angle_id}"))
        .spawn(move || {
            let mut packet = vec![0u8; 1024 * 1024];
            while let Ok(job) = rx.recv() {
                let started = std::time::Instant::now();
                let encoded = codec
                    .encode_from_texture_opaque(&device, &queue, &job.texture)
                    .is_ok();
                if encoded {
                    let size = loop {
                        match codec.save_to(&mut packet) {
                            Ok(size) => break size,
                            Err(_) => {
                                packet.resize(packet.len().saturating_mul(2).max(1024), 0);
                            }
                        }
                    };
                    crate::replay::engine().write_gpu_encoded(
                        angle_id,
                        job.frame,
                        job.pts,
                        &packet[..size],
                    );
                } else {
                    crate::diag::error(&format!(
                        "replay GPU encode failed angle={angle_id} {width}x{height} format={:?}",
                        job.texture.format()
                    ));
                    crate::replay::engine().note_encoder_unavailable();
                }
                let _ = started;
                let mut free = free.lock().unwrap_or_else(|error| error.into_inner());
                free.push(job.texture);
                let _ = &dropped;
            }
        });
}

/// Hands a copied texture to the worker of `angle_id`.
///
/// Returns false when the queue is full, which the caller counts as a dropped frame.
pub fn dispatch(angle_id: u32, job: CopyJob) -> bool {
    let guard = WORKERS.lock().unwrap_or_else(|error| error.into_inner());
    let Some(workers) = guard.as_ref() else {
        return false;
    };
    let Some(worker) = workers.get(&angle_id) else {
        return false;
    };
    match worker.tx.try_send(Job {
        texture: job.texture,
        frame: job.frame,
        pts: job.pts,
    }) {
        Ok(()) => true,
        Err(TrySendError::Full(job)) => {
            worker.dropped.fetch_add(1, Ordering::Relaxed);
            // The texture goes straight back: it will be reused next frame.
            let mut free = worker
                .free
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            free.push(job.texture);
            false
        }
        Err(TrySendError::Disconnected(job)) => {
            let mut free = worker
                .free
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            free.push(job.texture);
            false
        }
    }
}
