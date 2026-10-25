//! Clean-up of the edge curves truck's booleans produce.
//!
//! Boolean results carry `IntersectionCurve` edges: two surfaces plus an
//! approximate polyline. Between planes that is just a straight line, and
//! truck-stepio writes `IntersectionCurve` incorrectly (both surfaces get
//! the same entity number), so we replace them with plain curves.

use crate::Tolerance;
use truck_modeling::geometry::Leader;
use truck_modeling::{BSplineCurve, Curve, KnotVec, Line, Point3, Solid, Surface};

/// Replaces intersection curves whose polyline is straight with exact lines.
pub fn straighten(solid: &Solid, tol: Tolerance) -> Solid {
    map_curves(solid, |curve| match curve {
        // Only two planes meet in a straight line; a curved intersection
        // with a coarse leader must not become a chord.
        Curve::IntersectionCurve(ic)
            if matches!(ic.surface0(), Surface::Plane(_))
                && matches!(ic.surface1(), Surface::Plane(_)) =>
        {
            match leader_points(ic.leader()) {
                Some(points) if collinear(&points, tol.boolean) => {
                    Curve::Line(Line(points[0], *points.last().unwrap()))
                }
                _ => curve.clone(),
            }
        }
        _ => curve.clone(),
    })
}

/// Replaces every remaining intersection curve with its leader as a
/// B-spline, for writers that can't handle intersection curves.
pub fn without_intersection_curves(solid: &Solid, tol: Tolerance) -> Solid {
    map_curves(&straighten(solid, tol), |curve| match curve {
        Curve::IntersectionCurve(ic) => match ic.leader() {
            Leader::BSpline(b) => Curve::BSplineCurve(b.clone()),
            Leader::Polyline(p) => {
                let points = p.0.clone();
                let knots = KnotVec::uniform_knot(1, points.len() - 1);
                Curve::BSplineCurve(BSplineCurve::new(knots, points))
            }
        },
        _ => curve.clone(),
    })
}

fn map_curves(solid: &Solid, f: impl Fn(&Curve) -> Curve) -> Solid {
    solid
        .try_mapped(|p| Some(*p), |c| Some(f(c)), |s| Some(s.clone()))
        .expect("mapping never fails")
}

fn leader_points(leader: &Leader) -> Option<Vec<Point3>> {
    match leader {
        Leader::Polyline(p) if p.len() >= 2 => Some(p.0.clone()),
        Leader::BSpline(b) => Some(b.control_points().clone()),
        _ => None,
    }
}

/// True if every point is within `eps` of the line through the end points.
fn collinear(points: &[Point3], eps: f64) -> bool {
    use truck_modeling::InnerSpace;
    let (a, b) = (points[0], *points.last().unwrap());
    let dir = b - a;
    let len = dir.magnitude();
    len > eps
        && points
            .iter()
            .all(|p| (p - a).cross(dir).magnitude() / len <= eps)
}
