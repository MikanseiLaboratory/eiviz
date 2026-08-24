use crate::{extract_sps_pps, split_annexb};
use eiviz_core::{AacEncoderProfile, H264EncoderProfile};
use eiviz_media::{
    AudioBuffer, EncodedAccessUnit, EncodedKind, EncodedStreamConfig, MediaError, Result,
    VideoFrame,
};
use eiviz_time::{FrameRate, MediaTime, Rational};
use openh264_sys2::{
    API, CAMERA_VIDEO_REAL_TIME, CONSTANT_ID, DynamicAPI, ISVCEncoder, MEDIUM_COMPLEXITY,
    PRO_BASELINE, RC_BITRATE_MODE, SEncParamExt, SFrameBSInfo, SSourcePicture, videoFormatI420,
    videoFrameTypeIDR, videoFrameTypeSkip,
};
use std::ffi::{c_int, c_uint, c_void};
use std::path::{Path, PathBuf};
use std::ptr::{from_mut, null_mut};
use std::sync::Arc;

const FDK_AAC_LC_OBJECT_TYPE: c_uint = 2;
const FDK_TRANSMUX_RAW: c_uint = 0;
const FDK_CHANNEL_MODE_MONO: c_uint = 1;
const FDK_CHANNEL_MODE_STEREO: c_uint = 2;
const FDK_AACENC_AOT: c_int = 0x0100;
const FDK_AACENC_BITRATE: c_int = 0x0101;
const FDK_AACENC_SAMPLERATE: c_int = 0x0103;
const FDK_AACENC_CHANNELMODE: c_int = 0x0104;
const FDK_AACENC_TRANSMUX: c_int = 0x0105;
const FDK_AACENC_AFTERBURNER: c_int = 0x0200;
const FDK_IN_AUDIO_DATA: c_int = 0;
const FDK_OUT_BITSTREAM_DATA: c_int = 3;

#[derive(Clone, Debug)]
pub struct EncoderSessionRequest {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub video: H264EncoderProfile,
    pub audio: AacEncoderProfile,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EncoderDiagnostics {
    pub video_backend: String,
    pub audio_backend: String,
    pub video_frames: u64,
    pub keyframes: u64,
    pub audio_access_units: u64,
    pub idr_requests: u64,
    pub last_error: Option<String>,
}

pub trait ProgramEncoder: Send {
    fn stream_config(&self) -> &EncodedStreamConfig;
    fn encode(
        &mut self,
        video: &VideoFrame,
        audio: &AudioBuffer,
    ) -> Result<Vec<Arc<EncodedAccessUnit>>>;
    fn request_idr(&mut self) -> Result<()>;
    fn diagnostics(&self) -> EncoderDiagnostics;
}

pub trait ProgramEncoderFactory: Send + Sync {
    fn create(&self, request: &EncoderSessionRequest) -> Result<Box<dyn ProgramEncoder>>;
    fn description(&self) -> String;
}

/// Explicit dynamic codec selection. Neither binary is bundled, discovered,
/// compiled from source, or replaced by another codec.
#[derive(Clone, Debug)]
pub struct DynamicEncoderFactory {
    openh264_binary: PathBuf,
    fdk_aac_binary: Option<PathBuf>,
}

impl DynamicEncoderFactory {
    pub fn new(openh264_binary: impl Into<PathBuf>, fdk_aac_binary: Option<PathBuf>) -> Self {
        Self {
            openh264_binary: openh264_binary.into(),
            fdk_aac_binary,
        }
    }
}

impl ProgramEncoderFactory for DynamicEncoderFactory {
    fn create(&self, request: &EncoderSessionRequest) -> Result<Box<dyn ProgramEncoder>> {
        let (bitrate_bps, keyframe_interval_frames, level_idc) = match request.video {
            H264EncoderProfile::CiscoOpenH26426 {
                bitrate_bps,
                keyframe_interval_frames,
                level_idc,
            } => (bitrate_bps, keyframe_interval_frames, level_idc),
            H264EncoderProfile::ExternalAnnexB { ref adapter, .. } => {
                return Err(MediaError::Unsupported(format!(
                    "external H.264 adapter {adapter:?} requires a registered ProgramEncoderFactory"
                )));
            }
        };
        let (audio_bitrate, sample_rate, channels) = match request.audio {
            AacEncoderProfile::FdkAacLc {
                bitrate_bps,
                sample_rate,
                channels,
            } => (bitrate_bps, sample_rate, channels),
            AacEncoderProfile::ExternalRawAacLc { ref adapter, .. } => {
                return Err(MediaError::Unsupported(format!(
                    "external AAC adapter {adapter:?} requires a registered ProgramEncoderFactory"
                )));
            }
        };
        let fdk_path = self.fdk_aac_binary.as_deref().ok_or_else(|| {
            MediaError::Unsupported(
                "FDK AAC-LC dynamic backend selected, but no explicit license-reviewed binary path was configured; PCM/test bytes are not substituted".into(),
            )
        })?;
        Ok(Box::new(DynamicProgramEncoder::new(
            &self.openh264_binary,
            fdk_path,
            request.width,
            request.height,
            request.frame_rate,
            bitrate_bps,
            keyframe_interval_frames,
            level_idc,
            audio_bitrate,
            sample_rate,
            channels,
        )?))
    }

