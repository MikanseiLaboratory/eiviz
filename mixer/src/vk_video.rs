use std::sync::Arc;

use gpu_video::parameters::VideoDeviceDescriptor;
use gpu_video::{VideoAdapterExt, VideoDevice, VideoDeviceExt};

use crate::device::{DeviceError, GpuDevice};

pub type VulkanDecode = Arc<VideoDevice>;

pub fn create_vulkan_gpu_device(
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
) -> Result<GpuDevice, DeviceError> {
    let info = adapter.get_info();
    let video_info = adapter.video_adapter_info();
    let h264 = video_info
        .as_ref()
        .and_then(|info| info.decode_capabilities.h264.as_ref())
        .is_some();
    crate::diag::info(&format!(
        "gpu adapter backend={:?} type={:?} name={} driver={} vulkan-video-h264={h264}",
        info.backend, info.device_type, info.name, info.driver
    ));
    let (device, queue) = if h264 {
        match adapter.request_device_with_video_support(&VideoDeviceDescriptor::default()) {
            Ok(pair) => pair,
            Err(error) => {
                crate::diag::info(&format!("Vulkan Video device request: {error}"));
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?
            }
        }
    } else {
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?
    };
    let vulkan = match device.video() {
        Ok(video) => Some(Arc::new(video)),
        Err(error) => {
            crate::diag::info(&format!("device.video(): {error}"));
            None
        }
    };
    device.on_uncaptured_error(Arc::new(crate::device::on_uncaptured_gpu_error));
    Ok(GpuDevice {
        instance,
        adapter,
        device,
        queue,
        vulkan,
    })
}

/// AVCC length-prefixed H.264 (MP4) to Annex-B start codes.
pub fn annexb_from_avcc(data: &[u8], nal_length_size: usize, out: &mut Vec<u8>) {
    if data.starts_with(&[0, 0, 0, 1]) || data.starts_with(&[0, 0, 1]) {
        out.extend_from_slice(data);
        return;
    }
    let nal_length_size = nal_length_size.clamp(1, 4);
    let mut offset = 0;
    while offset + nal_length_size <= data.len() {
        let mut len = 0usize;
        for i in 0..nal_length_size {
            len = (len << 8) | data[offset + i] as usize;
        }
        offset += nal_length_size;
        if len == 0 || offset + len > data.len() {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&data[offset..offset + len]);
        offset += len;
    }
}

/// Convert AVCDecoderConfigurationRecord (or Annex-B) into start-code NALs.
pub fn annexb_from_avc_config(blob: &[u8]) -> (Vec<u8>, usize) {
    if blob.starts_with(&[0, 0, 0, 1]) || blob.starts_with(&[0, 0, 1]) {
        return (blob.to_vec(), 4);
    }
    if blob.len() < 7 || blob[0] != 1 {
        return (Vec::new(), 4);
    }
    let nal_length_size = ((blob[4] & 0x03) + 1) as usize;
    let mut out = Vec::new();
    let mut offset = 5;
    let sps_count = (blob.get(offset).copied().unwrap_or(0) & 0x1f) as usize;
    offset += 1;
    for _ in 0..sps_count {
        if offset + 2 > blob.len() {
            break;
        }
        let len = u16::from_be_bytes([blob[offset], blob[offset + 1]]) as usize;
        offset += 2;
        if offset + len > blob.len() {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&blob[offset..offset + len]);
        offset += len;
    }
    if offset >= blob.len() {
        return (out, nal_length_size);
    }
    let pps_count = blob[offset] as usize;
    offset += 1;
    for _ in 0..pps_count {
        if offset + 2 > blob.len() {
            break;
        }
        let len = u16::from_be_bytes([blob[offset], blob[offset + 1]]) as usize;
        offset += 2;
        if offset + len > blob.len() {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&blob[offset..offset + len]);
        offset += len;
    }
    (out, nal_length_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annexb_passthrough() {
        let mut out = Vec::new();
        annexb_from_avcc(&[0, 0, 0, 1, 0x67, 0x42], 4, &mut out);
        assert_eq!(out, vec![0, 0, 0, 1, 0x67, 0x42]);
    }

    #[test]
    fn annexb_from_length_prefixed() {
        let mut out = Vec::new();
        annexb_from_avcc(&[0, 0, 0, 2, 0x67, 0x42, 0, 0, 0, 1, 0x68], 4, &mut out);
        assert_eq!(out, vec![0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x68]);
    }

    #[test]
    fn avc_config_extracts_sps_pps() {
        let blob = vec![
            1, 0x64, 0, 0x1e, 0xff, 0xe1, 0x00, 0x03, 0x67, 0x42, 0x00, 0x01, 0x00, 0x03, 0x68,
            0xce, 0x00,
        ];
        let (out, nal_len) = annexb_from_avc_config(&blob);
        assert_eq!(nal_len, 4);
        assert_eq!(
            out,
            vec![0, 0, 0, 1, 0x67, 0x42, 0x00, 0, 0, 0, 1, 0x68, 0xce, 0x00]
        );
    }
}
