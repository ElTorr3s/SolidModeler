//! Turns rebuilt bodies into the kernel-agnostic [`sm_mesh::Mesh`] the
//! viewport draws.
//!
//! This truck adapter is temporary: once the kernel facade emits
//! `sm_mesh::Mesh` itself (with persistent names from sm-naming), this file
//! and the app's truck dependency go away. Until then face and edge names
//! are positional placeholders (`b0.f3`), stable only between identical
//! rebuilds.

use glam::Vec3;
use sm_document::FeatureId;
use sm_kernel::Model;
use sm_mesh::{EdgeRun, FaceRange, Mesh};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use truck_meshalgo::prelude::*;

/// What the viewport shows for one rebuild.
#[derive(Debug, Default)]
pub struct SceneMesh {
    pub mesh: Mesh,
    /// The feature that created each body, by body index.
    pub body_features: Vec<FeatureId>,
    pub center: Vec3,
    pub radius: f32,
}

impl SceneMesh {
    /// Highlight group of a body: the first body its feature created, so
    /// selecting a feature lights up every body it made.
    pub fn group(&self, body: u32) -> u32 {
        let Some(feature) = self.body_features.get(body as usize) else {
            return u32::MAX;
        };
        self.group_of(feature).unwrap_or(u32::MAX)
    }

    pub fn group_of(&self, feature: FeatureId) -> Option<u32> {
        self.body_features
            .iter()
            .position(|&f| f == feature)
            .map(|i| i as u32)
    }
}

/// Chord tolerance as a fraction of the model's bounding-box diagonal.
const RELATIVE_TOLERANCE: f64 = 1e-3;
/// Most triangles a scene may have (about 300 MB of GPU buffers).
pub const MAX_TRIANGLES: usize = 10_000_000;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TessellateError {
    #[error("cancelled")]
    Cancelled,
    #[error("the model needs more than {MAX_TRIANGLES} triangles to display")]
    TooManyTriangles,
    #[error("tessellation produced an invalid mesh: {0}")]
    Invalid(#[from] sm_mesh::MeshError),
}

pub fn tessellate(model: &Model, cancel: &AtomicBool) -> Result<SceneMesh, TessellateError> {
    let to_mm = model.units.to_mm();
    let mut scene = SceneMesh::default();
    let Some((min, max)) = vertex_bounds(model) else {
        return Ok(scene);
    };
    let diagonal = f64::from((max - min).length());
    let tol = (diagonal * RELATIVE_TOLERANCE).max(1e-6);
    let mm = |p: Point3| {
        [
            (p.x * to_mm) as f32,
            (p.y * to_mm) as f32,
            (p.z * to_mm) as f32,
        ]
    };
    let mesh = &mut scene.mesh;

    for (b, body) in model.bodies.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(TessellateError::Cancelled);
        }
        mesh.bodies.push(format!("body{b}"));
        scene.body_features.push(body.feature);
        let meshed = body.solid.triangulation(tol);

        // One face at a time, each with its own vertices, so faces can be
        // picked and highlighted on their own.
        let mut face_index = 0;
        for shell in meshed.boundaries() {
            for face in shell.face_iter() {
                let Some(mut poly) = face.surface() else {
                    continue;
                };
                if !face.orientation() {
                    poly.invert();
                }
                let first_index = mesh.indices.len() as u32;
                let positions = poly.positions();
                let normals = poly.normals();
                for tri in poly.faces().triangle_iter() {
                    let p = tri.map(|v| mm(positions[v.pos]));
                    let [a, b, c] = p.map(Vec3::from);
                    let flat = (b - a).cross(c - a).normalize_or_zero();
                    if flat == Vec3::ZERO {
                        continue; // degenerate sliver
                    }
                    for (v, pos) in tri.iter().zip(p) {
                        let normal = v
                            .nor
                            .and_then(|i| normals.get(i))
                            .map(|n| Vec3::new(n.x as f32, n.y as f32, n.z as f32))
                            .map(|n| n.normalize_or_zero())
                            .filter(|n| *n != Vec3::ZERO)
                            .unwrap_or(flat);
                        mesh.indices.push(mesh.positions.len() as u32);
                        mesh.positions.push(pos);
                        mesh.normals.push(normal.into());
                    }
                }
                if mesh.indices.len() / 3 > MAX_TRIANGLES {
                    return Err(TessellateError::TooManyTriangles);
                }
                let index_count = mesh.indices.len() as u32 - first_index;
                if index_count > 0 {
                    mesh.faces.push(FaceRange {
                        name: format!("b{b}.f{face_index}"),
                        body: b as u32,
                        first_index,
                        index_count,
                    });
                    face_index += 1;
                }
            }
        }

        // A shell lists each edge once per adjacent face; keep one copy.
        let mut seen = HashSet::new();
        for shell in meshed.boundaries() {
            for edge in shell.edge_iter() {
                if !seen.insert(edge.id()) {
                    continue;
                }
                let points = edge.curve().0;
                if points.len() < 2 {
                    continue;
                }
                mesh.edges.push(EdgeRun {
                    name: format!("b{b}.e{}", seen.len() - 1),
                    body: b as u32,
                    first_point: mesh.edge_points.len() as u32,
                    point_count: points.len() as u32,
                });
                mesh.edge_points.extend(points.into_iter().map(mm));
            }
        }
    }
    mesh.validate()?;
    if let Some((lo, hi)) = mesh.bounds() {
        let (lo, hi) = (Vec3::from(lo), Vec3::from(hi));
        scene.center = (lo + hi) / 2.0;
        scene.radius = ((hi - lo).length() / 2.0).max(1e-3);
    }
    Ok(scene)
}

