use super::*;
use std::f64::consts::PI;

const EPS: f64 = 1e-8;

fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < EPS && (a[1] - b[1]).abs() < EPS
}

/// A roughly drawn quadrilateral: four points and four lines in a loop.
fn quad(s: &mut Sketch, pts: [[f64; 2]; 4]) -> ([PointId; 4], [CurveId; 4]) {
    let p = pts.map(|[x, y]| s.add_point(x, y));
    let l = [0, 1, 2, 3].map(|i| s.add_line(p[i], p[(i + 1) % 4]).unwrap());
    (p, l)
}

/// The usual first sketch: a rectangle with one corner on the origin,
/// fully constrained by two dimensions.
fn rectangle(s: &mut Sketch) -> ([PointId; 4], [CurveId; 4], [ConstraintId; 2]) {
    let (p, l) = quad(s, [[0.3, -0.2], [41.0, 1.0], [39.0, 22.0], [-1.0, 19.0]]);
    s.add_constraint(Constraint::Coincident(p[0], s.origin()))
        .unwrap();
    s.add_constraint(Constraint::Horizontal(l[0])).unwrap();
    s.add_constraint(Constraint::Horizontal(l[2])).unwrap();
    s.add_constraint(Constraint::Vertical(l[1])).unwrap();
    s.add_constraint(Constraint::Vertical(l[3])).unwrap();
    let w = s
        .add_constraint(Constraint::Distance(p[0], p[1], 40.0))
        .unwrap();
    let h = s
        .add_constraint(Constraint::VerticalDistance(p[1], p[2], 20.0))
        .unwrap();
    (p, l, [w, h])
}

#[test]
fn rectangle_becomes_fully_constrained() {
    let mut s = Sketch::new();
    let (p, l, _) = rectangle(&mut s);
    assert!(close(s.point(p[0]).unwrap(), [0.0, 0.0]));
    assert!(close(s.point(p[1]).unwrap(), [40.0, 0.0]));
    assert!(close(s.point(p[2]).unwrap(), [40.0, 20.0]));
    assert!(close(s.point(p[3]).unwrap(), [0.0, 20.0]));
    let a = s.analyse();
    assert_eq!(a.dof, 0);
    assert_eq!(a.status, ConstraintStatus::FullyConstrained);
    assert!(a.free_points.is_empty() && a.free_curves.is_empty());
    assert!(l.iter().all(|&c| a.is_fully_constrained(Entity::Curve(c))));
}

#[test]
fn degrees_of_freedom_per_entity() {
    let mut s = Sketch::new();
    assert_eq!(s.analyse().dof, 0, "the origin is grounded");
    let a = s.add_point(1.0, 1.0);
    let b = s.add_point(5.0, 2.0);
    let line = s.add_line(a, b).unwrap();
    assert_eq!(s.analyse().dof, 4);
    s.add_constraint(Constraint::Horizontal(line)).unwrap();
    assert_eq!(s.analyse().dof, 3);

    let c = s.add_point(20.0, 0.0);
    s.add_circle(c, 3.0).unwrap();
    assert_eq!(s.analyse().dof, 3 + 3);

    let ac = s.add_point(0.0, 30.0);
    let a0 = s.add_point(5.0, 30.0);
    let a1 = s.add_point(0.0, 34.0);
    let arc = s.add_arc(ac, a0, a1).unwrap();
    assert_eq!(s.analyse().dof, 3 + 3 + 5);
    let Curve::Arc { radius, .. } = s.curve(arc).unwrap() else {
        panic!()
    };
    assert!((radius - 5.0).abs() < EPS);
    assert!(
        close(s.point(a1).unwrap(), [0.0, 35.0]),
        "end snapped onto the arc's circle"
    );
}

#[test]
fn over_constraining_dimension_is_refused_and_can_be_driven() {
    let mut s = Sketch::new();
    let (p, _, [w, _]) = rectangle(&mut s);
    let before: Vec<_> = s.points().map(|q| s.point(q).unwrap()).collect();
    let extra = Constraint::HorizontalDistance(p[3], p[2], 50.0);
    match s.add_constraint(extra) {
        Err(SketchError::OverConstrained { conflicts_with }) => {
            assert!(conflicts_with.contains(&w), "{conflicts_with:?}")
        }
        other => panic!("expected over-constrained, got {other:?}"),
    }
    let after: Vec<_> = s.points().map(|q| s.point(q).unwrap()).collect();
    assert_eq!(
        before, after,
        "a refused constraint leaves the sketch alone"
    );
    assert_eq!(s.constraints().count(), 7);

    let driven = s.add_driven_dimension(extra).unwrap();
    assert!((s.dimension_value(driven).unwrap() - 40.0).abs() < EPS);
    s.set_dimension(w, 55.0).unwrap();
    assert!(
        (s.dimension_value(driven).unwrap() - 55.0).abs() < EPS,
        "driven dims follow"
    );
    assert_eq!(s.analyse().status, ConstraintStatus::FullyConstrained);
    assert_eq!(
        s.set_dimension(driven, 3.0),
        Err(SketchError::NotADimension(driven))
    );
}