    fn description(&self) -> String {
        format!(
            "Cisco OpenH264 2.6.0={} (SHA-256 allow-list + runtime version); FDK AAC-LC={}",
            self.openh264_binary.display(),
            self.fdk_aac_binary.as_deref().map_or_else(
                || "not configured".into(),
                |path| path.display().to_string()
            )
        )
    }
}

struct DynamicProgramEncoder {
    video: OpenH264Encoder,
    audio: FdkAacEncoder,
    stream_config: EncodedStreamConfig,
    diagnostics: EncoderDiagnostics,
}

impl DynamicProgramEncoder {
    #[allow(clippy::too_many_arguments)]
    fn new(
        openh264_path: &Path,
        fdk_path: &Path,
        width: u32,
        height: u32,
        frame_rate: FrameRate,
        video_bitrate: u32,
        keyframe_interval: u32,
        level_idc: u8,
        audio_bitrate: u32,
        sample_rate: u32,
        channels: u16,
    ) -> Result<Self> {
        let video = OpenH264Encoder::new(
            openh264_path,
            width,
            height,
            frame_rate,
            video_bitrate,
            keyframe_interval,
            level_idc,
        )?;
        let audio = FdkAacEncoder::new(fdk_path, audio_bitrate, sample_rate, channels)?;
        let video_width = u16::try_from(width)
            .map_err(|_| MediaError::Unsupported("video width exceeds u16".into()))?;
        let video_height = u16::try_from(height)
            .map_err(|_| MediaError::Unsupported("video height exceeds u16".into()))?;
        let stream_config = EncodedStreamConfig {
            h264_sps: video.sps.clone().into(),
            h264_pps: video.pps.clone().into(),
            aac_audio_specific_config: audio.audio_specific_config.clone().into(),
            video_width,
            video_height,
            video_timescale: frame_rate.numerator(),
            video_sample_duration: frame_rate.denominator(),
            audio_sample_rate: sample_rate,
            audio_channels: channels,
        };
        Ok(Self {
            video,
            audio,
            stream_config,
            diagnostics: EncoderDiagnostics {
                video_backend: format!(
                    "Cisco OpenH264 2.6.0 dynamic ({})",
                    openh264_path.display()
                ),
                audio_backend: format!("FDK AAC-LC dynamic ({})", fdk_path.display()),
                ..Default::default()
            },
        })
    }
}

impl ProgramEncoder for DynamicProgramEncoder {
    fn stream_config(&self) -> &EncodedStreamConfig {
        &self.stream_config
    }

    fn encode(
        &mut self,
        video: &VideoFrame,
        audio: &AudioBuffer,
    ) -> Result<Vec<Arc<EncodedAccessUnit>>> {
        let result: Result<Vec<Arc<EncodedAccessUnit>>> = (|| {
            let video = Arc::new(self.video.encode(video)?);
            self.diagnostics.video_frames = self.diagnostics.video_frames.saturating_add(1);
            if video.keyframe {
                self.diagnostics.keyframes = self.diagnostics.keyframes.saturating_add(1);
            }
            let mut units = vec![video];
            let audio_units = self.audio.encode(audio)?;
            self.diagnostics.audio_access_units = self
                .diagnostics
                .audio_access_units
                .saturating_add(audio_units.len() as u64);
            units.extend(audio_units.into_iter().map(Arc::new));
            Ok(units)
        })();
        if let Err(error) = &result {
            self.diagnostics.last_error = Some(error.to_string());
        }
        result
    }

    fn request_idr(&mut self) -> Result<()> {
        self.video.request_idr()?;
        self.diagnostics.idr_requests = self.diagnostics.idr_requests.saturating_add(1);
        Ok(())
    }

