//! References taken on one version of a part must find the same face, edge
//! or vertex after upstream edits, or say clearly that they can't.

mod common;

use common::*;
use sm_document::BodyOperation::{Cut, Join, NewBody};
use sm_document::{ExtrudeExtent, FeatureKind};
use sm_naming::{EntityKind, EntityRef, FaceRole, Naming, Resolution, TopoName};
use std::collections::BTreeSet;

const UP: [f64; 3] = [0.0, 0.0, 1.0];
const PLUS_X: [f64; 3] = [1.0, 0.0, 0.0];

/// Block 40 x 20 x 10 with a pocket [10,5]-[30,15] from z = 5 upwards.
/// Returns the document and the (block, pocket) extrude ids and the
/// block sketch id.
fn pocketed_block() -> (sm_document::PartDocument, [sm_document::FeatureId; 3]) {
    let mut doc = new_doc();
    let (block_sketch, block) = add_block(&mut doc, [0.0, 0.0, 40.0, 20.0], 0.0, 10.0, Join);
    let (_, pocket) = add_block(&mut doc, [10.0, 5.0, 30.0, 15.0], 5.0, 10.0, Cut);
    (doc, [block_sketch, block, pocket])
}

fn no_unknown_faces(naming: &Naming) {
    for body in &naming.bodies {
        assert!(
            body.untraced.is_empty(),
            "untraced faces {:?}",
            body.untraced
        );
        for face in &body.faces {
            assert!(
                !matches!(face.origin.role, FaceRole::Unknown { .. }),
                "untraced face {face}"
            );
        }
    }
}

/// Every entity of every body has a name no other entity shares.
fn names_unique(naming: &Naming) {
    let mut seen = BTreeSet::new();
    for body in &naming.bodies {
        for f in &body.faces {
            assert!(seen.insert(format!("{f}")), "duplicate face name {f}");
        }
        for e in &body.edges {
            assert!(seen.insert(format!("{e}")), "duplicate edge name {e}");
        }
        for v in &body.vertices {
            assert!(seen.insert(format!("{v}")), "duplicate vertex name {v}");
        }
    }
}

fn exact(rebuilt: &sm_naming::NamedRebuild, n: &TopoName) -> EntityRef {
    match rebuilt.naming().resolve(n) {
        Resolution::Exact(e) => e,
        other => panic!("{n} resolved to {other:?}, expected exact"),
    }
}

#[test]
fn every_face_traced_and_every_name_unique() {
    let (doc, _) = pocketed_block();
    let r = build(&doc);
    no_unknown_faces(r.naming());
    names_unique(r.naming());
    // 11 faces, 24 edges, 16 vertices.
    let b = &r.naming().bodies[0];
    assert_eq!(
        (b.faces.len(), b.edges.len(), b.vertices.len()),
        (11, 24, 16)
    );
}

#[test]
fn pocket_references_survive_dimension_edits() {
    let (mut doc, [block_sketch, _, pocket]) = pocketed_block();
    let r = build(&doc);
    let floor = name(&r, find_face(&r, |f| f.normal == UP && f.max[2] == 5.0));
    let wall = name(
        &r,
        find_face(&r, |f| f.normal == PLUS_X && f.min[0] == 10.0),
    );
    let top = name(&r, find_face(&r, |f| f.normal == UP && f.max[2] == 10.0));
    let end = name(
        &r,
        find_face(&r, |f| f.normal == PLUS_X && f.min[0] == 40.0),
    );
    assert_eq!(floor.to_string(), format!("#{}.start(p0)", pocket.0));

    // Bigger block, pocket moved and deepened.
    sketch_mut(&mut doc, block_sketch).profiles = vec![rect([0.0, 0.0, 60.0, 30.0])];
    let pocket_sketch = match &doc.feature(pocket).unwrap().kind {
        FeatureKind::Extrude(e) => e.sketch,
        _ => unreachable!(),
    };
    let s = sketch_mut(&mut doc, pocket_sketch);
    s.profiles = vec![rect([25.0, 10.0, 50.0, 20.0])];
    s.offset = 2.0;
    let r = build(&doc);
    no_unknown_faces(r.naming());

    let f = plane_face(&r, exact(&r, &floor));
    assert_eq!(
        (f.normal, f.min, f.max),
        (UP, [25.0, 10.0, 2.0], [50.0, 20.0, 2.0])
    );
    let f = plane_face(&r, exact(&r, &wall));
    assert_eq!((f.normal, f.min[0], f.max[2]), (PLUS_X, 25.0, 10.0));
    let f = plane_face(&r, exact(&r, &top));
    assert_eq!((f.normal, f.max), (UP, [60.0, 30.0, 10.0]));
    let f = plane_face(&r, exact(&r, &end));
    assert_eq!(f.min[0], 60.0);
}

