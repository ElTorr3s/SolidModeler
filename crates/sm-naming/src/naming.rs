//! Turns face origins into full names for every face, edge and vertex of a
//! body, telling apart fragments of split faces by their neighbours.

use crate::tools::{faces, surface_key, FaceOrigins};
use sm_document::FeatureId;
use sm_naming_types::{EdgeName, FaceName, FaceOrigin, FaceRole, VertexName, MAX_NAME_LIST};
use std::collections::{BTreeMap, BTreeSet, HashMap};
#[cfg(test)]
use truck_modeling::Vector3;
use truck_modeling::{Edge, Face, Point3, Solid, Vertex};

/// The faces, edges and vertices of a solid in a fixed order: faces as
/// truck stores them, edges and vertices by first appearance in that walk.
/// [`BodyNaming`] lists names in the same order.
#[derive(Debug, Clone)]
pub struct Entities {
    pub faces: Vec<Face>,
    pub edges: Vec<Edge>,
    pub vertices: Vec<Vertex>,
    /// Indices of the faces bounded by each edge (one face for a seam).
    pub edge_faces: Vec<Vec<usize>>,
    /// Indices of the faces that meet at each vertex.
    pub vertex_faces: Vec<Vec<usize>>,
}

impl Entities {
    pub fn of(solid: &Solid) -> Entities {
        let faces: Vec<Face> = faces(solid).collect();
        let mut edges = Vec::new();
        let mut vertices = Vec::new();
        let mut edge_faces: Vec<Vec<usize>> = Vec::new();
        let mut vertex_faces: Vec<Vec<usize>> = Vec::new();
        let mut edge_index = HashMap::new();
        let mut vertex_index = HashMap::new();
        for (f, face) in faces.iter().enumerate() {
            for edge in face.edge_iter() {
                let e = *edge_index.entry(edge.id()).or_insert_with(|| {
                    edges.push(edge.clone());
                    edge_faces.push(Vec::new());
                    edges.len() - 1
                });
                if !edge_faces[e].contains(&f) {
                    edge_faces[e].push(f);
                }
            }
            for vertex in face.vertex_iter() {
                let v = *vertex_index.entry(vertex.id()).or_insert_with(|| {
                    vertices.push(vertex.clone());
                    vertex_faces.push(Vec::new());
                    vertices.len() - 1
                });
                if !vertex_faces[v].contains(&f) {
                    vertex_faces[v].push(f);
                }
            }
        }
        Entities {
            faces,
            edges,
            vertices,
            edge_faces,
            vertex_faces,
        }
    }
}

/// Names of one body's entities, in [`Entities`] order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BodyNaming {
    pub faces: Vec<FaceName>,
    pub edges: Vec<EdgeName>,
    pub vertices: Vec<VertexName>,
    /// Faces whose surface matched no input face, or more than one. Their
    /// names (and those of their edges and vertices) aren't reliable. This
    /// stays empty while truck's booleans copy surfaces unchanged; if a
    /// kernel change ever merges or rebuilds surfaces, it shows up here and
    /// in the tests, rather than as silently wrong references.
    pub untraced: Vec<usize>,
    /// Origins of the faces next to each face, sorted. Resolution uses them
    /// to check that a stored fragment's neighbours still touch a candidate.
    pub neighbours: Vec<Vec<FaceOrigin>>,
}

#[cfg(test)]
pub(crate) fn name_body(solid: &Solid, origins: &FaceOrigins) -> BodyNaming {
    name_model(&[(solid, origins)]).pop().unwrap()
}

/// One body's entities with the origin of each face.
struct Traced {
    ent: Entities,
    face_origins: Vec<FaceOrigin>,
    untraced: Vec<usize>,
    neighbours: Vec<BTreeSet<FaceOrigin>>,
    vertex_index: HashMap<truck_modeling::topology::VertexID, usize>,
}

