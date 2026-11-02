//! Builds extrusion tool bodies, nudging faces that would be flush with the
//! target body.
//!
//! truck's booleans fail when a face of the tool lies in the same plane as a
//! face of the target, which is exactly what CAD users do all the time: a
//! sketch on the top face cut downwards, a boss stacked on a face, a cut that
//! ends at a side wall. We avoid the coincidence by pushing each flush tool
//! face a little way into space where it can't change the result:
//!
//! - Cut: if the tool face points the same way as the target face, the far
//!   side is empty space, so push it outwards. If they face each other, the
//!   far side is the target's material, so pull the face back instead.
//! - Join: the reverse. Grow into the target's material, shrink away from
//!   empty space.
//!
//! Every move is checked against the real bodies before it is used: the thin
//! slab the move adds to (or removes from) the tool must lie entirely in
//! empty space (Cut) or entirely in material (Join), so the final shape is
//! exactly what an exact boolean would give. If the check fails (a boss that
//! overhangs the edge of the face it sits on, say), the face is left flush
//! and truck's boolean reports the failure instead of building a wrong part.

use crate::profile::{edge, Point2, Region};
use crate::Tolerance;
use sm_document::BodyOperation;
use truck_modeling::{builder, InnerSpace, Point3, Solid, Surface, Vector3, Wire};

/// A sketch plane placed in model space.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub origin: Point3,
    pub u: Vector3,
    pub v: Vector3,
    pub normal: Vector3,
}

impl Frame {
    fn at(&self, [x, y]: Point2, height: f64) -> Point3 {
        self.origin + self.u * x + self.v * y + self.normal * height
    }
}

/// A planar face of the target body, as plane plus bounding box.
pub struct TargetPlane {
    normal: Vector3,
    point: Point3,
    min: Point3,
    max: Point3,
}

/// Extrudes `region` from `start` to `start + length` along the frame
/// normal (`length > 0`), adjusted for faces flush with `planes` (the
/// target body's planar faces, from [`target_planes`]).
///
/// `slab_ok` checks a candidate move: it gets the thin slab between the flush
/// face and its moved position and says whether that slab is safely in
/// material (Join) or empty space (Cut).
#[allow(clippy::too_many_arguments)]
pub fn tool<E>(
    frame: &Frame,
    region: &Region,
    start: f64,
    length: f64,
    planes: &[TargetPlane],
    op: BodyOperation,
    tol: Tolerance,
    slab_ok: &dyn Fn(&Solid) -> Result<bool, E>,
) -> Result<Option<Solid>, E> {
    let op = Some(op);
    let (bottom, top) = (start, start + length);
    // Slabs keep clear of the flush face itself and of neighbouring faces,
    // so the check never has to judge touching surfaces.
    let gap = tol.boolean;
    let inset = 2.0 * tol.boolean;

    // Offsets along each face's outward normal; positive grows the tool.
    let cap = |height: f64, normal: Vector3| -> Result<f64, E> {
        let corners: Vec<_> = std::iter::once(&region.outer)
            .chain(&region.holes)
            .flatten()
            .map(|&p| frame.at(p, height))
            .collect();
        let d = flush_offset(planes, op, normal, &corners, tol);
        if d == 0.0 {
            return Ok(0.0);
        }
        // The slab covers the face region, shrunk by `inset`, between
        // `gap` and `|d|` from the face, on the side the move goes.
        let shrunk = |loop_: &Vec<Point2>| offset_loop(loop_, &vec![-inset; loop_.len()]);
        let side = normal * d.signum();
        let near = frame.at([0.0, 0.0], height) + side * gap - frame.origin;
        let place = |l: &Vec<Point2>| -> Vec<Point3> {
            shrunk(l).iter().map(|&p| frame.at(p, 0.0) + near).collect()
        };
        let outer = place(&region.outer);
        let holes: Vec<_> = region.holes.iter().map(place).collect();
        let slab = prism(&outer, &holes, side * (d.abs() - gap));
        Ok(match slab {
            Some(slab) if slab_ok(&slab)? => d,
            _ => 0.0,
        })
    };
    let d_bottom = cap(bottom, -frame.normal)?;
    let d_top = cap(top, frame.normal)?;
    let (bottom, top) = (bottom - d_bottom, top + d_top);

    let mut wires: Vec<Wire> = Vec::new();
    for points in std::iter::once(&region.outer).chain(&region.holes) {
        let mut offsets = Vec::with_capacity(points.len());
        for i in 0..points.len() {
            let (a, b) = edge(points, i);
            let corners = [
                frame.at(a, bottom),
                frame.at(b, bottom),
                frame.at(a, top),
                frame.at(b, top),
            ];
            let normal = side_normal(frame, a, b);
            let d = flush_offset(planes, op, normal, &corners, tol);
            if d == 0.0 {
                offsets.push(0.0);
                continue;
            }
            // The wall shrunk by `inset` along the edge and in height.
            let along = (corners[1] - corners[0]).normalize() * inset;
            let up = frame.normal * inset;
            let side = normal * d.signum();
            let shift = side * gap;
            let quad = [
                corners[0] + along + up + shift,
                corners[1] - along + up + shift,
                corners[3] - along - up + shift,
                corners[2] + along - up + shift,
            ];
            let ok = match prism(&quad, &[], side * (d.abs() - gap)) {
                Some(slab) => slab_ok(&slab)?,
                None => false,
            };
            offsets.push(if ok { d } else { 0.0 });
        }
        let moved = offset_loop(points, &offsets);
        let vertices: Vec<_> = moved
            .iter()
            .map(|&p| builder::vertex(frame.at(p, bottom)))
            .collect();
        wires.push(
            (0..vertices.len())
                .map(|i| builder::line(&vertices[i], &vertices[(i + 1) % vertices.len()]))
                .collect(),
        );
    }
    let Ok(face) = builder::try_attach_plane(&wires) else {
        return Ok(None);
    };
    Ok(Some(builder::tsweep(&face, frame.normal * (top - bottom))))
}

