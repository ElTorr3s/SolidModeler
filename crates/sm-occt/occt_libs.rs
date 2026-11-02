// Which OpenCascade libraries to link. Shared by build.rs (through
// `include!`) and tests/occt_libs.rs, so the version logic is tested.

/// Libraries every supported OpenCascade version has.
pub const LIBS: &[&str] = &[
    "TKernel",
    "TKMath",
    "TKG2d",
    "TKG3d",
    "TKGeomBase",
    "TKGeomAlgo",
    "TKBRep",
    "TKTopAlgo",
    "TKPrim",
    "TKBO",
    "TKBool",
    "TKFillet",
    "TKOffset",
    "TKFeat",
    "TKShHealing",
    "TKMesh",
    "TKXSBase",
    "TKXCAF",
    "TKLCAF",
    "TKCAF",
    "TKCDF",
    "TKVCAF",
    "TKService",
    "TKV3d",
];

/// STEP libraries before 7.8 (Debian/Ubuntu 7.6).
pub const STEP_LIBS_PRE_7_8: &[&str] = &[
    "TKSTEP",
    "TKSTEPBase",
    "TKSTEP209",
    "TKSTEPAttr",
    "TKXDESTEP",
];

/// 7.8 merged the STEP libraries into TKDESTEP on top of TKDE (Arch ships
/// 7.8 and later).
pub const STEP_LIBS_7_8: &[&str] = &["TKDE", "TKDESTEP"];

/// Reads `OCC_VERSION_MAJOR` and `OCC_VERSION_MINOR` from the text of
/// `Standard_Version.hxx`.
pub fn occt_version(header: &str) -> Option<(u32, u32)> {
    let define = |name: &str| {
        header.lines().find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some("#define") && words.next() == Some(name))
                .then(|| words.next()?.parse().ok())
                .flatten()
        })
    };
    Some((define("OCC_VERSION_MAJOR")?, define("OCC_VERSION_MINOR")?))
}

pub fn step_libs(version: (u32, u32)) -> &'static [&'static str] {
    if version >= (7, 8) {
        STEP_LIBS_7_8
    } else {
        STEP_LIBS_PRE_7_8
    }
}
