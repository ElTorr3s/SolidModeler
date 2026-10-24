//! Rebuilds geometry from a [`PartDocument`] by replaying its feature history.
//!
//! Features combine their new volume with the part body using
//! [`BodyOperation`], like Inventor's Join / Cut / Intersect / New Solid.

mod extrude;
pub mod profile;

use extrude::Frame;
use profile::ProfileError;
use sm_document::{
    BodyOperation, ExtrudeExtent, FeatureId, FeatureKind, LengthUnit, OriginPlane, PartDocument,
    Sketch,
};
use std::collections::HashMap;
use thiserror::Error;
use truck_modeling::{Point3, Solid, Vector3};

/// Distances below this, in model units, are treated as zero: repeated sketch
/// points, zero-length extrudes, flush faces.
pub const LINEAR_TOLERANCE: f64 = 1e-6;

/// Geometric tolerance for boolean operations, in model units. truck
/// approximates intersection curves to this accuracy.
pub const BOOLEAN_TOLERANCE: f64 = 0.01;

/// Caps that keep a small hostile or corrupt file from tying up the machine.
/// Real parts are far below these.
pub const MAX_SKETCH_POINTS: usize = 10_000;
pub const MAX_BODIES: usize = 1_000;
/// Total extruded regions per rebuild, each of which costs a boolean.
pub const MAX_TOOLS: usize = 10_000;

