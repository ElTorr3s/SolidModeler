//! STEP is the exchange format: geometry only, no feature history.
//! The editable history lives in `.smp` files (see `sm-format`).

use sm_kernel::Model;
use truck_modeling::{builder, Point3, Vector3};
use truck_stepio::out::{CompleteStepDisplay, StepHeaderDescriptor, StepModels};

/// The schema matching the entities truck writes (`automotive_design`,
/// i.e. AP214). AP242 output is on the roadmap.
const FILE_SCHEMA: &str = "AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }";

/// Writes every body of `model` to a STEP Part 21 string, in millimetres.
pub fn export(model: &Model, file_name: &str) -> String {
    // truck always declares millimetres, so scale other units to match.
    let scale = model.units.to_mm();
    let compressed: Vec<_> = model
        .bodies
        .iter()
        .map(|b| {
            // truck-stepio miswrites intersection curves; see sm_kernel::curves.
            let solid = sm_kernel::curves::without_intersection_curves(&b.solid);
            if scale == 1.0 {
                solid.compress()
            } else {
                builder::scaled(
                    &solid,
                    Point3::new(0.0, 0.0, 0.0),
                    Vector3::new(scale, scale, scale),
                )
                .compress()
            }
        })
        .collect();
    let models: StepModels<_, _, _> = compressed.iter().collect();
    let truck_output =
        CompleteStepDisplay::new(models, StepHeaderDescriptor::default()).to_string();

    // Replace truck's header: it doesn't escape strings, writes a
    // non-ISO-8601 time stamp, and names a schema readers don't recognise.
    let data = truck_output
        .find("DATA;")
        .map_or(truck_output.as_str(), |i| &truck_output[i..]);
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    format!(
        "ISO-10303-21;\nHEADER;\n\
         FILE_DESCRIPTION(('SolidModeller part'), '2;1');\n\
         FILE_NAME('{}', '{timestamp}', (''), (''), 'truck', '{}', '');\n\
         FILE_SCHEMA(('{FILE_SCHEMA}'));\nENDSEC;\n{data}",
        step_string(file_name),
        step_string(concat!("SolidModeller ", env!("CARGO_PKG_VERSION"))),
    )
}

/// Encodes text for a Part 21 string literal: quotes and backslashes are
/// doubled, and anything outside printable ASCII uses the `\X2\` escape.
fn step_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\'' => out.push_str("''"),
            '\\' => out.push_str("\\\\"),
            ' '..='~' => out.push(c),
            _ => {
                out.push_str("\\X2\\");
                for unit in c.encode_utf16(&mut [0; 2]) {
                    out.push_str(&format!("{unit:04X}"));
                }
                out.push_str("\\X0\\");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sm_document::{
        BodyOperation, Extrude, ExtrudeExtent, FeatureKind, LengthUnit, OriginPlane, PartDocument,
        Profile, Sketch,
    };
    use truck_stepio::r#in::Table;

    fn block(units: LengthUnit) -> Model {
        let mut doc = PartDocument::new("Block", units);
        let sketch = doc.add_feature(
            "Sketch1",
            FeatureKind::Sketch(Sketch {
                plane: OriginPlane::XY,
                offset: 0.0,
                profiles: vec![Profile {
                    points: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]],
                }],
            }),
        );
        doc.add_feature(
            "Extrusion1",
            FeatureKind::Extrude(Extrude {
                sketch,
                extent: ExtrudeExtent::Distance { distance: 1.0 },
                operation: BodyOperation::Join,
            }),
        );
        sm_kernel::rebuild(&doc).unwrap()
    }

    /// Reads `step` back with truck and returns (faces, max corner).
    fn read_back(step: &str) -> (usize, [f64; 3]) {
        let table = Table::from_step(step).expect("valid Part 21");
        assert_eq!(table.shell.len(), 1);
        let shell = table
            .to_compressed_shell(table.shell.values().next().unwrap())
            .unwrap();
        let mut max = [f64::MIN; 3];
        for p in &shell.vertices {
            for k in 0..3 {
                max[k] = max[k].max(p[k]);
            }
        }
        (shell.faces.len(), max.map(|x| (x * 1e6).round() / 1e6))
    }

    /// Every `#n = ...` defined once and every `#n` referenced is defined.
    fn assert_well_formed(step: &str) {
        let data = &step[step.find("DATA;").unwrap()..];
        let mut defined = std::collections::HashSet::new();
        let mut referenced = Vec::new();
        for line in data.lines() {
            let mut rest = line;
            if let Some(def) = line.strip_prefix('#') {
                let (id, tail) = def.split_once(" =").expect("entity line");
                assert!(defined.insert(id.to_string()), "#{id} defined twice");
                rest = tail;
            }
            for part in rest.split('#').skip(1) {
                let id: String = part.chars().take_while(char::is_ascii_digit).collect();
                referenced.push(id);
            }
        }
        for id in referenced {
            assert!(defined.contains(&id), "#{id} referenced but not defined");
        }
    }

    #[test]
    fn boolean_results_export_well_formed() {
        let mut model = block(LengthUnit::Mm);
        let mut doc = PartDocument::new("Pocket", LengthUnit::Mm);
        let rect = |[x0, y0, x1, y1]: [f64; 4]| Profile {
            points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
        };
        for (r, z, d, op) in [
            ([0.0, 0.0, 4.0, 2.0], 0.0, 1.0, BodyOperation::Join),
            ([1.0, 0.5, 3.0, 1.5], 1.0, -0.5, BodyOperation::Cut),
            ([3.5, 0.0, 4.0, 2.0], 0.5, 0.5, BodyOperation::Cut),
        ] {
            let sketch = doc.add_feature(
                "S",
                FeatureKind::Sketch(Sketch {
                    plane: OriginPlane::XY,
                    offset: z,
                    profiles: vec![rect(r)],
                }),
            );
            doc.add_feature(
                "E",
                FeatureKind::Extrude(Extrude {
                    sketch,
                    extent: ExtrudeExtent::Distance { distance: d },
                    operation: op,
                }),
            );
        }
        model.bodies = sm_kernel::rebuild(&doc).unwrap().bodies;
        let step = export(&model, "pocket.step");
        assert_well_formed(&step);
        assert!(!step.contains("INTERSECTION_CURVE"));
        assert_eq!(read_back(&step), (13, [4.0, 2.0, 1.0]));
    }

    #[test]
    fn exports_millimetres_as_is() {
        let step = export(&block(LengthUnit::Mm), "block.step");
        assert_well_formed(&step);
        assert_eq!(read_back(&step), (6, [4.0, 2.0, 1.0]));
    }

    #[test]
    fn exports_inches_scaled_to_millimetres() {
        let step = export(&block(LengthUnit::In), "block.step");
        assert_eq!(read_back(&step), (6, [101.6, 50.8, 25.4]));
    }

    #[test]
    fn header_is_escaped_and_standard() {
        let step = export(&block(LengthUnit::Mm), "Bob's part ø.step");
        assert!(step.contains("FILE_NAME('Bob''s part \\X2\\00F8\\X0\\.step', '20"));
        assert!(step.contains("FILE_SCHEMA(('AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }'));"));
        assert!(step.is_ascii());
    }
}