#[test]
fn redundant_geometric_constraint_is_refused() {
    let mut s = Sketch::new();
    let (_, l, _) = rectangle(&mut s);
    assert!(matches!(
        s.add_constraint(Constraint::Parallel(l[0], l[2])),
        Err(SketchError::OverConstrained { .. })
    ));
    // Not even on an under-constrained sketch: two horizontals are parallel.
    let mut s = Sketch::new();
    let (_, l) = quad(&mut s, [[0.0, 0.0], [10.0, 1.0], [9.0, 8.0], [1.0, 7.0]]);
    s.add_constraint(Constraint::Horizontal(l[0])).unwrap();
    s.add_constraint(Constraint::Horizontal(l[2])).unwrap();
    let Err(SketchError::OverConstrained { conflicts_with }) =
        s.add_constraint(Constraint::Parallel(l[0], l[2]))
    else {
        panic!()
    };
    assert_eq!(conflicts_with, vec![ConstraintId(0), ConstraintId(1)]);
}

#[test]
fn impossible_dimension_is_refused() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(10.0, 0.0);
    let c = s.add_point(5.0, 8.0);
    s.add_constraint(Constraint::Distance(a, b, 10.0)).unwrap();
    s.add_constraint(Constraint::Distance(b, c, 10.0)).unwrap();
    let before = s.point(c).unwrap();
    assert!(s.add_constraint(Constraint::Distance(a, c, 30.0)).is_err());
    assert_eq!(s.point(c).unwrap(), before);
    // Shrinking a dimension past what the others allow is refused too.
    let ac = s.add_constraint(Constraint::Distance(a, c, 12.0)).unwrap();
    assert_eq!(s.set_dimension(ac, 25.0), Err(SketchError::Unsolvable));
    assert_eq!(s.dimension_value(ac).unwrap(), 12.0);
}

#[test]
fn solving_moves_geometry_as_little_as_possible() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(10.0, 1.0);
    let line = s.add_line(a, b).unwrap();
    let lonely = s.add_point(3.0, 7.0);
    s.add_constraint(Constraint::Horizontal(line)).unwrap();
    // Minimum-norm: both endpoints meet halfway.
    assert!(close(s.point(a).unwrap(), [0.0, 0.5]));
    assert!(close(s.point(b).unwrap(), [10.0, 0.5]));
    assert_eq!(s.point(lonely).unwrap(), [3.0, 7.0]);
}

#[test]
fn tangency_keeps_the_drawn_side() {
    let mut s = Sketch::new();
    let a = s.add_point(-10.0, 0.0);
    let b = s.add_point(10.0, 0.0);
    let line = s.add_line(a, b).unwrap();
    s.add_constraint(Constraint::Horizontal(line)).unwrap();
    let c = s.add_point(0.0, -4.0);
    let circle = s.add_circle(c, 3.0).unwrap();
    s.add_constraint(Constraint::Tangent(line, circle)).unwrap();
    let center = s.point(c).unwrap();
    let Curve::Circle { radius, .. } = s.curve(circle).unwrap() else {
        panic!()
    };
    let y_line = s.point(a).unwrap()[1];
    assert!(center[1] < y_line, "circle stays below the line");
    assert!((y_line - center[1] - radius).abs() < EPS);

    // Two circles drawn overlapping a little become externally tangent.
    let d = s.add_point(6.0, -4.0);
    let other = s.add_circle(d, 3.5).unwrap();
    s.add_constraint(Constraint::Tangent(circle, other))
        .unwrap();
    let (c1, c2) = (s.point(c).unwrap(), s.point(d).unwrap());
    let Curve::Circle { radius: r1, .. } = s.curve(circle).unwrap() else {
        panic!()
    };
    let Curve::Circle { radius: r2, .. } = s.curve(other).unwrap() else {
        panic!()
    };
    assert!(((c2[0] - c1[0]).hypot(c2[1] - c1[1]) - (r1 + r2)).abs() < EPS);
}

