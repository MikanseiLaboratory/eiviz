use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=NDI_SDK_DIR");
    if cfg!(windows) {
        copy_windows_ndi_dll();
    }
    if cfg!(target_os = "macos") {
        copy_macos_ndi_dylib();
    }
}

fn copy_windows_ndi_dll() {
    let Some(dll) = ndi_runtime_dll() else {
        return;
    };
    println!("cargo:rerun-if-changed={}", dll.display());
    let Ok(out_dir) = env::var("OUT_DIR") else {
        return;
    };
    let Some(profile_dir) = Path::new(&out_dir).ancestors().nth(3) else {
        return;
    };
    copy_file(&dll, &profile_dir.join("Processing.NDI.Lib.x64.dll"));
    copy_file(&dll, &profile_dir.join("deps").join("Processing.NDI.Lib.x64.dll"));
}

fn ndi_runtime_dll() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(dir) = env::var("NDI_SDK_DIR") {
        roots.push(PathBuf::from(dir));
    }
    roots.push(PathBuf::from(r"C:\Program Files\NDI\NDI 6 SDK"));
    for root in roots {
        let dll = root.join(r"Bin\x64\Processing.NDI.Lib.x64.dll");
        if dll.is_file() {
            return Some(dll);
        }
    }
    None
}

fn copy_macos_ndi_dylib() {
    let Some(dylib) = ndi_runtime_dylib() else {
        return;
    };
    println!("cargo:rerun-if-changed={}", dylib.display());
    let Ok(out_dir) = env::var("OUT_DIR") else {
        return;
    };
    let Some(profile_dir) = Path::new(&out_dir).ancestors().nth(3) else {
        return;
    };
    copy_file(&dylib, &profile_dir.join("libndi.dylib"));
    copy_file(&dylib, &profile_dir.join("deps").join("libndi.dylib"));
}

fn ndi_runtime_dylib() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(dir) = env::var("NDI_SDK_DIR") {
        roots.push(PathBuf::from(dir));
    }
    roots.extend([
        PathBuf::from("/Library/NDI SDK for Apple"),
        PathBuf::from("/Library/NDI SDK for macOS"),
        PathBuf::from("/Library/NDI 6 SDK"),
        PathBuf::from("/Library/NDI SDK"),
    ]);
    let names = ["libndi.dylib", "libndi.6.dylib", "libndi.5.dylib", "libndi.4.dylib"];
    for root in roots {
        for libdir in [root.join("lib/macOS"), root.join("lib")] {
            for name in names {
                let path = libdir.join(name);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn copy_file(src: &Path, dest: &Path) {
    if let Some(parent) = dest.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::copy(src, dest);
}
