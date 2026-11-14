//! Host-side VideoToolbox H.264 encoder for the Pro RTMP backend.
//!
//! The Pro module runs from its own dylib and cannot safely create a second
//! gpu-video device (its VT encoder hangs on flush). The host owns the working
//! wgpu/VT device, so Pro sends CPU UYVY frames through this small C ABI and gets
//! Annex-B H.264 packets back. Pro converts them to AVCC for RTMP.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use gpu_video::parameters::{EncoderParametersH264, RateControl, VideoParameters};
use gpu_video::{InputFrame, VideoAdapterExt, VideoDeviceExt};
use std::num::NonZeroU32;

use crate::abi::{ERR_BUFFER_TOO_SMALL, ERR_DEVICE, ERR_INVALID_ARGUMENT, OK};
use crate::guard::LockExt;

struct Encoder {
    encoder: gpu_video::WgpuTexturesEncoderH264,
    queue: wgpu::Queue,
    chunks: std::sync::mpsc::Receiver<gpu_video::EncodedOutputChunk<Vec<u8>>>,
    width: u32,
    height: u32,
    sps: Vec<u8>,
    pps: Vec<u8>,
}

unsafe impl Send for Encoder {}

fn uyvy_to_nv12(src: &[u8], stride: usize, width: usize, height: usize) -> (Vec<u8>, Vec<u8>) {
    let mut y = vec![0u8; width * height];
    let mut uv = vec![0u8; width * height / 2];
    for row in (0..height).step_by(2) {
        for x in (0..width).step_by(2) {
            let a = row * stride + x * 2;
            let b = (row + 1) * stride + x * 2;
            if b + 3 >= src.len() {
                continue;
            }
            y[row * width + x] = src[a + 1];
            y[row * width + x + 1] = src[a + 3];
            y[(row + 1) * width + x] = src[b + 1];
            y[(row + 1) * width + x + 1] = src[b + 3];
            let u = ((u16::from(src[a]) + u16::from(src[b]) + 1) / 2) as u8;
            let v = ((u16::from(src[a + 2]) + u16::from(src[b + 2]) + 1) / 2) as u8;
            let o = (row / 2) * width + x;
            uv[o] = u;
            uv[o + 1] = v;
        }
    }
    (y, uv)
}

fn strip_start_code(mut nal: Vec<u8>) -> Vec<u8> {
    if nal.starts_with(&[0, 0, 0, 1]) {
        nal.drain(..4);
    } else if nal.starts_with(&[0, 0, 1]) {
        nal.drain(..3);
    }
    nal
}

impl Encoder {
    fn new(
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
        bitrate: u32,
        keyint: u32,
    ) -> Result<Self, String> {
        let gpu = crate::device::GpuDevice::with_backend(crate::device::BackendRequest::Auto)
            .map_err(|error| error.to_string())?;
        let video = gpu
            .vulkan
            .as_ref()
            .ok_or_else(|| "VideoToolbox/Vulkan video device is unavailable".to_string())?;
        let width_nz = NonZeroU32::new(width.max(2)).ok_or("bad width")?;
        let height_nz = NonZeroU32::new(height.max(2)).ok_or("bad height")?;
        let (tx, rx) = std::sync::mpsc::channel();
        let encoder = video
            .create_wgpu_textures_encoder_h264(
                &gpu.queue,
                EncoderParametersH264 {
                    input_parameters: VideoParameters {
                        width: width_nz,
                        height: height_nz,
                        target_framerate: (fps_num.max(1) / fps_den.max(1)).max(1).into(),
                    },
                    output_parameters: video
                        .encoder_output_parameters_h264_high_quality(RateControl::VariableBitrate {
                            average_bitrate: u64::from(bitrate).max(500_000),
                            max_bitrate: u64::from(bitrate).saturating_mul(3) / 2,
                            virtual_buffer_size: Duration::from_secs(2),
                        })
                        .map_err(|error| error.to_string())?,
                    max_in_flight_submissions: None,
                },
                move |chunk| {
                    let _ = tx.send(chunk);
                },
            )
            .map_err(|error| error.to_string())?;
        let _ = keyint;
        Ok(Self {
            encoder,
            queue: gpu.queue,
            chunks: rx,
            width,
            height,
            sps: Vec::new(),
            pps: Vec::new(),
        })
    }

