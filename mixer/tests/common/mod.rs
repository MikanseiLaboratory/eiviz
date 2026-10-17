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

/// One pixel of a scene's composed picture, read the way a monitor shows it. The
/// first request can land before a frame exists, so it is retried once.
pub fn scene_pixel(scene: u64, x: u32, y: u32) -> [u8; 3] {
    let path = std::env::temp_dir().join(format!(
        "eiviz-scene-{}-{scene}-{x}-{y}.png",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let cpath = CString::new(path.to_string_lossy().as_bytes()).unwrap();
    let mut code = unsafe { mixer_snapshot(scene, 0, cpath.as_ptr()) };
    if code != OK {
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
