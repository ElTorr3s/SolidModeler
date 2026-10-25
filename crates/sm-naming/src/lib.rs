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

mod naming;
mod tools;
mod track;

pub use naming::{BodyNaming, Entities};
pub use sm_naming_types::{
    EdgeName, FaceName, FaceOrigin, FaceRole, SketchEdge, TopoName, VertexName, MAX_NAME_LIST,
};
pub use tools::{extrude_tool, revolve_tool, FaceOrigins, RevolveAxis};

use sm_document::{FeatureId, PartDocument};
use sm_kernel::{Model, RebuildError};
use std::collections::{BTreeSet, HashMap, HashSet};
use track::{Change, Tracker};

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

    /// Finds the entity a stored name refers to. To resolve many names
    /// against one model, build a [`Resolver`] once instead.
    pub fn resolve(&self, name: &TopoName) -> Resolution {
        self.resolver().resolve(name)
    }

    /// An index of this naming for resolving many names.
    pub fn resolver(&self) -> Resolver<'_> {
        let mut index: HashMap<Key, Vec<EntityRef>> = HashMap::new();
        for (body, naming) in self.bodies.iter().enumerate() {
            let at = |kind, index| EntityRef { kind, body, index };
            for (i, f) in naming.faces.iter().enumerate() {
                index
                    .entry(face_key(f))
                    .or_default()
                    .push(at(EntityKind::Face, i));
            }
            for (i, e) in naming.edges.iter().enumerate() {
                index
                    .entry(edge_key(e))
                    .or_default()
                    .push(at(EntityKind::Edge, i));
            }
            for (i, v) in naming.vertices.iter().enumerate() {
                index
                    .entry(vertex_key(v))
                    .or_default()
                    .push(at(EntityKind::Vertex, i));
            }
        }
        Resolver {
            naming: self,
            index,
        }
    }
}

/// The part of a name every candidate must share exactly: the origins of
/// the faces involved. Scoring only ever compares entities with equal keys.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Face(FaceOrigin),
    Edge([FaceOrigin; 2]),
    Vertex(Vec<FaceOrigin>),
}

fn face_key(f: &FaceName) -> Key {
    Key::Face(f.origin)
}

fn edge_key(e: &EdgeName) -> Key {
    let mut pair = [e.faces[0].origin, e.faces[1].origin];
    pair.sort();
    Key::Edge(pair)
}

fn vertex_key(v: &VertexName) -> Key {
    let mut origins: Vec<FaceOrigin> = v.faces.iter().map(|f| f.origin).collect();
    origins.sort();
    Key::Vertex(origins)
}

/// A [`Naming`] indexed by origin, so each name is scored only against
/// entities from the same features and roles.
pub struct Resolver<'a> {
    naming: &'a Naming,
    index: HashMap<Key, Vec<EntityRef>>,
}

impl Resolver<'_> {
    /// Finds the entity a stored name refers to.
    pub fn resolve(&self, name: &TopoName) -> Resolution {
        let key = match name {
            TopoName::Face(n) => face_key(n),
            TopoName::Edge(n) => edge_key(n),
            TopoName::Vertex(n) => vertex_key(n),
        };
        let candidates = self.index.get(&key).map_or(&[][..], Vec::as_slice);
        let scored: Vec<(EntityRef, Score)> = candidates
            .iter()
            .filter_map(|&e| {
                let body = &self.naming.bodies[e.body];
                let score = match name {
                    TopoName::Face(n) => {
                        let touching = body.neighbours.get(e.index).map(Vec::as_slice);
                        if !still_touches(&n.split, touching) {
                            return None;
                        }
                        score_face(n, &body.faces[e.index])
                    }
                    TopoName::Edge(n) => score_edge(n, &body.edges[e.index]),
                    TopoName::Vertex(n) => score_vertex(n, &body.vertices[e.index]),
                };
                score.map(|s| (e, s))
            })
            .collect();
        let exact: Vec<EntityRef> = scored
            .iter()
            .map(|&(e, _)| e)
            .filter(|&e| &self.naming.name_of(e) == name)
            .collect();
        match exact.as_slice() {
            [e] => return Resolution::Exact(*e),
            [_, _, ..] => return Resolution::Ambiguous(exact),
            [] => {}
        }
        let Some(best) = scored
            .iter()
            .map(|&(_, s)| s)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        else {
            return Resolution::Lost;
        };
        let top: Vec<EntityRef> = scored
            .iter()
            .filter(|&&(_, s)| s == best)
            .map(|&(e, _)| e)
            .collect();
        match top.as_slice() {
            [e] => Resolution::Healed(*e),
            _ => Resolution::Ambiguous(top),
        }
    }
}

/// How well a candidate fits a stored name. Neighbour overlap decides;
/// matching ordinals only break exact ties, since ordinals are the one
/// unstable part of a name.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
struct Score {
    overlap: f64,
    ordinals: u32,
}