fn trace(solid: &Solid, origins: &FaceOrigins, unknown: &mut u32) -> Traced {
    let ent = Entities::of(solid);
    let untraced: Vec<usize> = ent
        .faces
        .iter()
        .enumerate()
        .filter(|(_, face)| origins.count(surface_key(face)) != 1)
        .map(|(i, _)| i)
        .collect();
    let face_origins: Vec<FaceOrigin> = ent
        .faces
        .iter()
        .map(|face| {
            origins.get(surface_key(face)).unwrap_or_else(|| {
                *unknown += 1;
                FaceOrigin {
                    feature: FeatureId(0),
                    role: FaceRole::Unknown { ordinal: *unknown },
                }
            })
        })
        .collect();
    // Neighbours of each face across its edges, by origin.
    let mut neighbours: Vec<BTreeSet<FaceOrigin>> = vec![BTreeSet::new(); ent.faces.len()];
    for faces in &ent.edge_faces {
        for &a in faces {
            for &b in faces {
                if face_origins[a] != face_origins[b] {
                    neighbours[a].insert(face_origins[b]);
                }
            }
        }
    }
    let vertex_index = ent
        .vertices
        .iter()
        .enumerate()
        .map(|(i, v)| (v.id(), i))
        .collect();
    Traced {
        ent,
        face_origins,
        untraced,
        neighbours,
        vertex_index,
    }
}

/// Names every entity of every body. Disambiguation runs across the whole
/// part, because a cut that splits a body in two leaves fragments of one
/// face in different bodies.
pub(crate) fn name_model(bodies: &[(&Solid, &FaceOrigins)]) -> Vec<BodyNaming> {
    let mut unknown = 0;
    let traced: Vec<Traced> = bodies
        .iter()
        .map(|(solid, origins)| trace(solid, origins, &mut unknown))
        .collect();
    // Flatten (body, index) into one numbering per entity kind.
    let flat = |count: fn(&Traced) -> usize| -> Vec<(usize, usize)> {
        traced
            .iter()
            .enumerate()
            .flat_map(|(b, t)| (0..count(t)).map(move |i| (b, i)))
            .collect()
    };
    let all_faces = flat(|t| t.ent.faces.len());
    let all_edges = flat(|t| t.ent.edges.len());
    let all_vertices = flat(|t| t.ent.vertices.len());

    let face_keys: Vec<FaceOrigin> = all_faces
        .iter()
        .map(|&(b, i)| traced[b].face_origins[i])
        .collect();
    let flat_faces = disambiguate(
        &face_keys,
        |g| {
            let (b, i) = all_faces[g];
            traced[b].neighbours[i].clone()
        },
        |g| {
            let (b, i) = all_faces[g];
            centroid(traced[b].ent.faces[i].vertex_iter().map(|v| v.point()))
        },
        |origin, split, ordinal| FaceName {
            origin: *origin,
            split,
            ordinal,
        },
    );
    // Face names per body, for building edge and vertex names.
    let mut faces: Vec<Vec<FaceName>> = traced.iter().map(|_| Vec::new()).collect();
    for (&(b, _), name) in all_faces.iter().zip(flat_faces) {
        faces[b].push(name);
    }

    let edge_keys: Vec<[FaceName; 2]> = all_edges
        .iter()
        .map(|&(b, e)| {
            let fs = &traced[b].ent.edge_faces[e];
            let first = fs[0];
            let second = *fs.get(1).unwrap_or(&first);
            let mut pair = [faces[b][first].clone(), faces[b][second].clone()];
            pair.sort();
            pair
        })
        .collect();
    let flat_edges = disambiguate(
        &edge_keys,
        |g| {
            // Faces at either end of the edge, other than its own two.
            let (b, e) = all_edges[g];
            let t = &traced[b];
            let own = &t.ent.edge_faces[e];
            let ends = [t.ent.edges[e].front(), t.ent.edges[e].back()];
            ends.iter()
                .filter_map(|v| t.vertex_index.get(&v.id()))
                .flat_map(|&i| t.ent.vertex_faces[i].iter().copied())
                .filter(|f| !own.contains(f))
                .map(|f| t.face_origins[f])
                .collect()
        },
        |g| {
            let (b, e) = all_edges[g];
            let edge = &traced[b].ent.edges[e];
            centroid([edge.front().point(), edge.back().point()].into_iter())
        },
        |pair, ends, ordinal| EdgeName {
            faces: pair.clone(),
            ends,
            ordinal,
        },
    );

    let vertex_keys: Vec<Vec<FaceName>> = all_vertices
        .iter()
        .map(|&(b, v)| {
            let fs = &traced[b].ent.vertex_faces[v];
            let mut names: Vec<FaceName> = fs.iter().map(|&f| faces[b][f].clone()).collect();
            names.sort();
            names.truncate(MAX_NAME_LIST);
            names
        })
        .collect();
    let flat_vertices = disambiguate(
        &vertex_keys,
        |_| BTreeSet::new(),
        |g| {
            let (b, v) = all_vertices[g];
            traced[b].ent.vertices[v].point()
        },
        |faces, _, ordinal| VertexName {
            faces: faces.clone(),
            ordinal,
        },
    );

    let mut out: Vec<BodyNaming> = traced
        .into_iter()
        .zip(faces)
        .map(|(t, faces)| BodyNaming {
            faces,
            edges: Vec::new(),
            vertices: Vec::new(),
            untraced: t.untraced,
            neighbours: t
                .neighbours
                .into_iter()
                .map(|n| n.into_iter().collect())
                .collect(),
        })
        .collect();
    for (&(b, _), name) in all_edges.iter().zip(flat_edges) {
        out[b].edges.push(name);
    }
    for (&(b, _), name) in all_vertices.iter().zip(flat_vertices) {
        out[b].vertices.push(name);
    }
    out
}

