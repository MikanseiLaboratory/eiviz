//! What this machine can sustain for replay recording.
//!
//! The recorder's ceiling is not the renderer: measuring a frame showed about 3.5 ms of
//! scene drawing at sixteen angles, while the encoders could not keep up with sixteen
//! streams at all. So the number worth reporting to an operator is the encoder's
//! throughput, plus the drive the recording is written to.
//!
//! The measurement is deliberately independent of the running session: it builds its own
//! texture and codec and writes its own scratch file, so pressing the button during a show
//! cannot disturb what is on air or what is being recorded.

use std::fs::File;
use std::io::{Read, Write};
use std::sync::OnceLock;
use std::time::Instant;

use vmx::{Codec, ColorSpace, Config, Profile};

/// The mixer's device, remembered by the render loop so the measurement can use it.
static DEVICE: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();

/// Frames encoded for the throughput measurement. Long enough to average out the first
/// frame, short enough that pressing the button feels immediate.
const ENCODE_FRAMES: u32 = 90;

/// Megabytes written and read back for the drive measurement.
const DISK_MB: usize = 192;

/// What the machine can do, and what that means for angles.
#[derive(Clone, Copy, Debug, Default)]
pub struct Capacity {
    /// Encoder frames a second at the replay format, one stream, encoding on the GPU.
    pub encode_fps: f64,
    /// The same with the CPU encoder. The codec is a wavelet encoder: it uses one slice
    /// pool of about six threads, so it is far slower than the GPU at 1080p, and the
    /// number is reported so an operator can see that rather than guess.
    pub encode_fps_cpu: f64,
    pub disk_write_mb_s: f64,
    pub disk_read_mb_s: f64,
    /// Angles whose recording fits the measured encoder throughput at 60 fps.
    pub angles_at_60: u32,
    /// Angles the drive can absorb at the measured write rate, for the usual 1080p rate.
    pub angles_by_disk: u32,
    /// Total frames a second the encoder can produce.
    pub total_fps: f64,
}

/// Remembers the device the render loop runs on. Called once, when it starts.
pub fn remember(device: &wgpu::Device, queue: &wgpu::Queue) {
    let _ = DEVICE.set((device.clone(), queue.clone()));
}

/// Bytes a second one 1080p angle costs on disk, used to turn a write rate into angles.
const BYTES_PER_ANGLE_FRAME: f64 = 1920.0 * 1080.0 * 1.5 / 8.0;

/// Measures the machine. Runs on the caller's thread; the caller is expected to keep it
/// off the UI thread.
pub fn run(path: &str, width: u32, height: u32) -> Result<Capacity, String> {
    let (device, queue) = DEVICE
        .get()
        .cloned()
        .ok_or_else(|| "the mixer has not started its GPU yet".to_owned())?;
    let encode_fps = measure_encode(&device, &queue, width, height)?;
    // A shorter run for the CPU encoder: it is an order of magnitude slower, and the
    // operator is waiting.
    let encode_fps_cpu = measure_encode_cpu(width, height, 12)?;
    let (disk_write_mb_s, disk_read_mb_s) = measure_disk(path)?;
    let total_fps = encode_fps;
    let angles_at_60 = (total_fps / 60.0).floor().max(0.0) as u32;
    // A compressed 1080p60 stream writes far less than the raw frame size, but using the
    // raw size keeps the estimate conservative: a drive that clears this clears anything.
    let bytes_per_second = disk_write_mb_s * 1024.0 * 1024.0;
    let angles_by_disk = (bytes_per_second / (BYTES_PER_ANGLE_FRAME * 60.0)).floor() as u32;
    Ok(Capacity {
        encode_fps,
        encode_fps_cpu,
        disk_write_mb_s,
        disk_read_mb_s,
        angles_at_60,
        angles_by_disk,
        total_fps,
    })
}

impl Capacity {
    /// The JSON the host displays, kept here so both hosts read the same field names.
    pub fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\"encodeFps\":{:.1},\"encodeFpsCpu\":{:.1},\"diskWriteMbS\":{:.1},\"diskReadMbS\":{:.1},",
                "\"anglesAt60\":{},\"anglesByDisk\":{},\"totalFps\":{:.1}}}"
            ),
            self.encode_fps,
            self.encode_fps_cpu,
            self.disk_write_mb_s,
            self.disk_read_mb_s,
            self.angles_at_60,
            self.angles_by_disk,
            self.total_fps
        )
    }
}