    fn diagnostics(&self) -> EncoderDiagnostics {
        self.diagnostics.clone()
    }
}

type EncoderUninitialize = unsafe extern "C" fn(*mut ISVCEncoder) -> c_int;
type EncoderEncodeFrame =
    unsafe extern "C" fn(*mut ISVCEncoder, *const SSourcePicture, *mut SFrameBSInfo) -> c_int;
type EncoderParameterSets = unsafe extern "C" fn(*mut ISVCEncoder, *mut SFrameBSInfo) -> c_int;
type EncoderForceIntra = unsafe extern "C" fn(*mut ISVCEncoder, bool) -> c_int;

struct OpenH264Encoder {
    api: DynamicAPI,
    encoder: *mut ISVCEncoder,
    uninitialize: EncoderUninitialize,
    encode_frame: EncoderEncodeFrame,
    force_intra: EncoderForceIntra,
    width: u32,
    height: u32,
    i420: Vec<u8>,
    sps: Vec<u8>,
    pps: Vec<u8>,
}

// OpenH264 instances may move to an Engine thread; calls remain serialized by &mut self.
unsafe impl Send for OpenH264Encoder {}

impl OpenH264Encoder {
    #[allow(clippy::too_many_arguments)]
    fn new(
        binary_path: &Path,
        width: u32,
        height: u32,
        frame_rate: FrameRate,
        bitrate_bps: u32,
        keyframe_interval_frames: u32,
        level_idc: u8,
    ) -> Result<Self> {
        if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(MediaError::Unsupported(
                "Cisco OpenH264 I420 encoding requires non-zero even dimensions".into(),
            ));
        }
        let width_i32 = i32::try_from(width)
            .map_err(|_| MediaError::Unsupported("video width exceeds i32".into()))?;
        let height_i32 = i32::try_from(height)
            .map_err(|_| MediaError::Unsupported("video height exceeds i32".into()))?;
        let bitrate_i32 = i32::try_from(bitrate_bps)
            .map_err(|_| MediaError::Unsupported("H.264 bitrate exceeds i32".into()))?;
        let i420_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(3))
            .map(|bytes| bytes / 2)
            .ok_or_else(|| MediaError::Unsupported("I420 allocation overflow".into()))?;

        // from_blob_path performs the crate's Cisco binary SHA-256 allow-list check
        // before loading. The reported ABI version is checked independently below.
        let api = DynamicAPI::from_blob_path(binary_path).map_err(|error| {
            MediaError::Unsupported(format!(
                "failed to load hash-verified Cisco OpenH264 2.6.0 binary {}: {error}",
                binary_path.display()
            ))
        })?;
        let version = unsafe { api.WelsGetCodecVersion() };
        if (version.uMajor, version.uMinor, version.uRevision) != (2, 6, 0) {
            return Err(MediaError::Unsupported(format!(
                "Cisco OpenH264 binary {} reports {}.{}.{}; exactly 2.6.0 is required",
                binary_path.display(),
                version.uMajor,
                version.uMinor,
                version.uRevision
            )));
        }
        let mut encoder = null_mut::<ISVCEncoder>();
        let create = unsafe { api.WelsCreateSVCEncoder(from_mut(&mut encoder)) };
        if create != 0 || encoder.is_null() {
            return Err(MediaError::Other(format!(
                "OpenH264 encoder creation failed with code {create}"
            )));
        }
        let vtable = unsafe { &**encoder };
        let get_default_params =
            required(vtable.GetDefaultParams, "GetDefaultParams", &api, encoder)?;
        let initialize_ext = required(vtable.InitializeExt, "InitializeExt", &api, encoder)?;
        let uninitialize = required(vtable.Uninitialize, "Uninitialize", &api, encoder)?;
        let encode_frame = required(vtable.EncodeFrame, "EncodeFrame", &api, encoder)?;
        let encode_parameter_sets = required(
            vtable.EncodeParameterSets,
            "EncodeParameterSets",
            &api,
            encoder,
        )?;
        let force_intra = required(vtable.ForceIntraFrame, "ForceIntraFrame", &api, encoder)?;

