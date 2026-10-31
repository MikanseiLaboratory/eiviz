//! Shared helpers for the Replay integration tests.
//!
//! A Pro module can only be loaded once per process, so each test binary owns its
//! own mixer. These helpers stage and sign the test module.

#![allow(dead_code)]

use std::ffi::CString;
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signer, SigningKey};
use eiviz_mixer::{OK, OverlayDesc, Rect, SCENE_BASE, load_pro_module, mixer_snapshot};
use eiviz_pro_api::{ABI_MAJOR, ModuleManifest, abi_hash, module_target};
use sha2::{Digest, Sha256};

/// Whether the runner has no graphics card, so only a software rasteriser is available.
///
/// The GPU encoder needs a real adapter: on a basic render driver its ring cannot be created and no
/// frame reaches the recorder, which is a property of the runner rather than of the code under test.
pub fn adapter_is_software() -> bool {
    // Asked of `wgpu` rather than of the mixer: this runs at the top of a test, before anything has
    // been created, and the mixer would answer "not created" — which used to read as "has a GPU".
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        ..Default::default()
    }));
    let Ok(adapter) = adapter else {
        // No adapter at all means nothing can be drawn; a test that needs pictures cannot run.
        return true;
    };
    let info = adapter.get_info();
    let name = info.name.to_ascii_lowercase();
    let software = name.contains("basic render")
        || name.contains("llvmpipe")
        || name.contains("swiftshader")
        || matches!(info.device_type, wgpu::DeviceType::Cpu);
    software
}

/// Key the debug builds accept for a test module.
pub const SECRET: [u8; 32] = [9; 32];

/// Stages the test plugin, signs it and loads it. Panics when it cannot load,
/// because every replay test needs the recording backend.
pub fn load_test_module(label: &str) {
    let public_key = hex(&SigningKey::from_bytes(&SECRET).verifying_key().to_bytes());
    unsafe { std::env::set_var("EIVIZ_PRO_MODULE_TEST_KEY", &public_key) };
    let module = stage(label, &find_cdylib("eiviz_pro_test_plugin"));
    sign(&module, "0.3.0", &SECRET);
    load_pro_module(&module).unwrap();
}

/// One layer that fills the frame with a source.
pub fn full_layer(source_id: u64) -> OverlayDesc {
    OverlayDesc {
        source_id,
        rect: Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        opacity: 1.0,
        z: 0,
        ..Default::default()
    }
}

/// Scene gpu id from a session scene id, matching the host mapping.
pub fn scene_id(id: u64) -> u64 {
    SCENE_BASE | id
}

/// Assert a Replay call succeeded, with the code in the message.
pub fn expect(action: &str, code: i32) {
    assert_eq!(code, OK, "{action} failed with {code}");
}

/// The reason the mixer gave for the last refusal.
pub fn last_error() -> String {
    let mut buffer = vec![0u8; 512];
    unsafe { eiviz_mixer::mixer_last_error(buffer.as_mut_ptr(), buffer.len()) };
    String::from_utf8_lossy(&buffer)
        .trim_end_matches('\0')
        .to_string()
}

