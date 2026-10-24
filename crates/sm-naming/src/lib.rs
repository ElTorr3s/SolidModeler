//! Persistent names for faces, edges and vertices: the topological naming
//! problem. A feature that refers to "this face" stores a [`TopoName`], and
//! after any upstream edit the name is resolved against the rebuilt model.
//!
//! Names are generative: a face is named by the feature and sketch segment
//! that created it, and booleans pass names on. Fragments of a split face
//! are told apart by their neighbours. See `docs/topological-naming.md`.
//!
//! ```text
//! let rebuilt = sm_naming::rebuild_named(&doc)?;
//! let name = rebuilt.naming().face(0, picked_face).clone();   // store this
//! // ... user edits upstream features, rebuild again ...
//! match rebuilt.naming().resolve(&TopoName::Face(name)) { ... }
//! ```

mod name;
mod naming;
mod tools;
mod track;

pub use name::{EdgeName, FaceName, FaceOrigin, FaceRole, SketchEdge, TopoName, VertexName};
pub use naming::{BodyNaming, Entities};
pub use tools::{extrude_tool, revolve_tool, FaceOrigins, RevolveAxis};

use sm_document::{FeatureId, PartDocument};
use sm_kernel::{Model, RebuildError};
use std::collections::{BTreeSet, HashSet};
use track::{BodyState, Tracker};

/// Which kind of entity a reference points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntityKind {
    Face,
    Edge,
    Vertex,
}

/// An entity of a rebuilt model: `index` counts in [`Entities`] order for
/// body `body`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityRef {
    pub kind: EntityKind,
    pub body: usize,
    pub index: usize,
}

/// What a stored name points at after a rebuild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The name exists unchanged.
    Exact(EntityRef),
    /// The entity is still there but its name changed, e.g. a split face is
    /// whole again or its neighbours changed. The best match is unique.
    /// Callers should store the entity's new name.
    Healed(EntityRef),
    /// Several entities fit equally well, typically because the face was
    /// split by an edit. Inventor would flag the feature as sick; a feature
    /// like a fillet may choose to use them all.
    Ambiguous(Vec<EntityRef>),
    /// Nothing carries this name any more: the creating feature is
    /// suppressed or deleted, or the entity was cut away.
    Lost,
}

impl Resolution {
    /// The entity, if the name resolved to exactly one.
    pub fn single(&self) -> Option<EntityRef> {
        match self {
            Resolution::Exact(e) | Resolution::Healed(e) => Some(*e),
            _ => None,
        }
    }
}

/// Names of every entity of a model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Naming {
    pub bodies: Vec<BodyNaming>,
}

impl Naming {
    pub fn face(&self, body: usize, index: usize) -> &FaceName {
        &self.bodies[body].faces[index]
    }

    pub fn edge(&self, body: usize, index: usize) -> &EdgeName {
        &self.bodies[body].edges[index]
    }

    pub fn vertex(&self, body: usize, index: usize) -> &VertexName {
        &self.bodies[body].vertices[index]
    }

    /// The name to store for an entity.
    pub fn name_of(&self, entity: EntityRef) -> TopoName {
        let EntityRef { kind, body, index } = entity;
        match kind {
            EntityKind::Face => TopoName::Face(self.face(body, index).clone()),
            EntityKind::Edge => TopoName::Edge(self.edge(body, index).clone()),
            EntityKind::Vertex => TopoName::Vertex(self.vertex(body, index).clone()),
        }
    }

    /// Finds the entity a stored name refers to.
    pub fn resolve(&self, name: &TopoName) -> Resolution {
        let mut candidates: Vec<(EntityRef, Option<f64>)> = Vec::new();
        for (body, naming) in self.bodies.iter().enumerate() {
            let at = |kind, index| EntityRef { kind, body, index };
            match name {
                TopoName::Face(n) => candidates.extend(
                    naming
                        .faces
                        .iter()
                        .enumerate()
                        .map(|(i, c)| (at(EntityKind::Face, i), score_face(n, c))),
                ),
                TopoName::Edge(n) => candidates.extend(
                    naming
                        .edges
                        .iter()
                        .enumerate()
                        .map(|(i, c)| (at(EntityKind::Edge, i), score_edge(n, c))),
                ),
                TopoName::Vertex(n) => candidates.extend(
                    naming
                        .vertices
                        .iter()
                        .enumerate()
                        .map(|(i, c)| (at(EntityKind::Vertex, i), score_vertex(n, c))),
                ),
            }
        }
        let scored: Vec<(EntityRef, f64)> = candidates
            .into_iter()
            .filter_map(|(e, s)| s.map(|s| (e, s)))
            .collect();
        if let Some(&(e, _)) = scored.iter().find(|(e, _)| &self.name_of(*e) == name) {
            return Resolution::Exact(e);
        }
        let Some(best) = scored.iter().map(|&(_, s)| s).max_by(f64::total_cmp) else {
            return Resolution::Lost;
        };
        let top: Vec<EntityRef> = scored
            .iter()
            .filter(|&&(_, s)| s >= best - 1e-9)
            .map(|&(e, _)| e)
            .collect();
        match top.as_slice() {
            [e] => Resolution::Healed(*e),
            _ => Resolution::Ambiguous(top),
        }
    }
}