#[derive(Debug, Error)]
pub enum RebuildError {
    #[error(transparent)]
    Document(#[from] sm_document::DocumentError),
    #[error("feature {0:?}: sketch is suppressed or rolled back")]
    InactiveSketch(FeatureId),
    #[error("feature {0:?}: {1}")]
    Profile(FeatureId, ProfileError),
    #[error("feature {0:?}: sketch has more than {MAX_SKETCH_POINTS} points")]
    SketchTooLarge(FeatureId),
    #[error("feature {0:?}: extrude distance is zero")]
    ZeroDistance(FeatureId),
    #[error("feature {0:?}: could not build a planar face from the profile")]
    InvalidFace(FeatureId),
    #[error("feature {0:?}: {1:?} needs an existing body")]
    NoTargetBody(FeatureId, BodyOperation),
    #[error("feature {0:?}: boolean operation failed")]
    BooleanFailed(FeatureId),
    #[error("feature {0:?}: part has more than {MAX_BODIES} bodies")]
    TooManyBodies(FeatureId),
    #[error("feature {0:?}: rebuild needs more than {MAX_TOOLS} extruded regions")]
    TooManyTools(FeatureId),
}

/// The result of a rebuild: the part's solid bodies, tagged with the feature
/// that created each one. Join, Cut and Intersect act on the first body.
#[derive(Debug)]
pub struct Model {
    /// Unit of every coordinate in `bodies`.
    pub units: LengthUnit,
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
    let mut model = Model {
        units: doc.units,
        bodies: Vec::new(),
    };
    let mut tools = 0usize;
    for feature in doc.active_features() {
        let id = feature.id;
        match &feature.kind {
            FeatureKind::Sketch(sketch) => {
                let points: usize = sketch.profiles.iter().map(|p| p.points.len()).sum();
                if points > MAX_SKETCH_POINTS {
                    return Err(RebuildError::SketchTooLarge(id));
                }
                sketches.insert(id, sketch);
            }
            FeatureKind::Extrude(extrude) => {
                let sketch = sketches
                    .get(&extrude.sketch)
                    .ok_or(RebuildError::InactiveSketch(extrude.sketch))?;
                let regions = profile::regions(&sketch.profiles)
                    .map_err(|e| RebuildError::Profile(extrude.sketch, e))?;
                tools += regions.len();
                if tools > MAX_TOOLS {
                    return Err(RebuildError::TooManyTools(id));
                }
                let (start, length) = span(id, &extrude.extent)?;
                let frame = plane_frame(sketch.plane, sketch.offset);
                for region in &regions {
                    let target = match extrude.operation {
                        BodyOperation::NewBody => None,
                        op => model.bodies.first().map(|b| (&b.solid, op)),
                    };
                    let tool = extrude::tool(&frame, region, start, length, target)
                        .ok_or(RebuildError::InvalidFace(id))?;
                    apply(&mut model, id, extrude.operation, tool)?;
                }
            }
        }
    }
    Ok(model)
}

/// Start height and positive length of an extrusion along the sketch normal.
fn span(id: FeatureId, extent: &ExtrudeExtent) -> Result<(f64, f64), RebuildError> {
    let (start, length) = match *extent {
        ExtrudeExtent::Distance { distance } => (0.0, distance),
        ExtrudeExtent::Symmetric { distance } => (-distance.abs() / 2.0, distance.abs()),
    };
    if length.abs() <= LINEAR_TOLERANCE {
        return Err(RebuildError::ZeroDistance(id));
    }
    // A negative distance sweeps backwards; start from the far side instead so
    // the sweep is always along +normal and the solid stays outward-oriented.
    Ok(if length < 0.0 {
        (start + length, -length)
    } else {
        (start, length)
    })
}

/// Combines `tool` with the part body according to `op`.
fn apply(
    model: &mut Model,
    id: FeatureId,
    op: BodyOperation,
    mut tool: Solid,
) -> Result<(), RebuildError> {
    let target = match (op, model.bodies.first_mut()) {
        (BodyOperation::NewBody, _) | (BodyOperation::Join, None) => {
            if model.bodies.len() >= MAX_BODIES {
                return Err(RebuildError::TooManyBodies(id));
            }
            model.bodies.push(Body {
                feature: id,
                solid: tool,
            });
            return Ok(());
        }
        (_, None) => return Err(RebuildError::NoTargetBody(id, op)),
        (_, Some(body)) => body,
    };
    // truck asserts on some degenerate results instead of returning None, so
    // treat a panic inside the boolean as a failed feature, not a crash.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match op {
        BodyOperation::Join => truck_shapeops::or(&target.solid, &tool, BOOLEAN_TOLERANCE),
        BodyOperation::Intersect => truck_shapeops::and(&target.solid, &tool, BOOLEAN_TOLERANCE),
        BodyOperation::Cut => {
            // A - B is A ∩ (complement of B); inverting B's faces gives the complement.
            tool.not();
            truck_shapeops::and(&target.solid, &tool, BOOLEAN_TOLERANCE)
        }
        BodyOperation::NewBody => unreachable!(),
    }));
    let solid = result
        .ok()
        .flatten()
        .ok_or(RebuildError::BooleanFailed(id))?;
    if solid.boundaries().is_empty() {
        // Cut or Intersect consumed the whole body.
        model.bodies.remove(0);
    } else {
        target.solid = solid;
    }
    Ok(())
}