/// Marks in as soon as the recorder has a frame to mark.
///
/// A fixed sleep is not enough on a software adapter: recording starts producing
/// frames when the renderer has drawn them, which can take longer than any sleep a
/// test is willing to write. This repeats the operator's own action until the engine
/// accepts it, and reports the refusal reason if it never does.
pub fn mark_in_when_ready(label: &str) -> u64 {
    // A shared CI runner with a software adapter has taken minutes for the rest of
    // the replay suite, so one snapshot gets a generous budget: failing here says
    // nothing about the picture the test is looking at.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let mut frame = 0u64;
        let code = unsafe { eiviz_mixer::mixer_replay_mark_in(&mut frame) };
        if code == OK {
            return frame;
        }
        if std::time::Instant::now() >= deadline {
            panic!("{label}: Mark In was never accepted: {}", last_error());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Frames the recorder has taken so far, from either encoder.
///
/// `frames_offered` counts what the readback path handed over; a GPU-encoded angle copies its frame
/// into the encoder's ring and never touches the readback, so it only raises `frames_written`. Both
/// mean "the recorder saw a frame", which is what a wait is for.
pub fn offered_frames() -> u64 {
    let mut stats = eiviz_mixer::MixerReplayStats::default();
    unsafe { eiviz_mixer::mixer_replay_copy_stats(&mut stats, std::ptr::null_mut(), 0) };
    stats.frames_offered.saturating_add(stats.frames_written)
}

/// Closes the mark once the recorder has taken at least `min_frames` frames since
/// Mark In.
///
/// A slow adapter needs the wait (the range would otherwise be empty and refused),
/// and a transition test needs the length: a clip shorter than the transition cannot
/// be measured.
pub fn mark_out_after(label: &str, min_frames: u64) -> (String, u64, u64) {
    // A software adapter renders a debug build very slowly: 60 frames can take
    // half a minute. The wait is generous on purpose; what the test needs is the
    // clip length, not a rate.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    let start = offered_frames();
    while offered_frames().saturating_sub(start) < min_frames {
        if std::time::Instant::now() >= deadline {
            panic!(
                "{label}: only {} frames were recorded after Mark In, wanted {min_frames}",
                offered_frames().saturating_sub(start)
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    mark_out_when_ready(label)
}

/// Closes the mark once a frame has arrived after Mark In.
///
/// A range with no frames in it is refused (it would become a clip that can never
/// show anything), so this waits for the recording to move on instead of sleeping a
/// fixed time that a slow adapter may not cover.
pub fn mark_out_when_ready(label: &str) -> (String, u64, u64) {
    // A shared CI runner with a software adapter has taken minutes for the rest of
    // the replay suite, so one snapshot gets a generous budget: failing here says
    // nothing about the picture the test is looking at.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let mut in_frame = 0u64;
        let mut out_frame = 0u64;
        let mut uuid = vec![0u8; 64];
        let code = unsafe {
            eiviz_mixer::mixer_replay_mark_out(
                &mut in_frame,
                &mut out_frame,
                uuid.as_mut_ptr(),
                uuid.len(),
            )
        };
        if code == OK {
            let uuid = String::from_utf8_lossy(&uuid)
                .trim_end_matches('\0')
                .to_string();
            return (uuid, in_frame, out_frame);
        }
        if std::time::Instant::now() >= deadline {
            panic!("{label}: Mark Out was never accepted: {}", last_error());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

pub fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

fn sign(module: &Path, version: &str, secret: &[u8; 32]) {
    let bytes = std::fs::read(module).unwrap();
    let manifest = ModuleManifest {
        abi_hash: abi_hash(),
        abi_major: ABI_MAJOR,
        module_version: version.into(),
        sha256_hex: hex(&Sha256::digest(&bytes)),
        target: module_target(),
    };
    let canonical = manifest.canonical().unwrap();
    let signature = SigningKey::from_bytes(secret).sign(canonical.as_bytes());
    let encoded = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        signature.to_bytes(),
    );
    let manifest_path = {
        let mut name = module.file_name().unwrap().to_os_string();
        name.push(".manifest");
        module.with_file_name(name)
    };
    std::fs::write(manifest_path, format!("{canonical}\n{encoded}\n")).unwrap();
}

fn stage(label: &str, source: &Path) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("eiviz-pro-module-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dest = dir.join(format!(
        "{label}{}",
        source
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| format!(".{ext}"))
            .unwrap_or_default()
    ));
    std::fs::copy(source, &dest).unwrap();
    dest
}

fn find_cdylib(prefix: &str) -> PathBuf {
    let deps = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&deps).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let library = name.ends_with(".dll") || name.ends_with(".dylib") || name.ends_with(".so");
        if library && (name.starts_with(prefix) || name.starts_with(&format!("lib{prefix}"))) {
            found.push(path);
        }
    }
    found.sort();
    found
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("missing {prefix} in {deps:?}"))
}

/// A C string for a Replay uuid argument.
pub fn cstring(text: &str) -> CString {
    CString::new(text).unwrap()
}

/// One pixel of a scene's composed picture, read the way a monitor shows it. The first
/// request can land before a frame exists, and a software adapter under load can take
/// seconds to produce one, so it is retried on a budget rather than once.
pub fn scene_pixel(scene: u64, x: u32, y: u32) -> [u8; 3] {
    let path = std::env::temp_dir().join(format!(
        "eiviz-scene-{}-{scene}-{x}-{y}.png",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let cpath = CString::new(path.to_string_lossy().as_bytes()).unwrap();
    let mut code = unsafe { mixer_snapshot(scene, 0, cpath.as_ptr()) };
    // A shared CI runner with a software adapter has taken minutes for the rest of
    // the replay suite, so one snapshot gets a generous budget: failing here says
    // nothing about the picture the test is looking at.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while code != OK && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(250));
        code = unsafe { mixer_snapshot(scene, 0, cpath.as_ptr()) };
    }
    assert_eq!(code, OK, "snapshot scene {scene}");
    let image = image::open(&path).expect("decode scene snapshot").to_rgb8();
    let _ = std::fs::remove_file(&path);
    let pixel = image.get_pixel(x, y);
    [pixel[0], pixel[1], pixel[2]]
}

/// The picture the tests record: a solid red angle.
pub fn is_red(pixel: [u8; 3]) -> bool {
    pixel[0] > 200 && pixel[1] < 40 && pixel[2] < 40
}

pub fn is_black(pixel: [u8; 3]) -> bool {
    pixel[0] < 40 && pixel[1] < 40 && pixel[2] < 40
}
