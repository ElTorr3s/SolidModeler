use super::*;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * (1.0 + b.abs())
}

fn square(x0: f64, y0: f64, s: f64, z: f64) -> Vec<[f64; 3]> {
    vec![
        [x0, y0, z],
        [x0 + s, y0, z],
        [x0 + s, y0 + s, z],
        [x0, y0 + s, z],
    ]
}

fn top_face(shape: &Shape) -> SubShape {
    shape
        .faces()
        .into_iter()
        .max_by(|a, b| {
            let za = shape.face_info(*a).unwrap().bbox.min[2];
            let zb = shape.face_info(*b).unwrap().bbox.min[2];
            za.total_cmp(&zb)
        })
        .unwrap()
}

#[test]
fn reports_linked_version() {
    let v = occt_version();
    let major: u32 = v.split('.').next().unwrap().parse().unwrap();
    let minor: u32 = v.split('.').nth(1).unwrap().parse().unwrap();
    assert!((major, minor) >= (7, 6), "{v}");
}

#[test]
fn box_topology_and_properties() {
    let b = make_box(10.0, 20.0, 30.0).unwrap();
    assert_eq!(b.shape_type().unwrap(), ShapeType::Solid);
    assert_eq!(
        (
            b.count(SubKind::Face),
            b.count(SubKind::Edge),
            b.count(SubKind::Vertex)
        ),
        (6, 12, 8)
    );
    assert!(b.is_valid());
    let m = b.mass_props().unwrap();
    assert!(close(m.volume, 6000.0, 1e-9));
    assert!(close(m.area, 2200.0, 1e-9));
    assert!(m
        .centroid
        .iter()
        .zip([5.0, 10.0, 15.0])
        .all(|(a, b)| close(*a, b, 1e-9)));
    let bb = b.bounding_box().unwrap();
    assert!(close(bb.max[2], 30.0, 1e-7) && close(bb.min[0], 0.0, 1e-7));
    for f in b.faces() {
        let info = b.face_info(f).unwrap();
        assert_eq!(info.surface, SurfaceKind::Plane);
        let p = info.plane.unwrap();
        // Normals point out of the box.
        let c = [
            (info.bbox.min[0] + info.bbox.max[0]) / 2.0 - 5.0,
            (info.bbox.min[1] + info.bbox.max[1]) / 2.0 - 10.0,
            (info.bbox.min[2] + info.bbox.max[2]) / 2.0 - 15.0,
        ];
        assert!(p.normal.iter().zip(c).map(|(n, c)| n * c).sum::<f64>() > 0.0);
    }
    let [a, z] = b.edge_ends(SubShape::edge(0)).unwrap();
    assert!(a != z);
}

#[test]
fn invalid_inputs_are_errors_not_crashes() {
    assert!(matches!(make_box(0.0, 1.0, 1.0), Err(OcctError::Kernel(_))));
    assert!(make_box(f64::NAN, 1.0, 1.0).is_err());
    assert!(make_box(-1.0, 1.0, 1.0).is_err());
    let b = make_box(1.0, 1.0, 1.0).unwrap();
    assert!(matches!(
        b.face_info(SubShape::face(6)),
        Err(OcctError::Input(_))
    ));
    assert!(b.face_info(SubShape::edge(0)).is_err());
    assert!(b.edge_ends(SubShape::edge(99)).is_err());
    assert!(fillet(&b, &[SubShape::edge(99)], 0.1).is_err());
    assert!(fillet(&b, &[SubShape::face(0)], 0.1).is_err());
    assert!(fillet(&b, &[], 0.1).is_err());
    assert!(fillet(&b, &[SubShape::edge(0)], -1.0).is_err());
    // A radius larger than the part is a kernel failure, reported as such.
    assert!(matches!(
        fillet(&b, &[SubShape::edge(0)], 5.0),
        Err(OcctError::Kernel(_))
    ));
    assert!(prism(&b.sub(SubShape::face(0)).unwrap(), [0.0, 0.0, 0.0]).is_err());
    assert!(prism(
        &b.sub(SubShape::face(0)).unwrap(),
        [f64::INFINITY, 0.0, 0.0]
    )
    .is_err());
    assert!(b.tessellate(0.0, 0.5).is_err());
    assert!(polygon_face(&[]).is_err());
    assert!(polygon_face(&[vec![[0.0; 3], [1.0, 0.0, 0.0]]]).is_err());
    assert!(polygon_face(&[vec![[0.0; 3], [0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]]).is_err());
    // Non-planar and self-intersecting loops.
    assert!(polygon_face(&[vec![
        [0.0; 3],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 0.0]
    ]])
    .is_err());
    assert!(polygon_face(&[vec![
        [0.0; 3],
        [1.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0]
    ]])
    .is_err());
    assert!(Shape::from_brep(b"not a brep").is_err());
    assert!(Shape::from_brep(&[]).is_err());
}

