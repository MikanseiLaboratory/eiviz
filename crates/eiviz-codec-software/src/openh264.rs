use eiviz_core::InputId;
use eiviz_media::{PixelFormat, VideoFrame};
use eiviz_time::{ClockDomain, MediaTime};
use openh264_sys2::{
    API, DynamicAPI, ISVCDecoder, SBufferInfo, SDecodingParam, SVideoProperty, VIDEO_BITSTREAM_AVC,
    dsErrorFree, videoFormatI420,
};
use std::os::raw::{c_int, c_uchar};
use std::path::{Path, PathBuf};
use std::ptr::{from_mut, null_mut};

const MAX_DECODE_DIMENSION: u32 = 8192;

#[derive(Debug, thiserror::Error)]
pub enum OpenH264Error {
    #[error("failed to load verified Cisco OpenH264 2.6.0 binary {path}: {reason}")]
    Load { path: PathBuf, reason: String },
    #[error(
        "OpenH264 binary {path} reports {actual_major}.{actual_minor}.{actual_revision}; exactly 2.6.0 is required"
    )]
    Version {
        path: PathBuf,
        actual_major: u32,
        actual_minor: u32,
        actual_revision: u32,
    },
    #[error("OpenH264 decoder creation failed with code {0}")]
    Create(i64),
    #[error("OpenH264 decoder vtable is missing {0}")]
    MissingFunction(&'static str),
    #[error("OpenH264 decoder initialization failed with code {0}")]
    Initialize(i64),
    #[error("H.264 access unit exceeds OpenH264's i32 input limit")]
    InputTooLarge,
    #[error("OpenH264 decode failed with state {0:#x}")]
    Decode(i32),
    #[error("OpenH264 produced invalid I420 output: {0}")]
    InvalidOutput(String),
    #[error("I420 to RGBA conversion failed: {0}")]
    ColorConversion(String),
}

type UninitializeFn = unsafe extern "C" fn(*mut ISVCDecoder) -> std::os::raw::c_long;
type DecodeFrameNoDelayFn = unsafe extern "C" fn(
    *mut ISVCDecoder,
    *const c_uchar,
    c_int,
    *mut *mut c_uchar,
    *mut SBufferInfo,
) -> c_int;

/// Decoder backed only by an explicitly supplied, hash-verified Cisco OpenH264 2.6.0 binary.
pub struct OpenH264Decoder {
    api: DynamicAPI,
    decoder: *mut ISVCDecoder,
    uninitialize: UninitializeFn,
    decode_frame_no_delay: DecodeFrameNoDelayFn,
}

// OpenH264 decoder instances may move between threads but all access still requires `&mut self`.
unsafe impl Send for OpenH264Decoder {}

impl OpenH264Decoder {
    pub fn new(binary_path: &Path) -> Result<Self, OpenH264Error> {
        let api = DynamicAPI::from_blob_path(binary_path).map_err(|error| OpenH264Error::Load {
            path: binary_path.to_path_buf(),
            reason: error.to_string(),
        })?;
        let version = unsafe { api.WelsGetCodecVersion() };
        if (version.uMajor, version.uMinor, version.uRevision) != (2, 6, 0) {
            return Err(OpenH264Error::Version {
                path: binary_path.to_path_buf(),
                actual_major: version.uMajor,
                actual_minor: version.uMinor,
                actual_revision: version.uRevision,
            });
        }

        let mut decoder = null_mut::<ISVCDecoder>();
        let create_result = unsafe { api.WelsCreateDecoder(from_mut(&mut decoder)) };
        if create_result != 0 || decoder.is_null() {
            return Err(OpenH264Error::Create(create_result as i64));
        }

        let vtable = unsafe { &**decoder };
        let Some(initialize) = vtable.Initialize else {
            unsafe { api.WelsDestroyDecoder(decoder) };
            return Err(OpenH264Error::MissingFunction("Initialize"));
        };
        let Some(uninitialize) = vtable.Uninitialize else {
            unsafe { api.WelsDestroyDecoder(decoder) };
            return Err(OpenH264Error::MissingFunction("Uninitialize"));
        };
        let Some(decode_frame_no_delay) = vtable.DecodeFrameNoDelay else {
            unsafe { api.WelsDestroyDecoder(decoder) };
            return Err(OpenH264Error::MissingFunction("DecodeFrameNoDelay"));
        };

        let params = SDecodingParam {
            pFileNameRestructed: null_mut(),
            uiCpuLoad: 0,
            uiTargetDqLayer: 0,
            eEcActiveIdc: 0,
            bParseOnly: false,
            sVideoProperty: SVideoProperty {
                size: std::mem::size_of::<SVideoProperty>() as u32,
                eVideoBsType: VIDEO_BITSTREAM_AVC,
            },
        };
        let initialize_result = unsafe { initialize(decoder, &raw const params) };
        if initialize_result != 0 {
            unsafe { api.WelsDestroyDecoder(decoder) };
            return Err(OpenH264Error::Initialize(initialize_result as i64));
        }

        Ok(Self {
            api,
            decoder,
            uninitialize,
            decode_frame_no_delay,
        })
    }

