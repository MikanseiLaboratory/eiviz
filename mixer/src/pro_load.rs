//! Load one signed Pro module and keep it mapped for the process lifetime.
//!
//! The library is never unloaded. A join timeout can leave a thread inside
//! the module, so `FreeLibrary` / `dlclose` would be unsafe. Hot reload is
//! not implemented.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ed25519_dalek::Verifier;
use eiviz_pro_api::{
    ABI_MAJOR, AudioFrame, CaptureHandle, CaptureSink, CaptureStats, DeckLinkBackend,
    DeckLinkCaptureConfig, DeckLinkDevice, DeckLinkMode, DeckLinkPlayoutConfig, Entitlements,
    FEATURE_DECKLINK, FEATURE_LICENSE, FEATURE_RTMP, GET_API_SYMBOL, GetApiFn, LAYOUT_BGRA,
    LAYOUT_UYVY, LicenseBackend, LicenseCondition, LicenseStatus, MediaOutput, ModuleManifest,
    OutputStats, PRO_BUSY, PRO_DEVICE, PRO_INVALID, PRO_NOT_SUPPORTED, PRO_UNAVAILABLE,
    PixelLayout, Plan, ProApi, ProAudioView, ProCaptureCallbacks, ProCaptureStatsAbi,
    ProEntitlementsAbi, ProError, ProErrorKind, ProLicenseStatusAbi, ProOutputStatsAbi,
    ProPlayoutConfig, ProRtmpConfig, ProStatus, ProVideoView, RtmpConfig, SIGNAL_NONE,
    SignalStatus, StreamBackend, VideoFrame, abi_hash, module_target,
};
use sha2::{Digest, Sha256};

use crate::abi::ERR_IO;

static LOAD_LOCK: Mutex<()> = Mutex::new(());
static LOADED: OnceLock<LoadedPro> = OnceLock::new();
static LOAD_ERROR: Mutex<String> = Mutex::new(String::new());

#[derive(Clone, Copy)]
struct ModuleSnapshot {
    entitlements: Entitlements,
    features: u32,
}

struct LoadedPro {
    path: PathBuf,
    /// Kept so the loader does not unmap the module.
    #[allow(dead_code)]
    library: libloading::Library,
    api: ProApi,
    module: *mut c_void,
    snapshot: Mutex<ModuleSnapshot>,
}

unsafe impl Send for LoadedPro {}
unsafe impl Sync for LoadedPro {}

pub(crate) fn loaded() -> Option<&'static dyn eiviz_pro_api::ProModule> {
    LOADED.get().map(|loaded| loaded as _)
}

pub fn load_pro_module(path: &Path) -> Result<(), String> {
    let _guard = LOAD_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    match load_locked(path) {
        Ok(()) => {
            *LOAD_ERROR
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = String::new();
            Ok(())
        }
        Err(error) => {
            *LOAD_ERROR
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = error.clone();
            Err(error)
        }
    }
}

pub fn last_load_error() -> String {
    LOAD_ERROR
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
}

/// Ask the module to confirm that no capture or output handle remains.
/// The library stays mapped either way.
pub fn shutdown_if_idle() {
    let Some(loaded) = LOADED.get() else {
        return;
    };
    let Some(shutdown) = loaded.api.shutdown else {
        return;
    };
    let status = unsafe { shutdown(loaded.module) };
    if status.code == PRO_BUSY {
        crate::diag::warn("pro module still has live handles; library stays loaded");
    }
}

