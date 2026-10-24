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
mod solver;

use equations::{EqKind, Equation, Owner};
use serde::{Deserialize, Serialize};
use solver::{ClusterOutcome, SolveSettings};
use std::f64::consts::FRAC_PI_2;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PointId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CurveId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConstraintId(pub u32);

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
    /// Loaded sketch data is inconsistent.
    #[error("invalid sketch data: {0}")]
    Corrupt(&'static str),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PointData {
    /// Index of x in the variable vector; y follows it.
    var: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CurveData {
    geom: CurveGeom,
    construction: bool,
}

/// Orientation choices recorded when a constraint is placed, so solving
/// keeps the geometry on the side the user drew it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Captured {
    None,
    Sign(f64),
    Tangency { s1: f64, s2: f64 },
    Fixed(Vec<f64>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConstraintEntry {
    constraint: Constraint,
    /// `false` for a driven (reference) dimension, which only measures.
    driving: bool,
    captured: Captured,
}

/// How a solve went.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolveReport {
    pub converged: bool,
    /// Most iterations any one cluster of geometry needed.
    pub iterations: usize,
    /// Largest constraint error left, in model units.
    pub max_residual: f64,
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
    /// Points that can still move.
    pub free_points: Vec<PointId>,
    /// Curves that can still move or change size.
    pub free_curves: Vec<CurveId>,
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
/// Deserializing checks the whole structure (indices, variable ownership,
/// constraint kinds and orientation data), so a
/// damaged or hand-edited file is rejected instead of panicking later.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "SketchData")]
pub struct Sketch {
    vars: Vec<f64>,
    /// Variables the solver may not change, such as the origin's.
    grounded: Vec<bool>,
    points: Vec<PointData>,
    curves: Vec<CurveData>,
    /// Indexed by [`ConstraintId`]; deleted constraints leave `None`.
    constraints: Vec<Option<ConstraintEntry>>,
}

/// The serialized layout of [`Sketch`], checked before it becomes one.
#[derive(Deserialize)]
struct SketchData {
    vars: Vec<f64>,
    grounded: Vec<bool>,
    points: Vec<PointData>,
    curves: Vec<CurveData>,
    constraints: Vec<Option<ConstraintEntry>>,
}

impl TryFrom<SketchData> for Sketch {
    type Error = SketchError;

    fn try_from(d: SketchData) -> Result<Self, SketchError> {
        let sketch = Sketch {
            vars: d.vars,
            grounded: d.grounded,
            points: d.points,
            curves: d.curves,
            constraints: d.constraints,
        };
        sketch.check()?;
        Ok(sketch)
    }
}

impl Default for Sketch {
    fn default() -> Self {
        Self::new()
    }
}

/// Stiffness of a dragged point relative to other geometry, so the rest of
/// the sketch gives way to the drag where the constraints allow.
const DRAG_WEIGHT: f64 = 1e4;

impl Sketch {
    /// An empty sketch with a grounded origin point at `(0, 0)`.
    pub fn new() -> Self {
        let mut sketch = Sketch {
            vars: Vec::new(),
            grounded: Vec::new(),
            points: Vec::new(),
            curves: Vec::new(),
            constraints: Vec::new(),
        };
        let origin = sketch.add_point(0.0, 0.0);
        let var = sketch.points[origin.0 as usize].var;
        sketch.grounded[var] = true;
        sketch.grounded[var + 1] = true;
        sketch
    }

    /// The projected origin, which never moves.
    pub fn origin(&self) -> PointId {
        PointId(0)
    }

    fn new_var(&mut self, value: f64) -> usize {
        self.vars.push(value);
        self.grounded.push(false);
        self.vars.len() - 1
    }

    pub fn add_point(&mut self, x: f64, y: f64) -> PointId {
        let var = self.new_var(x);
        self.new_var(y);
        self.points.push(PointData { var });
        PointId(self.points.len() as u32 - 1)
    }

    fn push_curve(&mut self, geom: CurveGeom) -> CurveId {
        self.curves.push(CurveData {
            geom,
            construction: false,
        });
        CurveId(self.curves.len() as u32 - 1)
    }

    pub fn add_line(&mut self, start: PointId, end: PointId) -> Result<CurveId, SketchError> {
        self.check_point(start)?;
        self.check_point(end)?;
        if start == end {
            return Err(SketchError::WrongGeometry(
                "a line needs two different points",
            ));
        }
        Ok(self.push_curve(CurveGeom::Line { start, end }))
    }

    pub fn add_circle(&mut self, center: PointId, radius: f64) -> Result<CurveId, SketchError> {
        self.check_point(center)?;
        if !(radius.is_finite() && radius > 0.0) {
            return Err(SketchError::InvalidValue(radius));
        }
        let radius = self.new_var(radius);
        Ok(self.push_curve(CurveGeom::Circle { center, radius }))
    }

    /// An arc counter-clockwise from `start` to `end` about `center`. The
    /// radius is taken from `start`; `end` is moved onto that circle.
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
        let r = (s[0] - c[0]).hypot(s[1] - c[1]);
        let re = (e[0] - c[0]).hypot(e[1] - c[1]);
        if r == 0.0 || re == 0.0 {
            return Err(SketchError::WrongGeometry(
                "arc endpoints must be away from the center",
            ));
        }
        if !self.is_grounded(end) {
            let v = self.points[end.0 as usize].var;
            self.vars[v] = c[0] + (e[0] - c[0]) * r / re;
            self.vars[v + 1] = c[1] + (e[1] - c[1]) * r / re;
        }
        let radius = self.new_var(r);
        Ok(self.push_curve(CurveGeom::Arc {
            center,
            start,
            end,
            radius,
        }))
    }

    /// Marks a curve as construction geometry, which constrains like any
    /// other curve but isn't part of profiles.
    pub fn set_construction(
        &mut self,
        curve: CurveId,
        construction: bool,
    ) -> Result<(), SketchError> {
        self.check_curve(curve)?;
        self.curves[curve.0 as usize].construction = construction;
        Ok(())
    }

    pub fn is_construction(&self, curve: CurveId) -> Result<bool, SketchError> {
        self.check_curve(curve)?;
        Ok(self.curves[curve.0 as usize].construction)
    }

    pub fn point(&self, id: PointId) -> Result<[f64; 2], SketchError> {
        self.check_point(id)?;
        Ok(self.xy(id))
    }

    pub fn curve(&self, id: CurveId) -> Result<Curve, SketchError> {
        self.check_curve(id)?;
        Ok(match self.curves[id.0 as usize].geom {
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

    pub fn points(&self) -> impl Iterator<Item = PointId> + '_ {
        (0..self.points.len() as u32).map(PointId)
    }

    pub fn curves(&self) -> impl Iterator<Item = CurveId> + '_ {
        (0..self.curves.len() as u32).map(CurveId)
    }

    /// Live constraints with whether each is driving.
    pub fn constraints(&self) -> impl Iterator<Item = (ConstraintId, Constraint, bool)> + '_ {
        self.constraints.iter().enumerate().filter_map(|(i, c)| {
            c.as_ref()
                .map(|c| (ConstraintId(i as u32), c.constraint, c.driving))
        })
    }

    pub fn constraint(&self, id: ConstraintId) -> Result<Constraint, SketchError> {
        Ok(self.entry(id)?.constraint)
    }

    /// Adds a driving constraint and solves, as when placing a constraint or
    /// dimension in Inventor. On error the sketch is left unchanged.
    pub fn add_constraint(&mut self, constraint: Constraint) -> Result<ConstraintId, SketchError> {
        self.validate(&constraint)?;
        let snapshot = self.vars.clone();
        let id = self.push_entry(constraint, true);
        let result = self.solve_checking(id);
        if result.is_err() {
            self.constraints.pop();
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
        let id = self.push_entry(constraint, false);
        let value = self.measure(&constraint);
        self.entry_mut(id).constraint.set_value(value);
        Ok(id)
    }

    pub fn remove_constraint(&mut self, id: ConstraintId) -> Result<Constraint, SketchError> {
        self.entry(id)?;
        Ok(self.constraints[id.0 as usize]
            .take()
            .expect("checked above")
            .constraint)
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
        let failed = outcomes
            .iter()
            .any(|(owners, o)| !o.converged && owners.contains(&Owner::Constraint(id.0)));
        if failed {
            self.entry_mut(id).constraint = old;
            self.vars = snapshot;
            return Err(SketchError::Unsolvable);
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
        let var = self.points[point.0 as usize].var;
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
        for cluster in solver::clusters(&eqs, &unknown) {
            let mut order: Vec<usize> = (0..cluster.eqs.len()).collect();
            order.sort_by_key(|&k| eqs[cluster.eqs[k]].owner);
            let a = solver::analyse_cluster(&self.vars, &eqs, &cluster, &order, &mut col);
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

        let tolerance = SolveSettings::default().tolerance;
        let mut unsatisfied: Vec<ConstraintId> = eqs
            .iter()
            .filter(|e| e.residual(&self.vars).abs() > tolerance)
            .filter_map(|e| match e.owner {
                Owner::Constraint(c) => Some(ConstraintId(c)),
                Owner::Curve(_) => None,
            })
            .collect();
        unsatisfied.dedup();
        for r in &mut redundant {
            r.satisfied = !unsatisfied.contains(&r.constraint);
        }
        redundant.sort_by_key(|r| r.constraint);

        let unknown_count = unknown.iter().filter(|&&u| u).count();
        let dof = unknown_count - rank;
        let point_free = |p: PointId| {
            let v = self.points[p.0 as usize].var;
            free_var[v] || free_var[v + 1]
        };
        let free_points: Vec<PointId> = self.points().filter(|&p| point_free(p)).collect();
        let free_curves = self
            .curves()
            .filter(|&c| match self.curves[c.0 as usize].geom {
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
        } else if dof == 0 {
            ConstraintStatus::FullyConstrained
        } else {
            ConstraintStatus::UnderConstrained
        };
        Analysis {
            dof,
            status,
            redundant,
            unsatisfied,
            free_points,
            free_curves,
        }
    }

    /// Checks that the sketch is internally consistent: every index in
    /// range, every variable owned by exactly one point or radius, the origin
    /// grounded, and every constraint valid for its geometry with the
    /// orientation data it needs.
    fn check(&self) -> Result<(), SketchError> {
        use SketchError::Corrupt;
        let n = self.vars.len();
        if self.grounded.len() != n {
            return Err(Corrupt("variable and grounded lists differ in length"));
        }
        if !self.vars.iter().all(|v| v.is_finite()) {
            return Err(Corrupt("non-finite coordinate"));
        }
        let mut owners = vec![0u8; n];
        let mut own = |v: usize| -> Result<(), SketchError> {
            let slot = owners
                .get_mut(v)
                .ok_or(Corrupt("variable index out of range"))?;
            *slot += 1;
            Ok(())
        };
        for p in &self.points {
            own(p.var)?;
            own(p
                .var
                .checked_add(1)
                .ok_or(Corrupt("variable index out of range"))?)?;
        }
        for c in &self.curves {
            let (pts, radius): (Vec<PointId>, Option<usize>) = match c.geom {
                CurveGeom::Line { start, end } => (vec![start, end], None),
                CurveGeom::Circle { center, radius } => (vec![center], Some(radius)),
                CurveGeom::Arc {
                    center,
                    start,
                    end,
                    radius,
                } => (vec![center, start, end], Some(radius)),
            };
            for &p in &pts {
                if p.0 as usize >= self.points.len() {
                    return Err(Corrupt("curve refers to a missing point"));
                }
            }
            if (1..pts.len()).any(|i| pts[..i].contains(&pts[i])) {
                return Err(Corrupt("curve uses the same point twice"));
            }
            if let Some(r) = radius {
                own(r)?;
            }
        }
        if owners.iter().any(|&o| o != 1) {
            return Err(Corrupt("variable not owned by exactly one point or radius"));
        }
        if self.points.is_empty()
            || !self.is_grounded(PointId(0))
            || !self.grounded[self.points[0].var + 1]
        {
            return Err(Corrupt("origin point missing or not grounded"));
        }
        for p in self.points().skip(1) {
            let v = self.points[p.0 as usize].var;
            if self.grounded[v] || self.grounded[v + 1] {
                return Err(Corrupt("only the origin may be grounded"));
            }
        }
        for entry in self.constraints.iter().flatten() {
            // A driven dimension's value is only a measurement and may be
            // zero, so check its geometry alone.
            let geometry = if entry.driving {
                entry.constraint
            } else {
                Self::placeholder_value(entry.constraint)
            };
            self.validate(&geometry)
                .map_err(|_| Corrupt("constraint invalid for its geometry"))?;
            if !entry.driving && !entry.constraint.is_dimension() {
                return Err(Corrupt("only dimensions can be driven"));
            }
            let expected = if entry.driving {
                self.capture(&entry.constraint)
            } else {
                Captured::None
            };
            let unit = |v: f64| v == 1.0 || v == -1.0;
            let matches = match (&entry.captured, &expected) {
                (Captured::None, Captured::None) => true,
                (Captured::Sign(s), Captured::Sign(_)) => unit(*s),
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
        }
        Ok(())
    }

    // ---- internals ----

    /// The same dimension with a value every dimension kind accepts, for
    /// checking geometry without the value.
    fn placeholder_value(mut c: Constraint) -> Constraint {
        c.set_value(1.0);
        c
    }

    fn check_point(&self, id: PointId) -> Result<(), SketchError> {
        if (id.0 as usize) < self.points.len() {
            Ok(())
        } else {
            Err(SketchError::UnknownPoint(id))
        }
    }

    fn check_curve(&self, id: CurveId) -> Result<(), SketchError> {
        if (id.0 as usize) < self.curves.len() {
            Ok(())
        } else {
            Err(SketchError::UnknownCurve(id))
        }
    }

    fn entry(&self, id: ConstraintId) -> Result<&ConstraintEntry, SketchError> {
        self.constraints
            .get(id.0 as usize)
            .and_then(Option::as_ref)
            .ok_or(SketchError::UnknownConstraint(id))
    }

    fn entry_mut(&mut self, id: ConstraintId) -> &mut ConstraintEntry {
        self.constraints[id.0 as usize]
            .as_mut()
            .expect("live constraint")
    }

    fn xy(&self, p: PointId) -> [f64; 2] {
        let v = self.points[p.0 as usize].var;
        [self.vars[v], self.vars[v + 1]]
    }

    fn is_grounded(&self, p: PointId) -> bool {
        self.grounded[self.points[p.0 as usize].var]
    }

    fn pv(&self, p: PointId) -> [usize; 2] {
        let v = self.points[p.0 as usize].var;
        [v, v + 1]
    }

    /// `[ax, ay, bx, by]` of a line's endpoints.
    fn line_vars(&self, c: CurveId) -> [usize; 4] {
        match self.curves[c.0 as usize].geom {
            CurveGeom::Line { start, end } => {
                let [a, b] = [self.pv(start), self.pv(end)];
                [a[0], a[1], b[0], b[1]]
            }
            _ => unreachable!("validated as a line"),
        }
    }

    /// Center variables and radius variable of a circle or arc.
    fn round_vars(&self, c: CurveId) -> ([usize; 2], usize) {
        match self.curves[c.0 as usize].geom {
            CurveGeom::Circle { center, radius } | CurveGeom::Arc { center, radius, .. } => {
                (self.pv(center), radius)
            }
            CurveGeom::Line { .. } => unreachable!("validated as round"),
        }
    }

    fn is_line(&self, c: CurveId) -> bool {
        matches!(self.curves[c.0 as usize].geom, CurveGeom::Line { .. })
    }

    /// Every variable of a point or curve, for [`Constraint::Fix`].
    fn entity_vars(&self, e: Entity) -> Vec<usize> {
        match e {
            Entity::Point(p) => self.pv(p).to_vec(),
            Entity::Curve(c) => match self.curves[c.0 as usize].geom {
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
                let collapses = self.curves.iter().any(|c| match c.geom {
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
        let captured = if driving {
            self.capture(&constraint)
        } else {
            Captured::None
        };
        self.constraints.push(Some(ConstraintEntry {
            constraint,
            driving,
            captured,
        }));
        ConstraintId(self.constraints.len() as u32 - 1)
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
        for (i, curve) in self.curves.iter().enumerate() {
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
                        owner: Owner::Curve(i as u32),
                    });
                }
            }
        }
        for (i, entry) in self.constraints.iter().enumerate() {
            if let Some(entry) = entry.as_ref().filter(|e| e.driving) {
                self.constraint_equations(Owner::Constraint(i as u32), entry, &mut eqs);
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
        let settings = SolveSettings::default();
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
            },
            |acc, (_, o)| SolveReport {
                converged: acc.converged && o.converged,
                iterations: acc.iterations.max(o.iterations),
                max_residual: acc.max_residual.max(o.max_residual),
            },
        )
    }

    /// Solves after `id` was added and decides whether to keep it: refused
    /// if its cluster won't converge, or if it repeats other constraints.
    fn solve_checking(&mut self, id: ConstraintId) -> Result<(), SketchError> {
        let owner = Owner::Constraint(id.0);
        let outcomes = self.solve_with(&self.unit_weights());
        let converged = outcomes
            .iter()
            .filter(|(owners, _)| owners.contains(&owner))
            .all(|(_, o)| o.converged);
        let analysis = self.analyse();
        if let Some(r) = analysis.redundant.iter().find(|r| r.constraint == id) {
            return Err(SketchError::OverConstrained {
                conflicts_with: r.conflicts_with.clone(),
            });
        }
        if converged {
            Ok(())
        } else {
            Err(SketchError::Unsolvable)
        }
    }
}

#[cfg(test)]
mod tests;