/// Builds names for entities that share a `key` with others: first by the
/// part of their `context` (neighbouring face origins) that their siblings
/// don't share, then, for any still tied, by an ordinal in `position` order.
fn disambiguate<K: Ord + Clone, N>(
    keys: &[K],
    context: impl Fn(usize) -> BTreeSet<FaceOrigin>,
    position: impl Fn(usize) -> Point3,
    make: impl Fn(&K, Vec<FaceOrigin>, Option<u32>) -> N,
) -> Vec<N> {
    let mut groups: BTreeMap<&K, Vec<usize>> = BTreeMap::new();
    for (i, key) in keys.iter().enumerate() {
        groups.entry(key).or_default().push(i);
    }
    let mut split: Vec<Vec<FaceOrigin>> = vec![Vec::new(); keys.len()];
    let mut ordinal: Vec<Option<u32>> = vec![None; keys.len()];
    for members in groups.values().filter(|m| m.len() > 1) {
        let contexts: Vec<BTreeSet<FaceOrigin>> = members.iter().map(|&i| context(i)).collect();
        let common = contexts
            .iter()
            .skip(1)
            .fold(contexts[0].clone(), |acc, c| &acc & c);
        let own: Vec<Vec<FaceOrigin>> = contexts
            .iter()
            .map(|c| c.difference(&common).copied().take(MAX_NAME_LIST).collect())
            .collect();
        let mut ties: BTreeMap<&Vec<FaceOrigin>, Vec<usize>> = BTreeMap::new();
        for (k, &i) in members.iter().enumerate() {
            split[i] = own[k].clone();
            ties.entry(&own[k]).or_default().push(i);
        }
        for mut tied in ties.into_values().filter(|t| t.len() > 1) {
            tied.sort_by(|&a, &b| {
                let (p, q) = (position(a), position(b));
                (0..3).fold(std::cmp::Ordering::Equal, |o, k| {
                    o.then(round(p[k]).total_cmp(&round(q[k])))
                })
            });
            for (n, i) in tied.into_iter().enumerate() {
                ordinal[i] = Some(n as u32);
            }
        }
    }
    keys.iter()
        .enumerate()
        .map(|(i, key)| make(key, std::mem::take(&mut split[i]), ordinal[i]))
        .collect()
}