fn load_locked(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("Pro module path must be absolute".into());
    }
    reject_reparse(path)?;
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Pro module path: {error}"))?;
    if let Some(existing) = LOADED.get() {
        if existing.path == canonical {
            return Ok(());
        }
        return Err("a different Pro module is already loaded".into());
    }
    let bytes = std::fs::read(&canonical).map_err(|error| format!("read Pro module: {error}"))?;
    let digest = hex_encode(&Sha256::digest(&bytes));
    let manifest_path = manifest_path(&canonical);
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("read Pro manifest: {error}"))?;
    let (canonical_json, signature) = split_manifest(&manifest_text)?;
    verify_signature(canonical_json.as_bytes(), &signature)?;
    let manifest = ModuleManifest::parse(canonical_json)?;
    if manifest.sha256_hex != digest {
        return Err("Pro module hash does not match the signed manifest".into());
    }
    if manifest.abi_major != ABI_MAJOR || manifest.abi_hash != abi_hash() {
        return Err("Pro module ABI does not match this mixer".into());
    }
    if manifest.target != module_target() {
        return Err(format!(
            "Pro module target is {}, this process is {}",
            manifest.target,
            module_target()
        ));
    }
    let library = unsafe { libloading::Library::new(&canonical) }
        .map_err(|error| format!("load Pro module: {error}"))?;
    let get_api: libloading::Symbol<GetApiFn> = unsafe { library.get(GET_API_SYMBOL) }
        .map_err(|_| "Pro module is missing eiviz_pro_get_api".to_string())?;
    let api_ptr = unsafe { get_api(ABI_MAJOR, abi_hash(), std::mem::size_of::<ProApi>() as u32) };
    if api_ptr.is_null() {
        return Err("Pro module rejected this mixer ABI".into());
    }
    let api = unsafe { *api_ptr };
    if api.struct_size != std::mem::size_of::<ProApi>() as u32
        || api.abi_major != ABI_MAJOR
        || api.abi_hash != abi_hash()
    {
        return Err("Pro module returned a different ABI table".into());
    }
    if !api_complete(&api) {
        return Err("Pro module function table is incomplete".into());
    }
    let Some(create) = api.create else {
        return Err("Pro module function table is incomplete".into());
    };
    let module = unsafe { create() };
    if module.is_null() {
        return Err("Pro module create returned null".into());
    }
    let Some(copy_version) = api.copy_version else {
        return Err("Pro module function table is incomplete".into());
    };
    let version = copy_text(module, copy_version)?;
    let Some(destroy) = api.destroy else {
        return Err("Pro module function table is incomplete".into());
    };
    if version != manifest.module_version {
        unsafe { destroy(module) };
        return Err("Pro module version does not match the signed manifest".into());
    }
    let (entitlements, features) = query_snapshot(&api, module).map_err(|_| {
        unsafe { destroy(module) };
        "Pro module entitlements query failed".to_string()
    })?;
    let loaded = LoadedPro {
        path: canonical,
        library,
        api,
        module,
        snapshot: Mutex::new(ModuleSnapshot {
            entitlements,
            features,
        }),
    };
    LOADED
        .set(loaded)
        .map_err(|_| "Pro module was loaded twice".to_string())?;
    Ok(())
}

fn api_complete(api: &ProApi) -> bool {
    api.create.is_some()
        && api.shutdown.is_some()
        && api.destroy.is_some()
        && api.copy_name.is_some()
        && api.copy_version.is_some()
        && api.query_entitlements.is_some()
        && api.copy_last_error.is_some()
        && api.decklink_enum_devices.is_some()
        && api.decklink_enum_modes.is_some()
        && api.capture_start.is_some()
        && api.capture_stats.is_some()
        && api.capture_destroy.is_some()
        && api.playout_start.is_some()
        && api.rtmp_start.is_some()
        && api.output_submit_video.is_some()
        && api.output_submit_audio.is_some()
        && api.output_stats.is_some()
        && api.output_destroy.is_some()
        && api.license_install.is_some()
        && api.license_status.is_some()
        && api.license_clear.is_some()
        && api.license_fingerprint.is_some()
}

fn query_snapshot(api: &ProApi, module: *mut c_void) -> Result<(Entitlements, u32), ()> {
    let mut raw = ProEntitlementsAbi {
        struct_size: std::mem::size_of::<ProEntitlementsAbi>() as u32,
        plan: 0,
        mixing_units: 0,
        decklink_inputs: 0,
        decklink_outputs: 0,
        rtmp_max_width: 0,
        rtmp_max_height: 0,
        rtmp_max_fps_num: 0,
        rtmp_max_fps_den: 0,
        recording: 0,
        srt: 0,
        hardware_encode: 0,
        features: 0,
    };
    let Some(query) = api.query_entitlements else {
        return Err(());
    };
    let status = unsafe { query(module, &mut raw) };
    if !status.is_ok() || raw.struct_size < std::mem::size_of::<ProEntitlementsAbi>() as u32 {
        return Err(());
    }
    Ok((Entitlements::from_abi(raw), raw.features))
}

