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

/// v0.5 delivers decoded frames through a callback. This keeps the rest of the
/// decoder paths synchronous by parking the frames in a small channel.
pub struct H264Decoder {
    decoder: gpu_video::WgpuTexturesDecoderH264,
    frames: std::sync::mpsc::Receiver<gpu_video::OutputFrame<gpu_video::VideoTexture>>,
}

impl H264Decoder {
    pub fn new(device: &VideoDevice, queue: &wgpu::Queue) -> Result<Self, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let decoder = device
            .create_wgpu_textures_decoder_h264(
                queue,
                gpu_video::parameters::DecoderParameters::default(),
                move |frame| {
                    let _ = tx.send(frame);
                },
            )
            .map_err(|error| error.to_string())?;
        Ok(Self {
            decoder,
            frames: rx,
        })
    }

    fn drain(&mut self) -> Vec<gpu_video::OutputFrame<gpu_video::VideoTexture>> {
        let mut out = Vec::new();
        while let Ok(frame) = self.frames.try_recv() {
            out.push(frame);
        }
        out
    }

    pub fn decode(
        &mut self,
        chunk: gpu_video::EncodedInputChunk<'_>,
    ) -> Result<Vec<gpu_video::OutputFrame<gpu_video::VideoTexture>>, String> {
        self.decoder
            .decode(chunk)
            .map_err(|error| error.to_string())?;
        // VideoToolbox may deliver on its callback thread just after decode
        // returns. Give the first frame a moment so the caller can show it in the
        // same iteration; later frames are drained on the next call.
        let first = self
            .frames
            .recv_timeout(std::time::Duration::from_millis(5))
            .ok();
        let mut out = first.into_iter().collect::<Vec<_>>();
        out.extend(self.drain());
        Ok(out)
    }

    pub fn flush(
        &mut self,
    ) -> Result<Vec<gpu_video::OutputFrame<gpu_video::VideoTexture>>, String> {
        self.decoder.flush().map_err(|error| error.to_string())?;
        Ok(self.drain())
    }
}

#[cfg(test)]
mod vt_probe {
    use super::*;
    use gpu_video::InputFrame;
    use gpu_video::parameters::{EncoderParametersH264, RateControl, VideoParameters};
    use std::num::NonZeroU32;
    use std::time::Instant;

    /// Explicit probe for the VideoToolbox encode/decode path. It is ignored in
    /// normal runs because it encodes 120 frames at 1080p.
    fn uyvy_to_nv12(src: &[u8], width: usize, height: usize) -> (Vec<u8>, Vec<u8>) {
        let mut y = vec![0u8; width * height];
        let mut uv = vec![0u8; width * height / 2];
        for row in (0..height).step_by(2) {
            for x in (0..width).step_by(2) {
                let i = (row * width + x) * 2;
                let b = (row + 1) * width * 2 + x * 2;
                y[row * width + x] = src[i + 1];
                y[row * width + x + 1] = src[i + 3];
                y[(row + 1) * width + x] = src[b + 1];
                y[(row + 1) * width + x + 1] = src[b + 3];
                let u = ((u16::from(src[i]) + u16::from(src[b]) + 1) / 2) as u8;
                let v = ((u16::from(src[i + 2]) + u16::from(src[b + 2]) + 1) / 2) as u8;
                let o = (row / 2) * width + x;
                uv[o] = u;
                uv[o + 1] = v;
            }
        }
        (y, uv)
    }