        let mut params = SEncParamExt::default();
        let defaults = unsafe { get_default_params(encoder, &raw mut params) };
        if defaults != 0 {
            unsafe { api.WelsDestroySVCEncoder(encoder) };
            return Err(MediaError::Other(format!(
                "OpenH264 GetDefaultParams failed with code {defaults}"
            )));
        }
        let fps = frame_rate.numerator() as f32 / frame_rate.denominator() as f32;
        params.iUsageType = CAMERA_VIDEO_REAL_TIME;
        params.iPicWidth = width_i32;
        params.iPicHeight = height_i32;
        params.iTargetBitrate = bitrate_i32;
        params.iRCMode = RC_BITRATE_MODE;
        params.fMaxFrameRate = fps;
        params.iTemporalLayerNum = 1;
        params.iSpatialLayerNum = 1;
        params.iComplexityMode = MEDIUM_COMPLEXITY;
        params.uiIntraPeriod = keyframe_interval_frames;
        params.eSpsPpsIdStrategy = CONSTANT_ID;
        params.bEnableFrameSkip = false;
        params.iMaxBitrate = bitrate_i32;
        params.bEnableFrameCroppingFlag = true;
        params.iMultipleThreadIdc = 1;
        let layer = &mut params.sSpatialLayers[0];
        layer.iVideoWidth = width_i32;
        layer.iVideoHeight = height_i32;
        layer.fFrameRate = fps;
        layer.iSpatialBitrate = bitrate_i32;
        layer.iMaxSpatialBitrate = bitrate_i32;
        layer.uiProfileIdc = PRO_BASELINE;
        layer.uiLevelIdc = level_idc as i32;
        layer.bVideoSignalTypePresent = true;
        layer.uiVideoFormat = 5; // unspecified source format
        layer.bFullRange = false;
        layer.bColorDescriptionPresent = true;
        layer.uiColorPrimaries = 1; // BT.709
        layer.uiTransferCharacteristics = 1; // BT.709
        layer.uiColorMatrix = 1; // BT.709
        let initialized = unsafe { initialize_ext(encoder, &raw const params) };
        if initialized != 0 {
            unsafe { api.WelsDestroySVCEncoder(encoder) };
            return Err(MediaError::Other(format!(
                "OpenH264 InitializeExt failed with code {initialized}"
            )));
        }
        let mut parameter_info = SFrameBSInfo::default();
        let parameter_result = unsafe { encode_parameter_sets(encoder, &raw mut parameter_info) };
        if parameter_result != 0 {
            unsafe {
                uninitialize(encoder);
                api.WelsDestroySVCEncoder(encoder);
            }
            return Err(MediaError::Other(format!(
                "OpenH264 EncodeParameterSets failed with code {parameter_result}"
            )));
        }
        let parameter_bytes = collect_openh264_layers(&parameter_info)?;
        let parameter_au = EncodedAccessUnit {
            pts: MediaTime::ZERO,
            dts: Some(MediaTime::ZERO),
            keyframe: true,
            bytes: parameter_bytes.into(),
            kind: EncodedKind::Avc,
        };
        let (sps, pps) = extract_sps_pps(&parameter_au);
        if sps.is_empty() || pps.is_empty() {
            unsafe {
                uninitialize(encoder);
                api.WelsDestroySVCEncoder(encoder);
            }
            return Err(MediaError::Other(
                "OpenH264 did not emit both SPS and PPS".into(),
            ));
        }
        Ok(Self {
            api,
            encoder,
            uninitialize,
            encode_frame,
            force_intra,
            width,
            height,
            i420: vec![0; i420_len],
            sps,
            pps,
        })
    }

    fn request_idr(&mut self) -> Result<()> {
        let code = unsafe { (self.force_intra)(self.encoder, true) };
        if code != 0 {
            return Err(MediaError::Other(format!(
                "OpenH264 ForceIntraFrame failed with code {code}"
            )));
        }
        Ok(())
    }

    fn encode(&mut self, frame: &VideoFrame) -> Result<EncodedAccessUnit> {
        if frame.width != self.width || frame.height != self.height {
            return Err(MediaError::Unsupported(format!(
                "OpenH264 session is {}x{}, received {}x{}",
                self.width, self.height, frame.width, frame.height
            )));
        }
        rgba_to_i420_bt709_limited(frame, &mut self.i420)?;
        let y_len = (self.width * self.height) as usize;
        let uv_len = y_len / 4;
        let picture = SSourcePicture {
            iColorFormat: videoFormatI420,
            iStride: [self.width as c_int, (self.width / 2) as c_int, 0, 0],
            pData: [
                self.i420.as_mut_ptr(),
                unsafe { self.i420.as_mut_ptr().add(y_len) },
                unsafe { self.i420.as_mut_ptr().add(y_len + uv_len) },
                null_mut(),
            ],
            iPicWidth: self.width as c_int,
            iPicHeight: self.height as c_int,
            uiTimeStamp: media_time_millis(frame.pts),
            ..Default::default()
        };
        let mut info = SFrameBSInfo::default();
        let code = unsafe { (self.encode_frame)(self.encoder, &raw const picture, &raw mut info) };
        if code != 0 {
            return Err(MediaError::Other(format!(
                "OpenH264 EncodeFrame failed with code {code}"
            )));
        }
        if info.eFrameType == videoFrameTypeSkip {
            return Err(MediaError::Other(
                "OpenH264 skipped a Program frame; frame skipping is disabled".into(),
            ));
        }
        let bytes = collect_openh264_layers(&info)?;
        if bytes.is_empty() {
            return Err(MediaError::Other(
                "OpenH264 returned an empty access unit".into(),
            ));
        }
        Ok(EncodedAccessUnit {
            pts: frame.pts,
            dts: Some(frame.pts),
            keyframe: info.eFrameType == videoFrameTypeIDR,
            bytes: bytes.into(),
            kind: EncodedKind::Avc,
        })
    }
}

impl Drop for OpenH264Encoder {
    fn drop(&mut self) {
        unsafe {
            (self.uninitialize)(self.encoder);
            self.api.WelsDestroySVCEncoder(self.encoder);
        }
    }
}