fn copy_text(
    module: *mut c_void,
    copy: unsafe extern "C" fn(*mut c_void, *mut u8, usize, *mut usize) -> ProStatus,
) -> Result<String, String> {
    let mut buffer = [0u8; 128];
    let mut written = 0usize;
    let status = unsafe { copy(module, buffer.as_mut_ptr(), buffer.len(), &mut written) };
    if !status.is_ok() {
        return Err("Pro module string copy failed".into());
    }
    let written = written.min(buffer.len());
    String::from_utf8(buffer[..written].to_vec())
        .map_err(|_| "Pro module string is not UTF-8".into())
}

fn manifest_path(module_path: &Path) -> PathBuf {
    let mut name = module_path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".manifest");
    module_path.with_file_name(name)
}

fn split_manifest(text: &str) -> Result<(&str, [u8; 64]), String> {
    let mut lines = text.lines();
    let canonical = lines.next().ok_or("Pro manifest is empty")?;
    let signature = lines
        .next()
        .ok_or("Pro manifest is missing its signature")?;
    if lines.next().is_some() {
        return Err("Pro manifest has trailing data".into());
    }
    let bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, signature.trim())
            .map_err(|_| "Pro manifest signature is not base64".to_string())?;
    let signature: [u8; 64] = bytes
        .try_into()
        .map_err(|_| "Pro manifest signature has the wrong length".to_string())?;
    Ok((canonical, signature))
}

fn verify_signature(message: &[u8], signature: &[u8; 64]) -> Result<(), String> {
    let keys = accepted_public_keys()?;
    let signature = ed25519_dalek::Signature::from_bytes(signature);
    for key in keys {
        let Ok(verifying) = ed25519_dalek::VerifyingKey::from_bytes(&key) else {
            continue;
        };
        if verifying.verify(message, &signature).is_ok() {
            return Ok(());
        }
    }
    Err("Pro module signature was not produced by a trusted key".into())
}

fn accepted_public_keys() -> Result<Vec<[u8; 32]>, String> {
    let mut keys = Vec::new();
    if let Some(key) = embedded_public_key()? {
        keys.push(key);
    }
    #[cfg(debug_assertions)]
    if keys.is_empty() {
        if let Ok(hex) = std::env::var("EIVIZ_PRO_MODULE_TEST_KEY") {
            keys.push(parse_key(&hex)?);
        }
    }
    if keys.is_empty() {
        return Err("this mixer has no Pro module public key".into());
    }
    Ok(keys)
}

fn embedded_public_key() -> Result<Option<[u8; 32]>, String> {
    let Some(hex) = option_env!("EIVIZ_PRO_MODULE_PUBLIC_KEY") else {
        return Ok(None);
    };
    if hex.is_empty() {
        return Ok(None);
    }
    let key = parse_key(hex)?;
    if key == [0; 32] {
        return Ok(None);
    }
    Ok(Some(key))
}

fn parse_key(hex: &str) -> Result<[u8; 32], String> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return Err("Pro module public key must be 64 hex characters".into());
    }
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .map_err(|_| "Pro module public key is not hex".to_string())?;
    }
    Ok(out)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn reject_reparse(path: &Path) -> Result<(), String> {
    ensure_not_reparse(path)?;
    if let Some(parent) = path.parent() {
        if parent.components().count() > 1 {
            ensure_not_reparse(parent)?;
        }
    }
    Ok(())
}

fn ensure_not_reparse(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    #[cfg(windows)]
    {
        const REPARSE: u32 = 0x400;
        let attributes = std::os::windows::fs::MetadataExt::file_attributes(&metadata);
        if attributes & REPARSE != 0 {
            return Err("Pro module path is a reparse point".into());
        }
    }
    #[cfg(unix)]
    {
        if metadata.file_type().is_symlink() {
            return Err("Pro module path is a symlink".into());
        }
    }
    let _ = metadata;
    Ok(())
}

impl eiviz_pro_api::ProModule for LoadedPro {
    fn module_name(&self) -> &'static str {
        "eiviz-pro"
    }

    fn entitlements(&self) -> Entitlements {
        self.refresh().entitlements
    }

    fn decklink(&self) -> Option<&dyn DeckLinkBackend> {
        (self.refresh().features & FEATURE_DECKLINK != 0).then_some(self)
    }

    fn streaming(&self) -> Option<&dyn StreamBackend> {
        (self.refresh().features & FEATURE_RTMP != 0).then_some(self)
    }

    fn license(&self) -> Option<&dyn LicenseBackend> {
        (self.refresh().features & FEATURE_LICENSE != 0).then_some(self)
    }
}