#[test]
fn internal_tangency_and_concentric() {
    let mut s = Sketch::new();
    let c = s.add_point(0.0, 0.0);
    let big = s.add_circle(c, 10.0).unwrap();
    let d = s.add_point(5.0, 0.0);
    let small = s.add_circle(d, 4.0).unwrap();
    s.add_constraint(Constraint::Tangent(big, small)).unwrap();
    let dist = s.point(d).unwrap()[0] - s.point(c).unwrap()[0];
    let Curve::Circle { radius: r1, .. } = s.curve(big).unwrap() else {
        panic!()
    };
    let Curve::Circle { radius: r2, .. } = s.curve(small).unwrap() else {
        panic!()
    };
    assert!((dist.abs() - (r1 - r2)).abs() < EPS);

    let e = s.add_point(30.0, 1.0);
    let ring = s.add_circle(e, 2.0).unwrap();
    s.add_constraint(Constraint::Concentric(big, ring)).unwrap();
    assert!(close(s.point(e).unwrap(), s.point(c).unwrap()));
}

#[test]
fn arcs_dimensions_and_equal() {
    let mut s = Sketch::new();
    let c = s.add_point(0.0, 0.0);
    let p0 = s.add_point(5.0, 0.0);
    let p1 = s.add_point(0.0, 5.0);
    let arc = s.add_arc(c, p0, p1).unwrap();
    s.add_constraint(Constraint::Coincident(c, s.origin()))
        .unwrap();
    s.add_constraint(Constraint::Radius(arc, 8.0)).unwrap();
    for p in [p0, p1] {
        let [x, y] = s.point(p).unwrap();
        assert!((x.hypot(y) - 8.0).abs() < EPS);
    }
    let q = s.add_point(20.0, 0.0);
    let circle = s.add_circle(q, 2.0).unwrap();
    s.add_constraint(Constraint::Equal(arc, circle)).unwrap();
    let Curve::Circle { radius, .. } = s.curve(circle).unwrap() else {
        panic!()
    };
    assert!((radius - 8.0).abs() < EPS);
    let dia = s
        .add_driven_dimension(Constraint::Diameter(circle, 1.0))
        .unwrap();
    assert!((s.dimension_value(dia).unwrap() - 16.0).abs() < EPS);
    assert!(matches!(
        s.add_constraint(Constraint::Diameter(circle, 16.0)),
        Err(SketchError::OverConstrained { .. })
    ));
}

#[test]
fn angle_parallel_perpendicular() {
    let mut s = Sketch::new();
    let o = s.origin();
    let a = s.add_point(10.0, 0.5);
    let base = s.add_line(o, a).unwrap();
    s.add_constraint(Constraint::Horizontal(base)).unwrap();
    let b = s.add_point(8.0, 5.0);
    let arm = s.add_line(o, b).unwrap();
    let ang = s
        .add_constraint(Constraint::Angle(base, arm, 30f64.to_radians()))
        .unwrap();
    let [x, y] = s.point(b).unwrap();
    assert!((y.atan2(x) - PI / 6.0).abs() < 1e-9);
    s.set_dimension(ang, 120f64.to_radians()).unwrap();
    let [x, y] = s.point(b).unwrap();
    assert!((y.atan2(x) - 2.0 * PI / 3.0).abs() < 1e-9);

    let c = s.add_point(0.0, 10.0);
    let d = s.add_point(9.0, 11.0);
    let top = s.add_line(c, d).unwrap();
    s.add_constraint(Constraint::Parallel(base, top)).unwrap();
    assert!((s.point(c).unwrap()[1] - s.point(d).unwrap()[1]).abs() < EPS);
    let e = s.add_point(20.0, 0.0);
    let f = s.add_point(21.0, 9.0);
    let side = s.add_line(e, f).unwrap();
    s.add_constraint(Constraint::Perpendicular(top, side))
        .unwrap();
    assert!((s.point(e).unwrap()[0] - s.point(f).unwrap()[0]).abs() < EPS);
}

