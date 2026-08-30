use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=NDI_SDK_DIR");
    if env::var("CARGO_CFG_WINDOWS").is_err() {
        return;
    }
    let Some(dll) = ndi_runtime_dll() else {
        return;
    };
    println!("cargo:rerun-if-changed={}", dll.display());
    let Ok(out_dir) = env::var("OUT_DIR") else {
        return;
    };
    // OUT_DIR = target/<profile>/build/<crate>/out → profile dir is three ancestors up.
    let Some(profile_dir) = Path::new(&out_dir).ancestors().nth(3) else {
        return;
    };
    copy_dll(&dll, &profile_dir.join("Processing.NDI.Lib.x64.dll"));
    copy_dll(&dll, &profile_dir.join("deps").join("Processing.NDI.Lib.x64.dll"));
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

fn copy_dll(src: &Path, dest: &Path) {
    if let Some(parent) = dest.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::copy(src, dest);
}
