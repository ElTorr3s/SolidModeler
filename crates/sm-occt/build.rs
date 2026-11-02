//! Builds the C++ shim and links the system OpenCascade (Arch `opencascade`,
//! Debian/Ubuntu `libocct-*-dev`, 7.6 or newer). Set `OCCT_INCLUDE_DIR` and
//! `OCCT_LIB_DIR` for an install outside the usual prefixes.

use std::path::{Path, PathBuf};

include!("occt_libs.rs");

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
        println!(
            "cargo:rustc-link-search=native={}",
            Path::new(&dir).display()
        );
    }
    let header = include.join("Standard_Version.hxx");
    println!("cargo:rerun-if-changed={}", header.display());
    let text = std::fs::read_to_string(&header)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", header.display()));
    let version = occt_version(&text)
        .unwrap_or_else(|| panic!("no OCC_VERSION_MAJOR/MINOR in {}", header.display()));
    if version < (7, 6) {
        panic!(
            "OpenCascade {}.{} is too old; sm-occt needs 7.6 or newer",
            version.0, version.1
        );
    }
    for lib in LIBS.iter().chain(step_libs(version)) {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
}