/// How well candidate `c` fits stored name `r`, or `None` if it can't be
/// the same entity. Origins must match; split neighbours are compared by
/// overlap, and a matching ordinal breaks remaining ties.
fn score_face(r: &FaceName, c: &FaceName) -> Option<f64> {
    if r.origin != c.origin {
        return None;
    }
    let ordinal = if r.ordinal == c.ordinal { 0.1 } else { 0.0 };
    Some(jaccard(&r.split, &c.split) + ordinal)
}

fn score_edge(r: &EdgeName, c: &EdgeName) -> Option<f64> {
    let straight = score_face(&r.faces[0], &c.faces[0])
        .zip(score_face(&r.faces[1], &c.faces[1]))
        .map(|(a, b)| a + b);
    let crossed = score_face(&r.faces[0], &c.faces[1])
        .zip(score_face(&r.faces[1], &c.faces[0]))
        .map(|(a, b)| a + b);
    let faces = match (straight, crossed) {
        (Some(a), Some(b)) => a.max(b),
        (a, b) => a.or(b)?,
    };
    let ordinal = if r.ordinal == c.ordinal { 0.1 } else { 0.0 };
    Some(faces + jaccard(&r.ends, &c.ends) + ordinal)
}

fn score_vertex(r: &VertexName, c: &VertexName) -> Option<f64> {
    // Cheap rejection first: stored names may come from untrusted files.
    if r.faces.len() != c.faces.len() {
        return None;
    }
    let origins = |v: &VertexName| {
        let mut o: Vec<FaceOrigin> = v.faces.iter().map(|f| f.origin).collect();
        o.sort();
        o
    };
    if origins(r) != origins(c) {
        return None;
    }
    // Pair faces by origin; within an origin, each stored face takes its
    // best-scoring unused candidate face.
    let mut used = HashSet::new();
    let mut total = 0.0;
    for rf in &r.faces {
        let best = c
            .faces
            .iter()
            .enumerate()
            .filter(|(i, _)| !used.contains(i))
            .filter_map(|(i, cf)| score_face(rf, cf).map(|s| (i, s)))
            .max_by(|a, b| a.1.total_cmp(&b.1))?;
        used.insert(best.0);
        total += best.1;
    }
    let ordinal = if r.ordinal == c.ordinal { 0.1 } else { 0.0 };
    Some(total + ordinal)
}

fn jaccard(a: &[FaceOrigin], b: &[FaceOrigin]) -> f64 {
    let (a, b): (BTreeSet<_>, BTreeSet<_>) = (a.iter().collect(), b.iter().collect());
    let union = a.union(&b).count();
    if union == 0 {
        1.0
    } else {
        a.intersection(&b).count() as f64 / union as f64
    }
}

/// A rebuilt model together with the names of its entities, after the last
/// feature and after each earlier one.
pub struct NamedRebuild {
    pub model: Model,
    naming: Naming,
    states: Vec<(FeatureId, Vec<BodyState>)>,
    order: Vec<FeatureId>,
}

impl NamedRebuild {
    /// Names for the final model.
    pub fn naming(&self) -> &Naming {
        &self.naming
    }

    /// Names for the model as `feature` sees it, i.e. after every feature
    /// above it. A feature's own references resolve against this.
    pub fn naming_before(&self, feature: FeatureId) -> Naming {
        let pos = |f: FeatureId| self.order.iter().position(|&g| g == f);
        let Some(limit) = pos(feature) else {
            return self.naming.clone();
        };
        self.states
            .iter()
            .rev()
            .find(|(f, _)| pos(*f).is_some_and(|p| p < limit))
            .map(|(_, bodies)| track::naming(bodies))
            .unwrap_or_default()
    }
}

/// [`sm_kernel::rebuild`], also naming every face, edge and vertex.
pub fn rebuild_named(doc: &PartDocument) -> Result<NamedRebuild, RebuildError> {
    let mut tracker = Tracker::new(doc);
    let model = sm_kernel::rebuild_observed(doc, &mut tracker)?;
    let naming = tracker
        .states
        .last()
        .map(|(_, bodies)| track::naming(bodies))
        .unwrap_or_default();
    Ok(NamedRebuild {
        model,
        naming,
        states: tracker.states,
        order: doc.features.iter().map(|f| f.id).collect(),
    })
}
