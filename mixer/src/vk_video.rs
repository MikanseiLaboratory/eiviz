use std::sync::Arc;

use gpu_video::parameters::VideoDeviceDescriptor;
use gpu_video::{VideoAdapterExt, VideoDevice, VideoDeviceExt};

use crate::device::{DeviceError, GpuDevice};

pub type VulkanDecode = Arc<VideoDevice>;

fn plain_device_descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        required_limits: crate::device::required_limits(adapter),
        ..Default::default()
    }
}

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
        let video_descriptor = VideoDeviceDescriptor {
            wgpu_limits: crate::device::required_limits(&adapter),
            ..Default::default()
        };
        match adapter.request_device_with_video_support(&video_descriptor) {
            Ok(pair) => pair,
            Err(error) => {
                crate::diag::info(&format!("Vulkan Video device request: {error}"));
                pollster::block_on(adapter.request_device(&plain_device_descriptor(&adapter)))?
            }
        }
    } else {
        pollster::block_on(adapter.request_device(&plain_device_descriptor(&adapter)))?
    };
    let vulkan = match device.video() {
        Ok(video) => Some(Arc::new(video)),
        Err(error) => {
            crate::diag::info(&format!("device.video(): {error}"));
            None
        }
    };
    crate::device::install_device_handlers(&device);
    Ok(GpuDevice {
        instance,
        adapter,
        device,
        queue,
        vulkan,
    })
}

/// How H.264 access units are framed in a stream. Decided once from the stream's codec
/// configuration; guessing per sample would misread a length-prefixed NAL whose length bytes
/// happen to look like a start code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NalFormat {
    AnnexB,
    Avcc { length_size: usize },
}

/// Appends one access unit to `out` as Annex-B. Malformed length prefixes are an error rather
/// than a silently truncated frame.
pub fn append_annexb(data: &[u8], format: NalFormat, out: &mut Vec<u8>) -> Result<(), String> {
    let length_size = match format {
        NalFormat::AnnexB => {
            out.extend_from_slice(data);
            return Ok(());
        }
        NalFormat::Avcc { length_size } => length_size.clamp(1, 4),
    };
    let mut offset = 0;
    while offset < data.len() {
        if offset + length_size > data.len() {
            return Err("truncated AVCC NAL length".into());
        }
        let mut len = 0usize;
        for i in 0..length_size {
            len = (len << 8) | data[offset + i] as usize;
        }
        offset += length_size;
        if len == 0 || offset + len > data.len() {
            return Err(format!(
                "invalid AVCC NAL length {len} at offset {offset} of {}",
                data.len()
            ));
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&data[offset..offset + len]);
        offset += len;
    }
    Ok(())
}

/// Splits codec configuration into an Annex-B parameter-set prefix and the sample framing.
/// An AVCDecoderConfigurationRecord means length-prefixed samples; start codes mean Annex-B.
pub fn annexb_from_avc_config(blob: &[u8]) -> Result<(Vec<u8>, NalFormat), String> {
    if blob.starts_with(&[0, 0, 0, 1]) || blob.starts_with(&[0, 0, 1]) {
        return Ok((blob.to_vec(), NalFormat::AnnexB));
    }
    if blob.len() < 7 || blob[0] != 1 {
        return Err("unrecognized H.264 codec configuration".into());
    }
    let length_size = ((blob[4] & 0x03) + 1) as usize;
    let mut out = Vec::new();
    let mut offset = 5;
    let sps_count = (blob[offset] & 0x1f) as usize;
    offset += 1;
    for _ in 0..sps_count {
        offset = push_parameter_set(blob, offset, &mut out)?;
    }
    let pps_count = *blob.get(offset).ok_or("truncated avcC")? as usize;
    offset += 1;
    for _ in 0..pps_count {
        offset = push_parameter_set(blob, offset, &mut out)?;
    }
    Ok((out, NalFormat::Avcc { length_size }))
}

fn push_parameter_set(blob: &[u8], offset: usize, out: &mut Vec<u8>) -> Result<usize, String> {
    let header = blob.get(offset..offset + 2).ok_or("truncated avcC")?;
    let len = u16::from_be_bytes([header[0], header[1]]) as usize;
    let start = offset + 2;
    let set = blob.get(start..start + len).ok_or("truncated avcC")?;
    out.extend_from_slice(&[0, 0, 0, 1]);
    out.extend_from_slice(set);
    Ok(start + len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annexb_passthrough() {
        let mut out = Vec::new();
        append_annexb(&[0, 0, 0, 1, 0x67, 0x42], NalFormat::AnnexB, &mut out).unwrap();
        assert_eq!(out, vec![0, 0, 0, 1, 0x67, 0x42]);
    }

    #[test]
    fn annexb_from_length_prefixed() {
        let mut out = Vec::new();
        let format = NalFormat::Avcc { length_size: 4 };
        append_annexb(
            &[0, 0, 0, 2, 0x67, 0x42, 0, 0, 0, 1, 0x68],
            format,
            &mut out,
        )
        .unwrap();
        assert_eq!(out, vec![0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x68]);
    }

    #[test]
    fn length_prefix_that_resembles_a_start_code_is_not_passed_through() {
        let mut out = Vec::new();
        let format = NalFormat::Avcc { length_size: 4 };
        append_annexb(&[0, 0, 0, 1, 0x65], format, &mut out).unwrap();
        assert_eq!(out, vec![0, 0, 0, 1, 0x65]);
        let mut out = Vec::new();
        let format = NalFormat::Avcc { length_size: 3 };
        append_annexb(&[0, 0, 1, 0x65], format, &mut out).unwrap();
        assert_eq!(out, vec![0, 0, 0, 1, 0x65]);
    }

    #[test]
    fn malformed_length_is_an_error() {
        let mut out = Vec::new();
        let format = NalFormat::Avcc { length_size: 4 };
        assert!(append_annexb(&[0, 0, 0, 9, 0x67], format, &mut out).is_err());
    }

    #[test]
    fn avc_config_extracts_sps_pps() {
        let blob = vec![
            1, 0x64, 0, 0x1e, 0xff, 0xe1, 0x00, 0x03, 0x67, 0x42, 0x00, 0x01, 0x00, 0x03, 0x68,
            0xce, 0x00,
        ];
        let (out, format) = annexb_from_avc_config(&blob).unwrap();
        assert_eq!(format, NalFormat::Avcc { length_size: 4 });
        assert_eq!(
            out,
            vec![0, 0, 0, 1, 0x67, 0x42, 0x00, 0, 0, 0, 1, 0x68, 0xce, 0x00]
        );
    }

    #[test]
    fn unknown_config_is_rejected() {
        assert!(annexb_from_avc_config(&[9, 9, 9]).is_err());
    }
}