fn required<T: Copy>(
    value: Option<T>,
    name: &'static str,
    api: &DynamicAPI,
    encoder: *mut ISVCEncoder,
) -> Result<T> {
    value.ok_or_else(|| {
        unsafe { api.WelsDestroySVCEncoder(encoder) };
        MediaError::Other(format!("OpenH264 encoder vtable is missing {name}"))
    })
}

fn collect_openh264_layers(info: &SFrameBSInfo) -> Result<Vec<u8>> {
    let layer_count = usize::try_from(info.iLayerNum)
        .map_err(|_| MediaError::Other("OpenH264 returned negative layer count".into()))?;
    if layer_count > info.sLayerInfo.len() {
        return Err(MediaError::Other(format!(
            "OpenH264 returned invalid layer count {layer_count}"
        )));
    }
    let mut output = Vec::new();
    for layer in &info.sLayerInfo[..layer_count] {
        let nal_count = usize::try_from(layer.iNalCount)
            .map_err(|_| MediaError::Other("OpenH264 returned negative NAL count".into()))?;
        if nal_count == 0 {
            continue;
        }
        if layer.pNalLengthInByte.is_null() || layer.pBsBuf.is_null() {
            return Err(MediaError::Other(
                "OpenH264 returned null encoded layer pointers".into(),
            ));
        }
        let lengths = unsafe { std::slice::from_raw_parts(layer.pNalLengthInByte, nal_count) };
        let mut offset = 0usize;
        for &length in lengths {
            let length = usize::try_from(length)
                .map_err(|_| MediaError::Other("OpenH264 returned negative NAL length".into()))?;
            let end = offset
                .checked_add(length)
                .ok_or_else(|| MediaError::Other("OpenH264 NAL size overflow".into()))?;
            let nal = unsafe { std::slice::from_raw_parts(layer.pBsBuf.add(offset), length) };
            output.extend_from_slice(nal);
            offset = end;
        }
    }
    // OpenH264's encoder API emits Annex-B. Refuse malformed/non-Annex-B bytes
    // rather than silently adapting an unexpected ABI result.
    if !output.is_empty() && split_annexb(&output).is_empty() {
        return Err(MediaError::Other("OpenH264 output is not Annex-B".into()));
    }
    Ok(output)
}

#[repr(C)]
struct FdkBufDesc {
    num_bufs: c_int,
    bufs: *mut *mut c_void,
    buffer_identifiers: *mut c_int,
    buf_sizes: *mut c_int,
    buf_el_sizes: *mut c_int,
}

#[repr(C)]
#[derive(Default)]
struct FdkInArgs {
    num_in_samples: c_int,
    num_anc_bytes: c_int,
}

#[repr(C)]
#[derive(Default)]
struct FdkOutArgs {
    num_out_bytes: c_int,
    num_in_samples: c_int,
    num_anc_bytes: c_int,
    bit_res_state: c_int,
}

#[repr(C)]
struct FdkInfo {
    max_out_buf_bytes: c_uint,
    max_anc_bytes: c_uint,
    in_buf_fill_level: c_uint,
    input_channels: c_uint,
    frame_length: c_uint,
    n_delay: c_uint,
    n_delay_core: c_uint,
    conf_buf: [u8; 64],
    conf_size: c_uint,
}

impl Default for FdkInfo {
    fn default() -> Self {
        Self {
            max_out_buf_bytes: 0,
            max_anc_bytes: 0,
            in_buf_fill_level: 0,
            input_channels: 0,
            frame_length: 0,
            n_delay: 0,
            n_delay_core: 0,
            conf_buf: [0; 64],
            conf_size: 0,
        }
    }
}

type FdkHandle = *mut c_void;
type FdkOpen = unsafe extern "C" fn(*mut FdkHandle, c_uint, c_uint) -> c_int;
type FdkSetParam = unsafe extern "C" fn(FdkHandle, c_int, c_uint) -> c_int;
type FdkEncode = unsafe extern "C" fn(
    FdkHandle,
    *mut FdkBufDesc,
    *mut FdkBufDesc,
    *mut FdkInArgs,
    *mut FdkOutArgs,
) -> c_int;
type FdkInfoFn = unsafe extern "C" fn(FdkHandle, *mut FdkInfo) -> c_int;
type FdkClose = unsafe extern "C" fn(*mut FdkHandle) -> c_int;

struct FdkAacEncoder {
    _library: libloading::Library,
    handle: FdkHandle,
    encode_fn: FdkEncode,
    close_fn: FdkClose,
    sample_rate: u32,
    channels: u16,
    frame_length: usize,
    max_output_bytes: usize,
    pending: Vec<Vec<f32>>,
    pending_start: Option<u64>,
    audio_specific_config: Vec<u8>,
}