    pub fn decode(
        &mut self,
        annexb: &[u8],
        id: u64,
        source: InputId,
        pts: MediaTime,
    ) -> Result<Option<VideoFrame>, OpenH264Error> {
        let input_len = i32::try_from(annexb.len()).map_err(|_| OpenH264Error::InputTooLarge)?;
        let mut planes = [null_mut::<u8>(); 3];
        let mut info = SBufferInfo::default();
        let state = unsafe {
            (self.decode_frame_no_delay)(
                self.decoder,
                annexb.as_ptr(),
                input_len,
                planes.as_mut_ptr(),
                &raw mut info,
            )
        };
        if state != dsErrorFree {
            return Err(OpenH264Error::Decode(state));
        }
        if info.iBufferStatus == 0 {
            return Ok(None);
        }

        let system = unsafe { info.UsrData.sSystemBuffer };
        if system.iFormat != videoFormatI420 {
            return Err(OpenH264Error::InvalidOutput(format!(
                "format {} is not I420 ({videoFormatI420})",
                system.iFormat
            )));
        }
        let width = positive_dimension(system.iWidth, "width")?;
        let height = positive_dimension(system.iHeight, "height")?;
        let y_stride = positive_stride(system.iStride[0], width, "Y")?;
        let chroma_width = width.div_ceil(2);
        let chroma_height = height.div_ceil(2);
        let uv_stride = positive_stride(system.iStride[1], chroma_width, "UV")?;
        if planes.iter().any(|plane| plane.is_null()) {
            return Err(OpenH264Error::InvalidOutput(
                "decoder returned a null I420 plane".into(),
            ));
        }

        let y_len = plane_len(y_stride, height, width)?;
        let uv_len = plane_len(uv_stride, chroma_height, chroma_width)?;
        let y_plane = unsafe { std::slice::from_raw_parts(planes[0], y_len) };
        let u_plane = unsafe { std::slice::from_raw_parts(planes[1], uv_len) };
        let v_plane = unsafe { std::slice::from_raw_parts(planes[2], uv_len) };
        let image = yuv::YuvPlanarImage {
            y_plane,
            y_stride,
            u_plane,
            u_stride: uv_stride,
            v_plane,
            v_stride: uv_stride,
            width,
            height,
        };
        let output_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| OpenH264Error::InvalidOutput("RGBA size overflow".into()))?;
        let mut rgba = vec![0; output_len];
        yuv::yuv420_to_rgba(
            &image,
            &mut rgba,
            width * 4,
            yuv::YuvRange::Limited,
            yuv::YuvStandardMatrix::Bt709,
        )
        .map_err(|error| OpenH264Error::ColorConversion(error.to_string()))?;

        Ok(Some(VideoFrame {
            id,
            source: Some(source),
            pts,
            capture_domain: ClockDomain::SourceMedia,
            width,
            height,
            format: PixelFormat::Rgba8,
            data: rgba.into(),
            discontinuity: false,
        }))
    }
}

impl Drop for OpenH264Decoder {
    fn drop(&mut self) {
        unsafe {
            (self.uninitialize)(self.decoder);
            self.api.WelsDestroyDecoder(self.decoder);
        }
    }
}

fn positive_dimension(value: i32, name: &str) -> Result<u32, OpenH264Error> {
    let value = u32::try_from(value)
        .map_err(|_| OpenH264Error::InvalidOutput(format!("{name} is not positive")))?;
    if value == 0 || value > MAX_DECODE_DIMENSION {
        return Err(OpenH264Error::InvalidOutput(format!(
            "{name} {value} is outside 1..={MAX_DECODE_DIMENSION}"
        )));
    }
    Ok(value)
}

fn positive_stride(value: i32, minimum: u32, plane: &str) -> Result<u32, OpenH264Error> {
    let value = u32::try_from(value)
        .map_err(|_| OpenH264Error::InvalidOutput(format!("{plane} stride is negative")))?;
    if value < minimum {
        return Err(OpenH264Error::InvalidOutput(format!(
            "{plane} stride {value} is less than width {minimum}"
        )));
    }
    Ok(value)
}

fn plane_len(stride: u32, height: u32, width: u32) -> Result<usize, OpenH264Error> {
    (stride as usize)
        .checked_mul(height.saturating_sub(1) as usize)
        .and_then(|offset| offset.checked_add(width as usize))
        .ok_or_else(|| OpenH264Error::InvalidOutput("plane size overflow".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_binary_is_a_hard_error() {
        let path = std::env::temp_dir().join(format!(
            "eiviz-openh264-does-not-exist-{}",
            std::process::id()
        ));
        let error = OpenH264Decoder::new(&path).unwrap_err();
        assert!(matches!(error, OpenH264Error::Load { .. }));
        assert!(error.to_string().contains("Cisco OpenH264 2.6.0"));
    }
}
