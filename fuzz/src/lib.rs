//! Shared helpers for the fuzz targets.

use arbitrary::{Arbitrary, Result, Unstructured};
use sm_document::{
    BodyOperation, Extrude, ExtrudeExtent, FeatureId, FeatureKind, LengthUnit, OriginPlane,
    Parameter, PartDocument, Profile, Sketch,
};
use std::io::{Cursor, Write};

/// libfuzzer-sys installs a panic hook that aborts on *any* panic, including
/// ones the kernel deliberately catches around truck's booleans (which report
/// `BooleanFailed`). Targets that reach the kernel call this from `init` to
/// put back a hook that only prints. A panic that escapes the target is still
/// a crash: libfuzzer-sys catches it and aborts.
pub fn allow_caught_panics() {
    std::panic::set_hook(Box::new(|info| eprintln!("panic: {info}")));
}

/// Wraps `document` in an archive with a manifest that `sm_format::read`
/// accepts, so fuzzing effort goes into the document rather than the zip.
pub fn smp_with_document(document: &[u8]) -> Vec<u8> {
    let manifest = br#"{"format":"smp","version":1,"generator":"fuzz","document_hash":"0"}"#;
    let mut out = Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut out);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("manifest.json", options).unwrap();
    zip.write_all(manifest).unwrap();
    zip.start_file("document.json", options).unwrap();
    zip.write_all(document).unwrap();
    zip.finish().unwrap();
    out.into_inner()
}

/// A history the fuzzer can steer: every value is drawn from the input, but
/// the result always passes `validate()`, so rebuilds get past validation.
/// Sizes go up to the kernel's own limits, so this also searches for inputs
/// that are within every limit and still take too long or too much memory.
#[derive(Debug)]
pub struct FuzzDocument(pub PartDocument);

#[derive(Debug, Arbitrary)]
enum Step {
    Sketch {
        plane: u8,
        offset: i16,
        loops: Vec<Loop>,
    },
    /// A polygon with `points` vertices on a circle, which is how the fuzzer
    /// can reach large but valid sketches with few input bytes.
    RegularSketch {
        plane: u8,
        offset: i16,
        center: (i16, i16),
        radius: u16,
        points: u16,
        copies: u8,
    },
    Extrude {
        sketch: u16,
        distance: i16,
        symmetric: bool,
        operation: u8,
        repeat: u8,
    },
}

#[derive(Debug, Arbitrary)]
struct Loop(Vec<(i16, i16)>);

fn plane(p: u8) -> OriginPlane {
    [OriginPlane::XY, OriginPlane::YZ, OriginPlane::XZ][p as usize % 3]
}

/// Input coordinates are integers scaled to 1/16 mm, which keeps them finite
/// while still producing coincident and nearly coincident geometry.
fn coord(v: i16) -> f64 {
    v as f64 / 16.0
}

impl<'a> Arbitrary<'a> for FuzzDocument {
    fn arbitrary(u: &mut Unstructured<'a>) -> Result<Self> {
        let units = [
            LengthUnit::Mm,
            LengthUnit::Cm,
            LengthUnit::M,
            LengthUnit::In,
        ][u.int_in_range(0..=3)?];
        let mut doc = PartDocument::new("Fuzz", units);
        for i in 0..u.int_in_range(0..=3)? {
            doc.parameters.push(Parameter {
                name: format!("p{i}"),
                value: coord(u.arbitrary()?),
                comment: String::new(),
            });
        }
        let mut sketches: Vec<FeatureId> = Vec::new();
        let steps: Vec<Step> = u.arbitrary()?;
        for step in steps {
            match step {
                Step::Sketch {
                    plane: p,
                    offset,
                    loops,
                } => {
                    let profiles = loops
                        .into_iter()
                        .map(|l| Profile {
                            points: l.0.into_iter().map(|(x, y)| [coord(x), coord(y)]).collect(),
                        })
                        .collect();
                    sketches.push(add_sketch(&mut doc, p, offset, profiles));
                }
                Step::RegularSketch {
                    plane: p,
                    offset,
                    center: (cx, cy),
                    radius,
                    points,
                    copies,
                } => {
                    let n = (points as usize % 2_000).max(3);
                    let r = (radius as f64 / 16.0).max(0.1);
                    let profiles = (0..copies.max(1) as usize % 16)
                        .map(|c| Profile {
                            points: (0..n)
                                .map(|k| {
                                    let a = std::f64::consts::TAU * k as f64 / n as f64;
                                    let s = 1.0 + c as f64;
                                    [coord(cx) + s * r * a.cos(), coord(cy) + s * r * a.sin()]
                                })
                                .collect(),
                        })
                        .collect();
                    sketches.push(add_sketch(&mut doc, p, offset, profiles));
                }
                Step::Extrude {
                    sketch,
                    distance,
                    symmetric,
                    operation,
                    repeat,
                } if !sketches.is_empty() => {
                    let sketch = sketches[sketch as usize % sketches.len()];
                    let distance = coord(distance);
                    let extent = if symmetric {
                        ExtrudeExtent::Symmetric { distance }
                    } else {
                        ExtrudeExtent::Distance { distance }
                    };
                    let operation = [
                        BodyOperation::Join,
                        BodyOperation::Cut,
                        BodyOperation::Intersect,
                        BodyOperation::NewBody,
                    ][operation as usize % 4];
                    // Repeats let a few input bytes ask for many features.
                    for _ in 0..=(repeat % 64) {
                        doc.add_feature(
                            "Extrusion",
                            FeatureKind::Extrude(Extrude {
                                sketch,
                                extent: extent.clone(),
                                operation,
                            }),
                        );
                    }
                }
                Step::Extrude { .. } => {}
            }
        }
        for f in doc.features.iter_mut() {
            f.suppressed = u.ratio(1, 16).unwrap_or(false);
        }
        if u.ratio(1, 8)? && !doc.features.is_empty() {
            let i = u.choose_index(doc.features.len())?;
            doc.end_of_part = Some(doc.features[i].id);
        }
        Ok(FuzzDocument(doc))
    }
}

fn add_sketch(doc: &mut PartDocument, p: u8, offset: i16, profiles: Vec<Profile>) -> FeatureId {
    doc.add_feature(
        "Sketch",
        FeatureKind::Sketch(Sketch {
            plane: plane(p),
            offset: coord(offset),
            profiles,
        }),
    )
}
