fn main() {
    println!("cargo:rerun-if-changed=proto/app.proto");
    // Use the bundled protoc unless PROTOC picks one, so builds need no
    // system protobuf compiler.
    if std::env::var_os("PROTOC").is_none() {
        let protoc = protoc_bin_vendored::protoc_bin_path().unwrap();
        // SAFETY: build scripts are single-threaded at this point.
        unsafe { std::env::set_var("PROTOC", protoc) };
    }
    tonic_prost_build::configure()
        .compile_protos(&["proto/app.proto"], &["proto"])
        .unwrap();
}
