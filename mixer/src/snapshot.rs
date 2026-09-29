use std::io::BufWriter;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use image::ImageEncoder;
use image::codecs::jpeg::JpegEncoder;
use wgpu::TextureFormat;

use crate::device::GpuDevice;

pub fn save_texture(device: &GpuDevice, texture: &wgpu::Texture, path: &str) -> Result<(), String> {
    if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) {
        return Err("snapshot texture is not copyable".into());
    }
    let size = texture.size();
    let width = size.width.max(1);
    let height = size.height.max(1);
    let stride = ((width * 4 + 255) / 256) * 256;
    let buffer = device.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("eiviz snapshot"),
        size: u64::from(stride * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("eiviz snapshot"),
        });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let index = device.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    let _ = device.device.poll(wgpu::PollType::Wait {
        submission_index: Some(index),
        timeout: Some(Duration::from_secs(2)),
    });
    rx.recv_timeout(Duration::from_secs(2))
        .map_err(|_| "snapshot map timeout".to_string())?
        .map_err(|error| format!("snapshot map failed: {error}"))?;
    let view = slice
        .get_mapped_range()
        .map_err(|error| error.to_string())?;
    let row = (width * 4) as usize;
    let mut rgba = vec![0u8; row * height as usize];
    for y in 0..height as usize {
        let src = y * stride as usize;
        let dest = y * row;
        rgba[dest..dest + row].copy_from_slice(&view[src..src + row]);
    }
    drop(view);
    buffer.unmap();
    if matches!(
        texture.format(),
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
    ) {
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
    }
    encode_rgba(width, height, &rgba, path)
}

fn encode_rgba(width: u32, height: u32, rgba: &[u8], path: &str) -> Result<(), String> {
    if wants_jpeg(path) {
        let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
        for px in rgba.chunks_exact(4) {
            rgb.extend_from_slice(&px[..3]);
        }
        let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
        let encoder = JpegEncoder::new_with_quality(BufWriter::new(file), 90);
        encoder
            .write_image(&rgb, width, height, image::ExtendedColorType::Rgb8)
            .map_err(|error| error.to_string())
    } else {
        image::RgbaImage::from_raw(width, height, rgba.to_vec())
            .ok_or_else(|| "snapshot encode".to_string())?
            .save_with_format(path, image::ImageFormat::Png)
            .map_err(|error| error.to_string())
    }
}

pub fn wants_jpeg(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("jpg") | Some("jpeg")
    )
}

/// Directory that remote (network) callers may write snapshots into.
pub fn snapshot_dir() -> std::path::PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .map(std::path::PathBuf::from);
    if let Some(pictures) = home.as_ref().map(|h| h.join("Pictures"))
        && pictures.is_dir()
    {
        return pictures;
    }
    std::env::temp_dir()
}

pub fn default_path() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    snapshot_dir()
        .join(format!("eiviz-{millis}.png"))
        .to_string_lossy()
        .into_owned()
}

/// Maps a file name supplied by a remote caller into [`snapshot_dir`]. Only a bare `.png` /
/// `.jpg` / `.jpeg` file name is accepted; directories, drive letters and `..` are refused so a
/// network client cannot write anywhere else on the machine.
pub fn resolve_remote_path(value: &str) -> Result<String, String> {
    let name = value.trim();
    if name.is_empty() {
        return Ok(default_path());
    }
    if name.contains(['/', '\\', ':', '\0']) || name == "." || name.contains("..") {
        return Err(format!(
            "snapshot name must be a plain file name inside the snapshot folder: {name}"
        ));
    }
    let extension = Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    if !matches!(extension.as_deref(), Some("png" | "jpg" | "jpeg")) {
        return Err(format!(
            "snapshot name must end in .png, .jpg or .jpeg: {name}"
        ));
    }
    Ok(snapshot_dir().join(name).to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    #[test]
    fn remote_snapshot_names_cannot_leave_the_snapshot_folder() {
        for bad in [
            "../evil.png",
            "..\\evil.png",
            "/etc/passwd.png",
            "C:\\Windows\\x.png",
            "sub/dir.png",
            "notes.txt",
            "noextension",
            "a..b.png",
        ] {
            assert!(super::resolve_remote_path(bad).is_err(), "{bad}");
        }
        let ok = super::resolve_remote_path("frame.PNG").expect("plain name");
        assert!(ok.ends_with("frame.PNG"));
        assert!(super::resolve_remote_path("").is_ok());
    }

    #[test]
    fn jpeg_extension_is_detected() {
        assert!(super::wants_jpeg("C:/Temp/out.jpg"));
        assert!(super::wants_jpeg("shot.JPEG"));
        assert!(!super::wants_jpeg("shot.png"));
        assert!(!super::wants_jpeg("shot"));
    }
}
