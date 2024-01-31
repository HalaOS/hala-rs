use std::{fs::create_dir, path::Path};

fn main() {
    let out_dir = Path::new("src/proto");

    if !out_dir.exists() {
        create_dir(out_dir).unwrap();
    }

    // Use this in build.rs
    protobuf_codegen::Codegen::new()
        .protoc()
        .protoc_path(&protoc_bin_vendored::protoc_bin_path().unwrap())
        .includes(&["proto"])
        .input("proto/profile.proto")
        .out_dir(out_dir)
        .run_from_script();
}
