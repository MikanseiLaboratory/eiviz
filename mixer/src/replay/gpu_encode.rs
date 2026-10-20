//! Encoding a replay angle straight from its scene texture.
//!
//! The CPU path copies the picture back to memory and encodes it there. At 1080p that is
//! eight megabytes per angle per frame, and the copy — not the encoder — is what capped
//! the recorder at a handful of frames a second. Encoding from the texture keeps the
//! picture on the GPU from the scene that produced it to the packet that reaches disk.
//!
//! Angles are spread over a small pool of encoder threads. One thread per angle let the
//! last angle starve: every thread waited for its own encode and the GPU runs them in
//! submission order, so past eight angles the ninth never finished in time and recorded
//! nothing while the earlier angles looked healthy.
//!
//! Each thread owns a short work list, and a full list drops its oldest entry rather than
//! refusing the newest frame. A bounded channel was wrong here: once full it kept replaying
//! the two oldest frames and refused everything else, so with sixteen angles the same eight
//! always won. A replay recorder wants the newest picture, so every angle makes progress on
//! every frame and the encoder's finite throughput is shared evenly.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use vmx::{Codec, ColorSpace, Config, Profile};

/// Textures kept per angle, so a frame can be copied while the previous one encodes.
const RING: usize = 3;

/// Frames each encoder thread holds. A deeper list absorbs jitter and lets a slow machine
/// lose its oldest frames rather than blocking the renderer.
const QUEUE: usize = 4;

/// Encoder threads: neither one per angle (the last angle starved) nor one in total (it
/// serialised the waits and measured 287 frames a second against 450).
const THREADS: usize = 4;

/// One frame waiting to be encoded.
struct Job {
    angle_id: u32,
    texture: wgpu::Texture,
    frame: u64,
    pts: i64,
}

/// Per-angle state shared with the renderer: the textures a frame is copied into.
struct Angle {
    free: Vec<wgpu::Texture>,
    width: u32,
    height: u32,
    profile: u32,
    /// Frames this angle lost because a list was full.
    dropped: Arc<AtomicU32>,
}

/// One encoder thread's work list.
type Pending = Arc<(Mutex<VecDeque<Job>>, Condvar)>;

static ANGLES: Mutex<Option<HashMap<u32, Angle>>> = Mutex::new(None);
static QUEUES: OnceLock<Vec<Pending>> = OnceLock::new();

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

/// One frame handed to an encoder: a ring texture holding the picture, and its place on
/// the timeline.
pub struct CopyJob {
    pub texture: wgpu::Texture,
    pub frame: u64,
    pub pts: i64,
}

/// Prepares an encode for one angle.
///
/// Returns a free ring texture to copy the scene into. The caller copies the picture into
/// it and later calls [`dispatch`].
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
    ensure_encoder(device, queue);
    let mut guard = ANGLES.lock().unwrap_or_else(|error| error.into_inner());
    let angles = guard.get_or_insert_with(HashMap::new);
    // A format or profile change invalidates the ring; the codecs notice and rebuild.
    let stale = angles.get(&angle_id).is_some_and(|angle| {
        angle.width != width || angle.height != height || angle.profile != profile
    });
    if stale {
        angles.remove(&angle_id);
    }
    let angle = angles.entry(angle_id).or_insert_with(|| Angle {
        free: (0..RING)
            .map(|_| make_texture(device, width, height))
            .collect(),
        width,
        height,
        profile,
        dropped: Arc::new(AtomicU32::new(0)),
    });
    // Reuse a returned texture; make one when the whole ring is still out being encoded.
    Some(
        angle
            .free
            .pop()
            .unwrap_or_else(|| make_texture(device, width, height)),
    )
}

/// Hands a copied texture to the encoder that owns this angle.
///
/// Never fails: when the list is full the oldest frame is dropped instead, so every angle
/// keeps moving.
pub fn dispatch(angle_id: u32, job: CopyJob) -> bool {
    let Some(queues) = QUEUES.get() else {
        return false;
    };
    let queue = &queues[angle_id as usize % queues.len()];
    let (list, wake) = &**queue;
    let mut pending = list.lock().unwrap_or_else(|error| error.into_inner());
    while pending.len() >= QUEUE {
        let Some(stale) = pending.pop_front() else {
            break;
        };
        note_dropped(stale.angle_id);
        return_to_ring(stale.angle_id, stale.texture);
    }
    pending.push_back(Job {
        angle_id,
        texture: job.texture,
        frame: job.frame,
        pts: job.pts,
    });
    drop(pending);
    wake.notify_one();
    true
}

