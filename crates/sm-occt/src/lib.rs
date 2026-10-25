//! OpenCascade backend for the SolidModeller kernel.
//!
//! Safe Rust over a small C++ shim (`cpp/shim.cpp`) that links the system
//! OpenCascade. No OpenCascade type appears in this API: shapes are opaque
//! [`Shape`] handles, sub-shapes are addressed by kind and index in the
//! shape's own indexed map ([`SubShape`]), and every operation that can
//! fail returns [`OcctError`] (OpenCascade exceptions are caught in C++ and
//! never unwind into Rust).
//!
//! Each modelling operation returns an [`Op`]: the result shape plus a
//! [`History`] saying, for every vertex, edge and face of each argument,
//! whether it was kept, modified, generated from, or deleted. The kernel
//! facade turns that into persistent names.
//!
//! STEP import never runs OpenCascade on untrusted bytes in this process:
//! see [`step::import_step`], which screens the file in Rust and then reads
//! it in a sandboxed worker process.

mod ffi;
mod history;
pub mod sandbox;
pub mod step;

pub use history::{History, Relation, Source};

use cxx::UniquePtr;
use ffi::ffi as sys;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum OcctError {
    /// OpenCascade rejected the operation or threw; the feature is sick but
    /// the program carries on.
    #[error("{0}")]
    Kernel(String),
    #[error("invalid input: {0}")]
    Input(String),
    #[error("STEP import: {0}")]
    Step(String),
}

pub type Result<T> = std::result::Result<T, OcctError>;

fn kernel(e: cxx::Exception) -> OcctError {
    OcctError::Kernel(e.what().to_string())
}

/// The OpenCascade version this build is linked against, e.g. "7.6.3".
/// Stored next to cached geometry, since versions can rebuild differently.
pub fn occt_version() -> String {
    sys::occt_version()
}

/// Sub-shape kinds that can be addressed and tracked through history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SubKind {
    Vertex = 0,
    Edge = 1,
    Face = 2,
    Solid = 3,
}

impl SubKind {
    fn from_u8(v: u8) -> Option<SubKind> {
        match v {
            0 => Some(SubKind::Vertex),
            1 => Some(SubKind::Edge),
            2 => Some(SubKind::Face),
            3 => Some(SubKind::Solid),
            _ => None,
        }
    }
}

/// A vertex, edge, face or solid of a particular shape: its position in
/// that shape's indexed map. Only meaningful together with that shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SubShape {
    pub kind: SubKind,
    pub index: u32,
}

impl SubShape {
    pub fn face(index: u32) -> Self {
        SubShape {
            kind: SubKind::Face,
            index,
        }
    }
    pub fn edge(index: u32) -> Self {
        SubShape {
            kind: SubKind::Edge,
            index,
        }
    }
    pub fn vertex(index: u32) -> Self {
        SubShape {
            kind: SubKind::Vertex,
            index,
        }
    }
}

/// What kind of top-level shape a [`Shape`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeType {
    Compound,
    CompSolid,
    Solid,
    Shell,
    Face,
    Wire,
    Edge,
    Vertex,
}

/// An OpenCascade shape. Cloning is cheap and shares the underlying
/// geometry, which operations never modify in place.
pub struct Shape {
    raw: UniquePtr<sys::Shape>,
}

// OpenCascade shapes are immutable once built and their reference counts
// are atomic (OCCT is built with thread-safe handles), so a handle can move
// between threads. Concurrent operations on shared shapes are not promised
// to be safe, so Shape is not Sync.
unsafe impl Send for Shape {}

impl Clone for Shape {
    fn clone(&self) -> Self {
        Shape {
            raw: sys::shape_clone(self.inner()),
        }
    }
}

impl std::fmt::Debug for Shape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Shape({:?}, {} faces)",
            self.shape_type().ok(),
            self.count(SubKind::Face)
        )
    }
}