    /// RTMP-shaped A/B: UYVY -> NV12 conversion, upload and VT encode, the work a
    /// Pro RTMP encoder would have to do if it stopped using the CPU-only path.
    #[test]
    #[ignore = "measures the machine; run with --ignored"]
    fn video_toolbox_uyvy_rtmp_probe() {
        let gpu = crate::device::GpuDevice::with_backend(crate::device::BackendRequest::Auto)
            .expect("gpu");
        let video = gpu.vulkan.as_ref().expect("video device");
        let width = NonZeroU32::new(1920).unwrap();
        let height = NonZeroU32::new(1080).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut encoder = video
            .create_wgpu_textures_encoder_h264(
                &gpu.queue,
                EncoderParametersH264 {
                    input_parameters: VideoParameters {
                        width,
                        height,
                        target_framerate: 60.into(),
                    },
                    output_parameters: video
                        .encoder_output_parameters_h264_high_quality(RateControl::VariableBitrate {
                            average_bitrate: 8_000_000,
                            max_bitrate: 12_000_000,
                            virtual_buffer_size: std::time::Duration::from_secs(2),
                        })
                        .expect("h264 parameters"),
                    max_in_flight_submissions: None,
                },
                move |chunk| {
                    let _ = tx.send(chunk);
                },
            )
            .expect("h264 encoder");
        let uyvy = vec![16u8; width.get() as usize * height.get() as usize * 2];
        let frames = 180u32;
        let started = Instant::now();
        for index in 0..frames {
            let texture = encoder.input_texture().expect("input texture");
            let (y, uv) = uyvy_to_nv12(&uyvy, width.get() as usize, height.get() as usize);
            let yp = texture.texture().y_plane();
            let ys = yp.size();
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: yp.texture(),
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: yp.aspect(),
                },
                &y,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ys.width),
                    rows_per_image: Some(ys.height),
                },
                ys,
            );
            let up = texture.texture().uv_plane();
            let us = up.size();
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: up.texture(),
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: up.aspect(),
                },
                &uv,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(us.width * 2),
                    rows_per_image: Some(us.height),
                },
                us,
            );
            encoder
                .encode(
                    InputFrame {
                        data: texture,
                        pts: Some(u64::from(index)),
                    },
                    index == 0,
                )
                .expect("encode");
        }
        encoder.flush().expect("flush encoder");
        drop(encoder);
        let secs = started.elapsed().as_secs_f64();
        let chunks = rx.into_iter().count();
        println!(
            "VT RTMP-shaped UYVY->NV12: {frames} frames in {secs:.3}s = {:.1} fps, {chunks} chunks",
            f64::from(frames) / secs
        );
    }

    #[test]
    #[ignore = "measures the machine; run with --ignored"]
    fn video_toolbox_encode_decode_probe() {
        let gpu = crate::device::GpuDevice::with_backend(crate::device::BackendRequest::Auto)
            .expect("gpu");
        let video = gpu.vulkan.as_ref().expect("video device");
        let width = NonZeroU32::new(1920).unwrap();
        let height = NonZeroU32::new(1080).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut encoder = video
            .create_wgpu_textures_encoder_h264(
                &gpu.queue,
                EncoderParametersH264 {
                    input_parameters: VideoParameters {
                        width,
                        height,
                        target_framerate: 60.into(),
                    },
                    output_parameters: video
                        .encoder_output_parameters_h264_high_quality(RateControl::VariableBitrate {
                            average_bitrate: 8_000_000,
                            max_bitrate: 12_000_000,
                            virtual_buffer_size: std::time::Duration::from_secs(2),
                        })
                        .expect("h264 parameters"),
                    max_in_flight_submissions: None,
                },
                move |chunk| {
                    let _ = tx.send(chunk);
                },
            )
            .expect("h264 encoder");
        let frames = 120u32;
        let started = Instant::now();
        for index in 0..frames {
            let texture = encoder.input_texture().expect("input texture");
            let plane = texture.texture().y_plane();
            let size = plane.size();
            let luma = vec![16u8; (size.width * size.height) as usize];
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: plane.texture(),
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: plane.aspect(),
                },
                &luma,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size.width),
                    rows_per_image: Some(size.height),
                },
                size,
            );
            let uv = texture.texture().uv_plane();
            let uv_size = uv.size();
            let chroma = vec![128u8; (uv_size.width * uv_size.height * 2) as usize];
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: uv.texture(),
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: uv.aspect(),
                },
                &chroma,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(uv_size.width * 2),
                    rows_per_image: Some(uv_size.height),
                },
                uv_size,
            );
            encoder
                .encode(
                    InputFrame {
                        data: texture,
                        pts: Some(u64::from(index)),
                    },
                    index == 0,
                )
                .expect("encode");
        }
        encoder.flush().expect("flush encoder");
        drop(encoder);
        let encode_secs = started.elapsed().as_secs_f64();
        let chunks: Vec<_> = rx.into_iter().collect();
        let bytes: usize = chunks.iter().map(|chunk| chunk.data.len()).sum();
        println!(
            "encode {frames} frames in {encode_secs:.3}s = {:.1} fps, {bytes} bytes",
            f64::from(frames) / encode_secs
        );

        let mut decoder = H264Decoder::new(video, &gpu.queue).expect("h264 decoder");
        let started = Instant::now();
        let mut decoded = 0usize;
        for chunk in &chunks {
            decoded += decoder
                .decode(gpu_video::EncodedInputChunk {
                    data: &chunk.data,
                    pts: chunk.pts,
                })
                .expect("decode")
                .len();
        }
        decoded += decoder.flush().expect("flush decoder").len();
        let decode_secs = started.elapsed().as_secs_f64();
        println!(
            "decode {decoded} frames in {decode_secs:.3}s = {:.1} fps",
            decoded as f64 / decode_secs
        );
    }
}