/// Rounds away float noise so tiny rebuild differences don't reorder ties.
fn round(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

fn centroid(points: impl Iterator<Item = Point3>) -> Point3 {
    let (mut sum, mut n) = (Point3::new(0.0, 0.0, 0.0), 0.0);
    for p in points {
        sum += p - Point3::new(0.0, 0.0, 0.0);
        n += 1.0;
    }
    if n > 0.0 {
        Point3::new(sum.x / n, sum.y / n, sum.z / n)
    } else {
        sum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(feature: u32) -> FaceOrigin {
        FaceOrigin {
            feature: FeatureId(feature),
            role: FaceRole::EndCap { profile: 0 },
        }
    }

    #[test]
    fn disambiguates_by_own_neighbours_then_position() {
        // Four entities share key "a"; two are told apart by neighbours,
        // two have identical neighbours and fall back to position.
        let keys = ["a", "a", "a", "a", "b"];
        let ctx = [vec![1, 9], vec![2, 9], vec![9, 3], vec![9, 3], vec![1]];
        let pos = [0.0, 1.0, 5.0, 2.0, 0.0];
        let names = disambiguate(
            &keys,
            |i| ctx[i].iter().map(|&f| o(f)).collect(),
            |i| Point3::new(pos[i], 0.0, 0.0),
            |k, split, ordinal| (k.to_string(), split, ordinal),
        );
        // Neighbour 9 is common to the group, so it is left out.
        assert_eq!(names[0], ("a".into(), vec![o(1)], None));
        assert_eq!(names[1], ("a".into(), vec![o(2)], None));
        assert_eq!(names[2], ("a".into(), vec![o(3)], Some(1)));
        assert_eq!(names[3], ("a".into(), vec![o(3)], Some(0)));
        // A unique key needs no help.
        assert_eq!(names[4], ("b".into(), vec![], None));
    }

    #[test]
    fn long_neighbour_lists_are_truncated_to_what_files_accept() {
        let n = MAX_NAME_LIST as u32 + 10;
        let names = disambiguate(
            &["a", "a"],
            |i| (0..n).map(|f| o(f * 2 + i as u32)).collect(),
            |_| Point3::new(0.0, 0.0, 0.0),
            |_, split, _| split,
        );
        assert_eq!(names[0].len(), MAX_NAME_LIST);
        assert_ne!(names[0], names[1]);
    }

    #[test]
    fn zero_or_several_matching_origins_are_reported() {
        use truck_modeling::builder;
        let v = builder::vertex(Point3::new(0.0, 0.0, 0.0));
        let face = builder::tsweep(&builder::tsweep(&v, Vector3::unit_x()), Vector3::unit_y());
        let solid = builder::tsweep(&face, Vector3::unit_z());
        let all: Vec<Face> = faces(&solid).collect();

        let mut origins = FaceOrigins::default();
        for face in &all[1..] {
            origins.insert(surface_key(face), o(1));
        }
        // Face 0 matches nothing; face 1 matches two features.
        origins.insert(surface_key(&all[1]), o(2));
        let naming = name_body(&solid, &origins);
        assert_eq!(naming.untraced, [0, 1]);
        assert!(matches!(
            naming.faces[0].origin.role,
            FaceRole::Unknown { .. }
        ));
        assert_eq!(naming.faces[1].origin, o(1), "older feature wins");

        let mut clean = FaceOrigins::default();
        for face in &all {
            clean.insert(surface_key(face), o(1));
        }
        assert!(name_body(&solid, &clean).untraced.is_empty());
    }

    #[test]
    fn empty_and_degenerate_input() {
        let names = disambiguate(
            &[] as &[u8],
            |_| BTreeSet::new(),
            |_| Point3::new(0.0, 0.0, 0.0),
            |_, s, o| (s, o),
        );
        assert!(names.is_empty());
        assert_eq!(centroid(std::iter::empty()), Point3::new(0.0, 0.0, 0.0));
        assert_eq!(round(-0.0000001), -0.0);
    }
}
