//! Loader checks run in one process, in one test, because a successful load
//! is permanent. Release builds do not accept the test key, so this file is
//! debug-only. `cargo test --release` still builds it as an empty crate.

#![cfg(debug_assertions)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use ed25519_dalek::{Signer, SigningKey};
use eiviz_mixer::{MixerCapabilities, load_pro_module, mixer_capabilities};
use eiviz_pro_api::{ABI_MAJOR, ModuleManifest, abi_hash, module_target};
use sha2::{Digest, Sha256};

const SECRET: [u8; 32] = [9; 32];
const OTHER_SECRET: [u8; 32] = [8; 32];

#[test]
fn signed_module_loads_and_rejects_tampering() {
    let public_key = public_hex(&SECRET);
    unsafe { std::env::set_var("EIVIZ_PRO_MODULE_TEST_KEY", &public_key) };
    unsafe { std::env::remove_var("EIVIZ_PRO_TEST_FAULT") };

    let mut caps = blank_caps();
    assert_eq!(unsafe { mixer_capabilities(&mut caps) }, 0);
    assert_eq!(caps.plan, 0);
    assert_eq!(caps.decklink_linked, 0);
    assert_eq!(caps.rtmp_linked, 0);

    let err = load_pro_module(Path::new("eiviz_pro.dll")).unwrap_err();
    assert!(err.contains("absolute"), "{err}");

    let missing =
        std::env::temp_dir().join(format!("eiviz-missing-pro-{}.dll", std::process::id()));
    let err = load_pro_module(&missing).unwrap_err();
    assert!(
        err.contains("Pro module path") || err.contains("reparse") || err.contains("cannot find"),
        "{err}"
    );

    let real = stage("good", &find_cdylib("eiviz_pro_test_plugin"));
    sign(&real, "0.3.0", &SECRET);
    let link = reparse_alias(&real);
    let err = load_pro_module(&link).unwrap_err();
    assert!(err.contains("reparse") || err.contains("symlink"), "{err}");

    let tampered = stage("tampered", &find_cdylib("eiviz_pro_test_plugin"));
    sign(&tampered, "0.3.0", &SECRET);
    flip_byte(&tampered);
    let err = load_pro_module(&tampered).unwrap_err();
    assert!(err.contains("hash"), "{err}");

    let wrong_key = stage("wrong-key", &find_cdylib("eiviz_pro_test_plugin"));
    sign(&wrong_key, "0.3.0", &OTHER_SECRET);
    let err = load_pro_module(&wrong_key).unwrap_err();
    assert!(err.contains("signature"), "{err}");

    let wrong_version = stage("wrong-version", &find_cdylib("eiviz_pro_test_plugin"));
    sign(&wrong_version, "9.9.9", &SECRET);
    let err = load_pro_module(&wrong_version).unwrap_err();
    assert!(err.contains("version"), "{err}");

    let empty = stage("empty", &find_cdylib("eiviz_pro_empty_plugin"));
    sign(&empty, "0.3.0", &SECRET);
    let err = load_pro_module(&empty).unwrap_err();
    assert!(err.contains("eiviz_pro_get_api"), "{err}");

    for fault in ["short", "hash", "incomplete", "reject"] {
        unsafe { std::env::set_var("EIVIZ_PRO_TEST_FAULT", fault) };
        let path = stage(fault, &find_cdylib("eiviz_pro_test_plugin"));
        sign(&path, "0.3.0", &SECRET);
        let err = load_pro_module(&path).unwrap_err();
        assert!(
            err.contains("ABI") || err.contains("incomplete") || err.contains("rejected"),
            "{fault}: {err}"
        );
    }
    unsafe { std::env::remove_var("EIVIZ_PRO_TEST_FAULT") };

    let module = stage("loaded", &find_cdylib("eiviz_pro_test_plugin"));
    sign(&module, "0.3.0", &SECRET);
    load_pro_module(&module).unwrap();
    load_pro_module(&module).unwrap();
    let err = load_pro_module(&real).unwrap_err();
    assert!(err.contains("already loaded"), "{err}");

    let mut caps = blank_caps();
    assert_eq!(unsafe { mixer_capabilities(&mut caps) }, 0);
    assert_eq!(caps.plan, 1);
    assert_eq!(caps.decklink_linked, 1);
    assert_eq!(caps.rtmp_linked, 1);
    assert_eq!(caps.mixing_unit_limit, u32::MAX);

    let library = unsafe { libloading::Library::new(&module) }.unwrap();
    let capture = eiviz_mixer::pro_test_open_capture().unwrap();
    emit(&library);
    assert_eq!(eiviz_mixer::pro_test_frames(), 1);
    assert!(capture.stats_frames() >= 1);
    drop(capture);
    emit(&library);
    assert_eq!(eiviz_mixer::pro_test_frames(), 1);

    let capture = eiviz_mixer::pro_test_open_capture().unwrap();
    call(&library, b"eiviz_pro_test_arm_hold\0");
    call(&library, b"eiviz_pro_test_fire_held\0");
    wait_entered(&library);
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        drop(capture);
        let _ = tx.send(());
    });
    std::thread::sleep(Duration::from_millis(40));
    assert!(rx.try_recv().is_err());
    call(&library, b"eiviz_pro_test_release_hold\0");
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    worker.join().unwrap();

    call(&library, b"eiviz_pro_test_arm_destroy_hold\0");
    let output = eiviz_mixer::pro_test_open_output().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        drop(output);
        let _ = tx.send(());
    });
    std::thread::sleep(Duration::from_millis(40));
    assert!(rx.try_recv().is_err());
    assert_eq!(handles(&library), 1);
    eiviz_mixer::pro_test_shutdown();
    assert_eq!(handles(&library), 1);
    call(&library, b"eiviz_pro_test_release_destroy_hold\0");
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    worker.join().unwrap();
    eiviz_mixer::pro_test_shutdown();
    std::mem::forget(library);
}

