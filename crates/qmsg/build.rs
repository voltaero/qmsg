fn main() {
    let proto = "../qmsg-orchestrator/proto/app.proto";
    println!("cargo:rerun-if-changed={proto}");
    if std::env::var_os("PROTOC").is_none() {
        // The build script has not started any threads.
        unsafe { std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path().unwrap()) };
    }
    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(&[proto], &["../qmsg-orchestrator/proto"])
        .unwrap();
}
