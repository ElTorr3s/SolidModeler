//! `smcli demo <out.smp>`              write a sample part (block, pocket, notch)
//! `smcli rebuild <in.smp> [out.step]` replay the history, report bodies, optionally export STEP

use sm_document::{
    BodyOperation, Extrude, ExtrudeExtent, FeatureKind, LengthUnit, OriginPlane, PartDocument,
    Profile, Sketch,
};
use std::path::Path;
use std::process::ExitCode;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["demo", out] => demo(out),
        ["rebuild", input] => rebuild(input, None),
        ["rebuild", input, step] => rebuild(input, Some(step)),
        _ => {
            eprintln!("usage: smcli demo <out.smp> | smcli rebuild <in.smp> [out.step]");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn demo(out: &str) -> Result<()> {
    // A 40 x 20 x 10 mm block with a 5 mm deep pocket sketched on its top
    // face and a notch through one end.
    let mut doc = PartDocument::new("Block", LengthUnit::Mm);
    let extrude =
        |doc: &mut PartDocument, n: usize, rect: [f64; 4], z: f64, distance: f64, operation| {
            let [x0, y0, x1, y1] = rect;
            let sketch = doc.add_feature(
                format!("Sketch{n}"),
                FeatureKind::Sketch(Sketch {
                    plane: OriginPlane::XY,
                    offset: z,
                    profiles: vec![Profile {
                        points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
                    }],
                }),
            );
            doc.add_feature(
                format!("Extrusion{n}"),
                FeatureKind::Extrude(Extrude {
                    sketch,
                    extent: ExtrudeExtent::Distance { distance },
                    operation,
                }),
            );
        };
    extrude(
        &mut doc,
        1,
        [0.0, 0.0, 40.0, 20.0],
        0.0,
        10.0,
        BodyOperation::Join,
    );
    extrude(
        &mut doc,
        2,
        [5.0, 5.0, 25.0, 15.0],
        10.0,
        -5.0,
        BodyOperation::Cut,
    );
    extrude(
        &mut doc,
        3,
        [32.0, 0.0, 40.0, 20.0],
        6.0,
        4.0,
        BodyOperation::Cut,
    );
    let model = sm_kernel::rebuild(&doc)?;
    let step = sm_step::export(&model, &file_name(out));
    sm_format::save(Path::new(out), &doc, Some(&step))?;
    println!(
        "wrote {out} ({} features, {} bodies)",
        doc.features.len(),
        model.bodies.len()
    );
    Ok(())
}

fn rebuild(input: &str, step_out: Option<&str>) -> Result<()> {
    let file = sm_format::open(Path::new(input))?;
    let doc = &file.document;
    println!(
        "{} (format v{}, cache {})",
        doc.name,
        file.manifest.version,
        if file.cached_step.is_some() {
            "valid"
        } else {
            "missing or stale"
        }
    );
    for feature in &doc.features {
        println!(
            "  {:>3}  {}{}",
            feature.id.0,
            feature.name,
            if feature.suppressed {
                " (suppressed)"
            } else {
                ""
            }
        );
    }
    let model = sm_kernel::rebuild(doc)?;
    println!("rebuilt {} bodies", model.bodies.len());
    if let Some(path) = step_out {
        std::fs::write(path, sm_step::export(&model, &file_name(path)))?;
        println!("exported {path}");
    }
    Ok(())
}

/// STEP headers record a file name, not the local path it was written to.
fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}
