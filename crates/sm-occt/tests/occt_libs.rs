//! The build script's OpenCascade version detection and library choice.

#[path = "../occt_libs.rs"]
#[allow(dead_code)]
mod occt_libs;

use occt_libs::{occt_version, step_libs, LIBS, STEP_LIBS_7_8, STEP_LIBS_PRE_7_8};

/// The define lines as OpenCascade 7.6 (Debian/Ubuntu) installs them.
const HEADER_7_6: &str = "\
//            OCC_VERSION_MAJOR       : (integer) number identifying major version
#define OCC_VERSION_MAJOR         7
#define OCC_VERSION_MINOR         6
#define OCC_VERSION_MAINTENANCE   3
";

/// The define lines as 7.9's CMake template generates them.
const HEADER_7_9: &str = "\
//            OCC_VERSION_MINOR       : (integer) number identifying minor version
#define OCC_VERSION_MAJOR 7
#define OCC_VERSION_MINOR 9
#define OCC_VERSION_MAINTENANCE 3
";

#[test]
fn reads_both_header_layouts() {
    assert_eq!(occt_version(HEADER_7_6), Some((7, 6)));
    assert_eq!(occt_version(HEADER_7_9), Some((7, 9)));
}

#[test]
fn installed_header_parses() {
    // The header the build script just used, when it is in the usual place.
    let path = std::path::Path::new("/usr/include/opencascade/Standard_Version.hxx");
    if let Ok(text) = std::fs::read_to_string(path) {
        let (major, minor) = occt_version(&text).expect("installed header has a version");
        assert!((major, minor) >= (7, 6));
    }
}

#[test]
fn comments_and_malformed_defines_are_not_versions() {
    assert_eq!(occt_version(""), None);
    // Only a comment mentions the names.
    assert_eq!(
        occt_version("// OCC_VERSION_MAJOR 7\n// OCC_VERSION_MINOR 9\n"),
        None
    );
    // A non-numeric value, and a minor version that is missing.
    assert_eq!(
        occt_version("#define OCC_VERSION_MAJOR x\n#define OCC_VERSION_MINOR 9\n"),
        None
    );
    assert_eq!(occt_version("#define OCC_VERSION_MAJOR 7\n"), None);
    assert_eq!(occt_version("#define OCC_VERSION_MAJOR\n"), None);
}

#[test]
fn step_libraries_follow_the_7_8_split() {
    assert_eq!(step_libs((7, 6)), STEP_LIBS_PRE_7_8);
    assert_eq!(step_libs((7, 7)), STEP_LIBS_PRE_7_8);
    assert_eq!(step_libs((7, 8)), STEP_LIBS_7_8);
    assert_eq!(step_libs((7, 9)), STEP_LIBS_7_8);
    assert_eq!(step_libs((8, 0)), STEP_LIBS_7_8);
}

#[test]
fn no_library_is_listed_twice() {
    for step in [STEP_LIBS_PRE_7_8, STEP_LIBS_7_8] {
        let mut all: Vec<_> = LIBS.iter().chain(step).collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n);
    }
}
