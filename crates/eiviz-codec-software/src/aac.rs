use std::ffi::{c_int, c_uint, c_void};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

const TT_MP4_RAW: c_int = 0;
const AAC_DEC_OK: c_int = 0;
const AAC_LC_OBJECT_TYPE: u8 = 2;
const AAC_LC_FRAME_LENGTH: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AacLcConfig {
    pub audio_specific_config: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u16,
    pub frame_length: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum AacDecoderError {
    #[error("invalid AAC AudioSpecificConfig: {0}")]
    InvalidConfig(String),
    #[error("unsupported AAC profile: {0}")]
    UnsupportedConfig(String),
    #[error("failed to load explicit license-reviewed FDK AAC binary {path}: {reason}")]
    Load { path: PathBuf, reason: String },
    #[error("FDK AAC binary {path} is missing {symbol}: {reason}")]
    MissingSymbol {
        path: PathBuf,
        symbol: &'static str,
        reason: String,
    },
    #[error("FDK aacDecoder_Open returned a null handle")]
    Open,
    #[error("FDK {operation} failed with AAC_DECODER_ERROR {code:#x}")]
    Decode {
        operation: &'static str,
        code: c_int,
    },
    #[error("AAC access unit is empty")]
    EmptyAccessUnit,
    #[error("AAC access unit exceeds FDK's u32 input limit")]
    AccessUnitTooLarge,
}

impl AacLcConfig {
    pub fn parse(audio_specific_config: &[u8]) -> Result<Self, AacDecoderError> {
        let mut bits = BitReader::new(audio_specific_config);
        let object_type = read_object_type(&mut bits)?;
        if object_type != AAC_LC_OBJECT_TYPE {
            return Err(AacDecoderError::UnsupportedConfig(format!(
                "only AAC-LC object type 2 is accepted, found {object_type}"
            )));
        }
        let frequency_index = bits.read(4)? as usize;
        let sample_rate = if frequency_index == 15 {
            bits.read(24)?
        } else {
            *[
                96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000,
                11_025, 8_000, 7_350,
            ]
            .get(frequency_index)
            .ok_or_else(|| {
                AacDecoderError::InvalidConfig(format!(
                    "reserved sampling-frequency index {frequency_index}"
                ))
            })?
        };
        let channel_configuration = bits.read(4)?;
        let channels = match channel_configuration {
            1 => 1,
            2 => 2,
            value => {
                return Err(AacDecoderError::UnsupportedConfig(format!(
                    "AAC-LC vertical slice accepts mono/stereo channel configuration 1 or 2, found {value}"
                )));
            }
        };
        let frame_length_flag = bits.read(1)?;
        let depends_on_core_coder = bits.read(1)?;
        let extension_flag = bits.read(1)?;
        if frame_length_flag != 0 {
            return Err(AacDecoderError::UnsupportedConfig(
                "960-sample AAC frames are not supported".into(),
            ));
        }
        if depends_on_core_coder != 0 || extension_flag != 0 {
            return Err(AacDecoderError::UnsupportedConfig(
                "AAC core-coder dependency and extension flags are not supported".into(),
            ));
        }
        Ok(Self {
            audio_specific_config: audio_specific_config.to_vec(),
            sample_rate,
            channels,
            frame_length: AAC_LC_FRAME_LENGTH,
        })
    }
}

fn read_object_type(bits: &mut BitReader<'_>) -> Result<u8, AacDecoderError> {
    let object_type = bits.read(5)? as u8;
    if object_type == 31 {
        let extended = bits.read(6)? as u8;
        Ok(32u8.saturating_add(extended))
    } else {
        Ok(object_type)
    }
}

struct BitReader<'a> {
    bytes: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, bit: 0 }
    }

    fn read(&mut self, count: usize) -> Result<u32, AacDecoderError> {
        let end = self
            .bit
            .checked_add(count)
            .ok_or_else(|| AacDecoderError::InvalidConfig("bit offset overflow".into()))?;
        if end > self.bytes.len().saturating_mul(8) {
            return Err(AacDecoderError::InvalidConfig(
                "truncated AudioSpecificConfig".into(),
            ));
        }
        let mut value = 0u32;
        for bit in self.bit..end {
            value = (value << 1) | u32::from((self.bytes[bit / 8] >> (7 - bit % 8)) & 1);
        }
        self.bit = end;
        Ok(value)
    }
}

type FdkHandle = *mut c_void;
type DecoderOpen = unsafe extern "C" fn(c_int, c_uint) -> FdkHandle;
type DecoderConfigRaw = unsafe extern "C" fn(FdkHandle, *mut *mut u8, *const c_uint) -> c_int;
type DecoderFill =
    unsafe extern "C" fn(FdkHandle, *mut *mut u8, *const c_uint, *mut c_uint) -> c_int;
type DecoderDecodeFrame = unsafe extern "C" fn(FdkHandle, *mut i16, c_int, c_uint) -> c_int;
type DecoderClose = unsafe extern "C" fn(FdkHandle);

/// AAC-LC raw access-unit decoder backed only by an explicitly supplied FDK binary.
pub struct FdkAacDecoder {
    _library: libloading::Library,
    handle: FdkHandle,
    fill: DecoderFill,
    decode_frame: DecoderDecodeFrame,
    close: DecoderClose,
    config: AacLcConfig,
}

// An FDK decoder is movable between media threads; calls still require exclusive access.
unsafe impl Send for FdkAacDecoder {}

