//! The display mesh: what a kernel backend hands the viewer, and what the
//! geometry cache stores.
//!
//! It is deliberately kernel-agnostic: truck today, OpenCASCADE behind the
//! kernel facade later, and a cache file read back without any kernel at all
//! all produce the same [`Mesh`]. Every face and edge carries its persistent
//! name, so a pick in the viewport resolves to the same reference a feature
//! would store. Coordinates are millimetres.

mod cache;

pub use cache::{decode, encode, CacheHeader, DecodeError, Limits, LAYOUT_VERSION, MAGIC};

use std::collections::HashSet;

/// Longest face, edge or body name accepted, in bytes.
pub const MAX_NAME_BYTES: usize = 4096;

/// One face of a body: a run of whole triangles in [`Mesh::indices`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceRange {
    /// Persistent face name in its canonical text form. Unique in the mesh.
    pub name: String,
    /// Index into [`Mesh::bodies`].
    pub body: u32,
    /// First index (not triangle) of the face in [`Mesh::indices`].
    pub first_index: u32,
    /// Number of indices; a multiple of 3.
    pub index_count: u32,
}

/// One B-rep edge: a polyline run in [`Mesh::edge_points`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeRun {
    /// Persistent edge name in its canonical text form. Unique in the mesh.
    pub name: String,
    pub body: u32,
    pub first_point: u32,
    /// At least 2.
    pub point_count: u32,
}

/// Triangulated bodies with named faces and edges.
///
/// Invariants (checked by [`Mesh::validate`] and by every decode):
/// - `normals` has one entry per position; all floats are finite.
/// - `indices` are triangles referring to existing positions.
/// - `faces` tile `indices` in order with no gaps or overlap, and every
///   vertex belongs to exactly one face (faces don't share vertices, so
///   normals stay sharp at edges and a face can be highlighted alone).
/// - `edges` tile `edge_points` in order, each with at least 2 points.
/// - Body indices are in range; names are non-empty, at most
///   [`MAX_NAME_BYTES`], and unique among faces, among edges and among bodies.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    /// Persistent body ids, in body order.
    pub bodies: Vec<String>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub faces: Vec<FaceRange>,
    pub edge_points: Vec<[f32; 3]>,
    pub edges: Vec<EdgeRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MeshError {
    #[error("{0} normals for {1} positions")]
    NormalCount(usize, usize),
    #[error("a coordinate is not a finite number")]
    NonFinite,
    #[error("index count {0} is not a multiple of 3")]
    PartialTriangle(usize),
    #[error("index {index} points past the {positions} positions")]
    IndexOutOfRange { index: u32, positions: usize },
    #[error("face {0} does not start where the previous one ended")]
    FaceGap(usize),
    #[error("face {0} has no triangles or a partial one")]
    FaceShape(usize),
    #[error("faces cover {covered} of {total} indices")]
    FaceCoverage { covered: u64, total: usize },
    #[error("vertex {0} is used by more than one face")]
    SharedVertex(u32),
    #[error("edge {0} does not start where the previous one ended")]
    EdgeGap(usize),
    #[error("edge {0} has fewer than 2 points")]
    EdgeShape(usize),
    #[error("edges cover {covered} of {total} points")]
    EdgeCoverage { covered: u64, total: usize },
    #[error("body index {0} is out of range")]
    BodyOutOfRange(u32),
    #[error("name {0:?} is empty, too long, or used twice")]
    BadName(String),
    #[error("more than u32::MAX elements")]
    TooLarge,
}