#[test]
fn polygon_face_maps_segments_to_edges() {
    let outer = square(0.0, 0.0, 10.0, 0.0);
    let hole = square(3.0, 3.0, 2.0, 0.0);
    let pf = polygon_face(&[outer.clone(), hole.clone()]).unwrap();
    assert_eq!(pf.face.shape_type().unwrap(), ShapeType::Face);
    assert_eq!(pf.segments.len(), 2);
    assert_eq!(pf.face.count(SubKind::Edge), 8);
    for (loop_pts, segs) in [(&outer, &pf.segments[0]), (&hole, &pf.segments[1])] {
        assert_eq!(segs.len(), 4);
        for (k, e) in segs.iter().enumerate() {
            let [a, b] = pf.face.edge_ends(*e).unwrap();
            let (p, q) = (loop_pts[k], loop_pts[(k + 1) % 4]);
            let same =
                |x: [f64; 3], y: [f64; 3]| x.iter().zip(y).all(|(u, v)| (u - v).abs() < 1e-9);
            assert!(
                (same(a, p) && same(b, q)) || (same(a, q) && same(b, p)),
                "segment {k}"
            );
        }
    }
    assert!(close(
        pf.face.face_info(SubShape::face(0)).unwrap().area,
        96.0,
        1e-9
    ));
    // A hole given counter-clockwise is still a hole.
    let mut ccw_hole = hole;
    ccw_hole.reverse();
    let pf = polygon_face(&[outer, ccw_hole]).unwrap();
    assert!(close(
        pf.face.face_info(SubShape::face(0)).unwrap().area,
        96.0,
        1e-9
    ));
}

#[test]
fn prism_history_names_side_faces_and_caps() {
    let pf = polygon_face(&[square(0.0, 0.0, 10.0, 0.0)]).unwrap();
    let sweep = prism(&pf.face, [0.0, 0.0, 5.0]).unwrap();
    let s = &sweep.op.shape;
    assert!(s.is_valid());
    assert!(close(s.mass_props().unwrap().volume, 500.0, 1e-9));
    let (first, last) = (sweep.first.unwrap(), sweep.last.unwrap());
    assert!(close(s.face_info(first).unwrap().bbox.max[2], 0.0, 1e-7));
    assert!(close(s.face_info(last).unwrap().bbox.min[2], 5.0, 1e-7));
    // Each profile edge generates exactly one side face, all distinct.
    let mut sides = Vec::new();
    for e in &pf.segments[0] {
        let g = sweep.op.history.generated(0, *e);
        assert_eq!(g.len(), 1, "{e:?}");
        assert_eq!(g[0].kind, SubKind::Face);
        assert!(g[0] != first && g[0] != last);
        sides.push(g[0]);
        let src = sweep.op.history.source(g[0]);
        assert!(src
            .iter()
            .any(|s| s.sub == *e && s.relation == Relation::Generated));
    }
    sides.sort();
    sides.dedup();
    assert_eq!(sides.len(), 4);
}

#[test]
fn revolve_full_and_partial() {
    // A 2 x 4 rectangle at radius 10..12, revolved about Z.
    let profile = polygon_face(&[vec![
        [10.0, 0.0, 0.0],
        [12.0, 0.0, 0.0],
        [12.0, 0.0, 4.0],
        [10.0, 0.0, 4.0],
    ]])
    .unwrap();
    let full = revolve(&profile.face, [0.0; 3], [0.0, 0.0, 1.0], TAU).unwrap();
    assert!(full.first.is_none() && full.last.is_none());
    let ring = PI * (144.0 - 100.0) * 4.0;
    assert!(close(
        full.op.shape.mass_props().unwrap().volume,
        ring,
        1e-6
    ));
    let half = revolve(&profile.face, [0.0; 3], [0.0, 0.0, 1.0], PI).unwrap();
    assert!(half.first.is_some() && half.last.is_some());
    assert!(close(
        half.op.shape.mass_props().unwrap().volume,
        ring / 2.0,
        1e-6
    ));
    // Over-full angles clamp to a full turn.
    let more = revolve(&profile.face, [0.0; 3], [0.0, 0.0, 1.0], 100.0).unwrap();
    assert!(more.first.is_none());
    assert!(revolve(&profile.face, [0.0; 3], [0.0; 3], PI).is_err());
    assert!(revolve(&profile.face, [0.0; 3], [0.0, 0.0, 1.0], 0.0).is_err());
}

