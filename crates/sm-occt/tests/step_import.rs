//! STEP import through the sandboxed worker, checked against what
//! OpenCascade itself reports for each corpus file (`expected/*.json`).

use sm_occt::step::{export_step, import_step, ExportBody, ImportOptions};
use sm_occt::{boolean, make_box, make_cylinder, BooleanKind, OcctError};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../sm-step/tests/corpus")
}

fn opts() -> ImportOptions {
    ImportOptions {
        worker: Some(PathBuf::from(env!("CARGO_BIN_EXE_sm-step-worker"))),
        timeout: Duration::from_secs(120),
        ..ImportOptions::default()
    }
}

/// (volume, bbox) per solid from the oracle, sorted like the oracle.
fn expected(name: &str) -> Vec<(f64, [f64; 6])> {
    let text = std::fs::read_to_string(corpus_dir().join("expected").join(format!("{name}.json")))
        .unwrap();
    // The oracle files are flat and machine-written; pull the numbers out
    // without a JSON dependency.
    let mut out = Vec::new();
    for chunk in text.split("\"volume\":").skip(1) {
        let nums: Vec<f64> = chunk
            .split(|c: char| {
                !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e' || c == 'E' || c == '+')
            })
            .filter(|s| !s.is_empty() && s.parse::<f64>().is_ok())
            .map(|s| s.parse().unwrap())
            .take(8)
            .collect();
        out.push((
            nums[0],
            [nums[1], nums[2], nums[3], nums[4], nums[5], nums[6]],
        ));
    }
    out
}

fn sorted_actual(import: &sm_occt::step::StepImport) -> Vec<(f64, [f64; 6])> {
    let mut v: Vec<(f64, [f64; 6])> = import
        .bodies
        .iter()
        .map(|b| {
            let m = b.shape.mass_props().unwrap();
            let bb = b.shape.bounding_box().unwrap();
            (
                m.volume,
                [
                    bb.min[0], bb.min[1], bb.min[2], bb.max[0], bb.max[1], bb.max[2],
                ],
            )
        })
        .collect();
    v.sort_by(|a, b| {
        a.1.iter()
            .zip(&b.1)
            .map(|(x, y)| ((x * 1e6).round()).total_cmp(&(y * 1e6).round()))
            .find(|o| o.is_ne())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.total_cmp(&b.0))
    });
    v
}

fn check_file(dir: &str, file: &str) {
    let path = corpus_dir().join(dir).join(file);
    let bytes = std::fs::read(&path).unwrap();
    let import = import_step(&bytes, &opts()).unwrap_or_else(|e| panic!("{file}: {e}"));
    let name = Path::new(file).file_stem().unwrap().to_str().unwrap();
    let want = expected(name);
    let got = sorted_actual(&import);
    assert_eq!(got.len(), want.len(), "{file}: solid count");
    for (i, ((gv, gb), (wv, wb))) in got.iter().zip(&want).enumerate() {
        assert!(
            (gv - wv).abs() <= 1e-6 * wv.abs().max(1.0),
            "{file} solid {i}: volume {gv} vs {wv}"
        );
        for k in 0..6 {
            // The oracle's optimal boxes are rounded to 1e-6 mm.
            assert!(
                (gb[k] - wb[k]).abs() <= 1e-4 * (1.0 + wb[k].abs()),
                "{file} solid {i}: bbox {gb:?} vs {wb:?}"
            );
        }
    }
    for b in &import.bodies {
        assert!(b.shape.is_valid(), "{file}: {}", b.name);
    }
}

#[test]
fn occt_corpus_matches_oracle() {
    let mut files: Vec<String> = std::fs::read_dir(corpus_dir().join("occt"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".step"))
        .collect();
    files.sort();
    assert!(files.len() >= 19);
    for f in files {
        check_file("occt", &f);
    }
}

#[test]
fn real_world_files_match_oracle() {
    check_file("external", "as1-oc-214.stp");
    check_file("external", "red_cube_blue_cylinder.step");
}