impl std::ops::Add for Score {
    type Output = Score;
    fn add(self, o: Score) -> Score {
        Score {
            overlap: self.overlap + o.overlap,
            ordinals: self.ordinals + o.ordinals,
        }
    }
}

fn ordinal_score(a: Option<u32>, b: Option<u32>) -> Score {
    Score {
        overlap: 0.0,
        ordinals: u32::from(a == b),
    }
}

/// Overlap of two neighbour lists, or `None` when both are non-empty and
/// share nothing: then they describe different pieces (e.g. the stored
/// fragment was cut away and only its sibling is left), not the same piece
/// with changed surroundings. An empty side means the face is whole on that
/// side, which still matches.
fn overlap(a: &[FaceOrigin], b: &[FaceOrigin]) -> Option<f64> {
    let j = jaccard(a, b);
    (a.is_empty() || b.is_empty() || j > 0.0).then_some(j)
}

/// A stored fragment is told apart by neighbours only it touched. A
/// candidate that touches none of them is a different piece, typically the
/// sibling left after the stored fragment was cut away, not the same face
/// made whole again (which keeps touching them).
fn still_touches(split: &[FaceOrigin], neighbours: Option<&[FaceOrigin]>) -> bool {
    match neighbours {
        Some(n) if !split.is_empty() => split.iter().any(|o| n.binary_search(o).is_ok()),
        _ => true,
    }
}

/// How well candidate `c` fits stored name `r`, or `None` if it can't be
/// the same entity: origins must match and split neighbours must not
/// contradict.
fn score_face(r: &FaceName, c: &FaceName) -> Option<Score> {
    if r.origin != c.origin {
        return None;
    }
    let overlap = overlap(&r.split, &c.split)?;
    Some(
        Score {
            overlap,
            ordinals: 0,
        } + ordinal_score(r.ordinal, c.ordinal),
    )
}

fn score_edge(r: &EdgeName, c: &EdgeName) -> Option<Score> {
    let pair = |a: &FaceName, b: &FaceName, x: &FaceName, y: &FaceName| {
        Some(score_face(a, x)? + score_face(b, y)?)
    };
    let straight = pair(&r.faces[0], &r.faces[1], &c.faces[0], &c.faces[1]);
    let crossed = pair(&r.faces[0], &r.faces[1], &c.faces[1], &c.faces[0]);
    let faces = match (straight, crossed) {
        (Some(a), Some(b)) => {
            if a >= b {
                a
            } else {
                b
            }
        }
        (a, b) => a.or(b)?,
    };
    let ends = Score {
        overlap: overlap(&r.ends, &c.ends)?,
        ordinals: 0,
    };
    Some(faces + ends + ordinal_score(r.ordinal, c.ordinal))
}

fn score_vertex(r: &VertexName, c: &VertexName) -> Option<Score> {
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
    let mut total = Score {
        overlap: 0.0,
        ordinals: 0,
    };
    for rf in &r.faces {
        let best = c
            .faces
            .iter()
            .enumerate()
            .filter(|(i, _)| !used.contains(i))
            .filter_map(|(i, cf)| score_face(rf, cf).map(|s| (i, s)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
        used.insert(best.0);
        total = total + best.1;
    }
    Some(total + ordinal_score(r.ordinal, c.ordinal))
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
    changes: Vec<(FeatureId, Change)>,
    position: HashMap<FeatureId, usize>,
}

impl NamedRebuild {
    /// Names for the final model.
    pub fn naming(&self) -> &Naming {
        &self.naming
    }

    /// Names for the model as `feature` sees it, i.e. after every feature
    /// above it. A feature's own references resolve against this. An id not
    /// in the document gives the final names. Each call replays the change
    /// log and names the bodies afresh, so call it once per feature.
    pub fn naming_before(&self, feature: FeatureId) -> Naming {
        let Some(&limit) = self.position.get(&feature) else {
            return self.naming.clone();
        };
        let mut bodies = Vec::new();
        for (f, change) in &self.changes {
            if self.position.get(f).is_none_or(|&p| p >= limit) {
                break;
            }
            change.apply(&mut bodies);
        }
        track::naming(&bodies)
    }
}

/// [`sm_kernel::rebuild`], also naming every face, edge and vertex.
pub fn rebuild_named(doc: &PartDocument) -> Result<NamedRebuild, RebuildError> {
    let mut tracker = Tracker::new(doc);
    let model = sm_kernel::rebuild_observed(doc, &mut tracker)?;
    Ok(NamedRebuild {
        model,
        naming: track::naming(&tracker.bodies),
        changes: tracker.changes,
        position: doc
            .features
            .iter()
            .enumerate()
            .map(|(i, f)| (f.id, i))
            .collect(),
    })
}
