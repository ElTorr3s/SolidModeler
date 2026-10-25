//! Turns rebuilt B-rep bodies into GPU-ready triangles and edge lines.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use sm_document::FeatureId;
use sm_kernel::Model;
use std::collections::HashSet;
use truck_meshalgo::prelude::*;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// Index into [`SceneMesh::bodies`], for highlighting.
    pub body: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LineVertex {
    pub pos: [f32; 3],
    pub body: u32,
}

/// Everything the viewport draws for one rebuilt model.
#[derive(Debug, Default)]
pub struct SceneMesh {
    /// Unindexed triangle list.
    pub triangles: Vec<Vertex>,
    /// Line list: B-rep edges as polylines, two vertices per segment.
    pub edges: Vec<LineVertex>,
    /// The feature that created each body.
    pub bodies: Vec<FeatureId>,
    pub center: Vec3,
    pub radius: f32,
}

/// Chord tolerance as a fraction of the model's bounding-box diagonal.
const RELATIVE_TOLERANCE: f64 = 1e-3;

pub fn tessellate(model: &Model) -> SceneMesh {
    let mut scene = SceneMesh::default();
    let (min, max) = bounds(model);
    if min.x > max.x {
        return scene;
    }
    scene.center = (min + max) / 2.0;
    scene.radius = ((max - min).length() / 2.0).max(1e-3);
    let tol = (2.0 * scene.radius as f64 * RELATIVE_TOLERANCE).max(1e-6);

    for (index, body) in model.bodies.iter().enumerate() {
        let index = index as u32;
        scene.bodies.push(body.feature);
        let meshed = body.solid.triangulation(tol);
        let polygon = meshed.to_polygon();
        let positions = polygon.positions();
        let normals = polygon.normals();
        for tri in polygon.faces().triangle_iter() {
            let p = tri.map(|v| to_vec(positions[v.pos]));
            let flat = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
            if flat == Vec3::ZERO {
                continue; // degenerate sliver
            }
            for (v, pos) in tri.iter().zip(p) {
                let normal = v
                    .nor
                    .and_then(|i| normals.get(i))
                    .map(|n| Vec3::new(n.x as f32, n.y as f32, n.z as f32).normalize_or_zero())
                    .filter(|n| *n != Vec3::ZERO)
                    .unwrap_or(flat);
                scene.triangles.push(Vertex {
                    pos: pos.into(),
                    normal: normal.into(),
                    body: index,
                });
            }
        }

        // A shell lists each edge once per adjacent face; draw it once.
        let mut seen = HashSet::new();
        for shell in meshed.boundaries() {
            for edge in shell.edge_iter() {
                if !seen.insert(edge.id()) {
                    continue;
                }
                let polyline = edge.curve();
                for pair in polyline.0.windows(2) {
                    for p in pair {
                        scene.edges.push(LineVertex {
                            pos: to_vec(*p).into(),
                            body: index,
                        });
                    }
                }
            }
        }
    }
    scene
}

fn to_vec(p: Point3) -> Vec3 {
    Vec3::new(p.x as f32, p.y as f32, p.z as f32)
}

/// Bounding box of every B-rep vertex. Exact for the planar solids the
/// kernel makes today; curved faces can bulge past their vertices, so the
/// viewer pads the fit.
fn bounds(model: &Model) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for body in &model.bodies {
        for shell in body.solid.boundaries() {
            for v in shell.vertex_iter() {
                let p = to_vec(v.point());
                min = min.min(p);
                max = max.max(p);
            }
        }
    }
    (min, max)
}

impl SceneMesh {
    /// The body hit first by a ray, if any.
    pub fn pick(&self, origin: Vec3, dir: Vec3) -> Option<u32> {
        let mut best: Option<(f32, u32)> = None;
        for tri in self.triangles.chunks_exact(3) {
            let [a, b, c] = [tri[0].pos, tri[1].pos, tri[2].pos].map(Vec3::from);
            if let Some(t) = ray_triangle(origin, dir, a, b, c) {
                if best.is_none_or(|(bt, _)| t < bt) {
                    best = Some((t, tri[0].body));
                }
            }
        }
        best.map(|(_, body)| body)
    }
}

/// Möller–Trumbore, two-sided.
fn ray_triangle(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t >= 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    #[test]
    fn demo_part_tessellates_with_outward_normals() {
        let model = sm_kernel::rebuild(&demo::document()).unwrap();
        let scene = tessellate(&model);
        assert_eq!(scene.bodies.len(), 1);
        assert!(scene.triangles.len() >= 3 * 12);
        assert_eq!(scene.edges.len() % 2, 0);
        // 40 x 20 x 10 block: centered at (20, 10, 5).
        assert!(scene.center.distance(Vec3::new(20.0, 10.0, 5.0)) < 1e-3);

        // Outward normals: every normal points away from the solid, so a
        // closed solid's normals, weighted by area, sum to ~zero, and the
        // divergence-theorem volume is positive.
        let mut volume = 0.0;
        for tri in scene.triangles.chunks_exact(3) {
            let [a, b, c] = [tri[0].pos, tri[1].pos, tri[2].pos].map(Vec3::from);
            volume += a.dot(b.cross(c)) / 6.0;
        }
        // Block minus pocket (20x10x5) minus notch (8x20x4).
        let expected = 40.0 * 20.0 * 10.0 - 20.0 * 10.0 * 5.0 - 8.0 * 20.0 * 4.0;
        assert!((volume - expected).abs() < 1.0, "volume {volume}");
    }

    #[test]
    fn picks_nearest_body() {
        let model = sm_kernel::rebuild(&demo::document()).unwrap();
        let scene = tessellate(&model);
        let hit = scene.pick(Vec3::new(2.0, 2.0, 100.0), Vec3::NEG_Z);
        assert_eq!(hit, Some(0));
        assert_eq!(scene.pick(Vec3::new(-50.0, 2.0, 100.0), Vec3::NEG_Z), None);
    }
}
