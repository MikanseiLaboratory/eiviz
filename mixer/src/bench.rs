//! What this machine can sustain for replay recording.
//!
//! Only two resources matter for the answer: the CPU encoder that turns moving
//! color bars into the recording stream, and the drive the recording is written
//! to. The GPU encoder and the readback path are deliberately not measured here;
//! the replay recorder's practical limit is the CPU encoder plus storage.
//!
//! The measurement is independent of the running session: it builds its own
//! codec and writes its own scratch file, so pressing the button during a show
//! cannot disturb what is on air or what is being recorded.

use std::fs::File;
use std::io::{Read, Write};
use std::time::Instant;

use vmx::{Codec, ColorSpace, Config, Profile};

/// Frames encoded for the CPU throughput measurement. Enough to average out the
/// first frame and the bar motion, short enough to feel immediate.
const ENCODE_FRAMES: u32 = 30;

/// Megabytes written and read back for the drive measurement.
const DISK_MB: usize = 192;

/// What the machine can do, and what that means for angles.
#[derive(Clone, Copy, Debug, Default)]
pub struct Capacity {
    /// CPU encoder frames a second at the session picture size, one stream.
    pub encode_fps: f64,
    /// Kept for host compatibility; the CPU encoder is the only encoder measured.
    pub encode_fps_cpu: f64,
    pub disk_write_mb_s: f64,
    pub disk_read_mb_s: f64,
    /// Measured H.264-equivalent recording bitrate for one angle, in megabits/s.
    pub bitrate_mbps: f64,
    /// Angles the measured CPU encoder can hold at 60 fps.
    pub angles_at_60: u32,
    /// Angles the drive can absorb at the measured write rate and bitrate.
    pub angles_by_disk: u32,
    /// Total frames a second the CPU encoder can produce.
    pub total_fps: f64,
    /// Readback is no longer part of the capacity measurement.
    pub readback_mb_s: f64,
    pub angles_by_readback: u32,
    pub angles_at_60_cpu: u32,
}

/// Measures the machine. Runs on the caller's thread; the caller is expected to
/// keep it off the UI thread.
pub fn run(path: &str, width: u32, height: u32) -> Result<Capacity, String> {
    let (encode_fps_cpu, bytes_per_frame) = measure_encode_cpu(width, height, ENCODE_FRAMES)?;
    let (disk_write_mb_s, disk_read_mb_s) = measure_disk(path)?;
    let bitrate_mbps = bytes_per_frame * 60.0 * 8.0 / 1_000_000.0;
    let write_bytes_s = disk_write_mb_s * 1024.0 * 1024.0;
    let angles_by_disk = if bytes_per_frame > 0.0 {
        (write_bytes_s / (bytes_per_frame * 60.0)).floor() as u32
    } else {
        0
    };
    let angles_at_60_cpu = (encode_fps_cpu / 60.0).floor().max(0.0) as u32;
    let angles_at_60 = angles_at_60_cpu.min(angles_by_disk);
    Ok(Capacity {
        encode_fps: encode_fps_cpu,
        encode_fps_cpu,
        disk_write_mb_s,
        disk_read_mb_s,
        bitrate_mbps,
        angles_at_60,
        angles_by_disk,
        total_fps: encode_fps_cpu,
        readback_mb_s: 0.0,
        angles_by_readback: 0,
        angles_at_60_cpu,
    })
}

impl Capacity {
    /// The JSON the host displays, kept here so both hosts read the same field names.
    pub fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\"encodeFps\":{:.1},\"encodeFpsCpu\":{:.1},\"diskWriteMbS\":{:.1},\"diskReadMbS\":{:.1},",
                "\"bitrateMbps\":{:.1},\"readbackMbS\":{:.1},\"anglesByReadback\":{},",
                "\"anglesAt60Cpu\":{},\"anglesAt60\":{},\"anglesByDisk\":{},\"totalFps\":{:.1}}}"
            ),
            self.encode_fps,
            self.encode_fps_cpu,
            self.disk_write_mb_s,
            self.disk_read_mb_s,
            self.bitrate_mbps,
            self.readback_mb_s,
            self.angles_by_readback,
            self.angles_at_60_cpu,
            self.angles_at_60,
            self.angles_by_disk,
            self.total_fps
        )
    }
}