#[test]
fn edge_and_vertex_references_survive_edits() {
    let (mut doc, [block_sketch, ..]) = pocketed_block();
    let r = build(&doc);
    let edges = &r.naming().bodies[0].edges;
    // The rim edge where the top face meets the pocket wall at x = 10.
    let rim = (0..edges.len())
        .map(|index| EntityRef {
            kind: EntityKind::Edge,
            body: 0,
            index,
        })
        .find(|&e| edge_points(&r, e) == [[10.0, 5.0, 10.0], [10.0, 15.0, 10.0]])
        .unwrap();
    let rim = name(&r, rim);
    let vertices = r.naming().bodies[0].vertices.len();
    let corner = (0..vertices)
        .map(|index| EntityRef {
            kind: EntityKind::Vertex,
            body: 0,
            index,
        })
        .find(|&v| vertex_point(&r, v) == [40.0, 20.0, 10.0])
        .unwrap();
    let corner = name(&r, corner);

    sketch_mut(&mut doc, block_sketch).profiles = vec![rect([-5.0, -5.0, 50.0, 25.0])];
    if let Some(e) = doc.features.iter_mut().find_map(|f| match &mut f.kind {
        FeatureKind::Extrude(e) if e.operation == Join => Some(e),
        _ => None,
    }) {
        e.extent = ExtrudeExtent::Distance { distance: 12.0 };
    }
    let r = build(&doc);
    assert_eq!(
        edge_points(&r, exact(&r, &rim)),
        [[10.0, 5.0, 12.0], [10.0, 15.0, 12.0]]
    );
    assert_eq!(vertex_point(&r, exact(&r, &corner)), [50.0, 25.0, 12.0]);
}

#[test]
fn inserting_an_upstream_feature_keeps_downstream_names() {
    let (mut doc, [_, _, pocket]) = pocketed_block();
    let before = build(&doc);
    let pocket_faces: Vec<TopoName> = before.naming().bodies[0]
        .faces
        .iter()
        .filter(|f| f.origin.feature == pocket)
        .map(|f| TopoName::Face(f.clone()))
        .collect();
    assert_eq!(pocket_faces.len(), 5);

    // A boss on the far end of the top face, inserted before the pocket:
    // face order and counts change, the pocket's names must not.
    let pocket_sketch = doc.features[2].id;
    add_block(&mut doc, [32.0, 2.0, 38.0, 18.0], 10.0, 5.0, Join);
    move_last_before(&mut doc, 2, pocket_sketch);
    let after = build(&doc);
    no_unknown_faces(after.naming());
    names_unique(after.naming());
    assert_ne!(
        before.naming().bodies[0].faces.len(),
        after.naming().bodies[0].faces.len()
    );
    for n in &pocket_faces {
        exact(&after, n);
    }
}

#[test]
fn unrelated_features_in_either_order_give_the_same_names() {
    // What a merge of two branches that each added one boss produces.
    let names = |swap: bool| {
        let mut doc = new_doc();
        add_block(&mut doc, [0.0, 0.0, 40.0, 20.0], 0.0, 10.0, Join);
        let (a, _) = add_block(&mut doc, [2.0, 2.0, 8.0, 8.0], 10.0, 5.0, Join);
        add_block(&mut doc, [30.0, 2.0, 38.0, 8.0], 10.0, 3.0, Join);
        if swap {
            move_last_before(&mut doc, 2, a);
        }
        let r = build(&doc);
        let b = &r.naming().bodies[0];
        let all: BTreeSet<String> = b
            .faces
            .iter()
            .map(|f| f.to_string())
            .chain(b.edges.iter().map(|e| e.to_string()))
            .chain(b.vertices.iter().map(|v| v.to_string()))
            .collect();
        all
    };
    assert_eq!(names(false), names(true));
}

