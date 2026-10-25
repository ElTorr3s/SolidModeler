//! Shared helpers: building documents and inspecting resolved faces.
#![allow(dead_code)]

use sm_document::{
    BodyOperation, Extrude, ExtrudeExtent, FeatureId, FeatureKind, LengthUnit, OriginPlane,
    PartDocument, Profile, Sketch,
};
use sm_naming::{Entities, EntityKind, EntityRef, NamedRebuild, Resolution, TopoName};
use truck_modeling::{Point3, Surface, Vector3};

pub fn rect([x0, y0, x1, y1]: [f64; 4]) -> Profile {
    Profile {
        points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
    }
}

/// Appends a sketch on XY at height `z` and an extrude of it; returns the
/// (sketch, extrude) ids.
pub fn add_extrude(
    doc: &mut PartDocument,
    profiles: Vec<Profile>,
    z: f64,
    height: f64,
    operation: BodyOperation,
) -> (FeatureId, FeatureId) {
    let sketch = doc.add_feature(
        "Sketch",
        FeatureKind::Sketch(Sketch {
            plane: OriginPlane::XY,
            offset: z,
            profiles,
        }),
    );
    let extrude = doc.add_feature(
        "Extrusion",
        FeatureKind::Extrude(Extrude {
            sketch,
            extent: ExtrudeExtent::Distance { distance: height },
            operation,
        }),
    );
    (sketch, extrude)
}

pub fn add_block(
    doc: &mut PartDocument,
    r: [f64; 4],
    z: f64,
    h: f64,
    op: BodyOperation,
) -> (FeatureId, FeatureId) {
    add_extrude(doc, vec![rect(r)], z, h, op)
}

pub fn new_doc() -> PartDocument {
    PartDocument::new("Test", LengthUnit::Mm)
}

pub fn sketch_mut(doc: &mut PartDocument, id: FeatureId) -> &mut Sketch {
    match &mut doc.features.iter_mut().find(|f| f.id == id).unwrap().kind {
        FeatureKind::Sketch(s) => s,
        _ => panic!("not a sketch"),
    }
}

pub fn extrude_mut(doc: &mut PartDocument, id: FeatureId) -> &mut Extrude {
    match &mut doc.features.iter_mut().find(|f| f.id == id).unwrap().kind {
        FeatureKind::Extrude(e) => e,
        _ => panic!("not an extrude"),
    }
}

/// Moves the last `n` features to just before feature `before`.
pub fn move_last_before(doc: &mut PartDocument, n: usize, before: FeatureId) {
    let tail: Vec<_> = doc.features.split_off(doc.features.len() - n);
    let at = doc.features.iter().position(|f| f.id == before).unwrap();
    for (k, f) in tail.into_iter().enumerate() {
        doc.features.insert(at + k, f);
    }
}

/// A planar face as (outward normal, bounding box min, max), rounded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaneFace {
    pub normal: [f64; 3],
    pub min: [f64; 3],
    pub max: [f64; 3],
}

fn r(x: f64) -> f64 {
    let v = (x * 1e4).round() / 1e4;
    if v == 0.0 {
        0.0
    } else {
        v
    }
}

pub fn plane_face(rebuilt: &NamedRebuild, e: EntityRef) -> PlaneFace {
    assert_eq!(e.kind, EntityKind::Face);
    let ent = Entities::of(&rebuilt.model.bodies[e.body].solid);
    let face = &ent.faces[e.index];
    let Surface::Plane(plane) = face.oriented_surface() else {
        panic!("not planar");
    };
    let n: Vector3 = plane.normal();
    let pts: Vec<Point3> = face.vertex_iter().map(|v| v.point()).collect();
    let mut min = [f64::MAX; 3];
    let mut max = [f64::MIN; 3];
    for p in &pts {
        for k in 0..3 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    PlaneFace {
        normal: [r(n.x), r(n.y), r(n.z)],
        min: min.map(r),
        max: max.map(r),
    }
}

/// End points of an edge, rounded and sorted.
pub fn edge_points(rebuilt: &NamedRebuild, e: EntityRef) -> [[f64; 3]; 2] {
    assert_eq!(e.kind, EntityKind::Edge);
    let ent = Entities::of(&rebuilt.model.bodies[e.body].solid);
    let edge = &ent.edges[e.index];
    let p = |v: Point3| [r(v.x), r(v.y), r(v.z)];
    let mut pts = [p(edge.front().point()), p(edge.back().point())];
    pts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    pts
}

pub fn vertex_point(rebuilt: &NamedRebuild, e: EntityRef) -> [f64; 3] {
    assert_eq!(e.kind, EntityKind::Vertex);
    let ent = Entities::of(&rebuilt.model.bodies[e.body].solid);
    let p = ent.vertices[e.index].point();
    [r(p.x), r(p.y), r(p.z)]
}

/// Every entity of every body, faces first, then edges, then vertices.
pub fn all_entities(rebuilt: &NamedRebuild) -> Vec<EntityRef> {
    let mut out = Vec::new();
    for (kind, count) in [
        (
            EntityKind::Face,
            (|b: &sm_naming::BodyNaming| b.faces.len()) as fn(&_) -> usize,
        ),
        (EntityKind::Edge, |b| b.edges.len()),
        (EntityKind::Vertex, |b| b.vertices.len()),
    ] {
        for (body, naming) in rebuilt.naming().bodies.iter().enumerate() {
            out.extend((0..count(naming)).map(|index| EntityRef { kind, body, index }));
        }
    }
    out
}

/// Every name in the model.
pub fn all_names(rebuilt: &NamedRebuild) -> Vec<TopoName> {
    all_entities(rebuilt)
        .into_iter()
        .map(|e| rebuilt.naming().name_of(e))
        .collect()
}

/// The unique face, on any body, matching `pred`.
pub fn find_face(rebuilt: &NamedRebuild, pred: impl Fn(&PlaneFace) -> bool) -> EntityRef {
    let hits: Vec<_> = all_entities(rebuilt)
        .into_iter()
        .filter(|e| e.kind == EntityKind::Face)
        .filter(|&e| pred(&plane_face(rebuilt, e)))
        .collect();
    assert_eq!(hits.len(), 1, "expected one matching face, got {hits:?}");
    hits[0]
}

pub fn name(rebuilt: &NamedRebuild, e: EntityRef) -> TopoName {
    rebuilt.naming().name_of(e)
}

/// Resolves and insists on a single entity.
pub fn resolve_one(rebuilt: &NamedRebuild, n: &TopoName) -> EntityRef {
    match rebuilt.naming().resolve(n) {
        Resolution::Exact(e) | Resolution::Healed(e) => e,
        other => panic!("{n} resolved to {other:?}"),
    }
}

/// Rebuilds with names, failing loudly if any face could not be traced to
/// exactly one input face (e.g. a kernel change that merges surfaces).
pub fn build(doc: &PartDocument) -> NamedRebuild {
    let rebuilt = sm_naming::rebuild_named(doc).expect("rebuild");
    for body in &rebuilt.naming().bodies {
        assert!(
            body.untraced.is_empty(),
            "untraced faces {:?}",
            body.untraced
        );
    }
    rebuilt
}
