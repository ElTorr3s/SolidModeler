//! The names themselves. They are plain data, ordered and serialisable, so
//! a reference stored in the feature tree diffs and merges as text.

use serde::{Deserialize, Serialize};
use sm_document::FeatureId;
use std::fmt;

/// One segment of a sketch loop: segment `segment` runs from point
/// `segment` to the next point of `profiles[profile]`.
///
/// This is a stand-in for the stable sketch-entity ids the sketcher will
/// add (roadmap step 3). Until then, inserting a point into a loop renames
/// the side faces after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SketchEdge {
    pub profile: u32,
    pub segment: u32,
}

/// What a face is to the feature that made it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum FaceRole {
    /// The face on the sketch side of an extrude, or at the start angle of a
    /// revolve. `profile` is the outer loop of the region.
    StartCap { profile: u32 },
    /// The far face of an extrude, or the face at the end angle of a revolve.
    EndCap { profile: u32 },
    /// The face swept by one sketch segment. A full revolve splits each
    /// swept face into pieces, numbered by angle from the sketch plane.
    Side {
        edge: SketchEdge,
        #[serde(default, skip_serializing_if = "is_zero")]
        piece: u32,
    },
    /// A face whose origin could not be traced. Never expected; references
    /// to it do not survive rebuilds.
    Unknown { ordinal: u32 },
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// The feature and role that created a face. Booleans trim and split faces
/// but never create new surfaces, so every face of a body has one origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FaceOrigin {
    pub feature: FeatureId,
    #[serde(flatten)]
    pub role: FaceRole,
}

/// A persistent face name.
///
/// When a later feature splits a face into fragments, each fragment adds
/// the origins of the neighbouring faces that only it touches (`split`),
/// e.g. "the part of the top face next to the left slot wall". Only when
/// neighbours can't tell fragments apart does `ordinal` number them by
/// position, which is the one part of a name that isn't stable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FaceName {
    pub origin: FaceOrigin,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub split: Vec<FaceOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<u32>,
}

/// A persistent edge name: the two faces that meet at it (sorted; equal
/// for a seam), plus, when those faces meet more than once, the origins of
/// the other faces at its ends that only this edge touches.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EdgeName {
    pub faces: [FaceName; 2],
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ends: Vec<FaceOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<u32>,
}

/// A persistent vertex name: the faces that meet at it, sorted.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct VertexName {
    pub faces: Vec<FaceName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<u32>,
}

/// A stored reference to one piece of topology, as a feature would keep it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TopoName {
    Face(FaceName),
    Edge(EdgeName),
    Vertex(VertexName),
}

impl fmt::Display for FaceOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}.", self.feature.0)?;
        match self.role {
            FaceRole::StartCap { profile } => write!(f, "start(p{profile})"),
            FaceRole::EndCap { profile } => write!(f, "end(p{profile})"),
            FaceRole::Side { edge, piece: 0 } => {
                write!(f, "side(p{}.s{})", edge.profile, edge.segment)
            }
            FaceRole::Side { edge, piece } => {
                write!(f, "side(p{}.s{}/{piece})", edge.profile, edge.segment)
            }
            FaceRole::Unknown { ordinal } => write!(f, "unknown({ordinal})"),
        }
    }
}

fn write_origins(f: &mut fmt::Formatter<'_>, origins: &[FaceOrigin]) -> fmt::Result {
    if origins.is_empty() {
        return Ok(());
    }
    f.write_str("[")?;
    for (i, o) in origins.iter().enumerate() {
        if i > 0 {
            f.write_str(",")?;
        }
        write!(f, "{o}")?;
    }
    f.write_str("]")
}

fn write_ordinal(f: &mut fmt::Formatter<'_>, ordinal: Option<u32>) -> fmt::Result {
    match ordinal {
        Some(n) => write!(f, "~{n}"),
        None => Ok(()),
    }
}

impl fmt::Display for FaceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.origin)?;
        write_origins(f, &self.split)?;
        write_ordinal(f, self.ordinal)
    }
}

impl fmt::Display for EdgeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "edge({} | {})", self.faces[0], self.faces[1])?;
        write_origins(f, &self.ends)?;
        write_ordinal(f, self.ordinal)
    }
}

impl fmt::Display for VertexName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("vertex(")?;
        for (i, face) in self.faces.iter().enumerate() {
            if i > 0 {
                f.write_str(" | ")?;
            }
            write!(f, "{face}")?;
        }
        f.write_str(")")?;
        write_ordinal(f, self.ordinal)
    }
}

impl fmt::Display for TopoName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TopoName::Face(n) => n.fmt(f),
            TopoName::Edge(n) => n.fmt(f),
            TopoName::Vertex(n) => n.fmt(f),
        }
    }
}

impl FaceOrigin {
    fn remap(&mut self, map: &impl Fn(FeatureId) -> FeatureId) {
        self.feature = map(self.feature);
    }
}

impl FaceName {
    fn remap(&mut self, map: &impl Fn(FeatureId) -> FeatureId) {
        self.origin.remap(map);
        self.split.iter_mut().for_each(|o| o.remap(map));
        self.split.sort();
    }
}

impl TopoName {
    /// Rewrites every feature id in the name, as a merge does when it
    /// renumbers features (see `docs/versioning.md`). Lists are re-sorted so
    /// the result equals the name a fresh rebuild would produce.
    pub fn remap_features(&mut self, map: impl Fn(FeatureId) -> FeatureId) {
        match self {
            TopoName::Face(n) => n.remap(&map),
            TopoName::Edge(n) => {
                n.faces.iter_mut().for_each(|f| f.remap(&map));
                n.faces.sort();
                n.ends.iter_mut().for_each(|o| o.remap(&map));
                n.ends.sort();
            }
            TopoName::Vertex(n) => {
                n.faces.iter_mut().for_each(|f| f.remap(&map));
                n.faces.sort();
            }
        }
    }

    /// Every feature the name depends on.
    pub fn features(&self) -> Vec<FeatureId> {
        let face = |f: &FaceName| {
            std::iter::once(f.origin.feature)
                .chain(f.split.iter().map(|o| o.feature))
                .collect::<Vec<_>>()
        };
        let mut out: Vec<FeatureId> = match self {
            TopoName::Face(n) => face(n),
            TopoName::Edge(n) => n
                .faces
                .iter()
                .flat_map(face)
                .chain(n.ends.iter().map(|o| o.feature))
                .collect(),
            TopoName::Vertex(n) => n.faces.iter().flat_map(face).collect(),
        };
        out.sort();
        out.dedup();
        out
    }
}
