//! Builds the C++ shim and links the system OpenCascade (Arch `opencascade`,
//! Debian/Ubuntu `libocct-*-dev`, 7.6 or newer). Set `OCCT_INCLUDE_DIR` and
//! `OCCT_LIB_DIR` for an install outside the usual prefixes.

use std::path::{Path, PathBuf};

const LIBS: &[&str] = &[
    "TKernel", "TKMath", "TKG2d", "TKG3d", "TKGeomBase", "TKGeomAlgo", "TKBRep", "TKTopAlgo",
    "TKPrim", "TKBO", "TKBool", "TKFillet", "TKOffset", "TKFeat", "TKShHealing", "TKMesh",
    "TKXSBase", "TKSTEP", "TKSTEPBase", "TKSTEP209", "TKSTEPAttr", "TKXDESTEP", "TKXCAF",
    "TKLCAF", "TKCAF", "TKCDF", "TKVCAF", "TKService", "TKV3d",
];

fn include_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("OCCT_INCLUDE_DIR") {
        return PathBuf::from(dir);
    }
    for dir in [
        "/usr/include/opencascade",
        "/usr/local/include/opencascade",
        "/opt/opencascade/include/opencascade",
    ] {
        if Path::new(dir).join("Standard_Version.hxx").exists() {
            return PathBuf::from(dir);
        }
    }
    panic!(
        "OpenCascade headers not found. Install OpenCascade 7.6+ \
         (Arch: `pacman -S opencascade`; Debian/Ubuntu: `apt install \
         libocct-modeling-algorithms-dev libocct-data-exchange-dev libocct-ocaf-dev`) \
         or set OCCT_INCLUDE_DIR"
    );
}

fn main() {
    println!("cargo:rerun-if-env-changed=OCCT_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=OCCT_LIB_DIR");
    println!("cargo:rerun-if-changed=src/ffi.rs");
    println!("cargo:rerun-if-changed=cpp/shim.h");
    println!("cargo:rerun-if-changed=cpp/shim.cpp");

    let include = include_dir();
    cxx_build::bridge("src/ffi.rs")
        .file("cpp/shim.cpp")
        .include(&include)
        .std("c++17")
        .flag_if_supported("-Wno-deprecated-declarations")
        .flag_if_supported("-Wno-unused-parameter")
        .compile("sm-occt-shim");

    if let Some(dir) = std::env::var_os("OCCT_LIB_DIR") {
        println!("cargo:rustc-link-search=native={}", Path::new(&dir).display());
    }
    for lib in LIBS {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
}