impl LoadedPro {
    fn refresh(&self) -> ModuleSnapshot {
        let mut guard = self
            .snapshot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Ok((entitlements, features)) = query_snapshot(&self.api, self.module) {
            guard.entitlements = entitlements;
            guard.features = features;
        }
        ModuleSnapshot {
            entitlements: guard.entitlements,
            features: guard.features,
        }
    }

    fn error(&self, status: ProStatus) -> ProError {
        let mut buffer = [0u8; 512];
        let mut written = 0usize;
        let _ = unsafe {
            if let Some(copy) = self.api.copy_last_error {
                copy(self.module, buffer.as_mut_ptr(), buffer.len(), &mut written)
            } else {
                ProStatus::new(PRO_UNAVAILABLE)
            }
        };
        let written = written.min(buffer.len());
        let message = String::from_utf8_lossy(&buffer[..written]).into_owned();
        let kind = match status.code {
            PRO_INVALID => ProErrorKind::InvalidArgument,
            PRO_UNAVAILABLE => ProErrorKind::Unavailable,
            PRO_NOT_SUPPORTED => ProErrorKind::NotSupportedPlan,
            PRO_DEVICE => ProErrorKind::Device,
            _ => ProErrorKind::Io,
        };
        ProError::new(
            kind,
            if message.is_empty() {
                format!("pro status {}", status.code)
            } else {
                message
            },
        )
    }

    fn read_json<T: serde::de::DeserializeOwned>(&self, bytes: &[u8]) -> Result<T, ProError> {
        serde_json::from_slice(bytes).map_err(|error| ProError::device(error.to_string()))
    }
}

impl DeckLinkBackend for LoadedPro {
    fn enumerate(&self) -> Result<Vec<DeckLinkDevice>, ProError> {
        let bytes = copy_plugin_bytes(self, |buffer, written| unsafe {
            if let Some(enumerate) = self.api.decklink_enum_devices {
                enumerate(self.module, buffer, 64 * 1024, written)
            } else {
                ProStatus::new(PRO_UNAVAILABLE)
            }
        })?;
        self.read_json(&bytes)
    }

    fn modes(&self, device_id: &str) -> Result<Vec<DeckLinkMode>, ProError> {
        let bytes = copy_plugin_bytes(self, |buffer, written| unsafe {
            if let Some(modes) = self.api.decklink_enum_modes {
                modes(
                    self.module,
                    device_id.as_ptr(),
                    device_id.len(),
                    buffer,
                    64 * 1024,
                    written,
                )
            } else {
                ProStatus::new(PRO_UNAVAILABLE)
            }
        })?;
        self.read_json(&bytes)
    }

    fn start_capture(
        &self,
        config: &DeckLinkCaptureConfig,
        sink: Box<dyn CaptureSink>,
    ) -> Result<Box<dyn CaptureHandle>, ProError> {
        let user = Box::into_raw(Box::new(SinkBox { sink }));
        let callbacks = ProCaptureCallbacks {
            struct_size: std::mem::size_of::<ProCaptureCallbacks>() as u32,
            user_data: user.cast(),
            video: Some(on_video),
            audio: Some(on_audio),
            signal: Some(on_signal),
        };
        let mut handle = std::ptr::null_mut();
        let status = unsafe {
            if let Some(start) = self.api.capture_start {
                start(
                    self.module,
                    config.device_id.as_ptr(),
                    config.device_id.len(),
                    config.mode_id.as_ptr(),
                    config.mode_id.len(),
                    callbacks,
                    &mut handle,
                )
            } else {
                ProStatus::new(PRO_UNAVAILABLE)
            }
        };
        if !status.is_ok() || handle.is_null() {
            unsafe { drop(Box::from_raw(user)) };
            return Err(self.error(status));
        }
        Ok(Box::new(PluginCapture {
            stats_fn: self.api.capture_stats,
            destroy_fn: self.api.capture_destroy,
            handle,
            sink: user,
        }))
    }