/// A planar polygon (with holes) swept along `sweep` into a closed solid,
/// oriented outwards whichever way the polygon winds.
fn prism(outer: &[Point3], holes: &[Vec<Point3>], sweep: Vector3) -> Option<Solid> {
    let wire = |points: &[Point3]| -> Wire {
        let vertices: Vec<_> = points.iter().map(|&p| builder::vertex(p)).collect();
        (0..vertices.len())
            .map(|i| builder::line(&vertices[i], &vertices[(i + 1) % vertices.len()]))
            .collect()
    };
    let wires: Vec<Wire> = std::iter::once(outer)
        .chain(holes.iter().map(Vec::as_slice))
        .map(wire)
        .collect();
    let mut face = builder::try_attach_plane(&wires).ok()?;
    let Surface::Plane(plane) = face.oriented_surface() else {
        return None;
    };
    if plane.normal().dot(sweep) < 0.0 {
        face.invert();
    }
    Some(builder::tsweep(&face, sweep))
}

/// Outward normal of the side wall built on edge `a -> b`. Loops keep the
/// material on their left, so the outward side is the right.
fn side_normal(frame: &Frame, a: Point2, b: Point2) -> Vector3 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    (frame.u * dy - frame.v * dx).normalize()
}

fn flush_offset(
    planes: &[TargetPlane],
    op: Option<BodyOperation>,
    normal: Vector3,
    corners: &[Point3],
    tol: Tolerance,
) -> f64 {
    let eps = tol.linear;
    let flush = tol.flush_offset();
    let (min, max) = bounds(corners);
    for plane in planes {
        let alignment = normal.dot(plane.normal);
        let coplanar = alignment.abs() > 1.0 - 1e-9
            && corners
                .iter()
                .all(|c| (c - plane.point).dot(plane.normal).abs() <= eps);
        let overlapping =
            (0..3).all(|k| min[k] <= plane.max[k] + eps && plane.min[k] <= max[k] + eps);
        if coplanar && overlapping {
            let same_direction = alignment > 0.0;
            let grow = match op {
                Some(BodyOperation::Cut) => same_direction,
                Some(BodyOperation::Join) => !same_direction,
                _ => return 0.0,
            };
            // The caller checks the move against the real bodies.
            return if grow { flush } else { -flush };
        }
    }
    0.0
}

/// Moves each edge of a closed loop along its outward normal by its offset
/// and rebuilds the corners where neighbouring edges meet.
fn offset_loop(points: &[Point2], offsets: &[f64]) -> Vec<Point2> {
    if offsets.iter().all(|&d| d == 0.0) {
        return points.to_vec();
    }
    let n = points.len();
    let line = |i: usize| {
        let (a, b) = edge(points, i);
        let dir = [b[0] - a[0], b[1] - a[1]];
        let len = dir[0].hypot(dir[1]);
        let normal = [dir[1] / len, -dir[0] / len];
        let p = [a[0] + normal[0] * offsets[i], a[1] + normal[1] * offsets[i]];
        (p, dir, normal)
    };
    (0..n)
        .flat_map(|i| {
            let (p0, d0, n0) = line((i + n - 1) % n);
            let (p1, d1, _) = line(i);
            let det = d0[0] * d1[1] - d0[1] * d1[0];
            if det.abs() < 1e-12 {
                // Collinear neighbours: move the shared point with them. If
                // they moved by different amounts, keep both positions and
                // join them with a short step.
                let (d0, d1) = (offsets[(i + n - 1) % n], offsets[i]);
                let p = points[i];
                if d0 == d1 {
                    vec![[p[0] + n0[0] * d0, p[1] + n0[1] * d0]]
                } else {
                    vec![
                        [p[0] + n0[0] * d0, p[1] + n0[1] * d0],
                        [p[0] + n0[0] * d1, p[1] + n0[1] * d1],
                    ]
                }
            } else {
                let t = ((p1[0] - p0[0]) * d1[1] - (p1[1] - p0[1]) * d1[0]) / det;
                vec![[p0[0] + d0[0] * t, p0[1] + d0[1] * t]]
            }
        })
        .collect()
}

pub fn target_planes(solid: &Solid) -> Vec<TargetPlane> {
    solid
        .boundaries()
        .iter()
        .flat_map(|shell| shell.face_iter())
        .filter_map(|face| {
            let Surface::Plane(plane) = face.oriented_surface() else {
                return None;
            };
            let corners: Vec<Point3> = face.vertex_iter().map(|v| v.point()).collect();
            let (min, max) = bounds(&corners);
            Some(TargetPlane {
                normal: plane.normal(),
                point: plane.origin(),
                min,
                max,
            })
        })
        .collect()
}

fn bounds(points: &[Point3]) -> (Point3, Point3) {
    let mut min = Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut max = Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in points {
        for k in 0..3 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    (min, max)
}