/// Block 40 x 20 x 10 with a through slot at x = [18, 22] across its width,
/// which cuts it in two.
fn slotted_block() -> (sm_document::PartDocument, [sm_document::FeatureId; 3]) {
    let mut doc = new_doc();
    let (block_sketch, _) = add_block(&mut doc, [0.0, 0.0, 40.0, 20.0], 0.0, 10.0, Join);
    let (slot_sketch, slot) = add_block(&mut doc, [18.0, -5.0, 22.0, 25.0], -5.0, 20.0, Cut);
    (doc, [block_sketch, slot_sketch, slot])
}

#[test]
fn split_fragments_are_told_apart_by_neighbours() {
    let (mut doc, [block_sketch, slot_sketch, _]) = slotted_block();
    let r = build(&doc);
    no_unknown_faces(r.naming());
    names_unique(r.naming());
    let left = name(&r, find_face(&r, |f| f.normal == UP && f.max[0] == 18.0));
    let right = name(&r, find_face(&r, |f| f.normal == UP && f.min[0] == 22.0));
    let TopoName::Face(l) = &left else {
        unreachable!()
    };
    assert!(!l.split.is_empty() && l.ordinal.is_none(), "{l}");

    // Longer block, slot moved right and widened: fragments keep their names.
    sketch_mut(&mut doc, block_sketch).profiles = vec![rect([0.0, 0.0, 60.0, 20.0])];
    sketch_mut(&mut doc, slot_sketch).profiles = vec![rect([30.0, -5.0, 40.0, 25.0])];
    let r = build(&doc);
    let f = plane_face(&r, exact(&r, &left));
    assert_eq!((f.min[0], f.max[0]), (0.0, 30.0));
    let f = plane_face(&r, exact(&r, &right));
    assert_eq!((f.min[0], f.max[0]), (40.0, 60.0));
}

#[test]
fn split_references_heal_when_the_face_is_whole_again() {
    let (mut doc, [_, slot_sketch, slot]) = slotted_block();
    let r = build(&doc);
    let left = name(&r, find_face(&r, |f| f.normal == UP && f.max[0] == 18.0));
    let right = name(&r, find_face(&r, |f| f.normal == UP && f.min[0] == 22.0));

    // Slot no longer reaches the edges: one top face with a hole in it.
    sketch_mut(&mut doc, slot_sketch).profiles = vec![rect([18.0, 5.0, 22.0, 15.0])];
    let r = build(&doc);
    let (a, b) = (r.naming().resolve(&left), r.naming().resolve(&right));
    assert!(matches!(a, Resolution::Healed(_)), "{a:?}");
    assert_eq!(a, b);
    let f = plane_face(&r, a.single().unwrap());
    assert_eq!((f.min, f.max), ([0.0, 0.0, 10.0], [40.0, 20.0, 10.0]));

    // Suppressing the slot heals too.
    let (mut doc, _) = slotted_block();
    doc.features
        .iter_mut()
        .find(|f| f.id == slot)
        .unwrap()
        .suppressed = true;
    let r = build(&doc);
    assert!(matches!(r.naming().resolve(&left), Resolution::Healed(_)));
}

#[test]
fn a_newly_split_face_is_ambiguous_not_guessed() {
    let mut doc = new_doc();
    add_block(&mut doc, [0.0, 0.0, 40.0, 20.0], 0.0, 10.0, Join);
    let r = build(&doc);
    let top = name(&r, find_face(&r, |f| f.normal == UP));
    add_block(&mut doc, [18.0, -5.0, 22.0, 25.0], -5.0, 20.0, Cut);
    let r = build(&doc);
    match r.naming().resolve(&top) {
        Resolution::Ambiguous(all) => {
            assert_eq!(all.len(), 2);
            let mut xs: Vec<f64> = all.iter().map(|&e| plane_face(&r, e).min[0]).collect();
            xs.sort_by(f64::total_cmp);
            assert_eq!(xs, [0.0, 22.0]);
        }
        other => panic!("expected ambiguous, got {other:?}"),
    }
}

#[test]
fn removed_geometry_is_reported_lost() {
    let (mut doc, [_, _, pocket]) = pocketed_block();
    let r = build(&doc);
    let floor = name(&r, find_face(&r, |f| f.normal == UP && f.max[2] == 5.0));
    let wall = name(
        &r,
        find_face(&r, |f| f.normal == PLUS_X && f.min[0] == 10.0),
    );

    // Pocket becomes a through hole: no floor any more, walls remain.
    let pocket_sketch = doc.features[2].id;
    sketch_mut(&mut doc, pocket_sketch).offset = -5.0;
    extrude_mut(&mut doc, pocket).extent = ExtrudeExtent::Distance { distance: 20.0 };
    let r = build(&doc);
    assert_eq!(r.naming().resolve(&floor), Resolution::Lost);
    exact(&r, &wall);

    // Pocket suppressed: its walls are gone too.
    doc.features
        .iter_mut()
        .find(|f| f.id == pocket)
        .unwrap()
        .suppressed = true;
    let r = build(&doc);
    assert_eq!(r.naming().resolve(&wall), Resolution::Lost);
}