    fn encode(
        &mut self,
        uyvy: &[u8],
        stride: usize,
        force_idr: bool,
    ) -> Result<Option<(Vec<u8>, bool)>, String> {
        let (y, uv) = uyvy_to_nv12(uyvy, stride, self.width as usize, self.height as usize);
        let texture = self
            .encoder
            .input_texture()
            .map_err(|error| error.to_string())?;
        let yp = texture.texture().y_plane();
        let ys = yp.size();
        self.queue.write_texture(
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
        self.queue.write_texture(
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
        self.encoder
            .encode(
                InputFrame {
                    data: texture,
                    pts: None,
                },
                force_idr,
            )
            .map_err(|error| error.to_string())?;
        // Do not wait for this frame's packet. The encoder callback is
        // asynchronous and the Pro RTMP caller submits every frame; waiting here
        // serialized the pipeline and cut throughput to ~95 fps. A missing packet
        // is returned as None (one frame of latency at most).
        let wait = if force_idr {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(16)
        };
        let chunk = match self.chunks.recv_timeout(wait) {
            Ok(chunk) => chunk,
            Err(_) => return Ok(None),
        };
        if self.sps.is_empty() {
            if let Ok(sps) = self.encoder.sps() {
                self.sps = strip_start_code(sps);
            }
            if let Ok(pps) = self.encoder.pps() {
                self.pps = strip_start_code(pps);
            }
        }
        Ok(Some((chunk.data, chunk.is_keyframe)))
    }
}

fn registry() -> &'static Mutex<HashMap<u64, Encoder>> {
    static REGISTRY: OnceLock<Mutex<HashMap<u64, Encoder>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_vt_encoder_create(
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    bitrate: u32,
    keyint: u32,
) -> u64 {
    match Encoder::new(width, height, fps_num, fps_den, bitrate, keyint) {
        Ok(encoder) => {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            registry().lock_or_recover().insert(id, encoder);
            id
        }
        Err(error) => {
            crate::diag::warn(&format!("VT encoder unavailable: {error}"));
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_vt_encoder_encode(
    id: u64,
    uyvy: *const u8,
    uyvy_len: usize,
    stride: usize,
    force_idr: u32,
    out: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
    key: *mut u32,
) -> i32 {
    if uyvy.is_null() || out.is_null() || out_len.is_null() || key.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let data = unsafe { std::slice::from_raw_parts(uyvy, uyvy_len) };
    let mut guard = registry().lock_or_recover();
    let Some(encoder) = guard.get_mut(&id) else {
        return ERR_DEVICE;
    };
    match encoder.encode(data, stride, force_idr != 0) {
        Ok(Some((packet, is_key))) => {
            unsafe {
                *out_len = packet.len();
                *key = u32::from(is_key);
            }
            if packet.len() > out_cap {
                return ERR_BUFFER_TOO_SMALL;
            }
            unsafe { std::ptr::copy_nonoverlapping(packet.as_ptr(), out, packet.len()) };
            OK
        }
        Ok(None) => {
            unsafe { *out_len = 0 };
            OK
        }
        Err(error) => {
            crate::diag::warn(&format!("VT encode: {error}"));
            ERR_DEVICE
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_vt_encoder_headers(
    id: u64,
    sps: *mut u8,
    sps_cap: usize,
    sps_len: *mut usize,
    pps: *mut u8,
    pps_cap: usize,
    pps_len: *mut usize,
) -> i32 {
    if sps.is_null() || pps.is_null() || sps_len.is_null() || pps_len.is_null() {
        return ERR_INVALID_ARGUMENT;
    }
    let guard = registry().lock_or_recover();
    let Some(encoder) = guard.get(&id) else {
        return ERR_DEVICE;
    };
    unsafe {
        *sps_len = encoder.sps.len();
        *pps_len = encoder.pps.len();
    }
    if encoder.sps.len() > sps_cap || encoder.pps.len() > pps_cap {
        return ERR_BUFFER_TOO_SMALL;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(encoder.sps.as_ptr(), sps, encoder.sps.len());
        std::ptr::copy_nonoverlapping(encoder.pps.as_ptr(), pps, encoder.pps.len());
    }
    OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_vt_encoder_destroy(id: u64) {
    let _ = registry().lock_or_recover().remove(&id);
}