    fn start_playout(
        &self,
        config: &DeckLinkPlayoutConfig,
    ) -> Result<Box<dyn MediaOutput>, ProError> {
        let raw = ProPlayoutConfig {
            struct_size: std::mem::size_of::<ProPlayoutConfig>() as u32,
            device: config.device_id.as_ptr(),
            device_len: config.device_id.len(),
            mode: config.mode_id.as_ptr(),
            mode_len: config.mode_id.len(),
            external_key: u32::from(config.external_key),
            fps_num: config.fps_num,
            fps_den: config.fps_den,
        };
        self.open_output(|handle| unsafe {
            if let Some(start) = self.api.playout_start {
                start(self.module, &raw, handle)
            } else {
                ProStatus::new(PRO_UNAVAILABLE)
            }
        })
    }
}

impl StreamBackend for LoadedPro {
    fn start_rtmp(&self, config: &RtmpConfig) -> Result<Box<dyn MediaOutput>, ProError> {
        let raw = ProRtmpConfig {
            struct_size: std::mem::size_of::<ProRtmpConfig>() as u32,
            url: config.url.as_ptr(),
            url_len: config.url.len(),
            video_bitrate: config.video_bitrate,
            audio_bitrate: config.audio.bitrate,
            keyint: config.keyint,
            video_only: u32::from(config.audio.video_only),
            width: config.width,
            height: config.height,
            fps_num: config.fps_num,
            fps_den: config.fps_den,
        };
        self.open_output(|handle| unsafe {
            if let Some(start) = self.api.rtmp_start {
                start(self.module, &raw, handle)
            } else {
                ProStatus::new(PRO_UNAVAILABLE)
            }
        })
    }
}

impl LicenseBackend for LoadedPro {
    fn install(&self, ticket: &str) -> Result<LicenseStatus, ProError> {
        let ticket = CString::new(ticket).map_err(|_| ProError::invalid("ticket contains nul"))?;
        let Some(install) = self.api.license_install else {
            return Err(ProError::unavailable("pro function is missing"));
        };
        let status = unsafe { install(self.module, ticket.as_ptr()) };
        if !status.is_ok() {
            return Err(self.error(status));
        }
        Ok(self.status())
    }

    fn status(&self) -> LicenseStatus {
        let mut raw = ProLicenseStatusAbi {
            struct_size: std::mem::size_of::<ProLicenseStatusAbi>() as u32,
            state: 0,
            plan: 0,
            expires_at: 0,
        };
        let mut ticket_id = [0u8; 128];
        let mut written = 0usize;
        let status = unsafe {
            if let Some(status_fn) = self.api.license_status {
                status_fn(
                    self.module,
                    &mut raw,
                    ticket_id.as_mut_ptr(),
                    ticket_id.len(),
                    &mut written,
                )
            } else {
                ProStatus::new(PRO_UNAVAILABLE)
            }
        };
        if !status.is_ok() {
            return LicenseStatus::unregistered();
        }
        let written = written.min(ticket_id.len());
        LicenseStatus {
            condition: LicenseCondition::from_abi(raw.state),
            plan: match raw.plan {
                1 => Plan::Professional,
                2 => Plan::Enterprise,
                _ => Plan::Community,
            },
            expires_at: raw.expires_at,
            ticket_id: String::from_utf8_lossy(&ticket_id[..written]).into_owned(),
        }
    }

    fn clear(&self) -> Result<(), ProError> {
        let Some(clear) = self.api.license_clear else {
            return Err(ProError::unavailable("pro function is missing"));
        };
        let status = unsafe { clear(self.module) };
        if status.is_ok() {
            Ok(())
        } else {
            Err(self.error(status))
        }
    }

    fn machine_fingerprint(&self) -> Result<String, ProError> {
        let bytes = copy_plugin_bytes(self, |buffer, written| unsafe {
            if let Some(fingerprint) = self.api.license_fingerprint {
                fingerprint(self.module, buffer, 64 * 1024, written)
            } else {
                ProStatus::new(PRO_UNAVAILABLE)
            }
        })?;
        String::from_utf8(bytes).map_err(|error| ProError::device(error.to_string()))
    }
}

impl LoadedPro {
    fn open_output(
        &self,
        start: impl FnOnce(*mut *mut c_void) -> ProStatus,
    ) -> Result<Box<dyn MediaOutput>, ProError> {
        let mut handle = std::ptr::null_mut();
        let status = start(&mut handle);
        if !status.is_ok() || handle.is_null() {
            return Err(self.error(status));
        }
        Ok(Box::new(PluginOutput {
            submit_video: self.api.output_submit_video,
            submit_audio: self.api.output_submit_audio,
            stats_fn: self.api.output_stats,
            destroy_fn: self.api.output_destroy,
            handle,
        }))
    }
}