impl Mesh {
    pub fn validate(&self) -> Result<(), MeshError> {
        let n = self.positions.len();
        if u32::try_from(n).is_err()
            || u32::try_from(self.indices.len()).is_err()
            || u32::try_from(self.edge_points.len()).is_err()
            || u32::try_from(self.bodies.len()).is_err()
        {
            return Err(MeshError::TooLarge);
        }
        if self.normals.len() != n {
            return Err(MeshError::NormalCount(self.normals.len(), n));
        }
        let finite = |v: &[[f32; 3]]| v.iter().flatten().all(|c| c.is_finite());
        if !finite(&self.positions) || !finite(&self.normals) || !finite(&self.edge_points) {
            return Err(MeshError::NonFinite);
        }
        if !self.indices.len().is_multiple_of(3) {
            return Err(MeshError::PartialTriangle(self.indices.len()));
        }
        if let Some(&index) = self.indices.iter().find(|&&i| i as usize >= n) {
            return Err(MeshError::IndexOutOfRange {
                index,
                positions: n,
            });
        }

        names(self.bodies.iter())?;
        names(self.faces.iter().map(|f| &f.name))?;
        names(self.edges.iter().map(|e| &e.name))?;
        let body_ok = |b: u32| {
            if (b as usize) < self.bodies.len() {
                Ok(())
            } else {
                Err(MeshError::BodyOutOfRange(b))
            }
        };

        // Faces tile the index buffer; each vertex has one owning face.
        let mut next = 0u64;
        let mut owner = vec![u32::MAX; n];
        for (i, face) in self.faces.iter().enumerate() {
            body_ok(face.body)?;
            if u64::from(face.first_index) != next {
                return Err(MeshError::FaceGap(i));
            }
            if face.index_count == 0 || !face.index_count.is_multiple_of(3) {
                return Err(MeshError::FaceShape(i));
            }
            let end = next + u64::from(face.index_count);
            if end > self.indices.len() as u64 {
                return Err(MeshError::FaceCoverage {
                    covered: end,
                    total: self.indices.len(),
                });
            }
            for &v in &self.indices[next as usize..end as usize] {
                let slot = &mut owner[v as usize];
                if *slot != u32::MAX && *slot != i as u32 {
                    return Err(MeshError::SharedVertex(v));
                }
                *slot = i as u32;
            }
            next = end;
        }
        if next != self.indices.len() as u64 {
            return Err(MeshError::FaceCoverage {
                covered: next,
                total: self.indices.len(),
            });
        }

        let mut next = 0u64;
        for (i, edge) in self.edges.iter().enumerate() {
            body_ok(edge.body)?;
            if u64::from(edge.first_point) != next {
                return Err(MeshError::EdgeGap(i));
            }
            if edge.point_count < 2 {
                return Err(MeshError::EdgeShape(i));
            }
            next += u64::from(edge.point_count);
        }
        if next != self.edge_points.len() as u64 {
            return Err(MeshError::EdgeCoverage {
                covered: next,
                total: self.edge_points.len(),
            });
        }
        Ok(())
    }

    /// Axis-aligned bounds of every position and edge point, or `None` for
    /// an empty mesh.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut points = self.positions.iter().chain(&self.edge_points);
        let first = *points.next()?;
        Some(points.fold((first, first), |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])],
                [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])],
            )
        }))
    }

    /// For each position, the face that owns it. Only meaningful on a valid
    /// mesh; positions no triangle uses get `u32::MAX`.
    pub fn vertex_faces(&self) -> Vec<u32> {
        let mut owner = vec![u32::MAX; self.positions.len()];
        for (i, face) in self.faces.iter().enumerate() {
            let range = face.first_index as usize..(face.first_index + face.index_count) as usize;
            for &v in self.indices.get(range).unwrap_or(&[]) {
                if let Some(slot) = owner.get_mut(v as usize) {
                    *slot = i as u32;
                }
            }
        }
        owner
    }

    /// The face hit first by the ray `origin + t * dir` (t >= 0), with its t.
    pub fn pick(&self, origin: [f32; 3], dir: [f32; 3]) -> Option<(usize, f32)> {
        let mut best: Option<(usize, f32)> = None;
        for (i, face) in self.faces.iter().enumerate() {
            let range = face.first_index as usize..(face.first_index + face.index_count) as usize;
            let Some(tris) = self.indices.get(range) else {
                continue;
            };
            for tri in tris.chunks_exact(3) {
                let p = |k: usize| self.positions.get(tri[k] as usize).copied();
                let (Some(a), Some(b), Some(c)) = (p(0), p(1), p(2)) else {
                    continue;
                };
                if let Some(t) = ray_triangle(origin, dir, a, b, c) {
                    if best.is_none_or(|(_, bt)| t < bt) {
                        best = Some((i, t));
                    }
                }
            }
        }
        best
    }
}

