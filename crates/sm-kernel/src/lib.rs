//! Rebuilds geometry from a [`PartDocument`] by replaying its feature history.
//!
//! Each extrude currently produces its own body. Joining, cutting and
//! intersecting bodies needs the boolean operations planned for step 2.

use sm_document::{
    ExtrudeExtent, FeatureId, FeatureKind, OriginPlane, PartDocument, Profile, Sketch,
};
use std::collections::HashMap;
use thiserror::Error;
use truck_modeling::{builder, Point3, Solid, Vector3, Wire};

#[derive(Debug, Error)]
pub enum RebuildError {
    #[error(transparent)]
    Document(#[from] sm_document::DocumentError),
    #[error("feature {0:?}: sketch is suppressed or rolled back")]
    InactiveSketch(FeatureId),
    #[error("feature {0:?}: profile needs at least 3 points")]
    DegenerateProfile(FeatureId),
    #[error("feature {0:?}: extrude distance must be non-zero")]
    ZeroDistance(FeatureId),
    #[error("feature {0:?}: could not build a planar face from the profile")]
    InvalidFace(FeatureId),
}

/// The result of a rebuild: one solid per body, tagged with the feature
/// that created it.
#[derive(Debug, Default)]
pub struct Model {
    pub bodies: Vec<Body>,
}

#[derive(Debug)]
pub struct Body {
    pub feature: FeatureId,
    pub solid: Solid,
}

/// Replays the active features of `doc` in order.
pub fn rebuild(doc: &PartDocument) -> Result<Model, RebuildError> {
    doc.validate()?;
    let mut sketches: HashMap<FeatureId, &Sketch> = HashMap::new();
    let mut model = Model::default();
    for feature in doc.active_features() {
        match &feature.kind {
            FeatureKind::Sketch(sketch) => {
                sketches.insert(feature.id, sketch);
            }
            FeatureKind::Extrude(extrude) => {
                let sketch = sketches
                    .get(&extrude.sketch)
                    .ok_or(RebuildError::InactiveSketch(extrude.sketch))?;
                for profile in &sketch.profiles {
                    let solid = extrude_profile(feature.id, sketch, profile, &extrude.extent)?;
                    model.bodies.push(Body {
                        feature: feature.id,
                        solid,
                    });
                }
            }
        }
    }
    Ok(model)
}

/// Origin, x axis and y axis of a sketch plane in model space, plus its normal.
fn plane_frame(plane: OriginPlane, offset: f64) -> (Point3, Vector3, Vector3, Vector3) {
    // Matches Inventor's origin planes: XY normal +Z, YZ normal +X, XZ normal -Y
    // (so sketch x/y map to model X/Z with a right-handed frame).
    let (u, v, n) = match plane {
        OriginPlane::XY => (Vector3::unit_x(), Vector3::unit_y(), Vector3::unit_z()),
        OriginPlane::YZ => (Vector3::unit_y(), Vector3::unit_z(), Vector3::unit_x()),
        OriginPlane::XZ => (Vector3::unit_x(), Vector3::unit_z(), -Vector3::unit_y()),
    };
    (Point3::new(0.0, 0.0, 0.0) + n * offset, u, v, n)
}

fn extrude_profile(
    id: FeatureId,
    sketch: &Sketch,
    profile: &Profile,
    extent: &ExtrudeExtent,
) -> Result<Solid, RebuildError> {
    if profile.points.len() < 3 {
        return Err(RebuildError::DegenerateProfile(id));
    }
    let (origin, u, v, n) = plane_frame(sketch.plane, sketch.offset);
    let (start, length) = match *extent {
        ExtrudeExtent::Distance { distance } => (0.0, distance),
        ExtrudeExtent::Symmetric { distance } => (-distance / 2.0, distance),
    };
    if length == 0.0 {
        return Err(RebuildError::ZeroDistance(id));
    }

    // A negative distance sweeps backwards; start from the far side instead so
    // the sweep is always along +normal and the solid stays outward-oriented.
    let (start, length) = if length < 0.0 {
        (start + length, -length)
    } else {
        (start, length)
    };
    let base = origin + n * start;

    // truck builds faces with outward normal along the wire's winding, so
    // orient the loop counter-clockwise in sketch space before sweeping.
    let mut points = profile.points.clone();
    if signed_area(&points) < 0.0 {
        points.reverse();
    }
    let vertices: Vec<_> = points
        .iter()
        .map(|[x, y]| builder::vertex(base + u * *x + v * *y))
        .collect();
    let wire: Wire = (0..vertices.len())
        .map(|i| builder::line(&vertices[i], &vertices[(i + 1) % vertices.len()]))
        .collect();
    let face = builder::try_attach_plane(&[wire]).map_err(|_| RebuildError::InvalidFace(id))?;
    Ok(builder::tsweep(&face, n * length))
}

fn signed_area(points: &[[f64; 2]]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let [x0, y0] = points[i];
            let [x1, y1] = points[(i + 1) % n];
            x0 * y1 - x1 * y0
        })
        .sum::<f64>()
        / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use sm_document::{Extrude, LengthUnit};

    fn doc_with(points: Vec<[f64; 2]>, extent: ExtrudeExtent) -> PartDocument {
        let mut doc = PartDocument::new("Test", LengthUnit::Mm);
        let sketch = doc.add_feature(
            "Sketch1",
            FeatureKind::Sketch(Sketch {
                plane: OriginPlane::XY,
                offset: 0.0,
                profiles: vec![Profile { points }],
            }),
        );
        doc.add_feature(
            "Extrusion1",
            FeatureKind::Extrude(Extrude { sketch, extent }),
        );
        doc
    }

    fn rect() -> Vec<[f64; 2]> {
        vec![[0.0, 0.0], [40.0, 0.0], [40.0, 20.0], [0.0, 20.0]]
    }

    #[test]
    fn extrudes_rectangle_into_closed_box() {
        let model = rebuild(&doc_with(
            rect(),
            ExtrudeExtent::Distance { distance: 10.0 },
        ))
        .unwrap();
        assert_eq!(model.bodies.len(), 1);
        let shell = &model.bodies[0].solid.boundaries()[0];
        assert_eq!(shell.len(), 6);
        assert_eq!(
            shell.shell_condition(),
            truck_topology::shell::ShellCondition::Closed
        );
    }

    #[test]
    fn clockwise_and_negative_extrudes_still_build() {
        let mut cw = rect();
        cw.reverse();
        rebuild(&doc_with(cw, ExtrudeExtent::Distance { distance: -5.0 })).unwrap();
        rebuild(&doc_with(
            rect(),
            ExtrudeExtent::Symmetric { distance: 8.0 },
        ))
        .unwrap();
    }

    #[test]
    fn rollback_removes_body() {
        let mut doc = doc_with(rect(), ExtrudeExtent::Distance { distance: 10.0 });
        doc.end_of_part = Some(1);
        assert!(rebuild(&doc).unwrap().bodies.is_empty());
    }

    #[test]
    fn rejects_degenerate_input() {
        let bad = doc_with(
            vec![[0.0, 0.0], [1.0, 0.0]],
            ExtrudeExtent::Distance { distance: 1.0 },
        );
        assert!(matches!(
            rebuild(&bad),
            Err(RebuildError::DegenerateProfile(_))
        ));
        let zero = doc_with(rect(), ExtrudeExtent::Distance { distance: 0.0 });
        assert!(matches!(rebuild(&zero), Err(RebuildError::ZeroDistance(_))));
    }
}
