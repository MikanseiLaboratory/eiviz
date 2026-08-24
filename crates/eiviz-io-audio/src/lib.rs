use eiviz_core::DeviceBinding;
use eiviz_media::{AudioBuffer, Capability};
use std::fmt;
use std::str::FromStr;

#[cfg(feature = "cpal")]
mod native;
#[cfg(feature = "cpal")]
pub use native::{CpalInput, CpalOutput, enumerate_devices};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioBackend {
    Wasapi,
    Asio,
    CoreAudio,
    Alsa,
    PipeWire,
}

impl AudioBackend {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Wasapi => "wasapi",
            Self::Asio => "asio",
            Self::CoreAudio => "coreaudio",
            Self::Alsa => "alsa",
            Self::PipeWire => "pipewire",
        }
    }

    pub fn binding_kind(self) -> String {
        format!("audio:{}", self.id())
    }

    /// Hosts compiled into this target/profile. This does not claim that a host
    /// or device is available at runtime.
    pub fn compiled() -> Vec<Self> {
        #[cfg(not(feature = "cpal"))]
        {
            Vec::new()
        }
        #[cfg(feature = "cpal")]
        {
            vec![
                #[cfg(target_os = "windows")]
                Self::Wasapi,
                #[cfg(all(target_os = "windows", feature = "asio"))]
                Self::Asio,
                #[cfg(target_os = "macos")]
                Self::CoreAudio,
                #[cfg(target_os = "linux")]
                Self::Alsa,
                #[cfg(all(target_os = "linux", feature = "pipewire"))]
                Self::PipeWire,
            ]
        }
    }
}

impl fmt::Display for AudioBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

