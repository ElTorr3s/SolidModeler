//! Rebuilds geometry from a [`PartDocument`] by replaying its feature history.
//!
//! Features combine their new volume with the part's bodies using
//! [`BodyOperation`], like Inventor's Join / Cut / Intersect / New Solid.

mod boolean;
pub mod curves;
mod extrude;
pub mod profile;

pub use extrude::Frame;
use profile::{ProfileError, Region};
use sm_document::{
    BodyOperation, ExtrudeExtent, FeatureId, FeatureKind, LengthUnit, OriginPlane, PartDocument,
    Sketch,
};
use std::collections::HashMap;
use thiserror::Error;
use truck_modeling::{Point3, Solid, Vector3};

/// Distances below this are treated as zero: repeated sketch points,
/// zero-length extrudes, flush faces.
pub const LINEAR_TOLERANCE_MM: f64 = 1e-6;

/// Accuracy of boolean operations. truck approximates intersection curves
/// to this, and flush faces are moved by four times this.
pub const BOOLEAN_TOLERANCE_MM: f64 = 0.01;

/// Tolerances in a part's own units, so a part in metres or inches behaves
/// exactly like the same part in millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    pub linear: f64,
    pub boolean: f64,
}

impl Tolerance {
    pub fn for_units(units: LengthUnit) -> Self {
        let mm = units.to_mm();
        Self {
            linear: LINEAR_TOLERANCE_MM / mm,
            boolean: BOOLEAN_TOLERANCE_MM / mm,
        }
    }

    /// How far flush tool faces are moved. Well above the boolean tolerance
    /// so truck sees two distinct planes.
    pub fn flush_offset(&self) -> f64 {
        4.0 * self.boolean
    }
}

/// Caps that keep a small hostile or corrupt file from tying up the machine.
/// Real parts are far below these.
pub const MAX_SKETCH_POINTS: usize = 10_000;
pub const MAX_BODIES: usize = 500;
/// Total extruded regions per rebuild, each of which costs a boolean.
pub const MAX_TOOLS: usize = 2_000;
/// Total faces across all bodies. Every face costs memory and meshing time
/// in every later feature; a detailed real part has a few thousand.
pub const MAX_FACES: usize = 200_000;
/// Work units charged per sketch point of each extruded region: building the
/// tool makes a side face per point, and classifying it meshes them all.
const REGION_POINT_COST: u64 = 10;
/// Total geometry work per rebuild. Meshing costs one unit per face; a truck
/// boolean costs the product of the two solids' face counts, since it
/// intersects every pair. Calibrated on a release build at roughly 55k units
/// a second, so the cap is about 35 s; a plate with a 12 x 12 grid of holes
/// uses about 300k (5 s), and 100 flush pockets about 190k (2 s).
pub const MAX_WORK: u64 = 2_000_000;

/// Tracks the work a rebuild has done, so a small hostile file can't run for
/// hours, and lets a caller (the UI) cancel a rebuild from another thread.
#[derive(Debug)]
pub struct Budget {
    used: std::cell::Cell<u64>,
    limit: u64,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl Default for Budget {
    fn default() -> Self {
        Self::with_limit(MAX_WORK)
    }
}

impl Budget {
    /// Work spent so far.
    pub fn used(&self) -> u64 {
        self.used.get()
    }

    /// A budget of `limit` work units instead of [`MAX_WORK`].
    pub fn with_limit(limit: u64) -> Self {
        Self {
            used: Default::default(),
            limit,
            cancel: None,
        }
    }

    /// A budget that also stops when `cancel` is set.
    pub fn with_cancel(cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        Self {
            cancel: Some(cancel),
            ..Self::default()
        }
    }