#[test]
fn midpoint_symmetric_collinear_point_on_curve() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(0.0, 10.0);
    let axis = s.add_line(a, b).unwrap();
    s.add_constraint(Constraint::Vertical(axis)).unwrap();
    let m = s.add_point(1.0, 4.0);
    s.add_constraint(Constraint::Midpoint(m, axis)).unwrap();
    let mid = s.point(m).unwrap();
    let (pa, pb) = (s.point(a).unwrap(), s.point(b).unwrap());
    assert!(close(mid, [(pa[0] + pb[0]) / 2.0, (pa[1] + pb[1]) / 2.0]));

    let p = s.add_point(-3.0, 2.0);
    let q = s.add_point(4.0, 2.5);
    s.add_constraint(Constraint::Symmetric(p, q, axis)).unwrap();
    let (pp, qq) = (s.point(p).unwrap(), s.point(q).unwrap());
    let x_axis = s.point(a).unwrap()[0];
    assert!((pp[1] - qq[1]).abs() < EPS);
    assert!(((pp[0] + qq[0]) / 2.0 - x_axis).abs() < EPS);

    let c = s.add_point(5.0, 0.0);
    let d = s.add_point(5.5, 3.0);
    let other = s.add_line(c, d).unwrap();
    s.add_constraint(Constraint::Collinear(axis, other))
        .unwrap();
    assert!((s.point(c).unwrap()[0] - s.point(a).unwrap()[0]).abs() < EPS);
    assert!((s.point(d).unwrap()[0] - s.point(a).unwrap()[0]).abs() < EPS);

    let center = s.add_point(30.0, 0.0);
    let circle = s.add_circle(center, 5.0).unwrap();
    let r = s.add_point(36.0, 1.0);
    s.add_constraint(Constraint::PointOnCurve(r, circle))
        .unwrap();
    let [x, y] = s.point(r).unwrap();
    let [cx, cy] = s.point(center).unwrap();
    let Curve::Circle { radius, .. } = s.curve(circle).unwrap() else {
        panic!()
    };
    assert!(((x - cx).hypot(y - cy) - radius).abs() < EPS);
}

#[test]
fn fix_and_point_line_distance() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(10.0, 0.0);
    let line = s.add_line(a, b).unwrap();
    s.add_constraint(Constraint::Fix(Entity::Curve(line)))
        .unwrap();
    let p = s.add_point(4.0, 3.0);
    s.add_constraint(Constraint::PointLineDistance(p, line, 7.0))
        .unwrap();
    assert!((s.point(p).unwrap()[1] - 7.0).abs() < EPS);
    assert!(close(s.point(b).unwrap(), [10.0, 0.0]));
    let a2 = s.analyse();
    assert_eq!(a2.dof, 1, "the point can still slide along the line");
    assert!(a2.is_fully_constrained(Entity::Curve(line)));
    assert!(!a2.is_fully_constrained(Entity::Point(p)));
    assert!(matches!(
        s.add_constraint(Constraint::Horizontal(line)),
        Err(SketchError::OverConstrained { .. })
    ));
}

#[test]
fn dragging_follows_constraints() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(10.0, 0.0);
    let line = s.add_line(a, b).unwrap();
    s.add_constraint(Constraint::Horizontal(line)).unwrap();
    s.add_constraint(Constraint::Distance(a, b, 10.0)).unwrap();
    let report = s.drag_point(b, [15.0, 3.0]).unwrap();
    assert!(report.converged);
    let (pa, pb) = (s.point(a).unwrap(), s.point(b).unwrap());
    assert!((pa[1] - pb[1]).abs() < EPS, "still horizontal");
    assert!(((pb[0] - pa[0]) - 10.0).abs() < EPS, "still 10 long");
    assert!(
        (pb[0] - 15.0).abs() < 1e-2 && (pb[1] - 3.0).abs() < 1e-2,
        "dragged end follows the cursor"
    );

    // A point the constraints pin can't be dragged away.
    let mut s = Sketch::new();
    let (p, ..) = rectangle(&mut s);
    s.drag_point(p[2], [100.0, 100.0]).unwrap();
    assert!(close(s.point(p[2]).unwrap(), [40.0, 20.0]));
}

#[test]
fn unchecked_conflicts_are_reported() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(10.0, 0.0);
    let first = s
        .add_constraint(Constraint::HorizontalDistance(a, b, 10.0))
        .unwrap();
    let second = s
        .add_constraint_unchecked(Constraint::HorizontalDistance(a, b, 12.0))
        .unwrap();
    assert!(!s.solve().converged);
    let analysis = s.analyse();
    assert_eq!(analysis.status, ConstraintStatus::OverConstrained);
    assert_eq!(analysis.redundant.len(), 1);
    assert_eq!(analysis.redundant[0].constraint, second);
    assert_eq!(analysis.redundant[0].conflicts_with, vec![first]);
    assert!(!analysis.redundant[0].satisfied);

    s.remove_constraint(second).unwrap();
    assert!(s.solve().converged);
    assert_eq!(s.analyse().status, ConstraintStatus::UnderConstrained);
    assert!(s.constraint(second).is_err());
}