impl Shape {
    fn wrap(raw: UniquePtr<sys::Shape>) -> Result<Shape> {
        if raw.is_null() || sys::shape_is_null(&raw) {
            return Err(OcctError::Kernel("operation produced no shape".into()));
        }
        Ok(Shape { raw })
    }

    fn inner(&self) -> &sys::Shape {
        // `wrap` guarantees a non-null pointer.
        self.raw.as_ref().expect("Shape is never null")
    }

    pub fn shape_type(&self) -> Result<ShapeType> {
        let t = sys::shape_type(self.inner()).map_err(kernel)?;
        Ok(match t {
            0 => ShapeType::Compound,
            1 => ShapeType::CompSolid,
            2 => ShapeType::Solid,
            3 => ShapeType::Shell,
            4 => ShapeType::Face,
            5 => ShapeType::Wire,
            6 => ShapeType::Edge,
            _ => ShapeType::Vertex,
        })
    }

    /// Number of distinct sub-shapes of `kind`.
    pub fn count(&self, kind: SubKind) -> u32 {
        sys::shape_count(self.inner(), kind as u8).unwrap_or(0)
    }

    /// Every sub-shape of `kind`, in index order.
    pub fn subs(&self, kind: SubKind) -> Vec<SubShape> {
        (0..self.count(kind))
            .map(|index| SubShape { kind, index })
            .collect()
    }

    pub fn faces(&self) -> Vec<SubShape> {
        self.subs(SubKind::Face)
    }
    pub fn edges(&self) -> Vec<SubShape> {
        self.subs(SubKind::Edge)
    }
    pub fn vertices(&self) -> Vec<SubShape> {
        self.subs(SubKind::Vertex)
    }

    fn check(&self, sub: SubShape) -> Result<()> {
        if sub.index < self.count(sub.kind) {
            Ok(())
        } else {
            Err(OcctError::Input(format!(
                "{:?} {} out of range ({} in shape)",
                sub.kind,
                sub.index,
                self.count(sub.kind)
            )))
        }
    }

    /// True when `a` of this shape and `b` of `other` are the same
    /// topological entity (same underlying shape and placement).
    pub fn same(&self, a: SubShape, other: &Shape, b: SubShape) -> Result<bool> {
        self.check(a)?;
        other.check(b)?;
        sys::shape_same(
            self.inner(),
            a.kind as u8,
            a.index,
            other.inner(),
            b.kind as u8,
            b.index,
        )
        .map_err(kernel)
    }

    /// The sub-shape as a shape of its own, e.g. a face to use as a profile.
    pub fn sub(&self, sub: SubShape) -> Result<Shape> {
        self.check(sub)?;
        Shape::wrap(sys::shape_sub(self.inner(), sub.kind as u8, sub.index).map_err(kernel)?)
    }

    pub fn is_valid(&self) -> bool {
        sys::check_valid(self.inner()).unwrap_or(false)
    }

    pub fn mass_props(&self) -> Result<MassProps> {
        let m = sys::mass_props(self.inner()).map_err(kernel)?;
        Ok(MassProps {
            volume: m.volume,
            area: m.area,
            centroid: [m.cx, m.cy, m.cz],
        })
    }

    pub fn bounding_box(&self) -> Result<BoundingBox> {
        let b = sys::bounding_box(self.inner()).map_err(kernel)?;
        Ok(BoundingBox {
            min: [b.xmin, b.ymin, b.zmin],
            max: [b.xmax, b.ymax, b.zmax],
        })
    }

    pub fn face_info(&self, face: SubShape) -> Result<FaceInfo> {
        if face.kind != SubKind::Face {
            return Err(OcctError::Input("face_info needs a face".into()));
        }
        self.check(face)?;
        let f = sys::face_info(self.inner(), face.index).map_err(kernel)?;
        let surface = SurfaceKind::from_u8(f.kind);
        Ok(FaceInfo {
            surface,
            plane: (surface == SurfaceKind::Plane).then_some(Plane {
                origin: [f.ox, f.oy, f.oz],
                normal: [f.nx, f.ny, f.nz],
            }),
            bbox: BoundingBox {
                min: [f.xmin, f.ymin, f.zmin],
                max: [f.xmax, f.ymax, f.zmax],
            },
            area: f.area,
        })
    }

