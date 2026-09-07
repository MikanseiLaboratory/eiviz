fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc");
    unsafe {
        std::env::set_var("PROTOC", protoc);
    }
    prost_build::Config::new()
        .compile_protos(&["proto/eiviz/control/v1/control.proto"], &["proto"])
        .expect("compile control.proto");
}
