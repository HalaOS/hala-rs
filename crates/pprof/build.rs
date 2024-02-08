use std::{fs::create_dir, path::Path};

#[cfg(feature = "gperf")]
fn gen_gperf() {
    let out_dir = Path::new("src/gperf");

    if !out_dir.exists() {
        create_dir(out_dir).unwrap();
    }

    protobuf_codegen::Codegen::new()
        .protoc()
        .protoc_path(&protoc_bin_vendored::protoc_bin_path().unwrap())
        .includes(&["proto"])
        .input("proto/gperf.proto")
        .out_dir(out_dir)
        .run_from_script();
}

fn main() {
    #[cfg(feature = "gperf")]
    gen_gperf();

    let mut build = cc::Build::new();

    build
        .cpp(true)
        .static_crt(true)
        .flag_if_supported("-std=c++17")
        .flag_if_supported("/std:c++17")
        .flag_if_supported("/MD")
        .opt_level(3);

    println!("cargo:rerun-if-changed=src/c/external.cpp");
    build.file("src/c/external.cpp");
    build.compile("hala_pprof_c");
}