#[test]
fn names_colours_and_units_come_through() {
    let read = |dir: &str, f: &str| {
        import_step(
            &std::fs::read(corpus_dir().join(dir).join(f)).unwrap(),
            &opts(),
        )
        .unwrap()
    };
    let asm = read("occt", "assembly.step");
    let mut names: Vec<&str> = asm.bodies.iter().map(|b| b.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["bracket", "pin", "pin", "pin"]);
    let colored = read("external", "red_cube_blue_cylinder.step");
    // Onshape's default red and blue.
    let color = |name: &str| {
        colored
            .bodies
            .iter()
            .find(|b| b.name == name)
            .unwrap()
            .color
            .unwrap()
    };
    let (red, blue) = (color("red_cube"), color("blue_cylinder"));
    assert!(red[0] > 0.8 && red[2] < 0.2, "{red:?}");
    assert!(blue[2] > 0.6 && blue[0] < 0.3, "{blue:?}");
    // The file holds a 10 x 20 x 30 inch box; it reads back in millimetres.
    let inch = read("occt", "box_inch.step");
    assert_eq!(inch.screened.length_unit, "inch");
    let v = inch.bodies[0].shape.mass_props().unwrap().volume;
    let want = 6000.0 * 25.4f64.powi(3);
    assert!((v - want).abs() < 1e-6 * want, "{v}");
    let top = inch.bodies[0].shape.bounding_box().unwrap().max[2];
    assert!((top - 762.0).abs() < 1e-4, "{top}");
}

#[test]
fn malformed_files_never_reach_opencascade() {
    let good = std::fs::read(corpus_dir().join("occt/box.step")).unwrap();
    for bad in [&b""[..], b"garbage", &good[..good.len() / 2]] {
        let err = import_step(bad, &opts()).unwrap_err();
        assert!(matches!(err, OcctError::Step(_)), "{err:?}");
    }
}

#[test]
fn worker_timeout_and_missing_worker_are_reported() {
    let bytes = std::fs::read(corpus_dir().join("as1-oc-214.stp")).ok();
    let bytes = bytes
        .unwrap_or_else(|| std::fs::read(corpus_dir().join("external/as1-oc-214.stp")).unwrap());
    let quick = ImportOptions {
        timeout: Duration::from_millis(1),
        ..opts()
    };
    let err = import_step(&bytes, &quick).unwrap_err();
    assert!(err.to_string().contains("longer than"), "{err}");
    let missing = ImportOptions {
        worker: Some(PathBuf::from("/nonexistent/sm-step-worker")),
        ..opts()
    };
    assert!(import_step(&bytes, &missing)
        .unwrap_err()
        .to_string()
        .contains("cannot start"));
}

#[test]
fn export_then_import_round_trips() {
    let part = boolean(
        BooleanKind::Cut,
        &make_box(40.0, 20.0, 10.0).unwrap(),
        &make_cylinder(4.0, 30.0).unwrap(),
    )
    .unwrap()
    .shape;
    let dir = std::env::temp_dir().join(format!("sm-occt-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("part.step");
    export_step(
        &[ExportBody {
            name: "Plate ø40",
            color: Some([0.2, 0.4, 0.8]),
            shape: &part,
        }],
        &path,
    )
    .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    let at = text.find("FILE_SCHEMA").unwrap();
    let schema = &text[at..text[at..].find(';').map_or(text.len(), |e| at + e)];
    assert!(schema.contains("AP242"), "{schema}");
    let back = import_step(&bytes, &opts()).unwrap();
    assert_eq!(back.bodies.len(), 1);
    let b = &back.bodies[0];
    assert_eq!(b.name, "Plate ø40");
    let c = b.color.unwrap();
    assert!(
        (c[0] - 0.2).abs() < 1e-3 && (c[2] - 0.8).abs() < 1e-3,
        "{c:?}"
    );
    let (v0, v1) = (
        part.mass_props().unwrap().volume,
        b.shape.mass_props().unwrap().volume,
    );
    assert!((v0 - v1).abs() < 1e-6 * v0, "{v0} vs {v1}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn sandbox_blocks_files_processes_and_network() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_sm-step-worker"))
        .arg("--sandbox-self-test")
        .env_clear()
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{report} {}",
        String::from_utf8_lossy(&out.stderr)
    );
    for line in [
        "read_file=denied",
        "list_dir=denied",
        "write_file=denied",
        "exec=denied",
        "socket=denied",
        "thread=allowed",
        "allocate=allowed",
        "occt=allowed",
    ] {
        assert!(
            report.lines().any(|l| l == line),
            "expected {line} in:\n{report}"
        );
    }
}
