//! `smcli demo <out.smp>`              write a sample block part
//! `smcli rebuild <in.smp> [out.step]` replay the history, report bodies, optionally export STEP

use sm_document::{
    Extrude, ExtrudeExtent, FeatureKind, LengthUnit, OriginPlane, PartDocument, Profile, Sketch,
};
use std::fs::File;
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
    let mut doc = PartDocument::new("Block", LengthUnit::Mm);
    let sketch = doc.add_feature(
        "Sketch1",
        FeatureKind::Sketch(Sketch {
            plane: OriginPlane::XY,
            offset: 0.0,
            profiles: vec![Profile {
                points: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 20.0], [0.0, 20.0]],
            }],
        }),
    );
    doc.add_feature(
        "Extrusion1",
        FeatureKind::Extrude(Extrude {
            sketch,
            extent: ExtrudeExtent::Distance { distance: 10.0 },
        }),
    );
    let model = sm_kernel::rebuild(&doc)?;
    let step = sm_step::export(&model, &file_name(out));
    sm_format::write(File::create(out)?, &doc, Some(&step))?;
    println!(
        "wrote {out} ({} features, {} bodies)",
        doc.features.len(),
        model.bodies.len()
    );
    Ok(())
}

fn rebuild(input: &str, step_out: Option<&str>) -> Result<()> {
    let file = sm_format::read(File::open(input)?)?;
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
