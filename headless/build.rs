fn main() {
    // NDI is a link-time dylib. A plain `cargo build` otherwise leaves every
    // binary with `@rpath/libndi.dylib` and no LC_RPATH, so it cannot start on a
    // machine that did not set DYLD_LIBRARY_PATH. The mixer's build script copies
    // libndi next to the binaries; point dyld there for both the development tree
    // and the packaged /usr/local/bin layout.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path");
    }
}