/// Bounds of every B-rep vertex, in model units, for the chord tolerance.
fn vertex_bounds(model: &Model) -> Option<(Vec3, Vec3)> {
    let mut points = model
        .bodies
        .iter()
        .flat_map(|b| b.solid.boundaries())
        .flat_map(|s| s.vertex_iter().map(|v| v.point()))
        .map(|p| Vec3::new(p.x as f32, p.y as f32, p.z as f32));
    let first = points.next()?;
    Some(points.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use sm_document::LengthUnit;

    fn demo_scene() -> SceneMesh {
        let model = sm_kernel::rebuild(&demo::document()).unwrap();
        tessellate(&model, &AtomicBool::new(false)).unwrap()
    }

    /// Signed volume by the divergence theorem: positive only if every
    /// triangle winds outward.
    fn volume(mesh: &Mesh) -> f32 {
        mesh.indices
            .chunks_exact(3)
            .map(|t| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(mesh.positions[i as usize]));
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    #[test]
    fn demo_part_tessellates_with_outward_normals() {
        let scene = demo_scene();
        let mesh = &scene.mesh;
        assert_eq!(mesh.validate(), Ok(()));
        assert_eq!(mesh.bodies.len(), 1);
        assert_eq!(scene.body_features.len(), 1);
        // Block (6 faces) + pocket (5) + notch (2 new faces, 1 shared top
        // face changes shape) gives at least 12 faces.
        assert!(mesh.faces.len() >= 12, "{} faces", mesh.faces.len());
        assert!(mesh.edges.len() >= 18, "{} edges", mesh.edges.len());
        assert!(scene.center.distance(Vec3::new(20.0, 10.0, 5.0)) < 1e-3);

        let expected = 40.0 * 20.0 * 10.0 - 20.0 * 10.0 * 5.0 - 8.0 * 20.0 * 4.0;
        let v = volume(mesh);
        assert!((v - expected).abs() < 1.0, "volume {v}");
    }

    #[test]
    fn coordinates_are_millimetres() {
        let mut doc = demo::document();
        doc.units = LengthUnit::Cm;
        let model = sm_kernel::rebuild(&doc).unwrap();
        let scene = tessellate(&model, &AtomicBool::new(false)).unwrap();
        let (_, hi) = scene.mesh.bounds().unwrap();
        assert!((hi[0] - 400.0).abs() < 1e-3, "{hi:?}");
    }

    #[test]
    fn picks_named_faces() {
        let scene = demo_scene();
        let mesh = &scene.mesh;
        // Straight down onto the pocket floor at z = 5.
        let (face, t) = mesh.pick([15.0, 10.0, 100.0], [0.0, 0.0, -1.0]).unwrap();
        assert!((t - 95.0).abs() < 1e-3);
        assert_eq!(mesh.faces[face].body, 0);
        // The top face around the pocket at z = 10 is a different face.
        let (top, _) = mesh.pick([2.0, 2.0, 100.0], [0.0, 0.0, -1.0]).unwrap();
        assert_ne!(face, top);
        assert_eq!(mesh.pick([-50.0, 2.0, 100.0], [0.0, 0.0, -1.0]), None);
    }

    #[test]
    fn cancel_and_empty() {
        let model = sm_kernel::rebuild(&demo::document()).unwrap();
        let cancelled = tessellate(&model, &AtomicBool::new(true));
        assert_eq!(cancelled.err(), Some(TessellateError::Cancelled));

        let empty = Model {
            units: LengthUnit::Mm,
            bodies: Vec::new(),
        };
        let scene = tessellate(&empty, &AtomicBool::new(false)).unwrap();
        assert_eq!(scene.mesh, Mesh::default());
    }
}