impl FromStr for AudioBackend {
    type Err = AudioError;

    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "wasapi" => Ok(Self::Wasapi),
            "asio" => Ok(Self::Asio),
            "coreaudio" => Ok(Self::CoreAudio),
            "alsa" => Ok(Self::Alsa),
            "pipewire" => Ok(Self::PipeWire),
            _ => Err(AudioError::BackendNotCompiled(value.to_owned())),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceDirection {
    Input,
    Output,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioDeviceInfo {
    pub backend: AudioBackend,
    pub persistent_id: String,
    pub display_name: String,
    pub supports_input: bool,
    pub supports_output: bool,
    pub default_input: bool,
    pub default_output: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingMatch {
    PersistentId,
    UniqueLogicalName,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedBinding {
    pub device: AudioDeviceInfo,
    pub matched_by: BindingMatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioStreamConfig {
    pub sample_rate: u32,
    pub channels: u16,
    /// Requested callback size. `None` explicitly requests the host default.
    pub buffer_frames: Option<u32>,
    /// Bounded adapter queue capacity in frames.
    pub ring_frames: usize,
}

impl Default for AudioStreamConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
            buffer_frames: Some(256),
            ring_frames: 48_000,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("CPAL audio is not compiled; enable the explicit `cpal` feature")]
    FeatureDisabled,
    #[error("audio backend `{0}` is not compiled for this target/profile")]
    BackendNotCompiled(String),
    #[error("audio backend `{backend}` is unavailable: {detail}")]
    BackendUnavailable {
        backend: AudioBackend,
        detail: String,
    },
    #[error("device binding kind `{actual}` does not select `{expected}`")]
    BackendMismatch { expected: String, actual: String },
    #[error("audio device binding `{0}` did not resolve")]
    DeviceNotFound(String),
    #[error("audio device logical name `{0}` is ambiguous")]
    AmbiguousDevice(String),
    #[error("audio device `{device}` does not support {direction:?}")]
    WrongDirection {
        device: String,
        direction: DeviceDirection,
    },
    #[error("audio device `{device}` has no exact {sample_rate} Hz/{channels} channel stream")]
    UnsupportedFormat {
        device: String,
        sample_rate: u32,
        channels: u16,
    },
    #[error("invalid audio buffer: {0}")]
    InvalidBuffer(String),
    #[error("audio backend error: {0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, AudioError>;

pub fn resolve_binding(
    binding: &DeviceBinding,
    backend: AudioBackend,
    direction: DeviceDirection,
    devices: &[AudioDeviceInfo],
) -> Result<ResolvedBinding> {
    let expected = backend.binding_kind();
    if binding.kind != expected {
        return Err(AudioError::BackendMismatch {
            expected,
            actual: binding.kind.clone(),
        });
    }
    let supports_direction = |device: &&AudioDeviceInfo| match direction {
        DeviceDirection::Input => device.supports_input,
        DeviceDirection::Output => device.supports_output,
    };
    let candidates = devices
        .iter()
        .filter(|device| device.backend == backend)
        .filter(supports_direction)
        .collect::<Vec<_>>();
    if let Some(persistent_id) = binding.last_seen_hardware_id.as_deref() {
        return candidates
            .iter()
            .find(|device| device.persistent_id == persistent_id)
            .map(|device| ResolvedBinding {
                device: (*device).clone(),
                matched_by: BindingMatch::PersistentId,
            })
            .ok_or_else(|| AudioError::DeviceNotFound(persistent_id.into()));
    }
    let mut named = candidates
        .into_iter()
        .filter(|device| device.display_name == binding.logical_name);
    let Some(device) = named.next() else {
        return Err(AudioError::DeviceNotFound(binding.logical_name.clone()));
    };
    if named.next().is_some() {
        return Err(AudioError::AmbiguousDevice(binding.logical_name.clone()));
    }
    Ok(ResolvedBinding {
        device: device.clone(),
        matched_by: BindingMatch::UniqueLogicalName,
    })
}

/// Performs real host/device enumeration. Platform `cfg` alone is never
/// reported as availability.
pub fn probe() -> Vec<Capability> {
    #[cfg(not(feature = "cpal"))]
    {
        vec![Capability {
            id: "cpal-audio".into(),
            available: false,
            detail: "not compiled (enable an explicit audio feature)".into(),
        }]
    }
    #[cfg(feature = "cpal")]
    {
        AudioBackend::compiled()
            .into_iter()
            .map(|backend| match enumerate_devices(backend) {
                Ok(devices) if !devices.is_empty() => Capability {
                    id: backend.id().into(),
                    available: true,
                    detail: format!("{} real device(s)", devices.len()),
                },
                Ok(_) => Capability {
                    id: backend.id().into(),
                    available: false,
                    detail: "host available, no devices enumerated".into(),
                },
                Err(error) => Capability {
                    id: backend.id().into(),
                    available: false,
                    detail: error.to_string(),
                },
            })
            .collect()
    }
}

/// Realtime-safe: no allocation after construction. Callers pre-size `out`.
pub fn mix_into(out: &mut AudioBuffer, src: &AudioBuffer, gain: f32) {
    let Some(out_first) = out.planes.first() else {
        return;
    };
    let Some(src_first) = src.planes.first() else {
        return;
    };
    let n = out_first.len().min(src_first.len());
    let ch = out.channels.min(src.channels) as usize;
    for c in 0..ch {
        for i in 0..n {
            out.planes[c][i] += src.planes[c][i] * gain;
        }
    }
}

/// Converts interleaved f32 into preallocated channel-major planes.
pub fn interleaved_to_planar(
    interleaved: &[f32],
    channels: usize,
    planes: &mut [Vec<f32>],
) -> Result<usize> {
    if channels == 0 || planes.len() < channels || !interleaved.len().is_multiple_of(channels) {
        return Err(AudioError::InvalidBuffer(
            "channel count or interleaved length mismatch".into(),
        ));
    }
    let frames = interleaved.len() / channels;
    if planes
        .iter()
        .take(channels)
        .any(|plane| plane.len() < frames)
    {
        return Err(AudioError::InvalidBuffer(
            "planar destination is not preallocated".into(),
        ));
    }
    for (frame_index, frame) in interleaved.chunks_exact(channels).enumerate() {
        for (channel, sample) in frame.iter().copied().enumerate() {
            planes[channel][frame_index] = sample;
        }
    }
    Ok(frames)
}

/// Converts channel-major f32 into a preallocated interleaved destination.
pub fn planar_to_interleaved(planes: &[Vec<f32>], out: &mut [f32]) -> Result<usize> {
    let channels = planes.len();
    if channels == 0 || !out.len().is_multiple_of(channels) {
        return Err(AudioError::InvalidBuffer(
            "channel count or interleaved length mismatch".into(),
        ));
    }
    let frames = out.len() / channels;
    if planes.iter().any(|plane| plane.len() < frames) {
        return Err(AudioError::InvalidBuffer(
            "planar source is shorter than destination".into(),
        ));
    }
    for (frame_index, frame) in out.chunks_exact_mut(channels).enumerate() {
        for (channel, sample) in frame.iter_mut().enumerate() {
            *sample = planes[channel][frame_index];
        }
    }
    Ok(frames)
}

pub fn peak_meter(buf: &AudioBuffer) -> f32 {
    buf.planes
        .iter()
        .flat_map(|p| p.iter())
        .fold(0.0f32, |a, x| a.max(x.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_core::DeviceBindingId;

    #[test]
    fn buffer_conversion_and_mix() {
        let mut out = AudioBuffer::silence(0, 48000, 2, 8);
        let mut src = AudioBuffer::silence(0, 48000, 2, 8);
        src.planes[0][0] = 0.5;
        mix_into(&mut out, &src, 2.0);
        assert_eq!(out.planes[0][0], 1.0);
        let interleaved = [0.1, -0.1, 0.2, -0.2, 0.3, -0.3];
        let mut planes = vec![vec![0.0; 3], vec![0.0; 3]];
        assert_eq!(
            interleaved_to_planar(&interleaved, 2, &mut planes).unwrap(),
            3
        );
        assert_eq!(planes[0], [0.1, 0.2, 0.3]);
        assert_eq!(planes[1], [-0.1, -0.2, -0.3]);
        let mut round_trip = [0.0; 6];
        assert_eq!(planar_to_interleaved(&planes, &mut round_trip).unwrap(), 3);
        assert_eq!(round_trip, interleaved);
        assert!(peak_meter(&src) > 0.4);
    }

    fn device(id: &str, name: &str) -> AudioDeviceInfo {
        AudioDeviceInfo {
            backend: AudioBackend::Wasapi,
            persistent_id: id.into(),
            display_name: name.into(),
            supports_input: true,
            supports_output: true,
            default_input: false,
            default_output: false,
        }
    }

    #[test]
    fn persistent_binding_wins_over_renamed_device() {
        let binding = DeviceBinding {
            id: DeviceBindingId::new(),
            kind: AudioBackend::Wasapi.binding_kind(),
            logical_name: "Old label".into(),
            last_seen_hardware_id: Some("wasapi:stable".into()),
        };
        let resolved = resolve_binding(
            &binding,
            AudioBackend::Wasapi,
            DeviceDirection::Input,
            &[device("wasapi:stable", "New label")],
        )
        .unwrap();
        assert_eq!(resolved.matched_by, BindingMatch::PersistentId);
        assert_eq!(resolved.device.display_name, "New label");
    }

    #[test]
    fn logical_name_rebind_requires_unique_match() {
        let binding = DeviceBinding {
            id: DeviceBindingId::new(),
            kind: AudioBackend::Wasapi.binding_kind(),
            logical_name: "Interface".into(),
            last_seen_hardware_id: None,
        };
        let devices = [
            device("wasapi:a", "Interface"),
            device("wasapi:b", "Interface"),
        ];
        assert!(matches!(
            resolve_binding(
                &binding,
                AudioBackend::Wasapi,
                DeviceDirection::Output,
                &devices
            ),
            Err(AudioError::AmbiguousDevice(_))
        ));
    }

    #[test]
    fn missing_persistent_device_never_falls_back_by_name() {
        let binding = DeviceBinding {
            id: DeviceBindingId::new(),
            kind: AudioBackend::Wasapi.binding_kind(),
            logical_name: "Interface".into(),
            last_seen_hardware_id: Some("wasapi:gone".into()),
        };
        assert!(matches!(
            resolve_binding(
                &binding,
                AudioBackend::Wasapi,
                DeviceDirection::Input,
                &[device("wasapi:replacement", "Interface")]
            ),
            Err(AudioError::DeviceNotFound(id)) if id == "wasapi:gone"
        ));
    }
}