fn note_dropped(angle_id: u32) {
    let guard = ANGLES.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(angle) = guard.as_ref().and_then(|angles| angles.get(&angle_id)) {
        angle.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

fn return_to_ring(angle_id: u32, texture: wgpu::Texture) {
    let mut guard = ANGLES.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(angle) = guard.as_mut().and_then(|angles| angles.get_mut(&angle_id)) {
        angle.free.push(texture);
    }
}

/// Spawns the encoder threads once, on first use.
fn ensure_encoder(device: &wgpu::Device, queue: &wgpu::Queue) {
    if QUEUES.get().is_some() {
        return;
    }
    let queues: Vec<Pending> = (0..THREADS)
        .map(|_| Arc::new((Mutex::new(VecDeque::new()), Condvar::new())))
        .collect();
    if QUEUES.set(queues).is_err() {
        return;
    }
    let Some(queues) = QUEUES.get() else {
        return;
    };
    for (index, entry) in queues.iter().cloned().enumerate() {
        let device = device.clone();
        let queue = queue.clone();
        let _ = std::thread::Builder::new()
            .name(format!("eiviz-replay-encode-{index}"))
            .spawn(move || encode_loop(entry, device, queue));
    }
}

fn encode_loop(pending: Pending, device: wgpu::Device, queue: wgpu::Queue) {
    let mut codecs: HashMap<u32, Codec> = HashMap::new();
    let mut packet = vec![0u8; 1024 * 1024];
    let (list, wake) = &*pending;
    loop {
        let job = {
            let mut guard = list.lock().unwrap_or_else(|error| error.into_inner());
            while guard.is_empty() {
                guard = wake.wait(guard).unwrap_or_else(|error| error.into_inner());
            }
            guard.pop_front()
        };
        let Some(job) = job else { break };
        if !codecs.contains_key(&job.angle_id) {
            match build_codec(job.angle_id) {
                Some(codec) => {
                    codecs.insert(job.angle_id, codec);
                }
                None => {
                    return_to_ring(job.angle_id, job.texture);
                    continue;
                }
            }
        }
        let Some(codec) = codecs.get_mut(&job.angle_id) else {
            return_to_ring(job.angle_id, job.texture);
            continue;
        };
        if codec
            .encode_from_texture_opaque(&device, &queue, &job.texture)
            .is_err()
        {
            crate::diag::error(&format!(
                "replay GPU encode failed angle={} {}x{} format={:?}",
                job.angle_id,
                job.texture.width(),
                job.texture.height(),
                job.texture.format()
            ));
            crate::replay::engine().note_encoder_unavailable();
            return_to_ring(job.angle_id, job.texture);
            continue;
        }
        let size = loop {
            match codec.save_to(&mut packet) {
                Ok(size) => break size,
                Err(_) => packet.resize(packet.len().saturating_mul(2).max(1024), 0),
            }
        };
        crate::replay::engine().write_gpu_encoded(
            job.angle_id,
            job.frame,
            job.pts,
            &packet[..size],
        );
        return_to_ring(job.angle_id, job.texture);
    }
}

/// Builds a codec for an angle, reporting a failure once instead of on every frame.
fn build_codec(angle_id: u32) -> Option<Codec> {
    let (width, height, profile) = {
        let guard = ANGLES.lock().unwrap_or_else(|error| error.into_inner());
        let angle = guard.as_ref().and_then(|angles| angles.get(&angle_id))?;
        (angle.width, angle.height, angle.profile)
    };
    match Codec::new(Config {
        width: width as i32,
        height: height as i32,
        profile: profile_of(profile),
        color_space: ColorSpace::Bt709,
    }) {
        Ok(codec) => Some(codec),
        Err(error) => {
            crate::diag::error(&format!(
                "replay GPU encoder could not be built angle={angle_id} {width}x{height} profile={profile}: {error}"
            ));
            crate::replay::engine().note_encoder_unavailable();
            None
        }
    }
}