#[test]
fn flipping_extrude_direction_keeps_cap_names() {
    // Pocket sketched on the top face and cut downwards (negative distance):
    // the face on the sketch plane is the start cap, the floor the end cap.
    let mut doc = new_doc();
    add_block(&mut doc, [0.0, 0.0, 40.0, 20.0], 0.0, 10.0, Join);
    let (_, pocket) = add_block(&mut doc, [10.0, 5.0, 30.0, 15.0], 10.0, -4.0, Cut);
    let r = build(&doc);
    let floor = find_face(&r, |f| f.normal == UP && f.max[2] == 6.0);
    assert_eq!(
        name(&r, floor).to_string(),
        format!("#{}.end(p0)", pocket.0)
    );

    // A boss extruded downwards from z = 0: its sketch-side cap faces up.
    let mut doc = new_doc();
    let (_, boss) = add_block(&mut doc, [0.0, 0.0, 4.0, 4.0], 0.0, -3.0, Join);
    let r = build(&doc);
    let top = find_face(&r, |f| f.normal == UP);
    assert_eq!(name(&r, top).to_string(), format!("#{}.start(p0)", boss.0));
}

#[test]
fn holes_and_islands_name_their_own_loops() {
    let mut doc = new_doc();
    let (sketch, frame) = add_extrude(
        &mut doc,
        vec![rect([0.0, 0.0, 10.0, 10.0]), rect([2.0, 2.0, 8.0, 8.0])],
        0.0,
        1.0,
        Join,
    );
    let r = build(&doc);
    no_unknown_faces(r.naming());
    names_unique(r.naming());
    // Inner wall facing +x is at x = 2, swept from the hole loop (p1).
    let inner = find_face(&r, |f| f.normal == PLUS_X && f.min[0] == 2.0);
    let inner = name(&r, inner);
    assert_eq!(inner.to_string(), format!("#{}.side(p1.s3)", frame.0));

    sketch_mut(&mut doc, sketch).profiles[1] = rect([3.0, 1.0, 9.0, 6.0]);
    let r = build(&doc);
    assert_eq!(plane_face(&r, exact(&r, &inner)).min[0], 3.0);
}

#[test]
fn references_on_a_second_body_survive_edits_to_the_first() {
    let mut doc = new_doc();
    let (first, _) = add_block(&mut doc, [0.0, 0.0, 10.0, 10.0], 0.0, 10.0, Join);
    add_block(&mut doc, [20.0, 0.0, 30.0, 10.0], 0.0, 5.0, NewBody);
    let r = build(&doc);
    let n = r.naming().bodies[1].faces.len();
    let top = (0..n)
        .map(|index| EntityRef {
            kind: EntityKind::Face,
            body: 1,
            index,
        })
        .find(|&e| plane_face(&r, e).normal == UP)
        .unwrap();
    let top = name(&r, top);

    sketch_mut(&mut doc, first).profiles = vec![rect([0.0, 0.0, 15.0, 15.0])];
    let r = build(&doc);
    let e = exact(&r, &top);
    assert_eq!(e.body, 1);
    assert_eq!(plane_face(&r, e).max, [30.0, 10.0, 5.0]);
}

#[test]
fn naming_before_a_feature_is_what_it_sees() {
    let (doc, [_, block, pocket]) = pocketed_block();
    let r = build(&doc);
    let seen = r.naming_before(pocket);
    assert_eq!(seen.bodies.len(), 1);
    assert_eq!(seen.bodies[0].faces.len(), 6);
    assert!(seen.bodies[0]
        .faces
        .iter()
        .all(|f| f.origin.feature == block));
    assert!(r.naming_before(doc.features[0].id).bodies.is_empty());
    // Unknown feature: the final naming.
    assert_eq!(&r.naming_before(sm_document::FeatureId(999)), r.naming());
}