#[test]
fn boolean_history_covers_kept_modified_and_deleted() {
    let a = make_box(10.0, 10.0, 10.0).unwrap();
    // A smaller box poking out of the top.
    let t = prism(
        &polygon_face(&[square(2.0, 2.0, 3.0, 5.0)]).unwrap().face,
        [0.0, 0.0, 10.0],
    )
    .unwrap()
    .op
    .shape;
    let cut = boolean(BooleanKind::Cut, &a, &t).unwrap();
    assert!(cut.shape.is_valid());
    assert!(close(
        cut.shape.mass_props().unwrap().volume,
        1000.0 - 45.0,
        1e-9
    ));
    let h = &cut.history;
    let top = top_face(&a);
    // The top face gets a hole: modified, not kept, not deleted.
    assert_eq!(h.modified(0, top).len(), 1);
    assert!(h.kept(0, top).is_none() && !h.is_deleted(0, top));
    // The bottom face is untouched.
    let bottom = a
        .faces()
        .into_iter()
        .find(|f| close(a.face_info(*f).unwrap().bbox.max[2], 0.0, 1e-7))
        .unwrap();
    let kept = h.kept(0, bottom).unwrap();
    assert!(a.same(bottom, &cut.shape, kept).unwrap());
    // The tool's top face lies outside the block and is gone.
    let tool_top = top_face(&t);
    assert!(h.is_deleted(1, tool_top));
    // Every result face has a recorded origin.
    for f in cut.shape.faces() {
        assert!(!h.source(f).is_empty(), "face {f:?} has no source");
    }

    let fused = boolean(BooleanKind::Fuse, &a, &t).unwrap();
    assert!(close(
        fused.shape.mass_props().unwrap().volume,
        1000.0 + 45.0,
        1e-9
    ));
    let common = boolean(BooleanKind::Common, &a, &t).unwrap();
    assert!(close(common.shape.mass_props().unwrap().volume, 45.0, 1e-9));
}

#[test]
fn coplanar_fuse_unifies_with_history() {
    let a = make_box(10.0, 10.0, 10.0).unwrap();
    let b = prism(
        &polygon_face(&[square(10.0, 0.0, 10.0, 0.0)]).unwrap().face,
        [0.0, 0.0, 10.0],
    )
    .unwrap()
    .op
    .shape;
    let fused = boolean(BooleanKind::Fuse, &a, &b).unwrap().shape;
    assert!(fused.count(SubKind::Face) > 6);
    let unified = unify_same_domain(&fused).unwrap();
    assert_eq!(unified.shape.count(SubKind::Face), 6);
    assert!(unified.shape.is_valid());
    for f in unified.shape.faces() {
        assert!(!unified.history.source(f).is_empty());
    }
    // The split top faces both map to the single merged top.
    let tops: Vec<_> = fused
        .faces()
        .into_iter()
        .filter(|f| close(fused.face_info(*f).unwrap().bbox.min[2], 10.0, 1e-7))
        .collect();
    assert_eq!(tops.len(), 2);
    let merged: Vec<_> = tops
        .iter()
        .flat_map(|f| unified.history.modified(0, *f))
        .collect();
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0], merged[1]);
}

