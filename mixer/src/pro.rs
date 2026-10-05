//! Plan gate. `eiviz_pro` is the module at `pro/eiviz_pro`.
//! Official builds replace that directory; the public tree ships a Free stand-in.

use std::ffi::{CStr, c_char};
use std::sync::{Mutex, OnceLock};

use eiviz_pro_api::{
    Entitlements, LicenseStatus, PixelLayout, Plan, ProError, ProErrorKind, Quota, VideoFrame,
};

use crate::abi::{
    ERR_DEVICE, ERR_INVALID_ARGUMENT, ERR_IO, ERR_NOT_SUPPORTED_PLAN, MixerCapabilities,
};
use crate::upload::AudioPacket;

pub fn module() -> &'static dyn eiviz_pro_api::ProModule {
    eiviz_pro::module()
}

pub fn entitlements() -> Entitlements {
    module().entitlements()
}

fn license_backend() -> Result<&'static dyn eiviz_pro_api::LicenseBackend, i32> {
    module().license().ok_or(ERR_IO)
}

pub fn license_install(ticket: &str) -> Result<LicenseStatus, i32> {
    let backend = license_backend()?;
    backend.install(ticket).map_err(license_error)
}

pub fn license_status() -> Result<LicenseStatus, i32> {
    Ok(license_backend()?.status())
}

pub fn license_clear() -> Result<(), i32> {
    license_backend()?.clear().map_err(license_error)
}

pub fn machine_fingerprint() -> Result<String, i32> {
    license_backend()?
        .machine_fingerprint()
        .map_err(license_error)
}

fn license_error(error: ProError) -> i32 {
    match error.kind {
        ProErrorKind::InvalidArgument | ProErrorKind::NotSupportedPlan => ERR_INVALID_ARGUMENT,
        ProErrorKind::Unavailable | ProErrorKind::Device | ProErrorKind::Io => ERR_IO,
    }
}

fn secrets() -> &'static Mutex<std::collections::HashMap<String, String>> {
    static SECRETS: OnceLock<Mutex<std::collections::HashMap<String, String>>> = OnceLock::new();
    SECRETS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

pub fn secret_set(name: &str, value: &str) {
    secrets()
        .lock()
        .expect("secrets")
        .insert(name.to_string(), value.to_string());
}

pub fn secret_clear(name: &str) {
    secrets().lock().expect("secrets").remove(name);
}

pub fn resolve_secret(reference: &str) -> Option<String> {
    secrets().lock().expect("secrets").get(reference).cloned()
}

pub fn cstr<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputExtras {
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub external_key: bool,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub secret_ref: String,
    #[serde(default)]
    pub video_bitrate: u32,
    #[serde(default)]
    pub audio_bitrate: u32,
    #[serde(default)]
    pub keyint: u32,
    #[serde(default)]
    pub video_only: bool,
}

pub fn parse_extras(json: Option<&str>) -> Result<OutputExtras, String> {
    let Some(json) = json.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(OutputExtras::default());
    };
    serde_json::from_str(json).map_err(|error| error.to_string())
}

pub fn fill_capabilities(out: &mut MixerCapabilities) {
    let ent = entitlements();
    let decklink = module().decklink().is_some();
    let rtmp = module().streaming().is_some();
    *out = MixerCapabilities {
        plan: ent.plan.abi(),
        mixing_unit_limit: ent.mixing_units.abi_count(),
        decklink_input_limit: ent.decklink_inputs.abi_count(),
        decklink_output_limit: ent.decklink_outputs.abi_count(),
        rtmp_max_width: video_width(ent.rtmp),
        rtmp_max_height: video_height(ent.rtmp),
        rtmp_max_fps_num: video_fps_num(ent.rtmp),
        rtmp_max_fps_den: video_fps_den(ent.rtmp),
        recording: u32::from(ent.recording),
        srt: u32::from(ent.srt),
        hardware_encode: u32::from(ent.hardware_encode),
        decklink_linked: u32::from(decklink),
        rtmp_linked: u32::from(rtmp),
    };
    let _ = Plan::Free;
}

fn video_width(quota: Quota<eiviz_pro_api::VideoLimit>) -> u32 {
    match quota {
        Quota::Denied => 0,
        Quota::Unlimited => u32::MAX,
        Quota::Limited(limit) => limit.width,
    }
}

fn video_height(quota: Quota<eiviz_pro_api::VideoLimit>) -> u32 {
    match quota {
        Quota::Denied => 0,
        Quota::Unlimited => u32::MAX,
        Quota::Limited(limit) => limit.height,
    }
}

fn video_fps_num(quota: Quota<eiviz_pro_api::VideoLimit>) -> u32 {
    match quota {
        Quota::Denied => 0,
        Quota::Unlimited => u32::MAX,
        Quota::Limited(limit) => limit.fps_num,
    }
}

fn video_fps_den(quota: Quota<eiviz_pro_api::VideoLimit>) -> u32 {
    match quota {
        Quota::Denied => 0,
        Quota::Unlimited => 1,
        Quota::Limited(limit) => limit.fps_den,
    }
}

pub fn pro_code(error: ProError) -> i32 {
    match error.kind {
        eiviz_pro_api::ProErrorKind::NotSupportedPlan => ERR_NOT_SUPPORTED_PLAN,
        eiviz_pro_api::ProErrorKind::InvalidArgument => ERR_INVALID_ARGUMENT,
        eiviz_pro_api::ProErrorKind::Device => ERR_DEVICE,
        eiviz_pro_api::ProErrorKind::Unavailable | eiviz_pro_api::ProErrorKind::Io => ERR_IO,
    }
}

pub fn video_frame(
    width: u32,
    height: u32,
    stride: u32,
    pts: i64,
    data: std::sync::Arc<[u8]>,
    fps_n: u32,
    fps_d: u32,
) -> VideoFrame {
    let layout = if stride >= width.saturating_mul(4) {
        PixelLayout::Bgra
    } else {
        PixelLayout::Uyvy
    };
    VideoFrame {
        width,
        height,
        stride,
        layout,
        pts_100ns: pts,
        fps_num: fps_n,
        fps_den: fps_d,
        data,
    }
}

pub fn audio_frame(packet: &AudioPacket) -> eiviz_pro_api::AudioFrame {
    eiviz_pro_api::AudioFrame {
        sample_rate: packet.sample_rate.max(1) as u32,
        channels: packet.channels.max(1) as u32,
        frames: packet.samples_per_channel.max(0) as u32,
        pts_100ns: packet.timestamp,
        planar_f32: packet.pcm_planar_f32.clone().into(),
    }
}

/// Resolve an RTMP URL whose stream name may be a secret reference.
pub fn rtmp_url(extras: &OutputExtras) -> Result<String, String> {
    if extras.url.is_empty() {
        return Err("RTMP output requires a url".into());
    }
    if extras.secret_ref.is_empty() {
        return Ok(extras.url.clone());
    }
    let key = resolve_secret(&extras.secret_ref)
        .ok_or_else(|| format!("RTMP secret '{}' is not set", extras.secret_ref))?;
    if extras.url.ends_with('/') {
        Ok(format!("{}{key}", extras.url))
    } else {
        Ok(format!("{}/{key}", extras.url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn license_backend_follows_the_linked_module() {
        if module().module_name() == "stub" {
            assert!(module().license().is_none());
            assert_eq!(license_install("ticket").unwrap_err(), ERR_IO);
        } else {
            assert!(module().license().is_some());
        }
    }
}