// The explicitly loaded FDK handle is used serially by its owning Engine session.
unsafe impl Send for FdkAacEncoder {}

impl FdkAacEncoder {
    fn new(path: &Path, bitrate_bps: u32, sample_rate: u32, channels: u16) -> Result<Self> {
        if !matches!(channels, 1 | 2) {
            return Err(MediaError::Unsupported(
                "FDK AAC-LC vertical slice supports explicit mono or stereo only".into(),
            ));
        }
        let library = unsafe { libloading::Library::new(path) }.map_err(|error| {
            MediaError::Unsupported(format!(
                "failed to load explicit license-reviewed FDK AAC binary {}: {error}",
                path.display()
            ))
        })?;
        let open: FdkOpen = load_fdk_symbol(&library, b"aacEncOpen\0", path)?;
        let set_param: FdkSetParam = load_fdk_symbol(&library, b"aacEncoder_SetParam\0", path)?;
        let encode_fn: FdkEncode = load_fdk_symbol(&library, b"aacEncEncode\0", path)?;
        let info_fn: FdkInfoFn = load_fdk_symbol(&library, b"aacEncInfo\0", path)?;
        let close_fn: FdkClose = load_fdk_symbol(&library, b"aacEncClose\0", path)?;
        let mut handle = null_mut();
        fdk_ok(
            unsafe { open(&raw mut handle, 0, channels.into()) },
            "aacEncOpen",
        )?;
        if handle.is_null() {
            return Err(MediaError::Other(
                "FDK aacEncOpen returned a null handle".into(),
            ));
        }
        let channel_mode = if channels == 1 {
            FDK_CHANNEL_MODE_MONO
        } else {
            FDK_CHANNEL_MODE_STEREO
        };
        let configure = (|| {
            for (parameter, value, name) in [
                (FDK_AACENC_AOT, FDK_AAC_LC_OBJECT_TYPE, "AAC object type"),
                (FDK_AACENC_BITRATE, bitrate_bps, "bitrate"),
                (FDK_AACENC_SAMPLERATE, sample_rate, "sample rate"),
                (FDK_AACENC_CHANNELMODE, channel_mode, "channel mode"),
                (FDK_AACENC_TRANSMUX, FDK_TRANSMUX_RAW, "raw transport"),
                (FDK_AACENC_AFTERBURNER, 1, "afterburner"),
            ] {
                fdk_ok(unsafe { set_param(handle, parameter, value) }, name)?;
            }
            fdk_ok(
                unsafe { encode_fn(handle, null_mut(), null_mut(), null_mut(), null_mut()) },
                "AAC encoder initialization",
            )?;
            let mut info = FdkInfo::default();
            fdk_ok(unsafe { info_fn(handle, &raw mut info) }, "aacEncInfo")?;
            let frame_length = usize::try_from(info.frame_length)
                .map_err(|_| MediaError::Other("FDK frame length overflow".into()))?;
            let max_output_bytes = usize::try_from(info.max_out_buf_bytes)
                .map_err(|_| MediaError::Other("FDK output size overflow".into()))?;
            let conf_size = usize::try_from(info.conf_size)
                .map_err(|_| MediaError::Other("FDK config size overflow".into()))?;
            if frame_length == 0 || max_output_bytes == 0 || !(2..=64).contains(&conf_size) {
                return Err(MediaError::Other(format!(
                    "FDK returned invalid info: frame={frame_length}, output={max_output_bytes}, config={conf_size}"
                )));
            }
            Ok((
                frame_length,
                max_output_bytes,
                info.conf_buf[..conf_size].to_vec(),
            ))
        })();
        let (frame_length, max_output_bytes, audio_specific_config) = match configure {
            Ok(values) => values,
            Err(error) => {
                unsafe { close_fn(&raw mut handle) };
                return Err(error);
            }
        };
        Ok(Self {
            _library: library,
            handle,
            encode_fn,
            close_fn,
            sample_rate,
            channels,
            frame_length,
            max_output_bytes,
            pending: vec![Vec::new(); channels as usize],
            pending_start: None,
            audio_specific_config,
        })
    }

