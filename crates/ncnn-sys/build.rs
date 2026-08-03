use std::env;
use std::path::PathBuf;

// NCNN_DIR is the install prefix of a source build of ncnn configured with
// -DNCNN_VULKAN=ON; docker/Dockerfile.spike produces one at /opt/ncnn.
fn main() {
    let prefix = PathBuf::from(env::var("NCNN_DIR").unwrap_or_else(|_| "/opt/ncnn".into()));
    let include = prefix.join("include/ncnn");
    let lib = prefix.join("lib");

    assert!(
        include.join("c_api.h").exists(),
        "ncnn headers not found under {} -- set NCNN_DIR to an ncnn install prefix",
        include.display()
    );

    cc::Build::new()
        .cpp(true)
        .file("csrc/c_api_ext.cpp")
        .include("csrc")
        .include(&include)
        .flag_if_supported("-std=c++11")
        .compile("ncnn_c_api_ext");

    println!("cargo:rustc-link-search=native={}", lib.display());
    // Shared, so glslang and the Vulkan loader stay ncnn's problem rather than
    // becoming link-order ones here.
    println!("cargo:rustc-link-lib=dylib=ncnn");
    // So the binary finds libncnn.so without LD_LIBRARY_PATH -- which is what
    // makes the Nix package work unwrapped.
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
    println!("cargo:rustc-link-lib=dylib=stdc++");
    println!("cargo:rerun-if-changed=csrc/c_api_ext.cpp");
    println!("cargo:rerun-if-changed=csrc/c_api_ext.h");
    println!("cargo:rerun-if-env-changed=NCNN_DIR");
}
