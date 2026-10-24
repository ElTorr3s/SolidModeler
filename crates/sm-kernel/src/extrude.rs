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
//! Either way the final shape is the same as an exact boolean would give.
//! Faces are only pulled back when they lie entirely on the target face;
//! an overhanging flush face is left alone and the boolean may still fail.

use crate::profile::{edge, Point2, Region};
use crate::LINEAR_TOLERANCE;
use sm_document::BodyOperation;
use truck_modeling::{builder, InnerSpace, Point3, Solid, Surface, Vector3, Wire};

/// How far flush faces are moved. Must be well above the boolean tolerance so
/// truck sees two distinct planes.
pub const FLUSH_OFFSET: f64 = 4.0 * crate::BOOLEAN_TOLERANCE;

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
struct TargetPlane {
    normal: Vector3,
    point: Point3,
    min: Point3,
    max: Point3,
}

/// Extrudes `region` from `start` to `start + length` along the frame
/// normal (`length > 0`), adjusted for flush faces of `target`.
pub fn tool(
    frame: &Frame,
    region: &Region,
    start: f64,
    length: f64,
    target: Option<(&Solid, BodyOperation)>,
) -> Option<Solid> {
    let planes = match target {
        Some((solid, BodyOperation::Join | BodyOperation::Cut)) => target_planes(solid),
        _ => Vec::new(),
    };
    let op = target.map(|(_, op)| op);
    let (bottom, top) = (start, start + length);

    // Offsets along each face's outward normal; positive grows the tool.
    let all_points = || region.outer.iter().chain(region.holes.iter().flatten());
    let cap = |height: f64, normal: Vector3| {
        let corners: Vec<_> = all_points().map(|&p| frame.at(p, height)).collect();
        flush_offset(&planes, op, normal, &corners)
    };
    let d_bottom = cap(bottom, -frame.normal);
    let d_top = cap(top, frame.normal);
    let (bottom, top) = (bottom - d_bottom, top + d_top);

    let mut wires: Vec<Wire> = Vec::new();
    for points in std::iter::once(&region.outer).chain(&region.holes) {
        let offsets: Vec<f64> = (0..points.len())
            .map(|i| {
                let (a, b) = edge(points, i);
                let corners = [
                    frame.at(a, bottom),
                    frame.at(b, bottom),
                    frame.at(a, top),
                    frame.at(b, top),
                ];
                flush_offset(&planes, op, side_normal(frame, a, b), &corners)
            })
            .collect();
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
    let face = builder::try_attach_plane(&wires).ok()?;
    Some(builder::tsweep(&face, frame.normal * (top - bottom)))
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
) -> f64 {
    let (min, max) = bounds(corners);
    for plane in planes {
        let alignment = normal.dot(plane.normal);
        let coplanar = alignment.abs() > 1.0 - 1e-9
            && corners
                .iter()
                .all(|c| (c - plane.point).dot(plane.normal).abs() <= LINEAR_TOLERANCE);
        let overlapping = (0..3).all(|k| {
            min[k] <= plane.max[k] + LINEAR_TOLERANCE && plane.min[k] <= max[k] + LINEAR_TOLERANCE
        });
        if coplanar && overlapping {
            let same_direction = alignment > 0.0;
            let grow = match op {
                Some(BodyOperation::Cut) => same_direction,
                Some(BodyOperation::Join) => !same_direction,
                _ => return 0.0,
            };
            if grow {
                return FLUSH_OFFSET;
            }
            // Shrinking is only safe where the whole tool face sits on the
            // target face. If it overhangs, shrinking would change the
            // result, so leave it and let the boolean report the failure.
            let inside = (0..3).all(|k| {
                plane.min[k] - LINEAR_TOLERANCE <= min[k]
                    && max[k] <= plane.max[k] + LINEAR_TOLERANCE
            });
            return if inside { -FLUSH_OFFSET } else { 0.0 };
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
        .map(|i| {
            let (p0, d0, n0) = line((i + n - 1) % n);
            let (p1, d1, _) = line(i);
            let det = d0[0] * d1[1] - d0[1] * d1[0];
            if det.abs() < 1e-12 {
                // Collinear neighbours: both edges moved by the same normal.
                let d = offsets[(i + n - 1) % n];
                [points[i][0] + n0[0] * d, points[i][1] + n0[1] * d]
            } else {
                let t = ((p1[0] - p0[0]) * d1[1] - (p1[1] - p0[1]) * d1[0]) / det;
                [p0[0] + d0[0] * t, p0[1] + d0[1] * t]
            }
        })
        .collect()
}

fn target_planes(solid: &Solid) -> Vec<TargetPlane> {
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