    fn encode(&mut self, audio: &AudioBuffer) -> Result<Vec<EncodedAccessUnit>> {
        if audio.sample_rate != self.sample_rate || audio.channels != self.channels {
            return Err(MediaError::Unsupported(format!(
                "FDK session is {} Hz/{} ch, received {} Hz/{} ch",
                self.sample_rate, self.channels, audio.sample_rate, audio.channels
            )));
        }
        if audio.planes.len() != self.channels as usize {
            return Err(MediaError::Other(
                "audio plane count does not match channel count".into(),
            ));
        }
        let frames = audio.planes.first().map_or(0, Vec::len);
        if audio.planes.iter().any(|plane| plane.len() != frames) {
            return Err(MediaError::Other(
                "audio planes have unequal frame counts".into(),
            ));
        }
        let expected = self
            .pending_start
            .map(|start| start.saturating_add(self.pending[0].len() as u64));
        if audio.discontinuity || expected.is_some_and(|expected| expected != audio.sample_index) {
            for plane in &mut self.pending {
                plane.clear();
            }
            self.pending_start = None;
        }
        if self.pending_start.is_none() && frames > 0 {
            self.pending_start = Some(audio.sample_index);
        }
        for (pending, plane) in self.pending.iter_mut().zip(&audio.planes) {
            pending.extend_from_slice(plane);
        }

        let mut output = Vec::new();
        while self.pending[0].len() >= self.frame_length {
            let start = self.pending_start.expect("non-empty audio has start");
            let mut interleaved = Vec::with_capacity(self.frame_length * self.channels as usize);
            for frame in 0..self.frame_length {
                for channel in 0..self.channels as usize {
                    let sample = self.pending[channel][frame].clamp(-1.0, 1.0);
                    interleaved.push((sample * i16::MAX as f32).round() as i16);
                }
            }
            let mut encoded = vec![0u8; self.max_output_bytes];
            let mut in_ptr = interleaved.as_mut_ptr().cast::<c_void>();
            let mut in_id = FDK_IN_AUDIO_DATA;
            let mut in_size = c_int::try_from(interleaved.len() * size_of::<i16>())
                .map_err(|_| MediaError::Other("FDK input size exceeds i32".into()))?;
            let mut in_element_size = size_of::<i16>() as c_int;
            let mut input = FdkBufDesc {
                num_bufs: 1,
                bufs: &raw mut in_ptr,
                buffer_identifiers: &raw mut in_id,
                buf_sizes: &raw mut in_size,
                buf_el_sizes: &raw mut in_element_size,
            };
            let mut out_ptr = encoded.as_mut_ptr().cast::<c_void>();
            let mut out_id = FDK_OUT_BITSTREAM_DATA;
            let mut out_size = c_int::try_from(encoded.len())
                .map_err(|_| MediaError::Other("FDK output size exceeds i32".into()))?;
            let mut out_element_size = 1;
            let mut output_desc = FdkBufDesc {
                num_bufs: 1,
                bufs: &raw mut out_ptr,
                buffer_identifiers: &raw mut out_id,
                buf_sizes: &raw mut out_size,
                buf_el_sizes: &raw mut out_element_size,
            };
            let mut input_args = FdkInArgs {
                num_in_samples: c_int::try_from(interleaved.len())
                    .map_err(|_| MediaError::Other("FDK sample count exceeds i32".into()))?,
                num_anc_bytes: 0,
            };
            let mut output_args = FdkOutArgs::default();
            fdk_ok(
                unsafe {
                    (self.encode_fn)(
                        self.handle,
                        &raw mut input,
                        &raw mut output_desc,
                        &raw mut input_args,
                        &raw mut output_args,
                    )
                },
                "aacEncEncode",
            )?;
            let bytes = usize::try_from(output_args.num_out_bytes)
                .map_err(|_| MediaError::Other("FDK returned negative output size".into()))?;
            if bytes == 0 || bytes > encoded.len() {
                return Err(MediaError::Other(format!(
                    "FDK returned invalid AAC access-unit size {bytes}"
                )));
            }
            encoded.truncate(bytes);
            let pts = MediaTime::new(
                i64::try_from(start)
                    .map_err(|_| MediaError::Other("audio PTS exceeds i64".into()))?,
                Rational::new(1, self.sample_rate as i64)
                    .map_err(|error| MediaError::Other(error.to_string()))?,
            );
            output.push(EncodedAccessUnit {
                pts,
                dts: Some(pts),
                keyframe: false,
                bytes: encoded.into(),
                kind: EncodedKind::Aac,
            });
            for plane in &mut self.pending {
                plane.drain(..self.frame_length);
            }
            self.pending_start = Some(start.saturating_add(self.frame_length as u64));
        }
        Ok(output)
    }
}

impl Drop for FdkAacEncoder {
    fn drop(&mut self) {
        unsafe {
            (self.close_fn)(&raw mut self.handle);
        }
    }
}

fn load_fdk_symbol<T: Copy>(
    library: &libloading::Library,
    symbol: &'static [u8],
    path: &Path,
) -> Result<T> {
    unsafe { library.get::<T>(symbol) }
        .map(|value| *value)
        .map_err(|error| {
            MediaError::Unsupported(format!(
                "FDK AAC binary {} is missing {}: {error}",
                path.display(),
                String::from_utf8_lossy(symbol).trim_end_matches('\0')
            ))
        })
}

fn fdk_ok(code: c_int, operation: &str) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(MediaError::Other(format!(
            "FDK {operation} failed with AACENC_ERROR {code:#x}"
        )))
    }
}