    pub(crate) fn spend(&self, id: FeatureId, work: u64) -> Result<(), RebuildError> {
        if self
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
        {
            return Err(RebuildError::Cancelled);
        }
        let used = self.used.get().saturating_add(work);
        self.used.set(used);
        if used > self.limit {
            return Err(RebuildError::TooMuchWork(id));
        }
        Ok(())
    }
}

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
    #[error("feature {0:?}: the operation leaves nothing of the part")]
    EmptyResult(FeatureId),
    #[error("feature {0:?}: part has more than {MAX_BODIES} bodies")]
    TooManyBodies(FeatureId),
    #[error("feature {0:?}: rebuild needs more than {MAX_TOOLS} extruded regions")]
    TooManyTools(FeatureId),
    #[error("feature {0:?}: part has more than {MAX_FACES} faces")]
    TooManyFaces(FeatureId),
    #[error("feature {0:?}: rebuild exceeds its work budget")]
    TooMuchWork(FeatureId),
    #[error("rebuild cancelled")]
    Cancelled,
}

/// The result of a rebuild: the part's solid bodies, tagged with the feature
/// that created each one.
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

/// Hooks for code that follows a rebuild step by step, such as topological
/// naming. Every method does nothing by default.
pub trait RebuildObserver {
    /// `feature` built a tool solid for one sketch region, before it is
    /// combined with the part.
    fn tool_built(&mut self, _feature: FeatureId, _tool: &ExtrudeTool<'_>) {}
    /// `feature` has combined all its tools with the part; `bodies` is the
    /// part as it now stands. Bodies the feature didn't touch are the same
    /// solids as before (their faces keep their truck ids).
    fn bodies_changed(&mut self, _feature: FeatureId, _bodies: &[Body]) {}
}

impl RebuildObserver for () {}

/// One extruded sketch region, as passed to [`RebuildObserver::tool_built`].
#[derive(Debug, Clone, Copy)]
pub struct ExtrudeTool<'a> {
    /// Index into the regions of the sketch, in [`profile::regions`] order.
    pub region_index: usize,
    pub region: &'a Region,
    pub frame: &'a Frame,
    /// Start height and positive length along `frame.normal`, before any
    /// flush-face adjustment.
    pub start: f64,
    pub length: f64,
    /// The part's tolerances, for matching tool geometry to the sketch.
    pub tol: Tolerance,
    /// The tool as built, including flush-face adjustments.
    pub solid: &'a Solid,
}

/// Replays the active features of `doc` in order.
pub fn rebuild(doc: &PartDocument) -> Result<Model, RebuildError> {
    rebuild_with(doc, &Budget::default())
}

/// [`rebuild`] with a caller-supplied budget, for example one that can be
/// cancelled from the UI thread.
pub fn rebuild_with(doc: &PartDocument, budget: &Budget) -> Result<Model, RebuildError> {
    rebuild_observed_with(doc, budget, &mut ())
}

/// [`rebuild`], reporting each tool and each feature's result to `observer`.
pub fn rebuild_observed(
    doc: &PartDocument,
    observer: &mut impl RebuildObserver,
) -> Result<Model, RebuildError> {
    rebuild_observed_with(doc, &Budget::default(), observer)
}