impl FdkAacDecoder {
    pub fn new(path: &Path, config: AacLcConfig) -> Result<Self, AacDecoderError> {
        let library =
            unsafe { libloading::Library::new(path) }.map_err(|error| AacDecoderError::Load {
                path: path.to_path_buf(),
                reason: error.to_string(),
            })?;
        let open: DecoderOpen =
            load_symbol(&library, b"aacDecoder_Open\0", "aacDecoder_Open", path)?;
        let configure: DecoderConfigRaw = load_symbol(
            &library,
            b"aacDecoder_ConfigRaw\0",
            "aacDecoder_ConfigRaw",
            path,
        )?;
        let fill = load_symbol(&library, b"aacDecoder_Fill\0", "aacDecoder_Fill", path)?;
        let decode_frame = load_symbol(
            &library,
            b"aacDecoder_DecodeFrame\0",
            "aacDecoder_DecodeFrame",
            path,
        )?;
        let close = load_symbol(&library, b"aacDecoder_Close\0", "aacDecoder_Close", path)?;
        let handle = unsafe { open(TT_MP4_RAW, 1) };
        if handle.is_null() {
            return Err(AacDecoderError::Open);
        }
        let mut asc = config.audio_specific_config.clone();
        let mut asc_ptr = asc.as_mut_ptr();
        let asc_len = c_uint::try_from(asc.len()).map_err(|_| {
            AacDecoderError::InvalidConfig("AudioSpecificConfig exceeds FDK's u32 limit".into())
        })?;
        let code = unsafe { configure(handle, &raw mut asc_ptr, &raw const asc_len) };
        if code != AAC_DEC_OK {
            unsafe { close(handle) };
            return Err(AacDecoderError::Decode {
                operation: "aacDecoder_ConfigRaw",
                code,
            });
        }
        Ok(Self {
            _library: library,
            handle,
            fill,
            decode_frame,
            close,
            config,
        })
    }

    pub fn config(&self) -> &AacLcConfig {
        &self.config
    }

    /// Decodes exactly one raw AAC-LC access unit to channel-major f32.
    pub fn decode(&mut self, access_unit: &[u8]) -> Result<Vec<Vec<f32>>, AacDecoderError> {
        if access_unit.is_empty() {
            return Err(AacDecoderError::EmptyAccessUnit);
        }
        let input_size =
            c_uint::try_from(access_unit.len()).map_err(|_| AacDecoderError::AccessUnitTooLarge)?;
        let mut bytes_valid = input_size;
        let mut input = access_unit.to_vec();
        let mut input_ptr = input.as_mut_ptr();
        let fill_code = unsafe {
            (self.fill)(
                self.handle,
                &raw mut input_ptr,
                &raw const input_size,
                &raw mut bytes_valid,
            )
        };
        fdk_ok(fill_code, "aacDecoder_Fill")?;
        if bytes_valid != 0 {
            return Err(AacDecoderError::Decode {
                operation: "aacDecoder_Fill (access unit not fully consumed)",
                code: bytes_valid as c_int,
            });
        }

        let sample_count = self
            .config
            .frame_length
            .saturating_mul(self.config.channels as usize);
        let mut interleaved = vec![0i16; sample_count];
        let capacity = c_int::try_from(sample_count).map_err(|_| {
            AacDecoderError::InvalidConfig("decoded PCM capacity exceeds FDK's i32 limit".into())
        })?;
        let decode_code =
            unsafe { (self.decode_frame)(self.handle, interleaved.as_mut_ptr(), capacity, 0) };
        fdk_ok(decode_code, "aacDecoder_DecodeFrame")?;

        let mut planes = vec![vec![0.0; self.config.frame_length]; self.config.channels as usize];
        for (frame, samples) in interleaved
            .chunks_exact(self.config.channels as usize)
            .enumerate()
        {
            for (channel, sample) in samples.iter().enumerate() {
                planes[channel][frame] = f32::from(*sample) / 32768.0;
            }
        }
        Ok(planes)
    }
}

impl Drop for FdkAacDecoder {
    fn drop(&mut self) {
        if self.handle != null_mut() {
            unsafe { (self.close)(self.handle) };
        }
    }
}

fn load_symbol<T: Copy>(
    library: &libloading::Library,
    symbol: &'static [u8],
    name: &'static str,
    path: &Path,
) -> Result<T, AacDecoderError> {
    unsafe { library.get::<T>(symbol) }
        .map(|value| *value)
        .map_err(|error| AacDecoderError::MissingSymbol {
            path: path.to_path_buf(),
            symbol: name,
            reason: error.to_string(),
        })
}

fn fdk_ok(code: c_int, operation: &'static str) -> Result<(), AacDecoderError> {
    if code == AAC_DEC_OK {
        Ok(())
    } else {
        Err(AacDecoderError::Decode { operation, code })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aac_lc_audio_specific_config() {
        let config = AacLcConfig::parse(&[0x12, 0x10]).unwrap();
        assert_eq!(config.sample_rate, 44_100);
        assert_eq!(config.channels, 2);
        assert_eq!(config.frame_length, 1024);
    }

    #[test]
    fn rejects_non_lc_and_truncated_config() {
        assert!(AacLcConfig::parse(&[0x2a, 0x10]).is_err());
        assert!(AacLcConfig::parse(&[0x12]).is_err());
    }

    #[test]
    fn missing_fdk_binary_is_a_hard_error() {
        let path =
            std::env::temp_dir().join(format!("eiviz-missing-fdk-decoder-{}", std::process::id()));
        let error = match FdkAacDecoder::new(&path, AacLcConfig::parse(&[0x11, 0x90]).unwrap()) {
            Ok(_) => panic!("missing FDK binary must not construct a decoder"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("license-reviewed FDK AAC binary")
        );
        assert!(error.to_string().contains(path.to_string_lossy().as_ref()));
    }
}