fn media_time_millis(time: MediaTime) -> i64 {
    let base = time.timebase();
    let millis =
        time.ticks() as i128 * base.numerator() as i128 * 1_000 / base.denominator() as i128;
    millis.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

fn rgba_to_i420_bt709_limited(frame: &VideoFrame, output: &mut [u8]) -> Result<()> {
    let width = frame.width as usize;
    let height = frame.height as usize;
    let pixels = width
        .checked_mul(height)
        .ok_or_else(|| MediaError::Other("video dimensions overflow".into()))?;
    if frame.data.len() != pixels.saturating_mul(4) || output.len() != pixels * 3 / 2 {
        return Err(MediaError::Other(
            "RGBA/I420 buffer size does not match frame dimensions".into(),
        ));
    }
    let (y_plane, chroma) = output.split_at_mut(pixels);
    let (u_plane, v_plane) = chroma.split_at_mut(pixels / 4);
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) * 4;
            let r = frame.data[index] as i32;
            let g = frame.data[index + 1] as i32;
            let b = frame.data[index + 2] as i32;
            y_plane[y * width + x] =
                (((47 * r + 157 * g + 16 * b + 128) >> 8) + 16).clamp(16, 235) as u8;
        }
    }
    for y in 0..height / 2 {
        for x in 0..width / 2 {
            let mut r = 0i32;
            let mut g = 0i32;
            let mut b = 0i32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let index = (((y * 2 + dy) * width + x * 2 + dx) * 4) as usize;
                    r += frame.data[index] as i32;
                    g += frame.data[index + 1] as i32;
                    b += frame.data[index + 2] as i32;
                }
            }
            r = (r + 2) / 4;
            g = (g + 2) / 4;
            b = (b + 2) / 4;
            let chroma_index = y * (width / 2) + x;
            u_plane[chroma_index] =
                (((-26 * r - 87 * g + 112 * b + 128) >> 8) + 128).clamp(16, 240) as u8;
            v_plane[chroma_index] =
                (((112 * r - 102 * g - 10 * b + 128) >> 8) + 128).clamp(16, 240) as u8;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_time::NTSC_5994;

    fn request() -> EncoderSessionRequest {
        EncoderSessionRequest {
            width: 1920,
            height: 1080,
            frame_rate: NTSC_5994,
            video: H264EncoderProfile::CiscoOpenH26426 {
                bitrate_bps: 8_000_000,
                keyframe_interval_frames: 120,
                level_idc: 42,
            },
            audio: AacEncoderProfile::FdkAacLc {
                bitrate_bps: 192_000,
                sample_rate: 48_000,
                channels: 2,
            },
        }
    }

    #[test]
    fn missing_fdk_binary_is_a_hard_error_before_any_fallback() {
        let factory = DynamicEncoderFactory::new("/missing/openh264", None);
        let error = match factory.create(&request()) {
            Ok(_) => panic!("missing FDK binary must not construct an encoder"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("no explicit license-reviewed binary")
        );
        assert!(error.to_string().contains("not substituted"));
    }

    #[test]
    fn missing_openh264_binary_is_a_hard_error() {
        let factory =
            DynamicEncoderFactory::new("/missing/openh264", Some("/missing/fdk-aac".into()));
        let error = match factory.create(&request()) {
            Ok(_) => panic!("missing OpenH264 binary must not construct an encoder"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("hash-verified Cisco OpenH264 2.6.0")
        );
    }

    #[test]
    #[ignore = "set EIVIZ_OPENH264_HIL_BINARY and EIVIZ_FDK_AAC_HIL_BINARY"]
    fn real_dynamic_encoder_hil() {
        let openh264 =
            std::env::var_os("EIVIZ_OPENH264_HIL_BINARY").expect("set EIVIZ_OPENH264_HIL_BINARY");
        let fdk =
            std::env::var_os("EIVIZ_FDK_AAC_HIL_BINARY").expect("set EIVIZ_FDK_AAC_HIL_BINARY");
        let factory = DynamicEncoderFactory::new(openh264, Some(fdk.into()));
        let mut encoder = factory.create(&request()).unwrap();
        let frame = VideoFrame::rgba_solid(0, MediaTime::ZERO, 1920, 1080, [20, 40, 80, 255]);
        let audio = AudioBuffer::silence(0, 48_000, 2, 1024);
        let units = encoder.encode(&frame, &audio).unwrap();
        assert!(units.iter().any(|unit| unit.kind == EncodedKind::Avc));
        assert!(units.iter().any(|unit| unit.kind == EncodedKind::Aac));
        assert!(!encoder.stream_config().h264_sps.is_empty());
        assert!(!encoder.stream_config().aac_audio_specific_config.is_empty());
    }
}