    /// Start and end points of an edge, in its own orientation.
    pub fn edge_ends(&self, edge: SubShape) -> Result<[[f64; 3]; 2]> {
        if edge.kind != SubKind::Edge {
            return Err(OcctError::Input("edge_ends needs an edge".into()));
        }
        self.check(edge)?;
        let v = sys::edge_ends(self.inner(), edge.index).map_err(kernel)?;
        match v.as_slice() {
            [a, b, c, d, e, f] => Ok([[*a, *b, *c], [*d, *e, *f]]),
            _ => Err(OcctError::Kernel(
                "edge_ends returned malformed data".into(),
            )),
        }
    }

    /// Triangulates every face. `linear` is the maximum chordal deviation
    /// in model units, `angular` the maximum angle between neighbouring
    /// facets in radians.
    pub fn tessellate(&self, linear: f64, angular: f64) -> Result<Mesh> {
        if !(linear.is_finite() && linear > 0.0 && angular.is_finite() && angular > 0.0) {
            return Err(OcctError::Input(
                "tessellation tolerances must be positive".into(),
            ));
        }
        let m = sys::tessellate(self.inner(), linear, angular).map_err(kernel)?;
        Ok(Mesh {
            positions: m.positions,
            normals: m.normals,
            indices: m.indices,
            face_ranges: m.face_ranges,
        })
    }

    /// Serialises to OpenCascade's binary B-rep format, for caches and the
    /// STEP worker.
    pub fn to_brep(&self) -> Result<Vec<u8>> {
        sys::to_brep(self.inner()).map_err(kernel)
    }