/// [`rebuild_observed`] with a caller-supplied budget.
pub fn rebuild_observed_with(
    doc: &PartDocument,
    budget: &Budget,
    observer: &mut impl RebuildObserver,
) -> Result<Model, RebuildError> {
    doc.validate()?;
    let tol = Tolerance::for_units(doc.units);
    let mut sketches: HashMap<FeatureId, &Sketch> = HashMap::new();
    let mut model = Model {
        units: doc.units,
        bodies: Vec::new(),
    };
    let mut tools = 0usize;
    let mut region_cache: HashMap<FeatureId, Vec<profile::Region>> = HashMap::new();
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
                // Several features often use one sketch; find its regions once.
                let regions = match region_cache.entry(extrude.sketch) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => e.insert(
                        profile::regions(&sketch.profiles, tol.linear)
                            .map_err(|err| RebuildError::Profile(extrude.sketch, err))?,
                    ),
                };
                tools += regions.len();
                if tools > MAX_TOOLS {
                    return Err(RebuildError::TooManyTools(id));
                }
                let (start, length) = span(id, &extrude.extent, tol)?;
                let frame = plane_frame(sketch.plane, sketch.offset);
                // Flush-face detection only needs the bodies as they were
                // before this feature: its regions don't touch each other.
                let planes: Vec<_> = match extrude.operation {
                    BodyOperation::Join | BodyOperation::Cut => model
                        .bodies
                        .iter()
                        .flat_map(|b| extrude::target_planes(&b.solid))
                        .collect(),
                    _ => Vec::new(),
                };
                let meshes = boolean::MeshCache::default();
                let cx = boolean::Ctx {
                    id,
                    tol,
                    budget,
                    meshes: &meshes,
                };
                let slab_ok =
                    |slab: &Solid| boolean::slab_ok(cx, &model.bodies, slab, extrude.operation);
                let mut solids = Vec::with_capacity(regions.len());
                for (region_index, region) in regions.iter().enumerate() {
                    let points: usize =
                        region.outer.len() + region.holes.iter().map(Vec::len).sum::<usize>();
                    budget.spend(id, (points as u64 + 2) * REGION_POINT_COST)?;
                    let solid = extrude::tool(
                        &frame,
                        region,
                        start,
                        length,
                        &planes,
                        extrude.operation,
                        tol,
                        &slab_ok,
                    )?
                    .ok_or(RebuildError::InvalidFace(id))?;
                    observer.tool_built(
                        id,
                        &ExtrudeTool {
                            region_index,
                            region,
                            frame: &frame,
                            start,
                            length,
                            tol,
                            solid: &solid,
                        },
                    );
                    solids.push(solid);
                }
                match extrude.operation {
                    BodyOperation::NewBody => {
                        for solid in solids {
                            boolean::add_body(&mut model, id, solid)?;
                        }
                    }
                    BodyOperation::Join => {
                        for solid in solids {
                            boolean::join(cx, &mut model, solid)?;
                        }
                    }
                    BodyOperation::Cut => boolean::cut(cx, &mut model, &solids)?,
                    BodyOperation::Intersect => boolean::intersect(cx, &mut model, &solids)?,
                }
                observer.bodies_changed(id, &model.bodies);
            }
        }
    }
    Ok(model)
}