#[test]
fn rebuilds_are_deterministic() {
    // truck may list entities in a different order on each run; the set of
    // names, and what each name points at, must not change.
    let (doc, _) = slotted_block();
    let all = |r: &sm_naming::NamedRebuild| -> BTreeSet<String> {
        let b = &r.naming().bodies[0];
        let mut out = BTreeSet::new();
        for (i, f) in b.faces.iter().enumerate() {
            let e = EntityRef {
                kind: EntityKind::Face,
                body: 0,
                index: i,
            };
            out.insert(format!("{f} {:?}", plane_face(r, e)));
        }
        for (i, n) in b.edges.iter().enumerate() {
            let e = EntityRef {
                kind: EntityKind::Edge,
                body: 0,
                index: i,
            };
            out.insert(format!("{n} {:?}", edge_points(r, e)));
        }
        for (i, n) in b.vertices.iter().enumerate() {
            let e = EntityRef {
                kind: EntityKind::Vertex,
                body: 0,
                index: i,
            };
            out.insert(format!("{n} {:?}", vertex_point(r, e)));
        }
        out
    };
    let first = all(&build(&doc));
    for _ in 0..5 {
        assert_eq!(all(&build(&doc)), first);
    }
}

#[test]
fn names_round_trip_through_json() {
    let (doc, _) = slotted_block();
    let r = build(&doc);
    let b = &r.naming().bodies[0];
    let all: Vec<TopoName> = b
        .faces
        .iter()
        .cloned()
        .map(TopoName::Face)
        .chain(b.edges.iter().cloned().map(TopoName::Edge))
        .chain(b.vertices.iter().cloned().map(TopoName::Vertex))
        .collect();
    for n in &all {
        let json = serde_json::to_string(n).unwrap();
        let back: TopoName = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, n);
        exact(&r, &back);
    }
    let face = serde_json::to_value(&all[0]).unwrap();
    assert_eq!(face["type"], "face");
    assert!(face["origin"]["feature"].is_u64());
}