    /// Reads OpenCascade's binary B-rep format. Only for bytes this program
    /// wrote (caches) or that came from the STEP worker and passed its
    /// checks; never for files from outside.
    pub(crate) fn from_brep(bytes: &[u8]) -> Result<Shape> {
        Shape::wrap(sys::from_brep(bytes).map_err(kernel)?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MassProps {
    pub volume: f64,
    pub area: f64,
    pub centroid: [f64; 3],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    Plane,
    Cylinder,
    Cone,
    Sphere,
    Torus,
    Bezier,
    BSpline,
    Revolution,
    Extrusion,
    Offset,
    Other,
}

impl SurfaceKind {
    fn from_u8(v: u8) -> SurfaceKind {
        match v {
            0 => SurfaceKind::Plane,
            1 => SurfaceKind::Cylinder,
            2 => SurfaceKind::Cone,
            3 => SurfaceKind::Sphere,
            4 => SurfaceKind::Torus,
            5 => SurfaceKind::Bezier,
            6 => SurfaceKind::BSpline,
            7 => SurfaceKind::Revolution,
            8 => SurfaceKind::Extrusion,
            9 => SurfaceKind::Offset,
            _ => SurfaceKind::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    pub origin: [f64; 3],
    /// Outward normal of the face (already flipped for reversed faces).
    pub normal: [f64; 3],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceInfo {
    pub surface: SurfaceKind,
    pub plane: Option<Plane>,
    pub bbox: BoundingBox,
    pub area: f64,
}

/// A triangle mesh of a shape.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    /// xyz per vertex.
    pub positions: Vec<f64>,
    /// Unit normal per vertex, pointing out of the solid.
    pub normals: Vec<f64>,
    /// Three vertex indices per triangle, counter-clockwise from outside.
    pub indices: Vec<u32>,
    /// Triangles of face `i` are `face_ranges[i]..face_ranges[i + 1]`.
    pub face_ranges: Vec<u32>,
}

/// The result of a modelling operation.
#[derive(Debug, Clone)]
pub struct Op {
    pub shape: Shape,
    pub history: History,
}

/// A profile face built from polygon loops.
#[derive(Debug, Clone)]
pub struct PolygonFace {
    pub face: Shape,
    /// `segments[l][k]` is the face's edge from point `k` to point `k + 1`
    /// of loop `l` (the last one closes the loop).
    pub segments: Vec<Vec<SubShape>>,
}

/// A prism or revolve, with its end caps when it has them.
#[derive(Debug, Clone)]
pub struct Sweep {
    pub op: Op,
    /// Start and end cap faces of the result; `None` for a full revolve.
    pub first: Option<SubShape>,
    pub last: Option<SubShape>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooleanKind {
    Fuse = 0,
    Cut = 1,
    Common = 2,
}

fn op_from(raw: UniquePtr<sys::OpResult>, args: usize) -> Result<(Op, UniquePtr<sys::OpResult>)> {
    let r = raw
        .as_ref()
        .ok_or_else(|| OcctError::Kernel("operation returned nothing".into()))?;
    let shape = Shape::wrap(sys::op_shape(r))?;
    let history = History::new(&sys::op_history(r), args, &shape)?;
    Ok((Op { shape, history }, raw))
}

fn finite(values: &[f64], what: &str) -> Result<()> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(OcctError::Input(format!("{what}: non-finite value")))
    }
}

fn indices(shape: &Shape, subs: &[SubShape], kind: SubKind) -> Result<Vec<u32>> {
    subs.iter()
        .map(|s| {
            if s.kind != kind {
                return Err(OcctError::Input(format!(
                    "expected a {kind:?}, got a {:?}",
                    s.kind
                )));
            }
            shape.check(*s)?;
            Ok(s.index)
        })
        .collect()
}

/// A rectangular block from the origin to (dx, dy, dz).
pub fn make_box(dx: f64, dy: f64, dz: f64) -> Result<Shape> {
    Shape::wrap(sys::make_box(dx, dy, dz).map_err(kernel)?)
}

/// A cylinder of radius `r` along +Z from the origin, height `h`.
pub fn make_cylinder(r: f64, h: f64) -> Result<Shape> {
    Shape::wrap(sys::make_cylinder(r, h).map_err(kernel)?)
}

/// A planar face from closed polygon loops: the first is the outer
/// boundary (its winding sets the face normal), the rest are holes.
pub fn polygon_face(loops: &[Vec<[f64; 3]>]) -> Result<PolygonFace> {
    let mut points = Vec::new();
    let mut lens = Vec::with_capacity(loops.len());
    for l in loops {
        lens.push(u32::try_from(l.len()).map_err(|_| OcctError::Input("loop too long".into()))?);
        for p in l {
            points.extend_from_slice(p);
        }
    }
    finite(&points, "polygon_face")?;
    let raw = sys::polygon_face(&points, &lens).map_err(kernel)?;
    let (op, raw) = op_from(raw, 0)?;
    let extra = sys::op_extra(raw.as_ref().expect("checked in op_from"));
    let total: usize = loops.iter().map(Vec::len).sum();
    if extra.len() != total {
        return Err(OcctError::Kernel(
            "polygon_face edge map is incomplete".into(),
        ));
    }
    let mut segments = Vec::with_capacity(loops.len());
    let mut at = 0;
    for l in loops {
        segments.push(
            extra[at..at + l.len()]
                .iter()
                .map(|&i| SubShape::edge(i))
                .collect(),
        );
        at += l.len();
    }
    Ok(PolygonFace {
        face: op.shape,
        segments,
    })
}

fn sweep_from(raw: UniquePtr<sys::OpResult>) -> Result<Sweep> {
    let (op, raw) = op_from(raw, 1)?;
    let r = raw.as_ref().expect("checked in op_from");
    let cap = |s: UniquePtr<sys::Shape>| -> Result<Option<SubShape>> {
        let Some(s) = s.as_ref() else { return Ok(None) };
        if sys::shape_is_null(s) {
            return Ok(None);
        }
        let cap = Shape::wrap(sys::shape_clone(s))?;
        for f in op.shape.faces() {
            if op.shape.same(f, &cap, SubShape::face(0))? {
                return Ok(Some(f));
            }
        }
        Ok(None)
    };
    let first = cap(sys::op_first(r))?;
    let last = cap(sys::op_last(r))?;
    Ok(Sweep { op, first, last })
}

/// Extrudes a profile (face, or a compound of faces) along a vector.
pub fn prism(profile: &Shape, vector: [f64; 3]) -> Result<Sweep> {
    finite(&vector, "prism")?;
    let [x, y, z] = vector;
    sweep_from(sys::prism(profile.inner(), x, y, z).map_err(kernel)?)
}

/// Revolves a profile about an axis by `angle` radians; a full turn (or
/// more) makes a closed revolve with no caps.
pub fn revolve(profile: &Shape, origin: [f64; 3], axis: [f64; 3], angle: f64) -> Result<Sweep> {
    finite(&origin, "revolve")?;
    finite(&axis, "revolve")?;
    finite(&[angle], "revolve")?;
    let ([ox, oy, oz], [dx, dy, dz]) = (origin, axis);
    let angle = angle.clamp(-std::f64::consts::TAU, std::f64::consts::TAU);
    sweep_from(sys::revolve(profile.inner(), ox, oy, oz, dx, dy, dz, angle).map_err(kernel)?)
}

/// Fuse, cut or common of `a` with `b`. History argument 0 is `a`, 1 is `b`.
pub fn boolean(kind: BooleanKind, a: &Shape, b: &Shape) -> Result<Op> {
    Ok(op_from(
        sys::boolean_op(kind as u8, a.inner(), b.inner()).map_err(kernel)?,
        2,
    )?
    .0)
}

/// Constant-radius fillet on `edges` of `shape`.
pub fn fillet(shape: &Shape, edges: &[SubShape], radius: f64) -> Result<Op> {
    let idx = indices(shape, edges, SubKind::Edge)?;
    Ok(op_from(sys::fillet(shape.inner(), &idx, radius).map_err(kernel)?, 1)?.0)
}

/// Equal-distance chamfer on `edges` of `shape`.
pub fn chamfer(shape: &Shape, edges: &[SubShape], distance: f64) -> Result<Op> {
    let idx = indices(shape, edges, SubKind::Edge)?;
    Ok(op_from(
        sys::chamfer(shape.inner(), &idx, distance).map_err(kernel)?,
        1,
    )?
    .0)
}

/// Hollows `shape` inwards to wall `thickness`, opening the faces in
/// `remove` (Inventor's Shell).
pub fn shell(shape: &Shape, remove: &[SubShape], thickness: f64) -> Result<Op> {
    let idx = indices(shape, remove, SubKind::Face)?;
    Ok(op_from(
        sys::shell(shape.inner(), &idx, thickness, 1e-4).map_err(kernel)?,
        1,
    )?
    .0)
}

/// Tapers `faces` by `angle` radians relative to the pull `direction`,
/// pivoting about the neutral plane through `plane_origin` with normal
/// `plane_normal` (Inventor's Face Draft).
pub fn draft(
    shape: &Shape,
    faces: &[SubShape],
    direction: [f64; 3],
    angle: f64,
    plane_origin: [f64; 3],
    plane_normal: [f64; 3],
) -> Result<Op> {
    let idx = indices(shape, faces, SubKind::Face)?;
    let ([dx, dy, dz], [px, py, pz], [nx, ny, nz]) = (direction, plane_origin, plane_normal);
    Ok(op_from(
        sys::draft(
            shape.inner(),
            &idx,
            dx,
            dy,
            dz,
            angle,
            px,
            py,
            pz,
            nx,
            ny,
            nz,
        )
        .map_err(kernel)?,
        1,
    )?
    .0)
}

/// Merges faces lying on the same surface and edges on the same curve,
/// e.g. the split faces a coplanar fuse leaves behind.
pub fn unify_same_domain(shape: &Shape) -> Result<Op> {
    Ok(op_from(sys::unify_same_domain(shape.inner()).map_err(kernel)?, 1)?.0)
}

#[cfg(test)]
mod tests;
