//! proptest strategies for part documents.

use crate::rect;
use proptest::prelude::*;
use sm_document::{
    BodyOperation, Extrude, ExtrudeExtent, FeatureId, FeatureKind, LengthUnit, OriginPlane,
    Parameter, PartDocument, Profile, Sketch,
};

pub fn units() -> impl Strategy<Value = LengthUnit> {
    prop_oneof![
        Just(LengthUnit::Mm),
        Just(LengthUnit::Cm),
        Just(LengthUnit::M),
        Just(LengthUnit::In),
    ]
}

pub fn plane() -> impl Strategy<Value = OriginPlane> {
    prop_oneof![
        Just(OriginPlane::XY),
        Just(OriginPlane::YZ),
        Just(OriginPlane::XZ),
    ]
}

pub fn operation() -> impl Strategy<Value = BodyOperation> {
    prop_oneof![
        Just(BodyOperation::Join),
        Just(BodyOperation::Cut),
        Just(BodyOperation::Intersect),
        Just(BodyOperation::NewBody),
    ]
}

pub fn extent() -> impl Strategy<Value = ExtrudeExtent> {
    prop_oneof![
        (-50.0..50.0f64).prop_map(|distance| ExtrudeExtent::Distance { distance }),
        (-50.0..50.0f64).prop_map(|distance| ExtrudeExtent::Symmetric { distance }),
    ]
}

/// Any closed polyline, including degenerate and self-crossing ones.
pub fn profile() -> impl Strategy<Value = Profile> + Clone {
    prop::collection::vec([-100.0..100.0f64, -100.0..100.0f64], 0..12)
        .prop_map(|points| Profile { points })
}

/// An axis-aligned rectangle of at least 1 x 1, so rebuilds mostly succeed.
pub fn rectangle() -> impl Strategy<Value = Profile> + Clone {
    (-50.0..50.0f64, -50.0..50.0f64, 1.0..40.0f64, 1.0..40.0f64)
        .prop_map(|(x, y, w, h)| rect([x, y, x + w, y + h]))
}

fn parameters() -> impl Strategy<Value = Vec<Parameter>> {
    prop::collection::btree_map("[a-z_][a-z0-9_]{0,8}", -1e6..1e6f64, 0..4).prop_map(|m| {
        m.into_iter()
            .map(|(name, value)| Parameter {
                name,
                value,
                comment: String::new(),
            })
            .collect()
    })
}

/// One step of a generated history: a sketch, or an extrude of the sketch
/// `index % sketches so far` (skipped if there is none yet).
#[derive(Debug, Clone)]
enum Step {
    Sketch(OriginPlane, f64, Vec<Profile>),
    Extrude(usize, ExtrudeExtent, BodyOperation),
}

fn build(
    base: bool,
    units: LengthUnit,
    parameters: Vec<Parameter>,
    steps: Vec<Step>,
    suppressed: Vec<bool>,
    end: Option<usize>,
) -> PartDocument {
    let mut doc = PartDocument::new("Generated", units);
    doc.parameters = parameters;
    let mut sketches: Vec<FeatureId> = Vec::new();
    if base {
        sketches.push(crate::add_extrude(
            &mut doc,
            vec![rect([-60.0, -60.0, 60.0, 60.0])],
            -10.0,
            20.0,
            BodyOperation::Join,
        ));
    }
    for step in steps {
        match step {
            Step::Sketch(plane, offset, profiles) => {
                let name = format!("Sketch{}", sketches.len() + 1);
                sketches.push(doc.add_feature(
                    name,
                    FeatureKind::Sketch(Sketch {
                        plane,
                        offset,
                        profiles,
                    }),
                ));
            }
            Step::Extrude(index, extent, operation) if !sketches.is_empty() => {
                let sketch = sketches[index % sketches.len()];
                doc.add_feature(
                    "Extrusion",
                    FeatureKind::Extrude(Extrude {
                        sketch,
                        extent,
                        operation,
                    }),
                );
            }
            Step::Extrude(..) => {}
        }
    }
    for (feature, s) in doc.features.iter_mut().zip(suppressed) {
        feature.suppressed = s;
    }
    if let Some(i) = end {
        doc.end_of_part = doc
            .features
            .get(i % doc.features.len().max(1))
            .map(|f| f.id);
    }
    doc
}

/// Generated histories. With `base`, the history starts with a 120 x 120 x 20
/// block centred on the origin, so later Cut / Intersect / Join features have
/// a body to act on.
fn document_with(
    base: bool,
    profile: impl Strategy<Value = Profile> + Clone + 'static,
    max_steps: usize,
) -> impl Strategy<Value = PartDocument> {
    let step = prop_oneof![
        (
            plane(),
            -20.0..20.0f64,
            prop::collection::vec(profile, 0..3)
        )
            .prop_map(|(p, o, l)| Step::Sketch(p, o, l)),
        (any::<usize>(), extent(), operation()).prop_map(|(i, e, o)| Step::Extrude(i, e, o)),
    ];
    (
        units(),
        parameters(),
        prop::collection::vec(step, 0..max_steps),
        prop::collection::vec(prop::bool::weighted(0.1), 0..max_steps),
        prop::option::weighted(0.2, any::<usize>()),
    )
        .prop_map(move |(u, p, s, sup, end)| build(base, u, p, s, sup, end))
}

/// Documents that pass `validate()`, with arbitrary (often unbuildable)
/// sketch geometry. Good for format and validation properties.
pub fn document() -> impl Strategy<Value = PartDocument> {
    document_with(false, profile(), 12)
}

/// Valid documents that start with a block and whose sketches are
/// rectangles, so most features build.
/// Good for kernel properties.
pub fn buildable_document() -> impl Strategy<Value = PartDocument> {
    document_with(true, rectangle(), 8)
}