fn copy_plugin_bytes(
    loaded: &LoadedPro,
    call: impl Fn(*mut u8, *mut usize) -> ProStatus,
) -> Result<Vec<u8>, ProError> {
    let mut buffer = vec![0u8; 64 * 1024];
    let mut written = 0usize;
    let status = call(buffer.as_mut_ptr(), &mut written);
    if !status.is_ok() {
        return Err(loaded.error(status));
    }
    buffer.truncate(written.min(buffer.len()));
    Ok(buffer)
}

struct SinkBox {
    sink: Box<dyn CaptureSink>,
}

unsafe extern "C" fn on_video(user: *mut c_void, view: *const ProVideoView) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if user.is_null() || view.is_null() {
            return;
        }
        let sink = unsafe { &mut *user.cast::<SinkBox>() };
        let view = unsafe { &*view };
        if view.data.is_null() && view.data_len != 0 {
            return;
        }
        let data = unsafe { std::slice::from_raw_parts(view.data, view.data_len) }.to_vec();
        sink.sink.video(VideoFrame {
            width: view.width,
            height: view.height,
            stride: view.stride,
            layout: if view.layout == LAYOUT_BGRA {
                PixelLayout::Bgra
            } else {
                PixelLayout::Uyvy
            },
            pts_100ns: view.pts_100ns,
            fps_num: view.fps_num,
            fps_den: view.fps_den,
            data: data.into(),
        });
    }));
}

unsafe extern "C" fn on_audio(user: *mut c_void, view: *const ProAudioView) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if user.is_null() || view.is_null() {
            return;
        }
        let sink = unsafe { &mut *user.cast::<SinkBox>() };
        let view = unsafe { &*view };
        if view.planar.is_null() && view.sample_count != 0 {
            return;
        }
        let samples =
            unsafe { std::slice::from_raw_parts(view.planar, view.sample_count) }.to_vec();
        sink.sink.audio(AudioFrame {
            sample_rate: view.sample_rate,
            channels: view.channels,
            frames: view.frames,
            pts_100ns: view.pts_100ns,
            planar_f32: samples.into(),
        });
    }));
}

unsafe extern "C" fn on_signal(user: *mut c_void, status: u32) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if user.is_null() {
            return;
        }
        let sink = unsafe { &mut *user.cast::<SinkBox>() };
        sink.sink.signal(if status == SIGNAL_NONE {
            SignalStatus::NoSignal
        } else {
            SignalStatus::Present
        });
    }));
}

struct PluginCapture {
    stats_fn: Option<
        unsafe extern "C" fn(*mut c_void, *mut ProCaptureStatsAbi, *mut u8, usize) -> ProStatus,
    >,
    destroy_fn: Option<unsafe extern "C" fn(*mut c_void) -> ProStatus>,
    handle: *mut c_void,
    sink: *mut SinkBox,
}

unsafe impl Send for PluginCapture {}

impl Drop for PluginCapture {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            if let Some(destroy) = self.destroy_fn {
                unsafe { destroy(self.handle) };
            }
            self.handle = std::ptr::null_mut();
        }
        if !self.sink.is_null() {
            unsafe { drop(Box::from_raw(self.sink)) };
            self.sink = std::ptr::null_mut();
        }
    }
}

impl CaptureHandle for PluginCapture {
    fn stats(&self) -> CaptureStats {
        let mut raw = ProCaptureStatsAbi {
            struct_size: std::mem::size_of::<ProCaptureStatsAbi>() as u32,
            frames: 0,
            no_signal: 0,
            error_len: 0,
        };
        let mut error = [0u8; 256];
        let Some(stats) = self.stats_fn else {
            return CaptureStats::default();
        };
        let status = unsafe { stats(self.handle, &mut raw, error.as_mut_ptr(), error.len()) };
        if !status.is_ok() {
            return CaptureStats::default();
        }
        let error_len = (raw.error_len as usize).min(error.len());
        CaptureStats {
            frames: raw.frames,
            no_signal: raw.no_signal != 0,
            last_error: String::from_utf8_lossy(&error[..error_len]).into_owned(),
        }
    }
}

