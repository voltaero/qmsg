fn main() {
    println!("cargo:rerun-if-changed=proto/app.proto");
    tonic_prost_build::configure()
        .compile_protos(&["proto/app.proto"], &["proto"])
        .unwrap();
}