/// A moving SMPTE-like color-bar picture in UYVY. The bars shift every frame so
/// the codec sees motion; a still picture lets this wavelet codec predict nearly
/// everything away and would overstate the frame rate.
fn color_bar_frame(width: usize, height: usize, frame_index: u32, out: &mut [u8]) {
    // White, yellow, cyan, green, magenta, red, blue, black.
    const BARS: [(u8, u8, u8); 8] = [
        (235, 128, 128),
        (210, 16, 146),
        (170, 166, 16),
        (145, 54, 34),
        (106, 202, 222),
        (81, 90, 240),
        (41, 240, 110),
        (16, 128, 128),
    ];
    let stride = width * 2;
    let bar_width = (width / BARS.len()).max(1);
    let shift = (frame_index as usize * (width / 30).max(1)) % width;
    for row in 0..height {
        for x in (0..width).step_by(2) {
            let index = ((x + shift) / bar_width) % BARS.len();
            let (luma, u, v) = BARS[index];
            let next = ((x + 1 + shift) / bar_width) % BARS.len();
            let luma_next = BARS[next].0;
            let i = row * stride + x * 2;
            out[i] = u;
            out[i + 1] = luma;
            out[i + 2] = v;
            out[i + 3] = luma_next;
        }
    }
}

fn measure_encode_cpu(width: u32, height: u32, frames: u32) -> Result<(f64, f64), String> {
    let width = width.max(16);
    let height = height.max(16);
    let mut codec = Codec::new(Config {
        width: width as i32,
        height: height as i32,
        profile: Profile::Sq,
        color_space: ColorSpace::Bt709,
    })
    .map_err(|error| format!("the CPU encoder could not be built: {error}"))?;
    let stride = width as usize * 2;
    let mut frame = vec![0u8; stride * height as usize];
    let mut packet = vec![0u8; 1024 * 1024];
    let mut total_bytes = 0usize;
    // One warm-up frame, so first-frame setup does not drag the average down.
    color_bar_frame(width as usize, height as usize, 0, &mut frame);
    codec
        .encode_uyvy(&frame, stride)
        .map_err(|error| format!("the CPU encoder failed: {error}"))?;
    let started = Instant::now();
    for index in 0..frames {
        color_bar_frame(width as usize, height as usize, index, &mut frame);
        codec
            .encode_uyvy(&frame, stride)
            .map_err(|error| format!("the CPU encoder failed: {error}"))?;
        loop {
            match codec.save_to(&mut packet) {
                Ok(len) => {
                    total_bytes += len;
                    break;
                }
                Err(_) => packet.resize(packet.len().saturating_mul(2).max(1024), 0),
            }
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    if seconds <= 0.0 || frames == 0 {
        return Err("the CPU encoder finished too quickly to measure".to_owned());
    }
    Ok((
        f64::from(frames) / seconds,
        total_bytes as f64 / f64::from(frames),
    ))
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
        assert!(!dir.join("eiviz-capacity-bench.bin").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn color_bars_move_between_frames() {
        let (width, height) = (32usize, 8usize);
        let mut first = vec![0u8; width * height * 2];
        let mut second = vec![0u8; width * height * 2];
        color_bar_frame(width, height, 0, &mut first);
        color_bar_frame(width, height, 1, &mut second);
        assert_ne!(first, second, "motion is required to stop prediction");
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
        assert!(json.contains("\"bitrateMbps\":"), "{json}");
    }

    /// Manual probe for the Replay Setup button. Ignored in normal runs because it
    /// encodes frames and writes the disk measurement file.
    #[test]
    #[ignore = "measures this machine; run with --ignored"]
    fn capacity_probe() {
        let capacity = run("/tmp", 1920, 1080).expect("capacity");
        println!("{}", capacity.to_json());
    }
}
