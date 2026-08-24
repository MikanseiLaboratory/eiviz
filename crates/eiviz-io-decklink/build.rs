use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=DECKLINK_SDK_DIR");
    println!("cargo:rerun-if-changed=native/eiviz_decklink_shim.cpp");
    println!("cargo:rerun-if-changed=native/eiviz_decklink_shim.h");

    if env::var_os("CARGO_FEATURE_DECKLINK_SDK").is_none() {
        return;
    }

    let root = env::var_os("DECKLINK_SDK_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("the `decklink-sdk` feature requires DECKLINK_SDK_DIR"));
    let include = locate(&root, "DeckLinkAPI.h").unwrap_or_else(|| {
        panic!(
            "DeckLinkAPI.h was not found below DECKLINK_SDK_DIR={}",
            root.display()
        )
    });
    let include_dir = include.parent().expect("DeckLinkAPI.h has a parent");
    println!("cargo:rerun-if-changed={}", include.display());
    let version = include_dir.join("DeckLinkAPIVersion.h");
    if !version.is_file() {
        panic!(
            "DeckLinkAPIVersion.h was not found next to {}",
            include.display()
        );
    }
    println!("cargo:rerun-if-changed={}", version.display());

    let mut shim = cc::Build::new();
    shim.cpp(true)
        .std("c++17")
        .warnings(true)
        .include(include_dir)
        .include("native")
        .file("native/eiviz_decklink_shim.cpp");
    if cfg!(target_env = "msvc") {
        shim.flag("/EHsc");
    } else {
        shim.flag("-fno-exceptions");
    }
    shim.compile("eiviz_decklink_shim");

    let target = env::var("CARGO_CFG_TARGET_OS").expect("Cargo sets target OS");
    match target.as_str() {
        "linux" | "macos" => {
            let dispatch = locate(&root, "DeckLinkAPIDispatch.cpp").unwrap_or_else(|| {
                panic!(
                    "DeckLinkAPIDispatch.cpp was not found below DECKLINK_SDK_DIR={}",
                    root.display()
                )
            });
            println!("cargo:rerun-if-changed={}", dispatch.display());
            cc::Build::new()
                .cpp(true)
                .std("c++17")
                .warnings(false)
                .include(include_dir)
                .file(dispatch)
                .compile("decklink_api_dispatch");
            if target == "linux" {
                println!("cargo:rustc-link-lib=dl");
                println!("cargo:rustc-link-lib=pthread");
            } else {
                println!("cargo:rustc-link-lib=framework=CoreFoundation");
            }
        }
        "windows" => {
            let interfaces = locate(&root, "DeckLinkAPI_i.c").unwrap_or_else(|| {
                panic!(
                    "DeckLinkAPI_i.c was not found below DECKLINK_SDK_DIR={}",
                    root.display()
                )
            });
            println!("cargo:rerun-if-changed={}", interfaces.display());
            cc::Build::new()
                .warnings(false)
                .include(include_dir)
                .file(interfaces)
                .compile("decklink_api_interfaces");
            println!("cargo:rustc-link-lib=ole32");
            println!("cargo:rustc-link-lib=oleaut32");
        }
        other => panic!("the DeckLink SDK shim does not support target OS {other}"),
    }
}

fn locate(root: &Path, name: &str) -> Option<PathBuf> {
    let direct = root.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = locate(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|file| file == name) {
            return Some(path);
        }
    }
    None
}
