//! Writes seed inputs for the fuzz targets into `fuzz/corpus/<target>/`:
//! real `.smp` files, their `document.json`s, and STEP exports, plus the
//! open-issue reproducers in `fuzz/regressions/`. Run with
//! `cargo run --manifest-path fuzz/Cargo.toml --example make_corpus`.

use sm_testkit::{large_document, pocketed_plate};
use std::io::Cursor;
use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    let docs = [
        ("plate1", pocketed_plate(1)),
        ("plate4", pocketed_plate(4)),
        ("large3", large_document(3, 5)),
    ];
    for (name, doc) in &docs {
        let model = sm_kernel::rebuild(doc).unwrap();
        let step = sm_step::export(&model, "seed.step");
        let mut smp = Cursor::new(Vec::new());
        sm_format::write(&mut smp, doc, Some(&step)).unwrap();
        write(&root.join("smp_read"), name, &smp.into_inner());
        write(
            &root.join("smp_document"),
            name,
            &serde_json::to_vec(doc).unwrap(),
        );
        write(&root.join("step_read"), name, step.as_bytes());
    }
    known_issues(Path::new(env!("CARGO_MANIFEST_DIR")));
}

fn write(dir: &Path, name: &str, bytes: &[u8]) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), bytes).unwrap();
}

/// Inputs that reproduce open resource-exhaustion findings from the security
/// re-test of PR #3. They are kept out of the seed corpus (they would fail
/// every smoke run) and replayed by the nightly job until the kernel has a
/// rebuild budget; then they move into `corpus/smp_document`.
fn known_issues(root: &Path) {
    let dir = root.join("regressions").join("smp_document");
    // One 9,999-point circle extruded 1,000 times as New Solid (MAX_BODIES).
    let mut doc = sm_document::PartDocument::new("Bodies", sm_document::LengthUnit::Mm);
    let n = 9_999;
    let points = (0..n)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            [10.0 * a.cos(), 10.0 * a.sin()]
        })
        .collect();
    let sketch = doc.add_feature(
        "Sketch1",
        sm_document::FeatureKind::Sketch(sm_document::Sketch {
            plane: sm_document::OriginPlane::XY,
            offset: 0.0,
            profiles: vec![sm_document::Profile { points }],
        }),
    );
    for i in 0..1_000 {
        doc.add_feature(
            format!("Extrusion{i}"),
            sm_document::FeatureKind::Extrude(sm_document::Extrude {
                sketch,
                extent: sm_document::ExtrudeExtent::Distance { distance: 1.0 },
                operation: sm_document::BodyOperation::NewBody,
            }),
        );
    }
    write(&dir, "bodies-1000x9999", &serde_json::to_vec(&doc).unwrap());

    // 2,500 disjoint 1 mm squares Join-extruded 4 times (MAX_TOOLS).
    let mut doc = sm_document::PartDocument::new("Tools", sm_document::LengthUnit::Mm);
    let squares = (0..2_500)
        .map(|i| {
            let (x, y) = ((i % 50) as f64 * 2.0, (i / 50) as f64 * 2.0);
            sm_testkit::rect([x, y, x + 1.0, y + 1.0])
        })
        .collect();
    let sketch = doc.add_feature(
        "Sketch1",
        sm_document::FeatureKind::Sketch(sm_document::Sketch {
            plane: sm_document::OriginPlane::XY,
            offset: 0.0,
            profiles: squares,
        }),
    );
    for i in 0..4 {
        doc.add_feature(
            format!("Extrusion{i}"),
            sm_document::FeatureKind::Extrude(sm_document::Extrude {
                sketch,
                extent: sm_document::ExtrudeExtent::Distance {
                    distance: 1.0 + i as f64,
                },
                operation: sm_document::BodyOperation::Join,
            }),
        );
    }
    write(&dir, "tools-4x2500", &serde_json::to_vec(&doc).unwrap());
}