#[test]
fn rejects_bad_input() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(1.0, 0.0);
    let line = s.add_line(a, b).unwrap();
    let c = s.add_circle(a, 1.0).unwrap();
    assert!(matches!(
        s.add_constraint(Constraint::Horizontal(c)),
        Err(SketchError::WrongGeometry(_))
    ));
    assert!(matches!(
        s.add_constraint(Constraint::Radius(line, 1.0)),
        Err(SketchError::WrongGeometry(_))
    ));
    assert!(matches!(
        s.add_constraint(Constraint::Equal(line, c)),
        Err(SketchError::WrongGeometry(_))
    ));
    assert!(matches!(
        s.add_constraint(Constraint::Distance(a, b, -1.0)),
        Err(SketchError::InvalidValue(_))
    ));
    assert!(matches!(
        s.add_constraint(Constraint::Radius(c, f64::NAN)),
        Err(SketchError::InvalidValue(_))
    ));
    assert!(matches!(
        s.add_constraint(Constraint::Coincident(a, a)),
        Err(SketchError::WrongGeometry(_))
    ));
    assert_eq!(
        s.add_line(a, PointId(99)),
        Err(SketchError::UnknownPoint(PointId(99)))
    );
    assert!(matches!(
        s.add_driven_dimension(Constraint::Horizontal(line)),
        Err(SketchError::WrongGeometry(_))
    ));
}

#[test]
fn round_trips_through_json() {
    let mut s = Sketch::new();
    let (p, ..) = rectangle(&mut s);
    let json = serde_json::to_string(&s).unwrap();
    let mut back: Sketch = serde_json::from_str(&json).unwrap();
    assert!(back.solve().converged);
    assert_eq!(back.point(p[2]).unwrap(), s.point(p[2]).unwrap());
    assert_eq!(back.analyse(), s.analyse());
}

#[test]
fn large_sketch_solves() {
    // 200 independent constrained rectangles: clusters keep this fast.
    let mut s = Sketch::new();
    for i in 0..200 {
        let x = i as f64 * 50.0;
        let (p, l) = quad(
            &mut s,
            [[x, 0.5], [x + 41.0, 1.0], [x + 39.0, 22.0], [x - 1.0, 19.0]],
        );
        s.add_constraint_unchecked(Constraint::Horizontal(l[0]))
            .unwrap();
        s.add_constraint_unchecked(Constraint::Horizontal(l[2]))
            .unwrap();
        s.add_constraint_unchecked(Constraint::Vertical(l[1]))
            .unwrap();
        s.add_constraint_unchecked(Constraint::Vertical(l[3]))
            .unwrap();
        s.add_constraint_unchecked(Constraint::HorizontalDistance(p[0], p[1], 40.0))
            .unwrap();
        s.add_constraint_unchecked(Constraint::VerticalDistance(p[1], p[2], 20.0))
            .unwrap();
    }
    let report = s.solve();
    assert!(report.converged, "{report:?}");
    assert_eq!(s.analyse().dof, 200 * 2);
}

#[test]
fn parallel_from_far_off_and_either_direction() {
    let mut s = Sketch::new();
    let o = s.origin();
    let a = s.add_point(10.0, 0.0);
    let base = s.add_line(o, a).unwrap();
    s.add_constraint(Constraint::Horizontal(base)).unwrap();
    // Drawn 80° off and pointing back the other way.
    let c = s.add_point(5.0, 10.0);
    let d = s.add_point(3.0, 0.0);
    let steep = s.add_line(c, d).unwrap();
    s.add_constraint(Constraint::Parallel(base, steep)).unwrap();
    assert!((s.point(c).unwrap()[1] - s.point(d).unwrap()[1]).abs() < EPS);
    let e = s.add_point(20.0, 0.0);
    let f = s.add_point(12.0, 1.0);
    let back = s.add_line(e, f).unwrap();
    s.add_constraint(Constraint::Parallel(base, back)).unwrap();
    let (pe, pf) = (s.point(e).unwrap(), s.point(f).unwrap());
    assert!((pe[1] - pf[1]).abs() < EPS);
    assert!(pf[0] < pe[0], "kept its drawn direction");
}

