use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn bundled_openh264_file_name() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("openh264-2.6.0-win64.dll")
    } else if cfg!(all(windows, target_arch = "x86")) {
        Some("openh264-2.6.0-win32.dll")
    } else if cfg!(all(windows, target_arch = "aarch64")) {
        Some("openh264-2.6.0-win-arm64.dll")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("libopenh264-2.6.0-linux64.8.so")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("libopenh264-2.6.0-mac-x64.dylib")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("libopenh264-2.6.0-mac-arm64.dylib")
    } else {
        None
    }
}

fn profile_dir(out_dir: &Path) -> Option<PathBuf> {
    out_dir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

fn main() {
    println!("cargo:rerun-if-changed=runtime/openh264");
    let Some(file_name) = bundled_openh264_file_name() else {
        return;
    };
    let src = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("runtime")
        .join("openh264")
        .join(file_name);
    if !src.is_file() {
        println!(
            "cargo:warning=bundled Cisco OpenH264 2.6.0 binary is missing at {}",
            src.display()
        );
        return;
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let Some(profile_dir) = profile_dir(&out_dir) else {
        println!("cargo:warning=could not locate cargo profile directory from OUT_DIR");
        return;
    };
    let dest = profile_dir.join(file_name);
    fs::copy(&src, &dest).unwrap_or_else(|error| {
        panic!(
            "failed to copy bundled OpenH264 from {} to {}: {error}",
            src.display(),
            dest.display()
        )
    });
}
