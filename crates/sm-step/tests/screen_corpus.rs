//! Every file in the corpus passes screening, with the solid count and
//! units OpenCascade agrees with.

use sm_step::screen::{screen, ImportError};
use std::path::{Path, PathBuf};

fn corpus() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    let mut files = Vec::new();
    for dir in ["occt", "external"] {
        for entry in std::fs::read_dir(root.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("step" | "stp")
            ) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Solids OpenCascade found in the file, from the oracle.
fn expected_solids(path: &Path) -> usize {
    let name = path.file_stem().unwrap().to_str().unwrap();
    let json = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/corpus/expected")
            .join(format!("{name}.json")),
    )
    .unwrap();
    json.matches("\"volume\"").count()
}

#[test]
fn whole_corpus_passes_screening() {
    let files = corpus();
    assert!(files.len() >= 20, "corpus is missing files");
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        let s = screen(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let name = path.file_name().unwrap().to_str().unwrap();
        // Assemblies instance solids, so OpenCascade can see more solids
        // than there are B-rep items; never fewer.
        assert!(s.solids >= 1, "{name}");
        assert!(
            s.solids <= expected_solids(&path),
            "{name}: {} solids",
            s.solids
        );
        let unit = match name {
            "box_inch.step" => "inch",
            "box_ap203.step" | "red_cube_blue_cylinder.step" => "metre",
            _ => "millimetre",
        };
        assert_eq!(s.length_unit, unit, "{name}");
        assert!(s.warnings.is_empty(), "{name}: {:?}", s.warnings);
    }
}

#[test]
fn assembly_lists_products_and_shared_solids() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/occt/assembly.step");
    let s = screen(&std::fs::read(path).unwrap()).unwrap();
    assert!(s.schema.starts_with("AP242"), "{}", s.schema);
    // The pin is defined once and placed three times.
    assert_eq!(s.solids, 2);
    for p in ["top", "pin_pair", "bracket", "pin"] {
        assert!(s.products.iter().any(|n| n == p), "{p} in {:?}", s.products);
    }
}

#[test]
fn truncated_and_corrupted_corpus_files_are_rejected_cleanly() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/occt/plate_with_holes.step");
    let bytes = std::fs::read(path).unwrap();
    // Every prefix that cuts the file short must fail, not panic.
    for cut in (0..bytes.len() - 30).step_by(97) {
        assert!(
            screen(&bytes[..cut]).is_err(),
            "prefix of {cut} bytes accepted"
        );
    }
    // Pointing a face at an undefined entity is caught before OpenCascade.
    let text = String::from_utf8(bytes).unwrap();
    let broken = text.replacen("ADVANCED_FACE('',(#", "ADVANCED_FACE('',(#999999,#", 1);
    assert!(matches!(
        screen(broken.as_bytes()),
        Err(ImportError::DanglingReference { to: 999999, .. })
    ));
}

#[test]
fn files_without_solids_are_rejected() {
    let text =
        "ISO-10303-21;HEADER;FILE_SCHEMA(('AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF'));\
                ENDSEC;DATA;#1=CARTESIAN_POINT('',(0.,0.,0.));ENDSEC;END-ISO-10303-21;";
    assert!(matches!(
        screen(text.as_bytes()),
        Err(ImportError::NoSolids)
    ));
}