struct PluginOutput {
    submit_video: Option<unsafe extern "C" fn(*mut c_void, *const ProVideoView) -> ProStatus>,
    submit_audio: Option<unsafe extern "C" fn(*mut c_void, *const ProAudioView) -> ProStatus>,
    stats_fn: Option<
        unsafe extern "C" fn(*mut c_void, *mut ProOutputStatsAbi, *mut u8, usize) -> ProStatus,
    >,
    destroy_fn: Option<unsafe extern "C" fn(*mut c_void) -> ProStatus>,
    handle: *mut c_void,
}

unsafe impl Send for PluginOutput {}

impl Drop for PluginOutput {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            if let Some(destroy) = self.destroy_fn {
                unsafe { destroy(self.handle) };
            }
            self.handle = std::ptr::null_mut();
        }
    }
}

impl MediaOutput for PluginOutput {
    fn submit_video(&mut self, frame: VideoFrame) -> Result<(), ProError> {
        let view = ProVideoView {
            width: frame.width,
            height: frame.height,
            stride: frame.stride,
            layout: match frame.layout {
                PixelLayout::Uyvy => LAYOUT_UYVY,
                PixelLayout::Bgra => LAYOUT_BGRA,
            },
            pts_100ns: frame.pts_100ns,
            fps_num: frame.fps_num,
            fps_den: frame.fps_den,
            data: frame.data.as_ptr(),
            data_len: frame.data.len(),
        };
        let Some(submit) = self.submit_video else {
            return Err(ProError::unavailable("pro function is missing"));
        };
        let status = unsafe { submit(self.handle, &view) };
        if status.is_ok() {
            Ok(())
        } else {
            Err(ProError::device(format!(
                "pro video status {}",
                status.code
            )))
        }
    }

    fn submit_audio(&mut self, frame: AudioFrame) -> Result<(), ProError> {
        let view = ProAudioView {
            sample_rate: frame.sample_rate,
            channels: frame.channels,
            frames: frame.frames,
            pts_100ns: frame.pts_100ns,
            planar: frame.planar_f32.as_ptr(),
            sample_count: frame.planar_f32.len(),
        };
        let Some(submit) = self.submit_audio else {
            return Err(ProError::unavailable("pro function is missing"));
        };
        let status = unsafe { submit(self.handle, &view) };
        if status.is_ok() {
            Ok(())
        } else {
            Err(ProError::device(format!(
                "pro audio status {}",
                status.code
            )))
        }
    }

    fn stats(&self) -> OutputStats {
        let mut raw = ProOutputStatsAbi {
            struct_size: std::mem::size_of::<ProOutputStatsAbi>() as u32,
            connected: 0,
            video_frames: 0,
            repeated: 0,
            dropped: 0,
            bitrate: 0,
        };
        let mut message = [0u8; 256];
        let Some(stats) = self.stats_fn else {
            return OutputStats::default();
        };
        let status = unsafe { stats(self.handle, &mut raw, message.as_mut_ptr(), message.len()) };
        if !status.is_ok() {
            return OutputStats::default();
        }
        let message_len = message
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(message.len());
        OutputStats {
            connected: raw.connected != 0,
            video_frames: raw.video_frames,
            repeated: raw.repeated,
            dropped: raw.dropped,
            bitrate: raw.bitrate,
            message: String::from_utf8_lossy(&message[..message_len]).into_owned(),
        }
    }
}

#[cfg(debug_assertions)]
mod probe {
    use std::sync::atomic::{AtomicU64, Ordering};

    use eiviz_pro_api::{
        AudioFrame, CaptureHandle, CaptureSink, MediaOutput, SignalStatus, VideoFrame,
    };

    use super::loaded;

    static FRAMES: AtomicU64 = AtomicU64::new(0);

    struct CountingSink;

    impl CaptureSink for CountingSink {
        fn video(&mut self, _frame: VideoFrame) {
            FRAMES.fetch_add(1, Ordering::SeqCst);
        }

        fn audio(&mut self, _frame: AudioFrame) {}

        fn signal(&mut self, _status: SignalStatus) {}
    }

    pub struct ProTestCapture {
        handle: Box<dyn CaptureHandle>,
    }

