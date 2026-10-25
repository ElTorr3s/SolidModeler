//! The sample part shown when the app starts without a file: the same block
//! with a pocket and a notch that `smcli demo` writes.

use sm_document::{
    BodyOperation, Extrude, ExtrudeExtent, FeatureKind, LengthUnit, OriginPlane, PartDocument,
    Profile, Sketch,
};

pub fn document() -> PartDocument {
    let mut doc = PartDocument::new("Block", LengthUnit::Mm);
    let mut extrude = |n: usize, [x0, y0, x1, y1]: [f64; 4], z: f64, distance, operation| {
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
    extrude(1, [0.0, 0.0, 40.0, 20.0], 0.0, 10.0, BodyOperation::Join);
    extrude(2, [5.0, 5.0, 25.0, 15.0], 10.0, -5.0, BodyOperation::Cut);
    extrude(3, [32.0, 0.0, 40.0, 20.0], 6.0, 4.0, BodyOperation::Cut);
    doc
}
