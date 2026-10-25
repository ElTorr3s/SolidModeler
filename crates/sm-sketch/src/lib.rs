//! 2D sketches: points, lines, circles and arcs held in place by geometric
//! and dimensional constraints, with an Inventor-style solver.
//!
//! The behaviour follows Inventor's sketch environment:
//!
//! - Every sketch starts with a grounded origin point, like Inventor's
//!   projected center point. Geometry is fully constrained once nothing can
//!   move relative to it.
//! - [`Sketch::add_constraint`] solves straight away and refuses a
//!   constraint that would over-constrain the sketch, naming the constraints
//!   it conflicts with. A dimension that is refused can be placed as a
//!   driven (reference) dimension with [`Sketch::add_driven_dimension`].
//! - Solving moves the geometry as little as possible, so unconstrained
//!   parts of a sketch stay where they were drawn.
//! - [`Sketch::analyse`] reports the remaining degrees of freedom and which
//!   points and curves can still move, which is what Inventor uses to color
//!   fully constrained geometry.
//!
//! Lengths are in the part's model unit, angles in radians.

mod equations;
mod profile;
mod solver;
mod sparse;

use equations::{EqKind, Equation, Owner};
pub use profile::{ProfileError, ProfileLoop, ProfileSegment, JOIN_TOLERANCE};
use serde::{Deserialize, Serialize};
use solver::{ClusterOutcome, SolveSettings};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::FRAC_PI_2;
use thiserror::Error;
use uuid::Uuid;

/// Identifies a point. Ids are random time-ordered UUIDs (v7): unique
/// across sketches, branches and machines, so they never need renumbering
/// when branches merge and stay valid as names across edits, solves, saves
/// and reloads. The origin is the nil UUID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PointId(pub Uuid);

/// Identifies a curve within its sketch; stable like [`PointId`]. Features
/// name the faces a curve sweeps out after this id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CurveId(pub Uuid);

/// Identifies a constraint; unique and stable like [`PointId`]. Newer
/// constraints sort after older ones, which decides which constraint is
/// reported as redundant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConstraintId(pub Uuid);

/// A point or a curve, for constraints that accept either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Entity {
    Point(PointId),
    Curve(CurveId),
}

/// A curve as the caller sees it. Arcs run counter-clockwise from `start`
/// to `end`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Curve {
    Line {
        start: PointId,
        end: PointId,
    },
    Circle {
        center: PointId,
        radius: f64,
    },
    Arc {
        center: PointId,
        start: PointId,
        end: PointId,
        radius: f64,
    },
}

/// Geometric constraints and dimensions. Dimension values are lengths in
/// model units or angles in radians.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Constraint {
    /// Two points in the same place.
    Coincident(PointId, PointId),
    /// A point on a line (extended), circle or arc (extended to its circle).
    PointOnCurve(PointId, CurveId),
    /// A line parallel to the sketch X axis.
    Horizontal(CurveId),
    /// A line parallel to the sketch Y axis.
    Vertical(CurveId),
    /// Two points at the same height.
    HorizontalAlign(PointId, PointId),
    /// Two points one above the other.
    VerticalAlign(PointId, PointId),
    Parallel(CurveId, CurveId),
    Perpendicular(CurveId, CurveId),
    /// Two lines on the same infinite line.
    Collinear(CurveId, CurveId),
    /// A line and a circle or arc, or two circles or arcs. Which side or
    /// whether the tangency is internal follows the current geometry.
    Tangent(CurveId, CurveId),
    /// Two lines of equal length, or two circles or arcs of equal radius.
    Equal(CurveId, CurveId),
    /// Two circles or arcs sharing a center.
    Concentric(CurveId, CurveId),
    /// A point at the middle of a line.
    Midpoint(PointId, CurveId),
    /// Two points mirrored about a line.
    Symmetric(PointId, PointId, CurveId),
    /// Pins a point or curve where it is now.
    Fix(Entity),
    /// Straight-line distance between two points.
    Distance(PointId, PointId, f64),
    /// Distance between two points along X.
    HorizontalDistance(PointId, PointId, f64),
    /// Distance between two points along Y.
    VerticalDistance(PointId, PointId, f64),
    /// Perpendicular distance from a point to a line.
    PointLineDistance(PointId, CurveId, f64),
    /// Angle between two lines, in `[0, π]`.
    Angle(CurveId, CurveId, f64),
    Radius(CurveId, f64),
    Diameter(CurveId, f64),
}

impl Constraint {
    /// The dimension value, if this is a dimension.
    pub fn value(&self) -> Option<f64> {
        use Constraint::*;
        match *self {
            Distance(.., v)
            | HorizontalDistance(.., v)
            | VerticalDistance(.., v)
            | PointLineDistance(.., v)
            | Angle(.., v)
            | Radius(_, v)
            | Diameter(_, v) => Some(v),
            _ => None,
        }
    }

    fn set_value(&mut self, value: f64) {
        use Constraint::*;
        match self {
            Distance(.., v)
            | HorizontalDistance(.., v)
            | VerticalDistance(.., v)
            | PointLineDistance(.., v)
            | Angle(.., v)
            | Radius(_, v)
            | Diameter(_, v) => *v = value,
            _ => {}
        }
    }

    pub fn is_dimension(&self) -> bool {
        self.value().is_some()
    }