/// Start height and positive length of an extrusion along the sketch normal.
fn span(id: FeatureId, extent: &ExtrudeExtent, tol: Tolerance) -> Result<(f64, f64), RebuildError> {
    let (start, length) = match *extent {
        ExtrudeExtent::Distance { distance } => (0.0, distance),
        ExtrudeExtent::Symmetric { distance } => (-distance.abs() / 2.0, distance.abs()),
    };
    if length.abs() <= tol.linear {
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

    /// Total volume of all bodies (hollows subtract), rounded to 1e-6.
    fn volume(model: &Model) -> f64 {
        use truck_meshalgo::prelude::*;
        let v: f64 = model
            .bodies
            .iter()
            .map(|b| b.solid.triangulation(0.001).to_polygon().volume())
            .sum();
        (v * 1e6).round() / 1e6
    }

    /// Axis-aligned bounds of the first body, rounded to 1e-6.
    fn bounds(model: &Model) -> [f64; 6] {
        body_bounds(&model.bodies[0])
    }

    fn body_bounds(body: &Body) -> [f64; 6] {
        let mut b = [f64::MAX, f64::MAX, f64::MAX, f64::MIN, f64::MIN, f64::MIN];
        for v in body.solid.boundaries()[0].vertex_iter() {
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
        // Block + boss - the 10 x 10 x 5 overlap.
        assert_eq!(volume(&model), 8000.0 + 2000.0 - 500.0);

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
        assert_eq!(volume(&model), 500.0);
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
    fn known_limitation_overhanging_flush_join() {
        // Two overlapping blocks of the same height: tops and bottoms are
        // coplanar, face the same way, and overhang each other, so no nudge
        // preserves the shape and truck's boolean fails. Flip this test when
        // the kernel handles it.
        let mut doc = PartDocument::new("T", LengthUnit::Mm);
        add_block(
            &mut doc,
            [0.0, 0.0, 10.0, 10.0],
            0.0,
            5.0,
            BodyOperation::Join,
        );
        add_block(
            &mut doc,
            [5.0, 5.0, 15.0, 15.0],
            0.0,
            5.0,
            BodyOperation::Join,
        );
        assert!(matches!(rebuild(&doc), Err(RebuildError::BooleanFailed(_))));
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
    fn cut_swallowing_a_body_removes_it() {
        let mut doc = base_block();
        let mut swallowed = doc.clone();
        add_block(
            &mut swallowed,
            [-5.0, -5.0, 45.0, 25.0],
            -5.0,
            20.0,
            BodyOperation::Cut,
        );
        // Removing the whole part is an error, as in Inventor.
        assert!(matches!(
            rebuild(&swallowed),
            Err(RebuildError::EmptyResult(_))
        ));
        // Removing one of two bodies is fine.
        add_block(
            &mut doc,
            [100.0, 0.0, 110.0, 10.0],
            0.0,
            10.0,
            BodyOperation::NewBody,
        );
        add_block(
            &mut doc,
            [-5.0, -5.0, 45.0, 25.0],
            -5.0,
            20.0,
            BodyOperation::Cut,
        );
        let model = rebuild_ok(&doc);
        assert_eq!(model.bodies.len(), 1);
        assert_eq!(volume(&model), 1000.0);
    }

    #[test]
    fn detailed_regions_are_charged_for_their_faces() {
        // Many New Solid extrusions of a 9,999-point circle: each builds a
        // 10k-face solid, so they must hit a limit quickly.
        let n = MAX_SKETCH_POINTS - 1;
        let circle: Vec<[f64; 2]> = (0..n)
            .map(|i| {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                [5.0 * a.cos(), 5.0 * a.sin()]
            })
            .collect();
        let mut doc = PartDocument::new("T", LengthUnit::Mm);
        let sketch = doc.add_feature(
            "S",
            FeatureKind::Sketch(Sketch {
                plane: OriginPlane::XY,
                offset: 0.0,
                profiles: vec![Profile { points: circle }],
            }),
        );
        for i in 0..500 {
            doc.add_feature(
                format!("E{i}"),
                FeatureKind::Extrude(Extrude {
                    sketch,
                    extent: ExtrudeExtent::Distance { distance: 1.0 },
                    operation: BodyOperation::NewBody,
                }),
            );
        }
        let started = std::time::Instant::now();
        assert!(matches!(
            rebuild(&doc),
            Err(RebuildError::TooMuchWork(_) | RebuildError::TooManyFaces(_))
        ));
        assert!(started.elapsed().as_secs() < 30, "{:?}", started.elapsed());
    }

    #[test]
    fn cut_that_misses_is_a_no_op() {
        let mut doc = base_block();
        add_block(
            &mut doc,
            [100.0, 0.0, 110.0, 10.0],
            0.0,
            10.0,
            BodyOperation::Cut,
        );
        let model = rebuild_ok(&doc);
        assert_eq!(model.bodies.len(), 1);
        assert_eq!(faces(&model), 6);
        assert_eq!(volume(&model), 8000.0);
    }

    #[test]
    fn enclosed_cut_leaves_a_hollow() {
        let mut doc = base_block();
        add_block(
            &mut doc,
            [10.0, 5.0, 30.0, 15.0],
            2.0,
            0.5,
            BodyOperation::Cut,
        );
        let model = rebuild_ok(&doc);
        let shells = model.bodies[0].solid.boundaries();
        assert_eq!(shells.len(), 2);
        assert_eq!(bounds(&model), [0.0, 0.0, 0.0, 40.0, 20.0, 10.0]);
        assert_eq!(volume(&model), 8000.0 - 100.0);
    }

    #[test]
    fn intersect_with_several_regions() {
        let mut doc = base_block();
        add_extrude(
            &mut doc,
            vec![rect([-5.0, 5.0, 10.0, 15.0]), rect([30.0, 5.0, 45.0, 15.0])],
            -5.0,
            20.0,
            BodyOperation::Intersect,
        );
        // Each region keeps its own piece of the block: two separate bodies.
        let model = rebuild_ok(&doc);
        assert_eq!(model.bodies.len(), 2);
        assert!(model.bodies.iter().all(|b| b.solid.boundaries().len() == 1));
        assert_eq!(volume(&model), 2.0 * 1000.0);

        let mut doc = base_block();
        add_block(
            &mut doc,
            [100.0, 0.0, 110.0, 10.0],
            0.0,
            10.0,
            BodyOperation::Intersect,
        );
        assert!(matches!(rebuild(&doc), Err(RebuildError::EmptyResult(_))));
    }

    #[test]
    fn join_and_cut_reach_every_body() {
        let mut doc = base_block();
        add_block(
            &mut doc,
            [100.0, 0.0, 120.0, 20.0],
            0.0,
            10.0,
            BodyOperation::NewBody,
        );
        // Join overlapping only the second body merges into it.
        add_block(
            &mut doc,
            [115.0, 5.0, 130.0, 15.0],
            5.0,
            10.0,
            BodyOperation::Join,
        );
        let model = rebuild_ok(&doc);
        assert_eq!(model.bodies.len(), 2);
        assert_eq!(model.bodies[0].solid.boundaries().len(), 1);
        assert_eq!(model.bodies[1].solid.boundaries().len(), 1);
        // A cut through only the second body leaves the first alone and
        // splits the second into two bodies.
        add_block(
            &mut doc,
            [105.0, -5.0, 110.0, 25.0],
            -5.0,
            20.0,
            BodyOperation::Cut,
        );
        let model = rebuild_ok(&doc);
        assert_eq!(model.bodies.len(), 3);
        assert_eq!(faces(&model), 6);
        assert_eq!(bounds(&model), [0.0, 0.0, 0.0, 40.0, 20.0, 10.0]);
        // Block + near piece + far piece + boss outside it.
        assert_eq!(volume(&model), 8000.0 + 1000.0 + 2000.0 + 1250.0);
        // A join bridging the block and the near piece merges those two.
        add_block(
            &mut doc,
            [30.0, 5.0, 104.0, 15.0],
            2.0,
            5.0,
            BodyOperation::Join,
        );
        let model = rebuild_ok(&doc);
        assert_eq!(model.bodies.len(), 2);
        assert!(model
            .bodies
            .iter()
            .any(|b| body_bounds(b) == [0.0, 0.0, 0.0, 105.0, 20.0, 10.0]));
        // As before, plus the bridge minus its overlaps with both ends.
        assert_eq!(
            volume(&model),
            12250.0 + 74.0 * 10.0 * 5.0 - 10.0 * 10.0 * 5.0 - 4.0 * 10.0 * 5.0
        );
    }

    #[test]
    fn join_that_touches_nothing_is_a_new_body() {
        let mut doc = base_block();
        add_block(
            &mut doc,
            [100.0, 0.0, 110.0, 10.0],
            0.0,
            10.0,
            BodyOperation::Join,
        );
        assert_eq!(rebuild_ok(&doc).bodies.len(), 2);
    }

    #[test]
    fn hollows_survive_later_features() {
        // Hollow, then a pocket from the top that reaches into the hollow,
        // then a join that fills part of the hollow.
        let mut doc = base_block();
        add_block(
            &mut doc,
            [10.0, 5.0, 30.0, 15.0],
            2.0,
            4.0,
            BodyOperation::Cut,
        );
        let model = rebuild_ok(&doc);
        assert_eq!(model.bodies[0].solid.boundaries().len(), 2);
        assert_eq!(volume(&model), 8000.0 - 800.0);

        let mut opened = doc.clone();
        add_block(
            &mut opened,
            [12.0, 7.0, 18.0, 13.0],
            10.0,
            -6.0,
            BodyOperation::Cut,
        );
        let model = rebuild_ok(&opened);
        assert_eq!(model.bodies.len(), 1);
        assert_eq!(model.bodies[0].solid.boundaries().len(), 1);
        assert_eq!(volume(&model), 8000.0 - 800.0 - 6.0 * 6.0 * 4.0);

        let mut filled = doc.clone();
        add_block(
            &mut filled,
            [5.0, 0.0, 20.0, 20.0],
            0.0,
            10.0,
            BodyOperation::Join,
        );
        let model = rebuild_ok(&filled);
        assert_eq!(model.bodies.len(), 1);
        assert_eq!(model.bodies[0].solid.boundaries().len(), 2);
        assert_eq!(volume(&model), 8000.0 - 400.0);

        // A second hollow overlapping the first merges with it. (Its side
        // faces stay off the first hollow's: truck can't merge coplanar
        // faces, a known limitation until the OCCT backend.)
        let mut merged = doc.clone();
        add_block(
            &mut merged,
            [25.0, 6.0, 35.0, 14.0],
            3.0,
            2.0,
            BodyOperation::Cut,
        );
        let model = rebuild_ok(&merged);
        assert_eq!(model.bodies[0].solid.boundaries().len(), 2);
        assert_eq!(volume(&model), 8000.0 - 800.0 - 5.0 * 8.0 * 2.0);
    }

    /// A 120 mm plate with a k x k grid of square cutouts from one sketch at
    /// `z`, cut `depth` along +Z.
    fn plate(k: usize, z: f64, depth: f64) -> PartDocument {
        let mut doc = PartDocument::new("P", LengthUnit::Mm);
        add_extrude(
            &mut doc,
            vec![rect([0.0, 0.0, 120.0, 120.0])],
            0.0,
            10.0,
            BodyOperation::Join,
        );
        let step = 120.0 / k as f64;
        let holes = (0..k * k)
            .map(|i| {
                let x = (i % k) as f64 * step + step / 4.0;
                let y = (i / k) as f64 * step + step / 4.0;
                rect([x, y, x + step / 2.0, y + step / 2.0])
            })
            .collect();
        add_extrude(&mut doc, holes, z, depth, BodyOperation::Cut);
        doc
    }

    #[test]
    fn ordinary_parts_fit_the_default_budget() {
        let side = |k: usize| 120.0 / k as f64 / 2.0;
        let holes = rebuild_ok(&plate(12, -1.0, 12.0));
        assert_eq!(holes.bodies.len(), 1);
        assert_eq!(volume(&holes), 144_000.0 - 144.0 * side(12).powi(2) * 10.0);
        let pockets = rebuild_ok(&plate(10, 10.0, -5.0));
        assert_eq!(volume(&pockets), 144_000.0 - 100.0 * side(10).powi(2) * 5.0);
    }

    #[test]
    fn work_budget_and_cancel() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        let cancel = Arc::new(AtomicBool::new(true));
        assert!(matches!(
            rebuild_with(&base_block(), &Budget::with_cancel(cancel)),
            Err(RebuildError::Cancelled)
        ));
        // Many cuts through a growing body stop at the budget.
        let mut doc = base_block();
        let holes: Vec<_> = (0..20)
            .map(|i| {
                let x = 1.0 + i as f64 * 1.9;
                rect([x, 2.0, x + 1.0, 3.0])
            })
            .collect();
        add_extrude(&mut doc, holes, -1.0, 12.0, BodyOperation::Cut);
        assert!(matches!(
            rebuild_with(&doc, &Budget::with_limit(2_000)),
            Err(RebuildError::TooMuchWork(_))
        ));
        // The default budget is enough for the same part.
        assert_eq!(volume(&rebuild_ok(&doc)), 8000.0 - 20.0 * 10.0);
    }

    #[test]
    fn same_part_in_every_unit() {
        for units in [
            LengthUnit::Mm,
            LengthUnit::Cm,
            LengthUnit::M,
            LengthUnit::In,
        ] {
            let k = 1.0 / units.to_mm();
            let mut doc = PartDocument::new("T", units);
            add_block(
                &mut doc,
                [0.0, 0.0, 40.0 * k, 20.0 * k],
                0.0,
                10.0 * k,
                BodyOperation::Join,
            );
            add_block(
                &mut doc,
                [10.0 * k, 5.0 * k, 30.0 * k, 15.0 * k],
                10.0 * k,
                -5.0 * k,
                BodyOperation::Cut,
            );
            let model = rebuild_ok(&doc);
            assert_eq!(faces(&model), 11, "{units:?}");
        }
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

    #[derive(Default)]
    struct Recorder {
        events: Vec<String>,
        last_bodies: Vec<usize>,
    }

    impl RebuildObserver for Recorder {
        fn tool_built(&mut self, feature: FeatureId, tool: &ExtrudeTool<'_>) {
            assert!(tool.length > 0.0);
            let faces = tool.solid.boundaries()[0].len();
            self.events.push(format!(
                "tool {} r{} f{faces}",
                feature.0, tool.region_index
            ));
        }
        fn bodies_changed(&mut self, feature: FeatureId, bodies: &[Body]) {
            self.events
                .push(format!("bodies {} n{}", feature.0, bodies.len()));
            self.last_bodies = bodies
                .iter()
                .map(|b| b.solid.boundaries().iter().map(|s| s.len()).sum())
                .collect();
        }
    }

    #[test]
    fn observer_sees_every_tool_and_feature_result() {
        let mut doc = base_block();
        add_extrude(
            &mut doc,
            vec![rect([10.0, 5.0, 15.0, 15.0]), rect([25.0, 5.0, 30.0, 15.0])],
            5.0,
            10.0,
            BodyOperation::Cut,
        );
        add_block(
            &mut doc,
            [100.0, 0.0, 110.0, 10.0],
            0.0,
            10.0,
            BodyOperation::NewBody,
        );
        let mut rec = Recorder::default();
        let model = rebuild_observed(&doc, &mut rec).unwrap();
        assert_eq!(
            rec.events,
            [
                "tool 2 r0 f6",
                "bodies 2 n1",
                "tool 4 r0 f6",
                "tool 4 r1 f6",
                "bodies 4 n1",
                "tool 6 r0 f6",
                "bodies 6 n2",
            ]
        );
        // The last report is the model as returned.
        let faces: Vec<usize> = model
            .bodies
            .iter()
            .map(|b| b.solid.boundaries().iter().map(|s| s.len()).sum())
            .collect();
        assert_eq!(rec.last_bodies, faces);
        // The unobserved rebuild gives the same model.
        let plain = rebuild(&doc).unwrap();
        assert_eq!(plain.bodies.len(), model.bodies.len());
        assert_eq!(volume(&plain), volume(&model));
    }

    #[test]
    fn observer_sees_nothing_after_a_failed_feature() {
        let mut doc = base_block();
        add_block(&mut doc, [0.0, 0.0, 1.0, 1.0], 0.0, 0.0, BodyOperation::Cut);
        let mut rec = Recorder::default();
        assert!(rebuild_observed(&doc, &mut rec).is_err());
        assert_eq!(rec.events, ["tool 2 r0 f6", "bodies 2 n1"]);
    }
}