fn blank_caps() -> MixerCapabilities {
    MixerCapabilities {
        plan: 0,
        mixing_unit_limit: 0,
        decklink_input_limit: 0,
        decklink_output_limit: 0,
        rtmp_max_width: 0,
        rtmp_max_height: 0,
        rtmp_max_fps_num: 0,
        rtmp_max_fps_den: 0,
        recording: 0,
        srt: 0,
        hardware_encode: 0,
        decklink_linked: 0,
        rtmp_linked: 0,
    }
}

fn public_hex(secret: &[u8; 32]) -> String {
    hex(&SigningKey::from_bytes(secret).verifying_key().to_bytes())
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

fn flip_byte(path: &Path) {
    let mut bytes = std::fs::read(path).unwrap();
    let index = bytes.len() / 2;
    bytes[index] ^= 0xff;
    std::fs::write(path, bytes).unwrap();
}

fn reparse_alias(target: &Path) -> PathBuf {
    let file_link = target.with_file_name("linked-pro.dll");
    let _ = std::fs::remove_file(&file_link);
    #[cfg(windows)]
    if std::os::windows::fs::symlink_file(target, &file_link).is_ok() {
        return file_link;
    }
    #[cfg(unix)]
    if std::os::unix::fs::symlink(target, &file_link).is_ok() {
        return file_link;
    }
    #[cfg(windows)]
    {
        let junction = target.parent().unwrap().join("junction-dir");
        let _ = std::fs::remove_dir(&junction);
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(target.parent().unwrap())
            .status()
            .expect("mklink");
        assert!(status.success(), "could not create a junction");
        return junction.join(target.file_name().unwrap());
    }
    #[cfg(not(windows))]
    panic!("could not create a symlink");
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn emit(library: &libloading::Library) {
    call(library, b"eiviz_pro_test_emit_video\0");
}

fn call(library: &libloading::Library, symbol: &[u8]) {
    let function: libloading::Symbol<extern "C" fn()> = unsafe { library.get(symbol) }.unwrap();
    function();
}

fn wait_entered(library: &libloading::Library) {
    let entered: libloading::Symbol<extern "C" fn() -> u32> =
        unsafe { library.get(b"eiviz_pro_test_callback_entered\0") }.unwrap();
    for _ in 0..1000 {
        if entered() == 1 {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("held callback did not start");
}

fn handles(library: &libloading::Library) -> u32 {
    let handles: libloading::Symbol<extern "C" fn() -> u32> =
        unsafe { library.get(b"eiviz_pro_test_handles\0") }.unwrap();
    handles()
}