    /// The points and curves this constraint refers to.
    fn references(&self) -> (Vec<PointId>, Vec<CurveId>) {
        use Constraint::*;
        match *self {
            Coincident(a, b)
            | HorizontalAlign(a, b)
            | VerticalAlign(a, b)
            | Distance(a, b, _)
            | HorizontalDistance(a, b, _)
            | VerticalDistance(a, b, _) => (vec![a, b], vec![]),
            PointOnCurve(p, c) | Midpoint(p, c) | PointLineDistance(p, c, _) => (vec![p], vec![c]),
            Horizontal(c) | Vertical(c) | Radius(c, _) | Diameter(c, _) => (vec![], vec![c]),
            Parallel(a, b)
            | Perpendicular(a, b)
            | Collinear(a, b)
            | Tangent(a, b)
            | Equal(a, b)
            | Concentric(a, b)
            | Angle(a, b, _) => (vec![], vec![a, b]),
            Symmetric(p, q, c) => (vec![p, q], vec![c]),
            Fix(Entity::Point(p)) => (vec![p], vec![]),
            Fix(Entity::Curve(c)) => (vec![], vec![c]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum SketchError {
    #[error("unknown point {0:?}")]
    UnknownPoint(PointId),
    #[error("unknown curve {0:?}")]
    UnknownCurve(CurveId),
    #[error("unknown constraint {0:?}")]
    UnknownConstraint(ConstraintId),
    #[error("{0}")]
    WrongGeometry(&'static str),
    #[error("invalid dimension value {0}")]
    InvalidValue(f64),
    #[error("{0:?} is not a dimension")]
    NotADimension(ConstraintId),
    /// Adding the constraint would over-constrain the sketch. Dimensions
    /// can be placed as driven dimensions instead.
    #[error("over-constrained: conflicts with {conflicts_with:?}")]
    OverConstrained { conflicts_with: Vec<ConstraintId> },
    /// The constraints can't all hold at once, for example a triangle with
    /// one side longer than the other two together.
    #[error("the sketch cannot be solved with this change")]
    Unsolvable,
    /// The constraint has no well-defined direction at the current
    /// geometry, such as an angle on a zero-length line.
    #[error("the geometry is degenerate for this constraint")]
    DegenerateGeometry,
    /// The sketch is over the size or work limits.
    #[error("the sketch is too large")]
    TooLarge,
    /// Loaded sketch data is inconsistent.
    #[error("invalid sketch data: {0}")]
    Corrupt(&'static str),
}

#[derive(Debug, Clone)]
struct PointData {
    /// Index of x in the variable vector; y follows it.
    var: usize,
}

#[derive(Debug, Clone, Copy)]
enum CurveGeom {
    Line {
        start: PointId,
        end: PointId,
    },
    Circle {
        center: PointId,
        radius: usize,
    },
    Arc {
        center: PointId,
        start: PointId,
        end: PointId,
        radius: usize,
    },
}

impl CurveGeom {
    fn points(&self) -> Vec<PointId> {
        match *self {
            CurveGeom::Line { start, end } => vec![start, end],
            CurveGeom::Circle { center, .. } => vec![center],
            CurveGeom::Arc {
                center, start, end, ..
            } => vec![center, start, end],
        }
    }
}

#[derive(Debug, Clone)]
struct CurveData {
    geom: CurveGeom,
    construction: bool,
}

/// Orientation choices recorded when a constraint is placed, so solving
/// keeps the geometry on the side the user drew it. Saved with the
/// constraint. `Fixed` holds the pinned values in the order x, y of each
/// point (center, start, end for arcs), then the radius.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Captured {
    None,
    Sign(f64),
    Tangency { s1: f64, s2: f64 },
    Fixed(Vec<f64>),
}

#[derive(Debug, Clone)]
struct ConstraintEntry {
    constraint: Constraint,
    /// `false` for a driven (reference) dimension, which only measures.
    driving: bool,
    captured: Captured,
}

/// What a delete took with it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Removed {
    pub points: Vec<PointId>,
    pub curves: Vec<CurveId>,
    pub constraints: Vec<ConstraintId>,
}

/// How a solve went.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolveReport {
    pub converged: bool,
    /// Most iterations any one cluster of geometry needed.
    pub iterations: usize,
    /// Largest constraint error left, in model units.
    pub max_residual: f64,
    /// Some cluster of geometry was too large to solve within the work
    /// budget and was left as it was.
    pub too_large: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintStatus {
    /// Some geometry can still move.
    UnderConstrained,
    FullyConstrained,
    /// Some constraints repeat or contradict others.
    OverConstrained,
}

/// A constraint that adds nothing new, because the constraints before it
/// already decide what it controls.
#[derive(Debug, Clone, PartialEq)]
pub struct Redundancy {
    pub constraint: ConstraintId,
    pub conflicts_with: Vec<ConstraintId>,
    /// `false` when it also contradicts them rather than just repeating.
    pub satisfied: bool,
}

/// Degrees-of-freedom report for a sketch at its current geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    /// Independent ways the geometry can still move, as in Inventor's
    /// "Show degrees of freedom".
    pub dof: usize,
    pub status: ConstraintStatus,
    pub redundant: Vec<Redundancy>,
    /// Driving constraints that don't currently hold.
    pub unsatisfied: Vec<ConstraintId>,
    /// Arcs whose endpoints are currently off their own circle, which
    /// happens only before a solve converges.
    pub inconsistent_curves: Vec<CurveId>,
    /// Points that can still move.
    pub free_points: Vec<PointId>,
    /// Curves that can still move or change size.
    pub free_curves: Vec<CurveId>,
    /// Some cluster of geometry was too large to analyse. Its points and
    /// curves are listed as free and its variables are left out of `dof`.
    pub too_large: bool,
}

impl Analysis {
    pub fn is_fully_constrained(&self, entity: Entity) -> bool {
        match entity {
            Entity::Point(p) => !self.free_points.contains(&p),
            Entity::Curve(c) => !self.free_curves.contains(&c),
        }
    }
}

/// A 2D sketch on a plane, in the plane's own coordinates.
///
/// Serializes as one record per point, curve and constraint, keyed by id,
/// never as solver state. Loading checks every record (unique ids, existing
/// references, finite numbers, size limits, constraints valid for their
/// geometry with the orientation data they need), so a damaged or
/// hand-edited file is rejected instead of panicking later.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(into = "SketchFile", try_from = "SketchFile")]
pub struct Sketch {
    /// Solver variables: point coordinates and radii. Rebuilt on load and
    /// after deletes, so indices are an in-memory detail.
    vars: Vec<f64>,
    /// Variables the solver may not change, such as the origin's.
    grounded: Vec<bool>,
    points: BTreeMap<PointId, PointData>,
    curves: BTreeMap<CurveId, CurveData>,
    constraints: BTreeMap<ConstraintId, ConstraintEntry>,
}

/// The saved form of a sketch: the user's geometry and constraints keyed by
/// stable ids, one record per entity so text diffs and merges line up.
/// Solver variables are rebuilt on load. The origin is implied.
///
/// Loading checks every record (unique ids, existing
/// references, finite numbers, constraints valid for their geometry with the
/// orientation data they need), so a damaged or hand-edited file is
/// rejected instead of panicking later.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SketchFile {
    #[serde(default)]
    points: Vec<PointRecord>,
    #[serde(default)]
    curves: Vec<CurveRecord>,
    #[serde(default)]
    constraints: Vec<ConstraintRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PointRecord {
    id: PointId,
    x: f64,
    y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CurveRecord {
    id: CurveId,
    geom: CurveRecordGeom,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    construction: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CurveRecordGeom {
    Line {
        start: PointId,
        end: PointId,
    },
    Circle {
        center: PointId,
        radius: f64,
    },
    Arc {
        center: PointId,
        start: PointId,
        end: PointId,
        radius: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConstraintRecord {
    id: ConstraintId,
    constraint: Constraint,
    driving: bool,
    captured: Captured,
}

impl From<Sketch> for SketchFile {
    fn from(s: Sketch) -> Self {
        let points = s
            .points
            .keys()
            .filter(|&&p| p != s.origin())
            .map(|&id| {
                let [x, y] = s.xy(id);
                PointRecord { id, x, y }
            })
            .collect();
        let curves = s
            .curves
            .iter()
            .map(|(&id, c)| CurveRecord {
                id,
                geom: match c.geom {
                    CurveGeom::Line { start, end } => CurveRecordGeom::Line { start, end },
                    CurveGeom::Circle { center, radius } => CurveRecordGeom::Circle {
                        center,
                        radius: s.vars[radius],
                    },
                    CurveGeom::Arc {
                        center,
                        start,
                        end,
                        radius,
                    } => CurveRecordGeom::Arc {
                        center,
                        start,
                        end,
                        radius: s.vars[radius],
                    },
                },
                construction: c.construction,
            })
            .collect();
        let constraints = s
            .constraints
            .iter()
            .map(|(&id, e)| ConstraintRecord {
                id,
                constraint: e.constraint,
                driving: e.driving,
                captured: e.captured.clone(),
            })
            .collect();
        SketchFile {
            points,
            curves,
            constraints,
        }
    }
}

impl TryFrom<SketchFile> for Sketch {
    type Error = SketchError;

    fn try_from(f: SketchFile) -> Result<Self, SketchError> {
        use SketchError::Corrupt;
        // The origin isn't in the file but counts towards MAX_POINTS.
        if f.points.len() >= MAX_POINTS
            || f.curves.len() > MAX_CURVES
            || f.constraints.len() > MAX_CONSTRAINTS
        {
            return Err(SketchError::TooLarge);
        }
        let mut s = Sketch::new();
        let finite = |v: f64| {
            if v.is_finite() {
                Ok(v)
            } else {
                Err(Corrupt("non-finite number"))
            }
        };

        for r in &f.points {
            if r.id == s.origin() {
                return Err(Corrupt("a point record reuses the origin's id"));
            }
            let var = s.new_var(finite(r.x)?);
            s.new_var(finite(r.y)?);
            if s.points.insert(r.id, PointData { var }).is_some() {
                return Err(Corrupt("duplicate point id"));
            }
        }

        for r in &f.curves {
            let geom = match r.geom {
                CurveRecordGeom::Line { start, end } => CurveGeom::Line { start, end },
                CurveRecordGeom::Circle { center, radius } => CurveGeom::Circle {
                    center,
                    radius: s.new_var(finite(radius)?),
                },
                CurveRecordGeom::Arc {
                    center,
                    start,
                    end,
                    radius,
                } => CurveGeom::Arc {
                    center,
                    start,
                    end,
                    radius: s.new_var(finite(radius)?),
                },
            };
            let pts = geom.points();
            if pts.iter().any(|p| !s.points.contains_key(p)) {
                return Err(Corrupt("curve refers to a missing point"));
            }
            if (1..pts.len()).any(|i| pts[..i].contains(&pts[i])) {
                return Err(Corrupt("curve uses the same point twice"));
            }
            let data = CurveData {
                geom,
                construction: r.construction,
            };
            if s.curves.insert(r.id, data).is_some() {
                return Err(Corrupt("duplicate curve id"));
            }
        }

        for r in &f.constraints {
            // A driven dimension's value is only a measurement and may be
            // zero, so check its geometry alone.
            let geometry = if r.driving {
                r.constraint
            } else {
                Self::placeholder_value(r.constraint)
            };
            s.validate(&geometry)
                .map_err(|_| Corrupt("constraint invalid for its geometry"))?;
            if !r.driving && !r.constraint.is_dimension() {
                return Err(Corrupt("only dimensions can be driven"));
            }
            let expected = if r.driving {
                s.capture(&r.constraint)
            } else {
                Captured::None
            };
            let unit = |v: f64| v == 1.0 || v == -1.0;
            let matches = match (&r.captured, &expected) {
                (Captured::None, Captured::None) => true,
                (Captured::Sign(v), Captured::Sign(_)) => unit(*v),
                (Captured::Tangency { s1, s2 }, Captured::Tangency { .. }) => {
                    unit(*s1) && unit(*s2) && (*s1 > 0.0 || *s2 > 0.0)
                }
                (Captured::Fixed(a), Captured::Fixed(b)) => {
                    a.len() == b.len() && a.iter().all(|v| v.is_finite())
                }
                _ => false,
            };
            if !matches {
                return Err(Corrupt(
                    "constraint orientation data doesn't match its kind",
                ));
            }
            let entry = ConstraintEntry {
                constraint: r.constraint,
                driving: r.driving,
                captured: r.captured.clone(),
            };
            if s.constraints.insert(r.id, entry).is_some() {
                return Err(Corrupt("duplicate constraint id"));
            }
        }

        Ok(s)
    }
}

impl Default for Sketch {
    fn default() -> Self {
        Self::new()
    }
}

/// Most points a sketch may hold, origin included. With the curve and
/// constraint limits this bounds the memory and time a loaded file can
/// demand; per-cluster work budgets in the solver bound the rest.
pub const MAX_POINTS: usize = 20_000;
pub const MAX_CURVES: usize = 20_000;
pub const MAX_CONSTRAINTS: usize = 40_000;

/// Stiffness of a dragged point relative to other geometry, so the rest of
/// the sketch gives way to the drag where the constraints allow.
const DRAG_WEIGHT: f64 = 1e4;

impl Sketch {
    /// An empty sketch with a grounded origin point at `(0, 0)`.
    pub fn new() -> Self {
        let mut sketch = Sketch {
            vars: Vec::new(),
            grounded: Vec::new(),
            points: BTreeMap::new(),
            curves: BTreeMap::new(),
            constraints: BTreeMap::new(),
        };
        let var = sketch.new_var(0.0);
        sketch.new_var(0.0);
        sketch.grounded[var] = true;
        sketch.grounded[var + 1] = true;
        sketch
            .points
            .insert(PointId(Uuid::nil()), PointData { var });
        sketch
    }

    /// The projected origin, which never moves.
    pub fn origin(&self) -> PointId {
        PointId(Uuid::nil())
    }

    fn new_var(&mut self, value: f64) -> usize {
        self.vars.push(value);
        self.grounded.push(false);
        self.vars.len() - 1
    }

    pub fn add_point(&mut self, x: f64, y: f64) -> Result<PointId, SketchError> {
        for v in [x, y] {
            if !v.is_finite() {
                return Err(SketchError::InvalidValue(v));
            }
        }
        if self.points.len() >= MAX_POINTS {
            return Err(SketchError::TooLarge);
        }
        let id = PointId(Uuid::now_v7());
        let var = self.new_var(x);
        self.new_var(y);
        self.points.insert(id, PointData { var });
        Ok(id)
    }

    fn new_curve_id(&self) -> Result<CurveId, SketchError> {
        if self.curves.len() >= MAX_CURVES {
            return Err(SketchError::TooLarge);
        }
        Ok(CurveId(Uuid::now_v7()))
    }

    fn push_curve(&mut self, id: CurveId, geom: CurveGeom) -> CurveId {
        self.curves.insert(
            id,
            CurveData {
                geom,
                construction: false,
            },
        );
        id
    }

    pub fn add_line(&mut self, start: PointId, end: PointId) -> Result<CurveId, SketchError> {
        self.check_point(start)?;
        self.check_point(end)?;
        if start == end {
            return Err(SketchError::WrongGeometry(
                "a line needs two different points",
            ));
        }
        let id = self.new_curve_id()?;
        Ok(self.push_curve(id, CurveGeom::Line { start, end }))
    }

    pub fn add_circle(&mut self, center: PointId, radius: f64) -> Result<CurveId, SketchError> {
        self.check_point(center)?;
        if !(radius.is_finite() && radius > 0.0) {
            return Err(SketchError::InvalidValue(radius));
        }
        let id = self.new_curve_id()?;
        let radius = self.new_var(radius);
        Ok(self.push_curve(id, CurveGeom::Circle { center, radius }))
    }

    /// An arc counter-clockwise from `start` to `end` about `center`. The
    /// radius is taken from `start` and `end` is moved onto that circle
    /// (the other way round if `end` is the grounded origin).
    pub fn add_arc(
        &mut self,
        center: PointId,
        start: PointId,
        end: PointId,
    ) -> Result<CurveId, SketchError> {
        for p in [center, start, end] {
            self.check_point(p)?;
        }
        if center == start || center == end || start == end {
            return Err(SketchError::WrongGeometry(
                "an arc needs three different points",
            ));
        }
        let c = self.xy(center);
        let s = self.xy(start);
        let e = self.xy(end);
        if (s[0] - c[0]).hypot(s[1] - c[1]) == 0.0 || (e[0] - c[0]).hypot(e[1] - c[1]) == 0.0 {
            return Err(SketchError::WrongGeometry(
                "arc endpoints must be away from the center",
            ));
        }
        let id = self.new_curve_id()?;
        // The radius comes from one endpoint and the other is moved onto the
        // circle. Normally that's `start`, but a grounded endpoint (the
        // origin) can't move, so it sets the radius instead.
        let (keep, moved) = if self.is_grounded(end) {
            (end, start)
        } else {
            (start, end)
        };
        let k = self.xy(keep);
        let p = self.xy(moved);
        let r = (k[0] - c[0]).hypot(k[1] - c[1]);
        let rp = (p[0] - c[0]).hypot(p[1] - c[1]);
        if !self.is_grounded(moved) {
            let v = self.points[&moved].var;
            self.vars[v] = c[0] + (p[0] - c[0]) * r / rp;
            self.vars[v + 1] = c[1] + (p[1] - c[1]) * r / rp;
        }
        let radius = self.new_var(r);
        Ok(self.push_curve(
            id,
            CurveGeom::Arc {
                center,
                start,
                end,
                radius,
            },
        ))
    }

    /// Marks a curve as construction geometry, which constrains like any
    /// other curve but isn't part of profiles.
    pub fn set_construction(
        &mut self,
        curve: CurveId,
        construction: bool,
    ) -> Result<(), SketchError> {
        self.check_curve(curve)?;
        self.curves.get_mut(&curve).expect("checked").construction = construction;
        Ok(())
    }

    pub fn is_construction(&self, curve: CurveId) -> Result<bool, SketchError> {
        self.check_curve(curve)?;
        Ok(self.curves[&curve].construction)
    }

    pub fn point(&self, id: PointId) -> Result<[f64; 2], SketchError> {
        self.check_point(id)?;
        Ok(self.xy(id))
    }

    pub fn curve(&self, id: CurveId) -> Result<Curve, SketchError> {
        self.check_curve(id)?;
        Ok(match self.curves[&id].geom {
            CurveGeom::Line { start, end } => Curve::Line { start, end },
            CurveGeom::Circle { center, radius } => Curve::Circle {
                center,
                radius: self.vars[radius],
            },
            CurveGeom::Arc {
                center,
                start,
                end,
                radius,
            } => Curve::Arc {
                center,
                start,
                end,
                radius: self.vars[radius],
            },
        })
    }

    /// Every point, origin included, in id order.
    pub fn points(&self) -> impl Iterator<Item = PointId> + '_ {
        self.points.keys().copied()
    }

    /// Every curve in id order.
    pub fn curves(&self) -> impl Iterator<Item = CurveId> + '_ {
        self.curves.keys().copied()
    }

    /// Live constraints with whether each is driving.
    pub fn constraints(&self) -> impl Iterator<Item = (ConstraintId, Constraint, bool)> + '_ {
        self.constraints
            .iter()
            .map(|(&id, c)| (id, c.constraint, c.driving))
    }

    pub fn constraint(&self, id: ConstraintId) -> Result<Constraint, SketchError> {
        Ok(self.entry(id)?.constraint)
    }

    /// Adds a driving constraint and solves, as when placing a constraint or
    /// dimension in Inventor. On error the sketch is left unchanged.
    pub fn add_constraint(&mut self, constraint: Constraint) -> Result<ConstraintId, SketchError> {
        self.validate(&constraint)?;
        self.check_constraint_room()?;
        let snapshot = self.vars.clone();
        self.separate_for_distance(&constraint);
        let id = self.push_entry(constraint, true);
        let result = self.solve_checking(id);
        if result.is_err() {
            self.constraints.remove(&id);
            self.vars = snapshot;
        }
        result.map(|()| id)
    }

    /// Adds a constraint without solving or checking it, for loading saved
    /// sketches that may be over-constrained. [`Sketch::analyse`] reports
    /// any problems.
    pub fn add_constraint_unchecked(
        &mut self,
        constraint: Constraint,
    ) -> Result<ConstraintId, SketchError> {
        self.validate(&constraint)?;
        self.check_constraint_room()?;
        Ok(self.push_entry(constraint, true))
    }

    /// Adds a driven (reference) dimension that measures the geometry
    /// instead of controlling it. Its value tracks the geometry.
    pub fn add_driven_dimension(
        &mut self,
        constraint: Constraint,
    ) -> Result<ConstraintId, SketchError> {
        if !constraint.is_dimension() {
            return Err(SketchError::WrongGeometry("only dimensions can be driven"));
        }
        self.validate(&Self::placeholder_value(constraint))?;
        self.check_constraint_room()?;
        let id = self.push_entry(constraint, false);
        let value = self.measure(&constraint);
        self.entry_mut(id).constraint.set_value(value);
        Ok(id)
    }

    pub fn remove_constraint(&mut self, id: ConstraintId) -> Result<Constraint, SketchError> {
        self.constraints
            .remove(&id)
            .map(|e| e.constraint)
            .ok_or(SketchError::UnknownConstraint(id))
    }

    /// Deletes a curve as Inventor does: with it go its points that no other
    /// curve uses, and every constraint on the curve or those points.
    pub fn remove_curve(&mut self, id: CurveId) -> Result<Removed, SketchError> {
        self.check_curve(id)?;
        Ok(self.remove_entities(BTreeSet::new(), BTreeSet::from([id])))
    }

    /// Deletes a point, every curve built on it (as [`Sketch::remove_curve`]
    /// does) and every constraint on any of them. The origin can't be deleted.
    pub fn remove_point(&mut self, id: PointId) -> Result<Removed, SketchError> {
        self.check_point(id)?;
        if id == self.origin() {
            return Err(SketchError::WrongGeometry("the origin can't be deleted"));
        }
        Ok(self.remove_entities(BTreeSet::from([id]), BTreeSet::new()))
    }

    /// The value of a dimension: the target for a driving dimension, the
    /// measured value for a driven one.
    pub fn dimension_value(&self, id: ConstraintId) -> Result<f64, SketchError> {
        let entry = self.entry(id)?;
        if entry.driving {
            entry
                .constraint
                .value()
                .ok_or(SketchError::NotADimension(id))
        } else {
            Ok(self.measure(&entry.constraint))
        }
    }

    /// Changes a driving dimension and re-solves. On error the sketch is
    /// left unchanged.
    pub fn set_dimension(
        &mut self,
        id: ConstraintId,
        value: f64,
    ) -> Result<SolveReport, SketchError> {
        let entry = self.entry(id)?;
        if !entry.driving || !entry.constraint.is_dimension() {
            return Err(SketchError::NotADimension(id));
        }
        let mut changed = entry.constraint;
        changed.set_value(value);
        self.validate(&changed)?;

        let old = std::mem::replace(&mut self.entry_mut(id).constraint, changed);
        let snapshot = self.vars.clone();
        let outcomes = self.solve_with(&self.unit_weights());
        let own = outcomes
            .iter()
            .find(|(owners, _)| owners.contains(&Owner::Constraint(id.0)))
            .map(|(_, o)| *o);
        if let Some(o) = own.filter(|o| !o.converged) {
            self.entry_mut(id).constraint = old;
            self.vars = snapshot;
            return Err(if o.too_large {
                SketchError::TooLarge
            } else {
                SketchError::Unsolvable
            });
        }
        Ok(Self::report(&outcomes))
    }

    /// Solves every driving constraint, moving geometry as little as
    /// possible.
    pub fn solve(&mut self) -> SolveReport {
        let outcomes = self.solve_with(&self.unit_weights());
        Self::report(&outcomes)
    }

    /// Moves a point toward `target` the way dragging does in Inventor:
    /// the point goes as close to the target as the constraints allow, and
    /// other geometry follows only as much as it must.
    pub fn drag_point(
        &mut self,
        point: PointId,
        target: [f64; 2],
    ) -> Result<SolveReport, SketchError> {
        self.check_point(point)?;
        if !target.iter().all(|v| v.is_finite()) {
            return Err(SketchError::WrongGeometry("drag target must be finite"));
        }
        if self.is_grounded(point) {
            return Ok(self.solve());
        }
        let var = self.points[&point].var;
        let snapshot = self.vars.clone();
        self.vars[var] = target[0];
        self.vars[var + 1] = target[1];
        let mut weights = self.unit_weights();
        weights[var] = DRAG_WEIGHT;
        weights[var + 1] = DRAG_WEIGHT;
        let report = Self::report(&self.solve_with(&weights));
        if !report.converged {
            self.vars = snapshot;
            return Ok(self.solve());
        }
        Ok(report)
    }

    /// Degrees of freedom, redundant constraints and free geometry at the
    /// current geometry. Call after [`Sketch::solve`] for meaningful results.
    pub fn analyse(&self) -> Analysis {
        let eqs = self.equations();
        let unknown: Vec<bool> = self.grounded.iter().map(|g| !g).collect();
        let mut col = vec![usize::MAX; self.vars.len()];
        let mut rank = 0;
        let mut free_var = unknown.clone();
        let mut redundant: Vec<Redundancy> = Vec::new();
        let mut too_large = false;
        let mut skipped_vars = 0;
        for cluster in solver::clusters(&eqs, &unknown) {
            let mut order: Vec<usize> = (0..cluster.eqs.len()).collect();
            order.sort_by_key(|&k| eqs[cluster.eqs[k]].owner);
            let a = solver::analyse_cluster(&self.vars, &eqs, &cluster, &order, &mut col);
            if a.too_large {
                too_large = true;
                skipped_vars += cluster.vars.len();
                continue;
            }
            rank += a.rank;
            for &v in &cluster.vars {
                free_var[v] = false;
            }
            for v in a.free_vars {
                free_var[v] = true;
            }
            for (eq, depends) in a.redundant {
                let Owner::Constraint(c) = eqs[eq].owner else {
                    continue;
                };
                let constraint = ConstraintId(c);
                let mut conflicts_with: Vec<ConstraintId> = depends
                    .iter()
                    .filter_map(|&d| match eqs[d].owner {
                        Owner::Constraint(o) if o != c => Some(ConstraintId(o)),
                        _ => None,
                    })
                    .collect();
                conflicts_with.sort();
                conflicts_with.dedup();
                match redundant.iter_mut().find(|r| r.constraint == constraint) {
                    Some(r) => {
                        r.conflicts_with.extend(conflicts_with);
                        r.conflicts_with.sort();
                        r.conflicts_with.dedup();
                    }
                    None => redundant.push(Redundancy {
                        constraint,
                        conflicts_with,
                        satisfied: true,
                    }),
                }
            }
        }

        let tolerance = self.tolerance();
        // A NaN residual counts as broken.
        let broken: Vec<&Equation> = eqs
            .iter()
            .filter(|e| {
                let r = e.residual(&self.vars).abs();
                r.is_nan() || r > tolerance
            })
            .collect();
        let mut unsatisfied: Vec<ConstraintId> = broken
            .iter()
            .filter_map(|e| match e.owner {
                Owner::Constraint(c) => Some(ConstraintId(c)),
                Owner::Curve(_) => None,
            })
            .collect();
        unsatisfied.sort();
        unsatisfied.dedup();
        let mut inconsistent_curves: Vec<CurveId> = broken
            .iter()
            .filter_map(|e| match e.owner {
                Owner::Curve(c) => Some(CurveId(c)),
                Owner::Constraint(_) => None,
            })
            .collect();
        inconsistent_curves.sort();
        inconsistent_curves.dedup();
        for r in &mut redundant {
            r.satisfied = !unsatisfied.contains(&r.constraint);
        }
        redundant.sort_by_key(|r| r.constraint);

        let unknown_count = unknown.iter().filter(|&&u| u).count();
        let dof = unknown_count - skipped_vars - rank;
        let point_free = |p: PointId| {
            let v = self.points[&p].var;
            free_var[v] || free_var[v + 1]
        };
        let free_points: Vec<PointId> = self.points().filter(|&p| point_free(p)).collect();
        let free_curves = self
            .curves()
            .filter(|&c| match self.curves[&c].geom {
                CurveGeom::Line { start, end } => point_free(start) || point_free(end),
                CurveGeom::Circle { center, radius } => point_free(center) || free_var[radius],
                CurveGeom::Arc {
                    center,
                    start,
                    end,
                    radius,
                } => point_free(center) || point_free(start) || point_free(end) || free_var[radius],
            })
            .collect();
        let status = if !redundant.is_empty() {
            ConstraintStatus::OverConstrained
        } else if dof == 0 && !too_large {
            ConstraintStatus::FullyConstrained
        } else {
            ConstraintStatus::UnderConstrained
        };
        Analysis {
            too_large,
            dof,
            status,
            redundant,
            unsatisfied,
            inconsistent_curves,
            free_points,
            free_curves,
        }
    }

    // ---- internals ----

    /// Residual tolerance for this sketch: 1e-10 model units, or a few ulps
    /// of the largest coordinate when the sketch is far from the origin, so
    /// large coordinates stay solvable.
    fn tolerance(&self) -> f64 {
        let scale = self.vars.iter().fold(1.0f64, |m, v| m.max(v.abs()));
        (16.0 * f64::EPSILON * scale).max(SolveSettings::default().tolerance)
    }

    fn check_constraint_room(&self) -> Result<(), SketchError> {
        if self.constraints.len() >= MAX_CONSTRAINTS {
            Err(SketchError::TooLarge)
        } else {
            Ok(())
        }
    }

    /// A distance between two points in the same place has no direction to
    /// grow in. Like Inventor, pull them apart along X first, moving
    /// whichever point isn't grounded.
    fn separate_for_distance(&mut self, c: &Constraint) {
        let Constraint::Distance(p, q, d) = *c else {
            return;
        };
        let (a, b) = (self.xy(p), self.xy(q));
        if (b[0] - a[0]).hypot(b[1] - a[1]) > self.tolerance() {
            return;
        }
        let (moved, sign) = if self.is_grounded(q) {
            (p, -1.0)
        } else {
            (q, 1.0)
        };
        let v = self.points[&moved].var;
        self.vars[v] += sign * d;
    }

    /// The same dimension with a value every dimension kind accepts, for
    /// checking geometry without the value.
    fn placeholder_value(mut c: Constraint) -> Constraint {
        c.set_value(1.0);
        c
    }

    fn check_point(&self, id: PointId) -> Result<(), SketchError> {
        if self.points.contains_key(&id) {
            Ok(())
        } else {
            Err(SketchError::UnknownPoint(id))
        }
    }

    fn check_curve(&self, id: CurveId) -> Result<(), SketchError> {
        if self.curves.contains_key(&id) {
            Ok(())
        } else {
            Err(SketchError::UnknownCurve(id))
        }
    }

    fn entry(&self, id: ConstraintId) -> Result<&ConstraintEntry, SketchError> {
        self.constraints
            .get(&id)
            .ok_or(SketchError::UnknownConstraint(id))
    }

    fn entry_mut(&mut self, id: ConstraintId) -> &mut ConstraintEntry {
        self.constraints.get_mut(&id).expect("live constraint")
    }

    fn xy(&self, p: PointId) -> [f64; 2] {
        let v = self.points[&p].var;
        [self.vars[v], self.vars[v + 1]]
    }

    fn is_grounded(&self, p: PointId) -> bool {
        self.grounded[self.points[&p].var]
    }

    fn pv(&self, p: PointId) -> [usize; 2] {
        let v = self.points[&p].var;
        [v, v + 1]
    }

    /// `[ax, ay, bx, by]` of a line's endpoints.
    fn line_vars(&self, c: CurveId) -> [usize; 4] {
        match self.curves[&c].geom {
            CurveGeom::Line { start, end } => {
                let [a, b] = [self.pv(start), self.pv(end)];
                [a[0], a[1], b[0], b[1]]
            }
            _ => unreachable!("validated as a line"),
        }
    }

    /// Center variables and radius variable of a circle or arc.
    fn round_vars(&self, c: CurveId) -> ([usize; 2], usize) {
        match self.curves[&c].geom {
            CurveGeom::Circle { center, radius } | CurveGeom::Arc { center, radius, .. } => {
                (self.pv(center), radius)
            }
            CurveGeom::Line { .. } => unreachable!("validated as round"),
        }
    }

    fn is_line(&self, c: CurveId) -> bool {
        matches!(self.curves[&c].geom, CurveGeom::Line { .. })
    }

    /// Every variable of a point or curve, for [`Constraint::Fix`].
    fn entity_vars(&self, e: Entity) -> Vec<usize> {
        match e {
            Entity::Point(p) => self.pv(p).to_vec(),
            Entity::Curve(c) => match self.curves[&c].geom {
                CurveGeom::Line { .. } => self.line_vars(c).to_vec(),
                CurveGeom::Circle { center, radius } => {
                    vec![self.pv(center)[0], self.pv(center)[1], radius]
                }
                CurveGeom::Arc {
                    center,
                    start,
                    end,
                    radius,
                } => {
                    let mut v: Vec<usize> = [center, start, end]
                        .iter()
                        .flat_map(|&p| self.pv(p))
                        .collect();
                    v.push(radius);
                    v
                }
            },
        }
    }

    fn validate(&self, c: &Constraint) -> Result<(), SketchError> {
        use Constraint::*;
        let line = |id: CurveId| -> Result<(), SketchError> {
            self.check_curve(id)?;
            if self.is_line(id) {
                Ok(())
            } else {
                Err(SketchError::WrongGeometry("this constraint needs a line"))
            }
        };
        let round = |id: CurveId| -> Result<(), SketchError> {
            self.check_curve(id)?;
            if self.is_line(id) {
                Err(SketchError::WrongGeometry(
                    "this constraint needs a circle or arc",
                ))
            } else {
                Ok(())
            }
        };
        let two = |a: CurveId, b: CurveId| {
            if a == b {
                Err(SketchError::WrongGeometry(
                    "this constraint needs two different curves",
                ))
            } else {
                Ok(())
            }
        };
        let distinct = |a: PointId, b: PointId| -> Result<(), SketchError> {
            self.check_point(a)?;
            self.check_point(b)?;
            if a == b {
                Err(SketchError::WrongGeometry(
                    "this constraint needs two different points",
                ))
            } else {
                Ok(())
            }
        };
        let value = |v: f64, allow_zero: bool| {
            if v.is_finite() && (v > 0.0 || (allow_zero && v == 0.0)) {
                Ok(())
            } else {
                Err(SketchError::InvalidValue(v))
            }
        };
        match *c {
            Coincident(a, b) => {
                distinct(a, b)?;
                let collapses = self.curves.values().any(|c| match c.geom {
                    CurveGeom::Line { start, end } => {
                        (start, end) == (a, b) || (start, end) == (b, a)
                    }
                    _ => false,
                });
                if collapses {
                    Err(SketchError::WrongGeometry(
                        "a line's endpoints can't be coincident",
                    ))
                } else {
                    Ok(())
                }
            }
            HorizontalAlign(a, b) | VerticalAlign(a, b) => distinct(a, b),
            PointOnCurve(p, curve) => {
                self.check_point(p)?;
                self.check_curve(curve)
            }
            Horizontal(l) | Vertical(l) => line(l),
            Parallel(a, b) | Perpendicular(a, b) | Collinear(a, b) => {
                line(a)?;
                line(b)?;
                two(a, b)
            }
            Tangent(a, b) => {
                self.check_curve(a)?;
                self.check_curve(b)?;
                two(a, b)?;
                if self.is_line(a) && self.is_line(b) {
                    Err(SketchError::WrongGeometry(
                        "tangency needs at least one circle or arc",
                    ))
                } else {
                    Ok(())
                }
            }
            Equal(a, b) => {
                self.check_curve(a)?;
                self.check_curve(b)?;
                two(a, b)?;
                if self.is_line(a) == self.is_line(b) {
                    Ok(())
                } else {
                    Err(SketchError::WrongGeometry(
                        "equal needs two lines or two circles/arcs",
                    ))
                }
            }
            Concentric(a, b) => {
                round(a)?;
                round(b)?;
                two(a, b)
            }
            Midpoint(p, l) => {
                self.check_point(p)?;
                line(l)
            }
            Symmetric(p, q, l) => {
                distinct(p, q)?;
                line(l)
            }
            Fix(Entity::Point(p)) => self.check_point(p),
            Fix(Entity::Curve(curve)) => self.check_curve(curve),
            Distance(p, q, v) => {
                distinct(p, q)?;
                value(v, false)
            }
            HorizontalDistance(p, q, v) | VerticalDistance(p, q, v) => {
                distinct(p, q)?;
                value(v, true)
            }
            PointLineDistance(p, l, v) => {
                self.check_point(p)?;
                line(l)?;
                value(v, true)
            }
            Angle(a, b, v) => {
                line(a)?;
                line(b)?;
                two(a, b)?;
                if (0.0..=std::f64::consts::PI).contains(&v) {
                    Ok(())
                } else {
                    Err(SketchError::InvalidValue(v))
                }
            }
            Radius(r, v) | Diameter(r, v) => {
                round(r)?;
                value(v, false)
            }
        }
    }

    fn push_entry(&mut self, constraint: Constraint, driving: bool) -> ConstraintId {
        let id = ConstraintId(Uuid::now_v7());
        let captured = if driving {
            self.capture(&constraint)
        } else {
            Captured::None
        };
        self.constraints.insert(
            id,
            ConstraintEntry {
                constraint,
                driving,
                captured,
            },
        );
        id
    }

    /// Removes the given points and curves plus everything that depends on
    /// them, then rebuilds the solver variables so none are left orphaned.
    fn remove_entities(
        &mut self,
        mut points: BTreeSet<PointId>,
        mut curves: BTreeSet<CurveId>,
    ) -> Removed {
        for (&id, c) in &self.curves {
            if c.geom.points().iter().any(|p| points.contains(p)) {
                curves.insert(id);
            }
        }
        let still_used: BTreeSet<PointId> = self
            .curves
            .iter()
            .filter(|(id, _)| !curves.contains(id))
            .flat_map(|(_, c)| c.geom.points())
            .collect();
        for id in &curves {
            for p in self.curves[id].geom.points() {
                if p != self.origin() && !still_used.contains(&p) {
                    points.insert(p);
                }
            }
        }
        let constraints: Vec<ConstraintId> = self
            .constraints
            .iter()
            .filter(|(_, e)| {
                let (ps, cs) = e.constraint.references();
                ps.iter().any(|p| points.contains(p)) || cs.iter().any(|c| curves.contains(c))
            })
            .map(|(&id, _)| id)
            .collect();

        for id in &constraints {
            self.constraints.remove(id);
        }
        for id in &curves {
            self.curves.remove(id);
        }
        for id in &points {
            self.points.remove(id);
        }
        self.compact();
        Removed {
            points: points.into_iter().collect(),
            curves: curves.into_iter().collect(),
            constraints,
        }
    }

    /// Renumbers the solver variables to just those of live points and
    /// curves.
    fn compact(&mut self) {
        let mut vars = Vec::new();
        let mut grounded = Vec::new();
        let mut take = |v: usize| {
            vars.push(self.vars[v]);
            grounded.push(self.grounded[v]);
            vars.len() - 1
        };
        for p in self.points.values_mut() {
            let x = take(p.var);
            take(p.var + 1);
            p.var = x;
        }
        for c in self.curves.values_mut() {
            match &mut c.geom {
                CurveGeom::Line { .. } => {}
                CurveGeom::Circle { radius, .. } | CurveGeom::Arc { radius, .. } => {
                    *radius = take(*radius)
                }
            }
        }
        self.vars = vars;
        self.grounded = grounded;
    }

    /// Records which way round the geometry currently is, for constraints
    /// that could be satisfied more than one way.
    fn capture(&self, c: &Constraint) -> Captured {
        use Constraint::*;
        let sign = |v: f64| if v < 0.0 { -1.0 } else { 1.0 };
        match *c {
            HorizontalDistance(p, q, _) => Captured::Sign(sign(self.xy(q)[0] - self.xy(p)[0])),
            VerticalDistance(p, q, _) => Captured::Sign(sign(self.xy(q)[1] - self.xy(p)[1])),
            PointLineDistance(p, l, _) => {
                Captured::Sign(sign(self.signed_line_distance(l, self.xy(p))))
            }
            Angle(a, b, _) => {
                let (u, w) = (self.direction(a), self.direction(b));
                Captured::Sign(sign(u[0] * w[1] - u[1] * w[0]))
            }
            Tangent(a, b) => match (self.is_line(a), self.is_line(b)) {
                (true, _) | (_, true) => {
                    let (l, r) = if self.is_line(a) { (a, b) } else { (b, a) };
                    let (center, _) = self.round_vars(r);
                    let c = [self.vars[center[0]], self.vars[center[1]]];
                    Captured::Sign(sign(self.signed_line_distance(l, c)))
                }
                _ => {
                    let (c1, r1) = self.round_vars(a);
                    let (c2, r2) = self.round_vars(b);
                    let d = (self.vars[c2[0]] - self.vars[c1[0]])
                        .hypot(self.vars[c2[1]] - self.vars[c1[1]]);
                    let (r1, r2) = (self.vars[r1], self.vars[r2]);
                    if (d - (r1 + r2)).abs() <= (d - (r1 - r2).abs()).abs() {
                        Captured::Tangency { s1: 1.0, s2: 1.0 }
                    } else if r1 >= r2 {
                        Captured::Tangency { s1: 1.0, s2: -1.0 }
                    } else {
                        Captured::Tangency { s1: -1.0, s2: 1.0 }
                    }
                }
            },
            Fix(e) => Captured::Fixed(self.entity_vars(e).iter().map(|&v| self.vars[v]).collect()),
            _ => Captured::None,
        }
    }

    fn direction(&self, l: CurveId) -> [f64; 2] {
        let v = self.line_vars(l);
        [
            self.vars[v[2]] - self.vars[v[0]],
            self.vars[v[3]] - self.vars[v[1]],
        ]
    }

    fn signed_line_distance(&self, l: CurveId, p: [f64; 2]) -> f64 {
        let v = self.line_vars(l);
        let a = [self.vars[v[0]], self.vars[v[1]]];
        let u = self.direction(l);
        let len = u[0].hypot(u[1]);
        if len == 0.0 {
            0.0
        } else {
            (u[0] * (p[1] - a[1]) - u[1] * (p[0] - a[0])) / len
        }
    }

    /// The current value of a dimension measured from the geometry.
    fn measure(&self, c: &Constraint) -> f64 {
        use Constraint::*;
        match *c {
            Distance(p, q, _) => {
                let (a, b) = (self.xy(p), self.xy(q));
                (b[0] - a[0]).hypot(b[1] - a[1])
            }
            HorizontalDistance(p, q, _) => (self.xy(q)[0] - self.xy(p)[0]).abs(),
            VerticalDistance(p, q, _) => (self.xy(q)[1] - self.xy(p)[1]).abs(),
            PointLineDistance(p, l, _) => self.signed_line_distance(l, self.xy(p)).abs(),
            Angle(a, b, _) => {
                let (u, w) = (self.direction(a), self.direction(b));
                (u[0] * w[1] - u[1] * w[0])
                    .atan2(u[0] * w[0] + u[1] * w[1])
                    .abs()
            }
            Radius(r, _) => self.vars[self.round_vars(r).1],
            Diameter(r, _) => 2.0 * self.vars[self.round_vars(r).1],
            _ => f64::NAN,
        }
    }

    /// All equations the solver must satisfy: those built into arcs, then
    /// those of each driving constraint.
    fn equations(&self) -> Vec<Equation> {
        let mut eqs = Vec::new();
        for (&id, curve) in &self.curves {
            if let CurveGeom::Arc {
                center,
                start,
                end,
                radius,
            } = curve.geom
            {
                let c = self.pv(center);
                for p in [start, end] {
                    let p = self.pv(p);
                    eqs.push(Equation {
                        kind: EqKind::OnCircle,
                        vars: vec![c[0], c[1], p[0], p[1], radius],
                        owner: Owner::Curve(id.0),
                    });
                }
            }
        }
        for (&id, entry) in &self.constraints {
            if entry.driving {
                self.constraint_equations(Owner::Constraint(id.0), entry, &mut eqs);
            }
        }
        eqs
    }

    fn constraint_equations(&self, owner: Owner, entry: &ConstraintEntry, eqs: &mut Vec<Equation>) {
        use Constraint::*;
        let mut push = |kind: EqKind, vars: Vec<usize>| eqs.push(Equation { kind, vars, owner });
        let diff = || EqKind::Linear {
            coeffs: vec![1.0, -1.0],
            constant: 0.0,
        };
        let sign = match entry.captured {
            Captured::Sign(s) => s,
            _ => 1.0,
        };
        let angle = |theta: f64, undirected: bool| EqKind::Angle {
            cos: theta.cos(),
            sin: theta.sin(),
            undirected,
        };
        let cat = |a: &[usize], b: &[usize]| -> Vec<usize> { a.iter().chain(b).copied().collect() };

        match entry.constraint {
            Coincident(p, q) => {
                let (a, b) = (self.pv(p), self.pv(q));
                push(diff(), vec![a[0], b[0]]);
                push(diff(), vec![a[1], b[1]]);
            }
            PointOnCurve(p, c) => {
                let p = self.pv(p);
                if self.is_line(c) {
                    push(EqKind::LineDistance(0.0), cat(&self.line_vars(c), &p));
                } else {
                    let (center, r) = self.round_vars(c);
                    push(EqKind::OnCircle, vec![center[0], center[1], p[0], p[1], r]);
                }
            }
            Horizontal(l) => {
                let v = self.line_vars(l);
                push(diff(), vec![v[1], v[3]]);
            }
            Vertical(l) => {
                let v = self.line_vars(l);
                push(diff(), vec![v[0], v[2]]);
            }
            HorizontalAlign(p, q) => push(diff(), vec![self.pv(p)[1], self.pv(q)[1]]),
            VerticalAlign(p, q) => push(diff(), vec![self.pv(p)[0], self.pv(q)[0]]),
            Parallel(a, b) => push(
                angle(0.0, true),
                cat(&self.line_vars(a), &self.line_vars(b)),
            ),
            Perpendicular(a, b) => push(
                angle(FRAC_PI_2, true),
                cat(&self.line_vars(a), &self.line_vars(b)),
            ),
            Collinear(a, b) => {
                let (la, lb) = (self.line_vars(a), self.line_vars(b));
                push(EqKind::LineDistance(0.0), cat(&la, &lb[0..2]));
                push(EqKind::LineDistance(0.0), cat(&la, &lb[2..4]));
            }
            Tangent(a, b) => {
                if self.is_line(a) || self.is_line(b) {
                    let (l, r) = if self.is_line(a) { (a, b) } else { (b, a) };
                    let (c, rad) = self.round_vars(r);
                    let mut vars = cat(&self.line_vars(l), &c);
                    vars.push(rad);
                    push(EqKind::LineRadius { sign }, vars);
                } else {
                    let (s1, s2) = match entry.captured {
                        Captured::Tangency { s1, s2 } => (s1, s2),
                        _ => (1.0, 1.0),
                    };
                    let (c1, r1) = self.round_vars(a);
                    let (c2, r2) = self.round_vars(b);
                    push(
                        EqKind::CenterDistance { s1, s2 },
                        vec![c1[0], c1[1], c2[0], c2[1], r1, r2],
                    );
                }
            }
            Equal(a, b) => {
                if self.is_line(a) {
                    push(
                        EqKind::EqualLength,
                        cat(&self.line_vars(a), &self.line_vars(b)),
                    );
                } else {
                    push(diff(), vec![self.round_vars(a).1, self.round_vars(b).1]);
                }
            }
            Concentric(a, b) => {
                let (c1, c2) = (self.round_vars(a).0, self.round_vars(b).0);
                push(diff(), vec![c1[0], c2[0]]);
                push(diff(), vec![c1[1], c2[1]]);
            }
            Midpoint(p, l) => {
                let (p, v) = (self.pv(p), self.line_vars(l));
                let mid = || EqKind::Linear {
                    coeffs: vec![2.0, -1.0, -1.0],
                    constant: 0.0,
                };
                push(mid(), vec![p[0], v[0], v[2]]);
                push(mid(), vec![p[1], v[1], v[3]]);
            }
            Symmetric(p, q, l) => {
                let v = self.line_vars(l);
                let pq = cat(&self.pv(p), &self.pv(q));
                push(EqKind::LineMidpoint, cat(&v, &pq));
                push(angle(FRAC_PI_2, true), cat(&v, &pq));
            }
            Fix(e) => {
                let Captured::Fixed(values) = &entry.captured else {
                    unreachable!("fix captures values")
                };
                for (v, &value) in self.entity_vars(e).into_iter().zip(values) {
                    push(
                        EqKind::Linear {
                            coeffs: vec![1.0],
                            constant: value,
                        },
                        vec![v],
                    );
                }
            }
            Distance(p, q, d) => push(EqKind::Distance(d), cat(&self.pv(p), &self.pv(q))),
            HorizontalDistance(p, q, d) => push(
                EqKind::Linear {
                    coeffs: vec![-1.0, 1.0],
                    constant: sign * d,
                },
                vec![self.pv(p)[0], self.pv(q)[0]],
            ),
            VerticalDistance(p, q, d) => push(
                EqKind::Linear {
                    coeffs: vec![-1.0, 1.0],
                    constant: sign * d,
                },
                vec![self.pv(p)[1], self.pv(q)[1]],
            ),
            PointLineDistance(p, l, d) => push(
                EqKind::LineDistance(sign * d),
                cat(&self.line_vars(l), &self.pv(p)),
            ),
            Angle(a, b, theta) => push(
                angle(sign * theta, false),
                cat(&self.line_vars(a), &self.line_vars(b)),
            ),
            Radius(c, r) => push(
                EqKind::Linear {
                    coeffs: vec![1.0],
                    constant: r,
                },
                vec![self.round_vars(c).1],
            ),
            Diameter(c, d) => push(
                EqKind::Linear {
                    coeffs: vec![2.0],
                    constant: d,
                },
                vec![self.round_vars(c).1],
            ),
        }
    }

    fn unit_weights(&self) -> Vec<f64> {
        vec![1.0; self.vars.len()]
    }

    /// Solves each cluster and returns who owns its equations and how it went.
    fn solve_with(&mut self, weights: &[f64]) -> Vec<(Vec<Owner>, ClusterOutcome)> {
        let eqs = self.equations();
        let unknown: Vec<bool> = self.grounded.iter().map(|g| !g).collect();
        let mut col = vec![usize::MAX; self.vars.len()];
        let settings = SolveSettings {
            tolerance: self.tolerance(),
            ..SolveSettings::default()
        };
        solver::clusters(&eqs, &unknown)
            .into_iter()
            .map(|cluster| {
                let outcome = solver::solve_cluster(
                    &mut self.vars,
                    &eqs,
                    &cluster,
                    weights,
                    &mut col,
                    &settings,
                );
                (cluster.eqs.iter().map(|&e| eqs[e].owner).collect(), outcome)
            })
            .collect()
    }

    fn report(outcomes: &[(Vec<Owner>, ClusterOutcome)]) -> SolveReport {
        outcomes.iter().fold(
            SolveReport {
                converged: true,
                iterations: 0,
                max_residual: 0.0,
                too_large: false,
            },
            |acc, (_, o)| SolveReport {
                converged: acc.converged && o.converged,
                iterations: acc.iterations.max(o.iterations),
                max_residual: acc.max_residual.max(o.max_residual),
                too_large: acc.too_large || o.too_large,
            },
        )
    }

    /// Solves after `id` was added and decides whether to keep it: refused
    /// if its geometry is degenerate, if it repeats or contradicts other
    /// constraints, or if its cluster won't converge.
    fn solve_checking(&mut self, id: ConstraintId) -> Result<(), SketchError> {
        // Degenerate geometry is judged before solving moves anything.
        self.own_rows_check(id, true)?;
        let owner = Owner::Constraint(id.0);
        let outcomes = self.solve_with(&self.unit_weights());
        let own: Vec<ClusterOutcome> = outcomes
            .iter()
            .filter(|(owners, _)| owners.contains(&owner))
            .map(|(_, o)| *o)
            .collect();
        if own.iter().any(|o| o.too_large) {
            return Err(SketchError::TooLarge);
        }
        self.own_rows_check(id, false)?;
        if own.iter().all(|o| o.converged) {
            Ok(())
        } else {
            Err(SketchError::Unsolvable)
        }
    }

    /// Tests each equation of constraint `id` against every other equation
    /// in its cluster, and against its own earlier equations. With
    /// `degenerate_only`, only reports equations with no usable slope.
    fn own_rows_check(&self, id: ConstraintId, degenerate_only: bool) -> Result<(), SketchError> {
        let owner = Owner::Constraint(id.0);
        let eqs = self.equations();
        let unknown: Vec<bool> = self.grounded.iter().map(|g| !g).collect();
        let mut col = vec![usize::MAX; self.vars.len()];
        for cluster in solver::clusters(&eqs, &unknown) {
            let own: Vec<usize> = (0..cluster.eqs.len())
                .filter(|&k| eqs[cluster.eqs[k]].owner == owner)
                .collect();
            if own.is_empty() {
                continue;
            }
            if cluster.vars.is_empty() {
                // Only grounded variables: the constraint repeats grounding.
                if degenerate_only {
                    continue;
                }
                return Err(SketchError::OverConstrained {
                    conflicts_with: Vec::new(),
                });
            }
            let mut base: Vec<usize> = (0..cluster.eqs.len())
                .filter(|&k| eqs[cluster.eqs[k]].owner != owner)
                .collect();
            for &k in &own {
                let dep = if degenerate_only {
                    solver::dependency(&self.vars, &eqs, &cluster, &mut col, &[], k)
                } else {
                    solver::dependency(&self.vars, &eqs, &cluster, &mut col, &base, k)
                };
                match dep {
                    solver::Dependency::Independent => base.push(k),
                    solver::Dependency::Degenerate => return Err(SketchError::DegenerateGeometry),
                    solver::Dependency::TooLarge => return Err(SketchError::TooLarge),
                    solver::Dependency::Redundant(depends) => {
                        let mut conflicts_with: Vec<ConstraintId> = depends
                            .iter()
                            .filter_map(|&e| match eqs[e].owner {
                                Owner::Constraint(o) if o != id.0 => Some(ConstraintId(o)),
                                _ => None,
                            })
                            .collect();
                        conflicts_with.sort();
                        conflicts_with.dedup();
                        return Err(SketchError::OverConstrained { conflicts_with });
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
