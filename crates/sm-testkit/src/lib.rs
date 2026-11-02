//! Test support shared by tests, benchmarks and fuzz targets. Not part of the
//! product: nothing outside `dev-dependencies` should depend on it.

use sm_document::{
    BodyOperation, Extrude, ExtrudeExtent, FeatureId, FeatureKind, LengthUnit, OriginPlane,
    PartDocument, Profile, Sketch,
};

#[cfg(feature = "proptest")]
pub mod strategies;

/// An axis-aligned rectangle `[x0, y0, x1, y1]` as a counter-clockwise loop.
pub fn rect([x0, y0, x1, y1]: [f64; 4]) -> Profile {
    Profile {
        points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
    }
}

/// Appends a sketch on XY at height `z` and an extrude of it, returning the
/// sketch's id.
pub fn add_extrude(
    doc: &mut PartDocument,
    profiles: Vec<Profile>,
    z: f64,
    distance: f64,
    operation: BodyOperation,
) -> FeatureId {
    let n = doc.features.len() / 2 + 1;
    let sketch = doc.add_feature(
        format!("Sketch{n}"),
        FeatureKind::Sketch(Sketch {
            plane: OriginPlane::XY,
            offset: z,
            profiles,
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
    sketch
}

/// A realistic history of `pockets + 1` extrudes: a plate with a row of
/// `pockets` blind pockets cut into its top, alternating with a boss joined
/// on every fourth one. Every feature is a boolean against a body that grows
/// with the history, which is what makes long histories expensive.
pub fn pocketed_plate(pockets: usize) -> PartDocument {
    let mut doc = PartDocument::new(format!("Plate{pockets}"), LengthUnit::Mm);
    let length = 10.0 * pockets.max(1) as f64;
    add_extrude(
        &mut doc,
        vec![rect([0.0, 0.0, length, 20.0])],
        0.0,
        10.0,
        BodyOperation::Join,
    );
    for i in 0..pockets {
        let x = 10.0 * i as f64;
        if i % 4 == 3 {
            add_extrude(
                &mut doc,
                vec![rect([x + 2.0, 12.0, x + 8.0, 18.0])],
                10.0,
                3.0,
                BodyOperation::Join,
            );
        } else {
            add_extrude(
                &mut doc,
                vec![rect([x + 2.0, 2.0, x + 8.0, 8.0])],
                10.0,
                -4.0,
                BodyOperation::Cut,
            );
        }
    }
    doc
}

/// A document with `features` sketch/extrude pairs whose sketches each hold
/// one loop of `points_per_loop` points. Cheap to rebuild as `NewBody`
/// extrudes; used to measure validation, serialization and loading, which
/// scale with document size rather than geometry.
pub fn large_document(features: usize, points_per_loop: usize) -> PartDocument {
    let mut doc = PartDocument::new("Large", LengthUnit::Mm);
    for i in 0..features {
        let n = points_per_loop.max(3);
        let points = (0..n)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / n as f64;
                [10.0 * i as f64 + a.cos(), a.sin()]
            })
            .collect();
        add_extrude(
            &mut doc,
            vec![Profile { points }],
            0.0,
            1.0,
            BodyOperation::NewBody,
        );
    }
    doc
}