#[test]
fn malformed_and_dangling_names_fail_safely() {
    let (doc, _) = pocketed_block();
    let r = build(&doc);
    for bad in [
        "{}",
        r#"{"type":"face"}"#,
        r#"{"type":"face","origin":{"feature":-1,"role":"start_cap","profile":0}}"#,
        r#"{"type":"face","origin":{"feature":2,"role":"bogus"}}"#,
        r#"{"type":"edge","faces":[]}"#,
        "[1,2,3]",
    ] {
        assert!(serde_json::from_str::<TopoName>(bad).is_err(), "{bad}");
    }
    // Oversized lists are refused while parsing, not after allocating.
    let origin = r#"{"feature":2,"role":"end_cap","profile":0}"#;
    let list = |n: usize| vec![origin; n].join(",");
    let face = |n: usize| {
        format!(
            r#"{{"type":"face","origin":{origin},"split":[{}]}}"#,
            list(n)
        )
    };
    assert!(serde_json::from_str::<TopoName>(&face(sm_naming::MAX_NAME_LIST)).is_ok());
    assert!(serde_json::from_str::<TopoName>(&face(sm_naming::MAX_NAME_LIST + 1)).is_err());
    let plain = format!(r#"{{"origin":{origin}}}"#);
    let vertex = |n: usize| {
        format!(
            r#"{{"type":"vertex","faces":[{}]}}"#,
            vec![plain.as_str(); n].join(",")
        )
    };
    assert!(serde_json::from_str::<TopoName>(&vertex(sm_naming::MAX_NAME_LIST)).is_ok());
    assert!(serde_json::from_str::<TopoName>(&vertex(sm_naming::MAX_NAME_LIST + 1)).is_err());
    let edge = format!(
        r#"{{"type":"edge","faces":[{plain},{plain}],"ends":[{}]}}"#,
        list(sm_naming::MAX_NAME_LIST + 1)
    );
    assert!(serde_json::from_str::<TopoName>(&edge).is_err());

    // Well-formed but pointing at nothing.
    for dangling in [
        r#"{"type":"face","origin":{"feature":999,"role":"end_cap","profile":0}}"#,
        r#"{"type":"face","origin":{"feature":2,"role":"side","edge":{"profile":7,"segment":0}}}"#,
        r#"{"type":"vertex","faces":[]}"#,
    ] {
        let n: TopoName = serde_json::from_str(dangling).unwrap();
        assert_eq!(r.naming().resolve(&n), Resolution::Lost, "{dangling}");
    }
    // Nothing resolves against an empty part.
    let empty = build(&new_doc());
    let any = r.naming().name_of(EntityRef {
        kind: EntityKind::Face,
        body: 0,
        index: 0,
    });
    assert_eq!(empty.naming().resolve(&any), Resolution::Lost);
}

#[test]
fn renumbered_features_keep_references_valid() {
    // A merge renumbers features; names rewritten with the same id map must
    // resolve on the renumbered part exactly as before.
    let (doc, _) = slotted_block();
    let r = build(&doc);
    let b = &r.naming().bodies[0];
    let names: Vec<TopoName> = b
        .faces
        .iter()
        .cloned()
        .map(TopoName::Face)
        .chain(b.edges.iter().cloned().map(TopoName::Edge))
        .chain(b.vertices.iter().cloned().map(TopoName::Vertex))
        .collect();

    // Renumber as 200 - id, which also reverses the sort order of ids.
    let map = |id: sm_document::FeatureId| sm_document::FeatureId(200 - id.0);
    let mut renumbered = doc.clone();
    for f in &mut renumbered.features {
        f.id = map(f.id);
        if let FeatureKind::Extrude(e) = &mut f.kind {
            e.sketch = map(e.sketch);
        }
    }
    let r2 = build(&renumbered);
    for n in &names {
        let mut m = n.clone();
        m.remap_features(map);
        assert!(m.features().iter().all(|f| f.0 > 100));
        let a = plane_or_point(&r, exact(&r, n));
        let b = plane_or_point(&r2, exact(&r2, &m));
        assert_eq!(a, b, "{n} -> {m}");
    }
}

fn plane_or_point(r: &sm_naming::NamedRebuild, e: EntityRef) -> String {
    match e.kind {
        EntityKind::Face => format!("{:?}", plane_face(r, e)),
        EntityKind::Edge => format!("{:?}", edge_points(r, e)),
        EntityKind::Vertex => format!("{:?}", vertex_point(r, e)),
    }
}

#[test]
fn a_cut_away_fragment_is_lost_not_moved_to_its_sibling() {
    let (mut doc, [_, slot_sketch, _]) = slotted_block();
    let r = build(&doc);
    let left = name(&r, find_face(&r, |f| f.normal == UP && f.max[0] == 18.0));
    // Widen the slot over the whole left part: only the right piece is left.
    sketch_mut(&mut doc, slot_sketch).profiles = vec![rect([-5.0, -5.0, 22.0, 25.0])];
    let r = build(&doc);
    assert_eq!(r.naming().resolve(&left), Resolution::Lost);
}

#[test]
fn ordinals_never_outweigh_neighbours() {
    use sm_naming::{FaceName, FaceOrigin};
    let origin = |feature, profile| FaceOrigin {
        feature: sm_document::FeatureId(feature),
        role: FaceRole::EndCap { profile },
    };
    let face = |split: Vec<FaceOrigin>, ordinal| FaceName {
        origin: origin(1, 0),
        split,
        ordinal,
    };
    // Stored: neighbours {a..j} (10), ordinal 0. Candidate A shares 9 of 10
    // with a different ordinal; candidate B shares 9 of 11 with ordinal 0.
    let n: Vec<FaceOrigin> = (0..10).map(|p| origin(9, p)).collect();
    let stored = TopoName::Face(face(n.clone(), Some(0)));
    let a = face(n[..9].to_vec(), Some(1));
    let mut b_split = n[..9].to_vec();
    b_split.extend([origin(8, 0), origin(8, 1)]);
    let b = face(b_split, Some(0));
    let naming = Naming {
        bodies: vec![sm_naming::BodyNaming {
            faces: vec![b, a],
            ..Default::default()
        }],
    };
    // A's overlap 0.9 beats B's 9/12 = 0.75, whatever the ordinals say.
    assert_eq!(
        naming.resolve(&stored),
        Resolution::Healed(EntityRef {
            kind: EntityKind::Face,
            body: 0,
            index: 1
        })
    );
}

#[test]
fn duplicate_exact_names_are_ambiguous() {
    let (doc, _) = pocketed_block();
    let r = build(&doc);
    let mut naming = r.naming().clone();
    let dup = naming.bodies[0].faces[0].clone();
    naming.bodies[0].faces[1] = dup.clone();
    assert!(matches!(
        naming.resolve(&TopoName::Face(dup)),
        Resolution::Ambiguous(v) if v.len() == 2
    ));
}
