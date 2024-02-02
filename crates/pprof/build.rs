use std::{fs::create_dir, path::Path};

fn main() {
    let out_dir = Path::new("src/proto");

    if !out_dir.exists() {
        create_dir(out_dir).unwrap();
    }

    protobuf_codegen::Codegen::new()
        .protoc()
        .protoc_path(&protoc_bin_vendored::protoc_bin_path().unwrap())
        .includes(&["proto"])
        .input("proto/profile.proto")
        .input("proto/backtrace.proto")
        .out_dir(out_dir)
        .run_from_script();

    let mut build = cc::Build::new();

    build
        .cpp(true)
        .static_crt(true)
        .flag_if_supported("-std=c++17")
        .flag_if_supported("/std:c++17")
        .opt_level(3);

    println!("cargo:rerun-if-changed=src/external.cpp");
    build.file("src/external.cpp");
    build.compile("external");
}