/// A sketch plane in model space.
fn plane_frame(plane: OriginPlane, offset: f64) -> Frame {
    // Matches Inventor's origin planes: XY normal +Z, YZ normal +X, XZ normal -Y
    // (so sketch x/y map to model X/Z with a right-handed frame).
    let (u, v, normal) = match plane {
        OriginPlane::XY => (Vector3::unit_x(), Vector3::unit_y(), Vector3::unit_z()),
        OriginPlane::YZ => (Vector3::unit_y(), Vector3::unit_z(), Vector3::unit_x()),
        OriginPlane::XZ => (Vector3::unit_x(), Vector3::unit_z(), -Vector3::unit_y()),
    };
    Frame {
        origin: Point3::new(0.0, 0.0, 0.0) + normal * offset,
        u,
        v,
        normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sm_document::{Extrude, Profile};
    use truck_topology::shell::ShellCondition;

    fn rect([x0, y0, x1, y1]: [f64; 4]) -> Profile {
        Profile {
            points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
        }
    }

    /// Appends a sketch with the given loops on XY at `z`, extruded by `height`.
    fn add_extrude(
        doc: &mut PartDocument,
        profiles: Vec<Profile>,
        z: f64,
        height: f64,
        operation: BodyOperation,
    ) {
        let sketch = doc.add_feature(
            "Sketch",
            FeatureKind::Sketch(Sketch {
                plane: OriginPlane::XY,
                offset: z,
                profiles,
            }),
        );
        doc.add_feature(
            "Extrusion",
            FeatureKind::Extrude(Extrude {
                sketch,
                extent: ExtrudeExtent::Distance { distance: height },
                operation,
            }),
        );
    }

    fn add_block(doc: &mut PartDocument, r: [f64; 4], z: f64, h: f64, op: BodyOperation) {
        add_extrude(doc, vec![rect(r)], z, h, op);
    }

    /// A 40 x 20 x 10 block at the origin.
    fn base_block() -> PartDocument {
        let mut doc = PartDocument::new("Test", LengthUnit::Mm);
        add_block(
            &mut doc,
            [0.0, 0.0, 40.0, 20.0],
            0.0,
            10.0,
            BodyOperation::Join,
        );
        doc
    }

    fn faces(model: &Model) -> usize {
        model.bodies[0]
            .solid
            .boundaries()
            .iter()
            .map(|s| s.len())
            .sum()
    }

    fn closed(model: &Model) -> bool {
        model.bodies.iter().all(|b| {
            b.solid
                .boundaries()
                .iter()
                .all(|s| s.shell_condition() == ShellCondition::Closed)
        })
    }

    /// Axis-aligned bounds of the first body, rounded to 1e-6.
    fn bounds(model: &Model) -> [f64; 6] {
        let mut b = [f64::MAX, f64::MAX, f64::MAX, f64::MIN, f64::MIN, f64::MIN];
        for v in model.bodies[0].solid.boundaries()[0].vertex_iter() {
            let p = v.point();
            for k in 0..3 {
                b[k] = b[k].min(p[k]);
                b[k + 3] = b[k + 3].max(p[k]);
            }
        }
        b.map(|x| (x * 1e6).round() / 1e6)
    }

    fn rebuild_ok(doc: &PartDocument) -> Model {
        let model = rebuild(doc).unwrap();
        assert!(closed(&model));
        model
    }

    #[test]
    fn extrudes_rectangle_into_closed_box() {
        let model = rebuild_ok(&base_block());
        assert_eq!(model.bodies.len(), 1);
        assert_eq!(faces(&model), 6);
        assert_eq!(bounds(&model), [0.0, 0.0, 0.0, 40.0, 20.0, 10.0]);
    }

    #[test]
    fn clockwise_negative_and_symmetric_extrudes() {
        let mut doc = PartDocument::new("T", LengthUnit::Mm);
        let mut cw = rect([0.0, 0.0, 4.0, 2.0]);
        cw.points.reverse();
        add_extrude(&mut doc, vec![cw], 0.0, -5.0, BodyOperation::Join);
        assert_eq!(bounds(&rebuild_ok(&doc)), [0.0, 0.0, -5.0, 4.0, 2.0, 0.0]);

        let mut doc = PartDocument::new("T", LengthUnit::Mm);
        let sketch = doc.add_feature(
            "S",
            FeatureKind::Sketch(Sketch {
                plane: OriginPlane::XY,
                offset: 0.0,
                profiles: vec![rect([0.0, 0.0, 4.0, 2.0])],
            }),
        );
        doc.add_feature(
            "E",
            FeatureKind::Extrude(Extrude {
                sketch,
                extent: ExtrudeExtent::Symmetric { distance: 8.0 },
                operation: BodyOperation::Join,
            }),
        );
        assert_eq!(bounds(&rebuild_ok(&doc)), [0.0, 0.0, -4.0, 4.0, 2.0, 4.0]);
    }

    #[test]
    fn rollback_and_suppression() {
        let mut doc = base_block();
        doc.end_of_part = Some(doc.features[1].id);
        assert!(rebuild(&doc).unwrap().bodies.is_empty());

        // Suppressing the sketch but not the extrude is reported, not ignored.
        let mut doc = base_block();
        doc.features[0].suppressed = true;
        assert!(matches!(
            rebuild(&doc),
            Err(RebuildError::InactiveSketch(_))
        ));
    }

    #[test]
    fn cut_makes_pocket() {
        let mut doc = base_block();
        // Pocket from inside the block up through the top face.
        add_block(
            &mut doc,
            [10.0, 5.0, 30.0, 15.0],
            5.0,
            10.0,
            BodyOperation::Cut,
        );
        // 6 outer faces (top now has a hole) plus 4 pocket walls and a floor.
        assert_eq!(faces(&rebuild_ok(&doc)), 11);
    }

    #[test]
    fn cut_through_makes_hole() {
        let mut doc = base_block();
        add_block(
            &mut doc,
            [10.0, 5.0, 30.0, 15.0],
            -5.0,
            20.0,
            BodyOperation::Cut,
        );
        assert_eq!(faces(&rebuild_ok(&doc)), 10);
    }

    #[test]
    fn join_and_intersect_overlapping_block() {
        let mut doc = base_block();
        add_block(
            &mut doc,
            [30.0, 5.0, 50.0, 15.0],
            5.0,
            10.0,
            BodyOperation::Join,
        );
        let model = rebuild_ok(&doc);
        assert_eq!(model.bodies.len(), 1);
        assert_eq!(bounds(&model), [0.0, 0.0, 0.0, 50.0, 20.0, 15.0]);

        let mut doc = base_block();
        add_block(
            &mut doc,
            [30.0, 5.0, 50.0, 15.0],
            5.0,
            10.0,
            BodyOperation::Intersect,
        );
        let model = rebuild_ok(&doc);
        assert_eq!(faces(&model), 6);
        assert_eq!(bounds(&model), [30.0, 5.0, 5.0, 40.0, 15.0, 10.0]);
    }

    // The flush cases below are what truck's booleans can't do on their own.

    #[test]
    fn flush_pocket_from_top_face() {
        let mut doc = base_block();
        // Sketch on the top face, cut 5 down.
        add_block(
            &mut doc,
            [10.0, 5.0, 30.0, 15.0],
            10.0,
            -5.0,
            BodyOperation::Cut,
        );
        let model = rebuild_ok(&doc);
        assert_eq!(faces(&model), 11);
        assert_eq!(bounds(&model), [0.0, 0.0, 0.0, 40.0, 20.0, 10.0]);
    }

    #[test]
    fn flush_boss_on_top_face() {
        let mut doc = base_block();
        add_block(
            &mut doc,
            [10.0, 5.0, 30.0, 15.0],
            10.0,
            5.0,
            BodyOperation::Join,
        );
        let model = rebuild_ok(&doc);
        assert_eq!(model.bodies.len(), 1);
        assert_eq!(faces(&model), 11);
        assert_eq!(bounds(&model), [0.0, 0.0, 0.0, 40.0, 20.0, 15.0]);
    }

    #[test]
    fn flush_notch_through_side() {
        let mut doc = base_block();
        // Flush with the end and both sides, through the full height.
        add_block(
            &mut doc,
            [30.0, 0.0, 40.0, 20.0],
            0.0,
            10.0,
            BodyOperation::Cut,
        );
        let model = rebuild_ok(&doc);
        assert_eq!(faces(&model), 6);
        assert_eq!(bounds(&model), [0.0, 0.0, 0.0, 30.0, 20.0, 10.0]);
    }

    #[test]
    fn flush_step_cut_from_corner() {
        let mut doc = base_block();
        add_block(
            &mut doc,
            [30.0, 10.0, 40.0, 20.0],
            5.0,
            5.0,
            BodyOperation::Cut,
        );
        let model = rebuild_ok(&doc);
        assert_eq!(faces(&model), 9);
        assert_eq!(bounds(&model), [0.0, 0.0, 0.0, 40.0, 20.0, 10.0]);
    }

    #[test]
    fn nested_loops_extrude_as_frame() {
        let mut doc = PartDocument::new("Frame", LengthUnit::Mm);
        add_extrude(
            &mut doc,
            vec![rect([0.0, 0.0, 10.0, 10.0]), rect([2.0, 2.0, 8.0, 8.0])],
            0.0,
            1.0,
            BodyOperation::Join,
        );
        let model = rebuild_ok(&doc);
        assert_eq!(model.bodies.len(), 1);
        assert_eq!(faces(&model), 10);
    }

    #[test]
    fn new_body_and_missing_target() {
        let mut doc = base_block();
        add_block(
            &mut doc,
            [100.0, 0.0, 110.0, 10.0],
            0.0,
            10.0,
            BodyOperation::NewBody,
        );
        assert_eq!(rebuild_ok(&doc).bodies.len(), 2);

        let mut empty = PartDocument::new("Empty", LengthUnit::Mm);
        add_block(
            &mut empty,
            [0.0, 0.0, 1.0, 1.0],
            0.0,
            1.0,
            BodyOperation::Cut,
        );
        assert!(matches!(
            rebuild(&empty),
            Err(RebuildError::NoTargetBody(_, BodyOperation::Cut))
        ));
    }

    #[test]
    fn cut_removing_everything_fails_cleanly() {
        // truck panics when a cut consumes the whole body; we report it as a
        // failed feature. Inventor reports this case as an error too.
        let mut doc = base_block();
        add_block(
            &mut doc,
            [-5.0, -5.0, 45.0, 25.0],
            -5.0,
            20.0,
            BodyOperation::Cut,
        );
        assert!(matches!(rebuild(&doc), Err(RebuildError::BooleanFailed(_))));
    }

    #[test]
    fn rejects_degenerate_input() {
        let mut doc = PartDocument::new("T", LengthUnit::Mm);
        add_extrude(
            &mut doc,
            vec![Profile {
                points: vec![[0.0, 0.0], [1.0, 0.0]],
            }],
            0.0,
            1.0,
            BodyOperation::Join,
        );
        assert!(matches!(
            rebuild(&doc),
            Err(RebuildError::Profile(_, ProfileError::TooFewPoints(0)))
        ));
        for distance in [0.0, 1e-9] {
            let mut doc = PartDocument::new("T", LengthUnit::Mm);
            add_block(
                &mut doc,
                [0.0, 0.0, 1.0, 1.0],
                0.0,
                distance,
                BodyOperation::Join,
            );
            assert!(matches!(rebuild(&doc), Err(RebuildError::ZeroDistance(_))));
        }
    }

    #[test]
    fn caps_runaway_documents() {
        let mut doc = PartDocument::new("T", LengthUnit::Mm);
        let points = (0..=MAX_SKETCH_POINTS)
            .map(|i| {
                let a = i as f64 / MAX_SKETCH_POINTS as f64 * std::f64::consts::TAU;
                [a.cos(), a.sin()]
            })
            .collect();
        add_extrude(
            &mut doc,
            vec![Profile { points }],
            0.0,
            1.0,
            BodyOperation::Join,
        );
        assert!(matches!(
            rebuild(&doc),
            Err(RebuildError::SketchTooLarge(_))
        ));

        // One small sketch, extruded many times: caught before any geometry work.
        let mut doc = PartDocument::new("T", LengthUnit::Mm);
        let profiles: Vec<_> = (0..100)
            .map(|i| rect([i as f64 * 2.0, 0.0, i as f64 * 2.0 + 1.0, 1.0]))
            .collect();
        add_extrude(&mut doc, profiles, 0.0, 1.0, BodyOperation::NewBody);
        let sketch = doc.features[0].id;
        for _ in 0..MAX_TOOLS / 100 {
            doc.add_feature(
                "E",
                FeatureKind::Extrude(Extrude {
                    sketch,
                    extent: ExtrudeExtent::Distance { distance: 1.0 },
                    operation: BodyOperation::NewBody,
                }),
            );
        }
        assert!(matches!(
            rebuild(&doc),
            Err(RebuildError::TooManyBodies(_) | RebuildError::TooManyTools(_))
        ));
    }
}