fn names<'a>(names: impl Iterator<Item = &'a String>) -> Result<(), MeshError> {
    let mut seen = HashSet::new();
    for name in names {
        if name.is_empty() || name.len() > MAX_NAME_BYTES || !seen.insert(name.as_str()) {
            return Err(MeshError::BadName(name.clone()));
        }
    }
    Ok(())
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Möller–Trumbore, two-sided.
fn ray_triangle(o: [f32; 3], d: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Option<f32> {
    let (e1, e2) = (sub(b, a), sub(c, a));
    let p = cross(d, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = sub(o, a);
    let u = dot(s, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(d, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(e2, q) * inv;
    (t >= 0.0).then_some(t)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A unit cube, 6 faces of 2 triangles with their own vertices, and its
    /// 12 edges.
    pub fn cube() -> Mesh {
        let mut m = Mesh {
            bodies: vec!["body-1".into()],
            ..Mesh::default()
        };
        let quads: [([f32; 3], [[f32; 3]; 4]); 6] = [
            (
                [0., 0., -1.],
                [[0., 0., 0.], [0., 1., 0.], [1., 1., 0.], [1., 0., 0.]],
            ),
            (
                [0., 0., 1.],
                [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]],
            ),
            (
                [0., -1., 0.],
                [[0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 1.]],
            ),
            (
                [0., 1., 0.],
                [[0., 1., 0.], [0., 1., 1.], [1., 1., 1.], [1., 1., 0.]],
            ),
            (
                [-1., 0., 0.],
                [[0., 0., 0.], [0., 0., 1.], [0., 1., 1.], [0., 1., 0.]],
            ),
            (
                [1., 0., 0.],
                [[1., 0., 0.], [1., 1., 0.], [1., 1., 1.], [1., 0., 1.]],
            ),
        ];
        for (f, (normal, corners)) in quads.iter().enumerate() {
            let base = m.positions.len() as u32;
            m.positions.extend(corners);
            m.normals.extend([*normal; 4]);
            let first = m.indices.len() as u32;
            m.indices
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            m.faces.push(FaceRange {
                name: format!("face-{f}"),
                body: 0,
                first_index: first,
                index_count: 6,
            });
        }
        for (e, (a, b)) in [
            ([0., 0., 0.], [1., 0., 0.]),
            ([1., 0., 0.], [1., 1., 0.]),
            ([1., 1., 0.], [0., 1., 0.]),
            ([0., 1., 0.], [0., 0., 0.]),
        ]
        .into_iter()
        .enumerate()
        {
            for z in [0.0f32, 1.0] {
                let first = m.edge_points.len() as u32;
                let lift = |p: [f32; 3]| [p[0], p[1], z];
                m.edge_points.extend([lift(a), lift(b)]);
                m.edges.push(EdgeRun {
                    name: format!("edge-{e}-{z}"),
                    body: 0,
                    first_point: first,
                    point_count: 2,
                });
            }
            let first = m.edge_points.len() as u32;
            m.edge_points.extend([a, [a[0], a[1], 1.0]]);
            m.edges.push(EdgeRun {
                name: format!("edge-{e}-v"),
                body: 0,
                first_point: first,
                point_count: 2,
            });
        }
        m
    }

    #[test]
    fn cube_is_valid() {
        let m = cube();
        assert_eq!(m.validate(), Ok(()));
        assert_eq!(m.bounds(), Some(([0.0; 3], [1.0; 3])));
        let owners = m.vertex_faces();
        assert_eq!(owners[0], 0);
        assert_eq!(owners[23], 5);
        assert_eq!(Mesh::default().validate(), Ok(()));
        assert_eq!(Mesh::default().bounds(), None);
    }

    #[test]
    fn rejects_broken_meshes() {
        let check = |f: fn(&mut Mesh), want: fn(&MeshError) -> bool| {
            let mut m = cube();
            f(&mut m);
            let err = m.validate().expect_err("should be invalid");
            assert!(want(&err), "{err:?}");
        };
        check(
            |m| {
                m.normals.pop();
            },
            |e| matches!(e, MeshError::NormalCount(..)),
        );
        check(
            |m| m.positions[3][1] = f32::NAN,
            |e| *e == MeshError::NonFinite,
        );
        check(
            |m| m.edge_points[0][0] = f32::INFINITY,
            |e| *e == MeshError::NonFinite,
        );
        check(
            |m| {
                m.indices.pop();
            },
            |e| matches!(e, MeshError::PartialTriangle(_)),
        );
        check(
            |m| m.indices[4] = 24,
            |e| matches!(e, MeshError::IndexOutOfRange { .. }),
        );
        check(
            |m| m.faces[2].first_index += 3,
            |e| matches!(e, MeshError::FaceGap(2)),
        );
        check(
            |m| m.faces[5].index_count = 4,
            |e| matches!(e, MeshError::FaceShape(5)),
        );
        check(
            |m| {
                m.faces.pop();
            },
            |e| matches!(e, MeshError::FaceCoverage { .. }),
        );
        check(
            |m| m.faces[5].index_count = 9,
            |e| matches!(e, MeshError::FaceCoverage { .. }),
        );
        check(
            |m| m.indices[6] = 0,
            |e| matches!(e, MeshError::SharedVertex(0)),
        );
        check(
            |m| m.edges[1].first_point = 0,
            |e| matches!(e, MeshError::EdgeGap(1)),
        );
        check(
            |m| {
                m.edges[0].point_count = 1;
            },
            |e| matches!(e, MeshError::EdgeShape(0)),
        );
        check(
            |m| {
                m.edges.pop();
            },
            |e| matches!(e, MeshError::EdgeCoverage { .. }),
        );
        check(
            |m| m.faces[0].body = 1,
            |e| matches!(e, MeshError::BodyOutOfRange(1)),
        );
        check(
            |m| m.edges[3].body = 7,
            |e| matches!(e, MeshError::BodyOutOfRange(7)),
        );
        check(
            |m| m.faces[1].name = "face-0".into(),
            |e| matches!(e, MeshError::BadName(_)),
        );
        check(
            |m| m.edges[1].name.clear(),
            |e| matches!(e, MeshError::BadName(_)),
        );
        check(
            |m| m.bodies[0] = "x".repeat(MAX_NAME_BYTES + 1),
            |e| matches!(e, MeshError::BadName(_)),
        );
    }

    #[test]
    fn picks_the_nearest_face() {
        let m = cube();
        // From above, straight down: the top face (+Z, index 1).
        let (face, t) = m.pick([0.3, 0.6, 5.0], [0.0, 0.0, -1.0]).unwrap();
        assert_eq!(m.faces[face].name, "face-1");
        assert!((t - 4.0).abs() < 1e-5);
        // From the side.
        let (face, _) = m.pick([-3.0, 0.5, 0.5], [1.0, 0.0, 0.0]).unwrap();
        assert_eq!(m.faces[face].name, "face-4");
        // Missing, and pointing away.
        assert_eq!(m.pick([3.0, 3.0, 5.0], [0.0, 0.0, -1.0]), None);
        assert_eq!(m.pick([0.5, 0.5, 5.0], [0.0, 0.0, 1.0]), None);
    }
}
