use std::sync::mpsc;
use std::time::Duration;

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
    slice.map_async(wgpu::MapMode::Read, move |_| {
        let _ = tx.send(());
    });
    let _ = device.device.poll(wgpu::PollType::Wait {
        submission_index: Some(index),
        timeout: Some(Duration::from_secs(2)),
    });
    rx.recv_timeout(Duration::from_secs(2))
        .map_err(|_| "snapshot map timeout".to_string())?;
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
    image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| "snapshot encode".to_string())?
        .save(path)
        .map_err(|error| error.to_string())
}

pub fn default_path() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let name = format!("eiviz-{millis}.png");
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .map(std::path::PathBuf::from);
    if let Some(pictures) = home.as_ref().map(|h| h.join("Pictures"))
        && pictures.is_dir()
    {
        return pictures.join(&name).to_string_lossy().into_owned();
    }
    std::env::temp_dir()
        .join(name)
        .to_string_lossy()
        .into_owned()
}
