//! Encoding a replay angle straight from its scene texture.
//!
//! The CPU path copies the picture back to memory and encodes it there. At 1080p that is
//! eight megabytes per angle per frame, and the copy — not the encoder — is what capped
//! the recorder at a handful of frames a second. Encoding from the texture keeps the
//! picture on the GPU from the scene that produced it to the packet that reaches disk.
//!
//! One codec per angle, kept because building one allocates planes and a thread pool.

use std::collections::HashMap;
use std::sync::Mutex;

use vmx::{Codec, ColorSpace, Config, Profile};

struct Encoder {
    codec: Codec,
    packet: Vec<u8>,
    width: u32,
    height: u32,
    profile: Profile,
}

static ENCODERS: Mutex<Option<HashMap<u32, Encoder>>> = Mutex::new(None);

/// The replay format's profile code (0 high, 1 standard, 2 low) as the codec's preset.
fn profile_of(code: u32) -> Profile {
    match code {
        0 => Profile::Hq,
        2 => Profile::Lq,
        _ => Profile::Sq,
    }
}

/// Encodes one frame from `texture` and returns the packet.
///
/// `None` means the angle could not be encoded this frame; the caller counts it rather
/// than letting a frame vanish from a recording without a trace.
pub fn encode_frame(
    angle_id: u32,
    width: u32,
    height: u32,
    profile: u32,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Option<Vec<u8>> {
    if width == 0 || height == 0 {
        return None;
    }
    let wanted = profile_of(profile);
    let mut guard = ENCODERS.lock().unwrap_or_else(|error| error.into_inner());
    let encoders = guard.get_or_insert_with(HashMap::new);
    if let Some(existing) = encoders.get(&angle_id) {
        // A format or profile change invalidates the codec; it is rebuilt below.
        if existing.width != width || existing.height != height || existing.profile != wanted {
            encoders.remove(&angle_id);
        }
    }
    if !encoders.contains_key(&angle_id) {
        let codec = match Codec::new(Config {
            width: width as i32,
            height: height as i32,
            profile: wanted,
            color_space: ColorSpace::Bt709,
        }) {
            Ok(codec) => codec,
            Err(error) => {
                crate::diag::error(&format!(
                    "replay GPU encoder could not be built angle={angle_id} {width}x{height} profile={wanted:?}: {error}"
                ));
                crate::replay::engine().note_encoder_unavailable();
                return None;
            }
        };
        encoders.insert(
            angle_id,
            Encoder {
                codec,
                packet: vec![0u8; 1024 * 1024],
                width,
                height,
                profile: wanted,
            },
        );
    }
    let encoder = encoders.get_mut(&angle_id)?;
    if let Err(error) = encoder
        .codec
        .encode_from_texture_opaque(device, queue, texture)
    {
        // Say why instead of losing the frame quietly: a texture the encoder cannot
        // sample is a configuration mistake, not a frame that happened to be late.
        crate::diag::error(&format!(
            "replay GPU encode failed angle={angle_id} {}x{} format={:?} usage={:?}: {error}",
            width,
            height,
            texture.format(),
            texture.usage()
        ));
        crate::replay::engine().note_encoder_unavailable();
        return None;
    }
    let size = loop {
        match encoder.codec.save_to(&mut encoder.packet) {
            Ok(size) => break size,
            Err(_) => encoder
                .packet
                .resize(encoder.packet.len().saturating_mul(2).max(1024), 0),
        }
    };
    Some(encoder.packet[..size].to_vec())
}

/// Forgets the codec for an angle, e.g. when a recording stops.
pub fn forget(angle_id: u32) {
    let mut guard = ENCODERS.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(encoders) = guard.as_mut() {
        encoders.remove(&angle_id);
    }
}

/// Forgets every codec. Called when the replay configuration changes.
pub fn forget_all() {
    let mut guard = ENCODERS.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(encoders) = guard.as_mut() {
        encoders.clear();
    }
}