#[test]
fn corrupt_files_are_rejected() {
    use serde_json::{json, Value};
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(10.0, 0.0);
    let line = s.add_line(a, b).unwrap();
    let c = s.add_point(0.0, 20.0);
    s.add_circle(c, 3.0).unwrap();
    s.add_constraint(Constraint::Fix(Entity::Curve(line)))
        .unwrap();
    s.add_constraint(Constraint::HorizontalDistance(a, c, 0.0))
        .unwrap();
    let good: Value = serde_json::to_value(&s).unwrap();
    assert!(serde_json::from_value::<Sketch>(good.clone()).is_ok());

    type Tamper = Box<dyn Fn(&mut Value)>;
    let tamper: Vec<(&str, Tamper)> = vec![
        (
            "dropped variable",
            Box::new(|v| drop(v["vars"].as_array_mut().unwrap().pop())),
        ),
        (
            "point past the end",
            Box::new(|v| v["points"][1]["var"] = json!(1_000_000)),
        ),
        (
            "point sharing variables",
            Box::new(|v| v["points"][2]["var"] = json!(2)),
        ),
        (
            "usize overflow",
            Box::new(|v| v["points"][1]["var"] = json!(u64::MAX)),
        ),
        (
            "line to missing point",
            Box::new(|v| v["curves"][0]["geom"]["line"]["end"] = json!(42)),
        ),
        (
            "line to itself",
            Box::new(|v| v["curves"][0]["geom"]["line"]["end"] = json!(1)),
        ),
        (
            "ungrounded origin",
            Box::new(|v| v["grounded"][0] = json!(false)),
        ),
        (
            "grounded other point",
            Box::new(|v| v["grounded"][2] = json!(true)),
        ),
        (
            "constraint on missing curve",
            Box::new(|v| v["constraints"][0]["constraint"]["fix"]["curve"] = json!(9)),
        ),
        (
            "short fix data",
            Box::new(|v| {
                drop(
                    v["constraints"][0]["captured"]["fixed"]
                        .as_array_mut()
                        .unwrap()
                        .pop(),
                )
            }),
        ),
        (
            "missing sign",
            Box::new(|v| v["constraints"][1]["captured"] = json!("none")),
        ),
        (
            "bad sign",
            Box::new(|v| v["constraints"][1]["captured"] = json!({"sign": 0.5})),
        ),
        (
            "driven non-dimension",
            Box::new(|v| v["constraints"][0]["driving"] = json!(false)),
        ),
    ];
    for (what, f) in tamper {
        let mut v = good.clone();
        f(&mut v);
        assert!(
            serde_json::from_value::<Sketch>(v).is_err(),
            "accepted: {what}"
        );
    }
}

#[test]
fn degenerate_geometry_never_reads_as_solved() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(10.0, 0.0);
    let line = s.add_line(a, b).unwrap();
    assert!(matches!(
        s.add_constraint(Constraint::Coincident(a, b)),
        Err(SketchError::WrongGeometry(_))
    ));
    // Collapse the line through a third point, which can't be caught up front.
    let m = s.add_point(5.0, 0.0);
    s.add_constraint(Constraint::Coincident(a, m)).unwrap();
    s.add_constraint(Constraint::Coincident(m, b)).unwrap();
    let p = s.add_point(3.0, 4.0);
    let before: Vec<_> = s.points().map(|q| s.point(q).unwrap()).collect();
    assert!(s.add_constraint(Constraint::PointOnCurve(p, line)).is_err());
    let after: Vec<_> = s.points().map(|q| s.point(q).unwrap()).collect();
    assert_eq!(before, after);
    assert!(after.iter().flatten().all(|v| v.is_finite()));
    assert!(s.solve().converged);
}

#[test]
fn driven_dimensions_accept_any_placeholder_and_reload_at_zero() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(5.0, 0.0);
    let d = s
        .add_driven_dimension(Constraint::Distance(a, b, f64::NAN))
        .unwrap();
    assert!((s.dimension_value(d).unwrap() - 5.0).abs() < EPS);
    s.add_constraint(Constraint::Coincident(a, b)).unwrap();
    assert!(s.dimension_value(d).unwrap().abs() < EPS);
    let json = serde_json::to_string(&s).unwrap();
    let back: Sketch = serde_json::from_str(&json).unwrap();
    assert!(back.dimension_value(d).unwrap().abs() < EPS);
}