    impl ProTestCapture {
        pub fn stats_frames(&self) -> u64 {
            self.handle.stats().frames
        }
    }

    pub fn frames() -> u64 {
        FRAMES.load(Ordering::SeqCst)
    }

    pub fn open_capture() -> Result<ProTestCapture, String> {
        let module = loaded().ok_or("Pro module is not loaded")?;
        let backend = module.decklink().ok_or("DeckLink is not linked")?;
        let handle = backend
            .start_capture(
                &eiviz_pro_api::DeckLinkCaptureConfig {
                    device_id: "test".into(),
                    mode_id: "test".into(),
                },
                Box::new(CountingSink),
            )
            .map_err(|error| error.message)?;
        Ok(ProTestCapture { handle })
    }

    pub fn open_output() -> Result<Box<dyn MediaOutput>, String> {
        let module = loaded().ok_or("Pro module is not loaded")?;
        let backend = module.decklink().ok_or("DeckLink is not linked")?;
        backend
            .start_playout(&eiviz_pro_api::DeckLinkPlayoutConfig {
                device_id: "test".into(),
                mode_id: "test".into(),
                external_key: false,
                fps_num: 30,
                fps_den: 1,
            })
            .map_err(|error| error.message)
    }

    pub fn shutdown() {
        super::shutdown_if_idle();
    }
}

#[cfg(debug_assertions)]
pub use probe::{
    ProTestCapture, frames as pro_test_frames, open_capture as pro_test_open_capture,
    open_output as pro_test_open_output, shutdown as pro_test_shutdown,
};

pub fn prepare_pro_module(explicit: Option<&Path>) -> Result<(), String> {
    let path = match resolve_pro_path(explicit) {
        Ok(path) => path,
        Err(error) => {
            *LOAD_ERROR
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = error.clone();
            return Err(error);
        }
    };
    let Some(path) = path else {
        return Ok(());
    };
    load_pro_module(&path)
}

fn resolve_pro_path(explicit: Option<&Path>) -> Result<Option<PathBuf>, String> {
    if let Some(path) = explicit {
        return Ok(Some(make_absolute(path)?));
    }
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let Some(dir) = exe.parent() else {
        return Err("could not locate the executable directory".into());
    };
    let marker = dir.join("eiviz-pro.required");
    if !marker.is_file() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(&marker).map_err(|error| format!("read Pro marker: {error}"))?;
    let text = text.trim();
    let name = if text.is_empty() {
        default_module_file()
    } else {
        text.to_string()
    };
    let path = PathBuf::from(name);
    Ok(Some(if path.is_absolute() {
        path
    } else {
        dir.join(path)
    }))
}

fn make_absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(path))
    }
}

fn default_module_file() -> String {
    if cfg!(windows) {
        "eiviz_pro.dll".into()
    } else if cfg!(target_os = "macos") {
        "libeiviz_pro.dylib".into()
    } else {
        "libeiviz_pro.so".into()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_pro_prepare(path: *const c_char) -> i32 {
    let explicit = if path.is_null() {
        None
    } else {
        let Some(text) = unsafe { CStr::from_ptr(path) }.to_str().ok() else {
            *LOAD_ERROR
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) =
                "Pro module path is not UTF-8".into();
            return ERR_IO;
        };
        Some(PathBuf::from(text))
    };
    match prepare_pro_module(explicit.as_deref()) {
        Ok(()) => 0,
        Err(_) => ERR_IO,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_pro_load(path: *const c_char) -> i32 {
    let Some(path) =
        unsafe { path.as_ref() }.and_then(|ptr| unsafe { CStr::from_ptr(ptr) }.to_str().ok())
    else {
        *LOAD_ERROR
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = "Pro module path is not UTF-8".into();
        return ERR_IO;
    };
    match load_pro_module(Path::new(path)) {
        Ok(()) => 0,
        Err(_) => ERR_IO,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mixer_pro_copy_error(out: *mut u8, cap: usize) -> i32 {
    if out.is_null() {
        return crate::abi::ERR_INVALID_ARGUMENT;
    }
    let message = last_load_error();
    let bytes = message.as_bytes();
    if cap < bytes.len() + 1 {
        return crate::abi::ERR_BUFFER_TOO_SMALL;
    }
    unsafe {
        if !bytes.is_empty() {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len());
        }
        *out.add(bytes.len()) = 0;
    }
    0
}