fn measure_encode(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
) -> Result<f64, String> {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("eiviz-capacity-bench"),
        size: wgpu::Extent3d {
            width: width.max(16),
            height: height.max(16),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut codec = Codec::new(Config {
        width: width.max(16) as i32,
        height: height.max(16) as i32,
        profile: Profile::Sq,
        color_space: ColorSpace::Bt709,
    })
    .map_err(|error| format!("the encoder could not be built: {error}"))?;
    let mut packet = vec![0u8; 1024 * 1024];
    // One warm-up frame, so the first-frame cost does not drag the average down.
    codec
        .encode_from_texture_opaque(device, queue, &texture)
        .map_err(|error| format!("the encoder failed: {error}"))?;
    let _ = codec.save_to(&mut packet);
    let started = Instant::now();
    for _ in 0..ENCODE_FRAMES {
        codec
            .encode_from_texture_opaque(device, queue, &texture)
            .map_err(|error| format!("the encoder failed: {error}"))?;
        loop {
            match codec.save_to(&mut packet) {
                Ok(_) => break,
                Err(_) => packet.resize(packet.len().saturating_mul(2).max(1024), 0),
            }
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    if seconds <= 0.0 {
        return Err("the encoder finished too quickly to measure".to_owned());
    }
    Ok(f64::from(ENCODE_FRAMES) / seconds)
}

fn measure_encode_cpu(width: u32, height: u32, frames: u32) -> Result<f64, String> {
    let width = width.max(16);
    let height = height.max(16);
    let mut codec = Codec::new(Config {
        width: width as i32,
        height: height as i32,
        profile: Profile::Sq,
        color_space: ColorSpace::Bt709,
    })
    .map_err(|error| format!("the CPU encoder could not be built: {error}"))?;
    // A mid-grey frame: the codec cost depends on the picture, and grey is the honest
    // middle for "what does a second of this cost".
    let frame = vec![0x80u8; width as usize * height as usize * 2];
    let stride = width as usize * 2;
    let mut packet = vec![0u8; 1024 * 1024];
    let warm = codec
        .encode_uyvy(&frame, stride)
        .map_err(|error| format!("the CPU encoder failed: {error}"))?;
    let _ = warm;
    let started = Instant::now();
    for _ in 0..frames {
        if codec.encode_uyvy(&frame, stride).is_err() {
            return Err("the CPU encoder failed".to_owned());
        }
        loop {
            match codec.save_to(&mut packet) {
                Ok(_) => break,
                Err(_) => packet.resize(packet.len().saturating_mul(2).max(1024), 0),
            }
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    if seconds <= 0.0 {
        return Err("the CPU encoder finished too quickly to measure".to_owned());
    }
    Ok(f64::from(frames) / seconds)
}

fn measure_disk(path: &str) -> Result<(f64, f64), String> {
    let root = std::path::Path::new(path);
    let file_path = if root.is_dir() {
        root.join("eiviz-capacity-bench.bin")
    } else {
        std::path::PathBuf::from(format!("{path}.eiviz-capacity-bench.bin"))
    };
    let block = vec![0xA5u8; 1 << 20];
    let started = Instant::now();
    let mut file = File::create(&file_path)
        .map_err(|error| format!("the scratch file could not be created: {error}"))?;
    for _ in 0..DISK_MB {
        file.write_all(&block)
            .map_err(|error| format!("the scratch file could not be written: {error}"))?;
    }
    file.sync_all()
        .map_err(|error| format!("the scratch file could not be flushed: {error}"))?;
    let write_seconds = started.elapsed().as_secs_f64();
    drop(file);
    let started = Instant::now();
    let mut file = File::open(&file_path)
        .map_err(|error| format!("the scratch file could not be reopened: {error}"))?;
    let mut buffer = vec![0u8; 1 << 20];
    let mut bytes = 0usize;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("the scratch file could not be read: {error}"))?;
        if read == 0 {
            break;
        }
        bytes += read;
    }
    let read_seconds = started.elapsed().as_secs_f64();
    let _ = std::fs::remove_file(&file_path);
    let write = if write_seconds > 0.0 {
        DISK_MB as f64 / write_seconds
    } else {
        0.0
    };
    let read = if read_seconds > 0.0 {
        bytes as f64 / (1024.0 * 1024.0) / read_seconds
    } else {
        0.0
    };
    Ok((write, read))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drive_measurement_reports_both_directions() {
        let dir = std::env::temp_dir().join(format!("eiviz-capacity-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (write, read) = measure_disk(&dir.display().to_string()).expect("measure");
        assert!(write > 0.0, "a write rate must be reported");
        assert!(read > 0.0, "a read rate must be reported");
        // The scratch file is cleaned up, so a measurement leaves nothing behind.
        assert!(!dir.join("eiviz-capacity-bench.bin").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn angles_at_sixty_follow_from_the_encoder_rate() {
        let capacity = Capacity {
            encode_fps: 420.0,
            total_fps: 420.0,
            angles_at_60: 7,
            ..Capacity::default()
        };
        assert_eq!(capacity.angles_at_60, 7);
        let json = capacity.to_json();
        assert!(json.contains("\"anglesAt60\":7"), "{json}");
    }
}