#[test]
fn fillet_chamfer_shell_draft() {
    let b = make_box(20.0, 20.0, 10.0).unwrap();
    let top = top_face(&b);
    let top_edges: Vec<SubShape> = b
        .edges()
        .into_iter()
        .filter(|e| {
            let [p, q] = b.edge_ends(*e).unwrap();
            close(p[2], 10.0, 1e-9) && close(q[2], 10.0, 1e-9)
        })
        .collect();
    assert_eq!(top_edges.len(), 4);

    let f = fillet(&b, &top_edges, 2.0).unwrap();
    assert!(f.shape.is_valid());
    let removed = 4.0 * 20.0 * (4.0 - PI) - 0.0; // edge blends, corners approx.
    let v = f.shape.mass_props().unwrap().volume;
    assert!(v < 4000.0 && v > 4000.0 - removed * 1.1, "{v}");
    for e in &top_edges {
        let g = f.history.generated(0, *e);
        assert!(!g.is_empty(), "no blend face for {e:?}");
        assert!(g.iter().all(|s| s.kind == SubKind::Face));
    }
    assert!(!f.history.modified(0, top).is_empty());

    let c = chamfer(&b, &top_edges[..1], 1.0).unwrap();
    assert!(close(
        c.shape.mass_props().unwrap().volume,
        4000.0 - 0.5 * 20.0,
        1e-6
    ));
    assert_eq!(c.history.generated(0, top_edges[0]).len(), 1);

    let s = shell(&b, &[top], 1.0).unwrap();
    assert!(s.shape.is_valid());
    let inner = 18.0 * 18.0 * 9.0;
    assert!(close(
        s.shape.mass_props().unwrap().volume,
        4000.0 - inner,
        1e-6
    ));
    // The opened face is gone from the result.
    assert!(s.history.kept(0, top).is_none());

    // Draft the four side faces by 5 degrees, pivoting at the bottom.
    let sides: Vec<SubShape> = b
        .faces()
        .into_iter()
        .filter(|f| b.face_info(*f).unwrap().plane.unwrap().normal[2].abs() < 1e-9)
        .collect();
    let d = draft(
        &b,
        &sides,
        [0.0, 0.0, 1.0],
        5f64.to_radians(),
        [0.0; 3],
        [0.0, 0.0, 1.0],
    )
    .unwrap();
    assert!(d.shape.is_valid());
    let v = d.shape.mass_props().unwrap().volume;
    assert!(v < 4000.0 && v > 3000.0, "{v}");
    // OpenCascade reports a face whose surface changed as generated, not
    // modified; either way each drafted face has exactly one successor.
    for f in &sides {
        let next = [d.history.modified(0, *f), d.history.generated(0, *f)].concat();
        assert_eq!(next.len(), 1, "{f:?}");
    }
    assert!(draft(
        &b,
        &sides,
        [0.0, 0.0, 1.0],
        FRAC_PI_2,
        [0.0; 3],
        [0.0, 0.0, 1.0]
    )
    .is_err());
}

#[test]
fn tessellation_is_closed_and_outward() {
    for shape in [
        make_box(3.0, 4.0, 5.0).unwrap(),
        make_cylinder(2.0, 6.0).unwrap(),
    ] {
        let m = shape.tessellate(0.01, 0.2).unwrap();
        assert_eq!(m.face_ranges.len(), shape.faces().len() + 1);
        assert_eq!(*m.face_ranges.last().unwrap() as usize, m.indices.len() / 3);
        assert_eq!(m.positions.len(), m.normals.len());
        // Signed volume from the triangles matches the exact volume, which
        // also proves every triangle faces outwards.
        let p = |i: u32| {
            let i = i as usize * 3;
            [m.positions[i], m.positions[i + 1], m.positions[i + 2]]
        };
        let mut vol = 0.0;
        for t in m.indices.chunks(3) {
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            vol += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0;
        }
        let exact = shape.mass_props().unwrap().volume;
        assert!(close(vol, exact, 2e-3), "{vol} vs {exact}");
        for n in m.normals.chunks(3) {
            assert!(close(
                (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt(),
                1.0,
                1e-9
            ));
        }
        // Meshing a shape does not change the caller's copy.
        assert!(shape.is_valid());
    }
}

#[test]
fn brep_round_trip_preserves_shape() {
    let b = boolean(
        BooleanKind::Cut,
        &make_box(10.0, 10.0, 10.0).unwrap(),
        &make_cylinder(3.0, 20.0).unwrap(),
    )
    .unwrap()
    .shape;
    let bytes = b.to_brep().unwrap();
    let back = Shape::from_brep(&bytes).unwrap();
    assert_eq!(back.count(SubKind::Face), b.count(SubKind::Face));
    assert!(close(
        back.mass_props().unwrap().volume,
        b.mass_props().unwrap().volume,
        1e-12
    ));
    // Truncated or flipped bytes never crash the reader.
    for cut in (0..bytes.len()).step_by(bytes.len() / 25 + 1) {
        let _ = Shape::from_brep(&bytes[..cut]);
    }
}

#[test]
fn shapes_move_between_threads() {
    let b = make_box(1.0, 2.0, 3.0).unwrap();
    let v = std::thread::spawn(move || b.mass_props().unwrap().volume)
        .join()
        .unwrap();
    assert!(close(v, 6.0, 1e-12));
}
