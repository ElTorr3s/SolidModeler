use super::*;
use sm_document::{
    BodyOperation, Extrude, ExtrudeExtent, FeatureId, FeatureKind, LengthUnit, OriginPlane,
    Parameter, Profile, Sketch,
};

fn sketch(size: f64) -> FeatureKind {
    FeatureKind::Sketch(Sketch {
        plane: OriginPlane::XY,
        offset: 0.0,
        profiles: vec![Profile {
            points: vec![[0.0, 0.0], [size, 0.0], [size, size], [0.0, size]],
        }],
    })
}

fn extrude(sketch: FeatureId, distance: f64) -> FeatureKind {
    FeatureKind::Extrude(Extrude {
        sketch,
        extent: ExtrudeExtent::Distance { distance },
        operation: BodyOperation::Join,
    })
}

/// Sketch1 (id 1) and Extrusion1 (id 2), 10 deep.
fn block() -> PartDocument {
    let mut doc = PartDocument::new("Block", LengthUnit::Mm);
    let s = doc.add_feature("Sketch1", sketch(40.0));
    doc.add_feature("Extrusion1", extrude(s, 10.0));
    doc
}

fn set_distance(doc: &mut PartDocument, id: u32, distance: f64) {
    let f = doc
        .features
        .iter_mut()
        .find(|f| f.id == FeatureId(id))
        .unwrap();
    if let FeatureKind::Extrude(e) = &mut f.kind {
        e.extent = ExtrudeExtent::Distance { distance };
    }
}

fn names(doc: &PartDocument) -> Vec<&str> {
    doc.features.iter().map(|f| f.name.as_str()).collect()
}

/// A history on `main` with one version, plus a branch `b` at the same spot.
fn forked() -> (History, PartDocument) {
    let doc = block();
    let mut h = History::new(&doc, "Initial", 1).unwrap();
    h.create_branch("b", "main").unwrap();
    (h, doc)
}

/// Commits `edit(doc)` on `branch` and returns the new document.
fn commit_on(
    h: &mut History,
    working: &PartDocument,
    branch: &str,
    edit: impl FnOnce(&mut PartDocument),
) -> PartDocument {
    let mut doc = h.checkout(branch, working).unwrap();
    edit(&mut doc);
    h.commit(&doc, &format!("edit on {branch}"), 2).unwrap();
    doc
}

#[test]
fn commit_log_and_changes() {
    let mut doc = block();
    let mut h = History::new(&doc, "Initial", 1).unwrap();
    assert_eq!(h.commit(&doc, "nothing", 2), Err(HistoryError::NoChanges));

    set_distance(&mut doc, 2, 25.0);
    doc.features[0].name = "Base sketch".into();
    let v2 = h.commit(&doc, "Deeper", 2).unwrap();
    assert_eq!(h.head_version().id, v2);
    assert_eq!(h.branches()["main"], v2);

    let log = h.log(&v2);
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].message, "Deeper");
    let changes: Vec<String> = h.changes(log[0]).iter().map(|c| c.to_string()).collect();
    assert_eq!(
        changes,
        [
            "changed Base sketch (name)",
            "changed Extrusion1 (definition)"
        ]
    );
    assert_eq!(h.changes(log[1]).len(), 2); // both features added
}

#[test]
fn checkout_refuses_to_lose_changes() {
    let (mut h, doc) = forked();
    let mut edited = doc.clone();
    set_distance(&mut edited, 2, 99.0);
    assert_eq!(
        h.checkout("b", &edited),
        Err(HistoryError::UncommittedChanges)
    );
    assert_eq!(h.checkout("b", &doc), Ok(doc));
    assert_eq!(h.current_branch(), Some("b"));
}

#[test]
fn checkout_tag_detaches_head() {
    let (mut h, doc) = forked();
    h.create_tag("Rev A", "main").unwrap();
    let doc = commit_on(&mut h, &doc, "main", |d| set_distance(d, 2, 5.0));
    let old = h.checkout("Rev A", &doc).unwrap();
    assert_eq!(old, block());
    assert!(matches!(h.head(), Head::Version(_)));
    assert_eq!(
        h.create_tag("Rev A", "main"),
        Err(HistoryError::NameTaken("Rev A".into()))
    );
    assert_eq!(
        h.create_branch(" x", "main"),
        Err(HistoryError::BadName(" x".into()))
    );
}

#[test]
fn edits_to_different_features_merge_cleanly() {
    let (mut h, doc) = forked();
    // main: deepen the extrusion. b: add a sketch and a cut that uses it.
    let doc = commit_on(&mut h, &doc, "main", |d| set_distance(d, 2, 30.0));
    let doc = commit_on(&mut h, &doc, "b", |d| {
        let s = d.add_feature("Sketch2", sketch(5.0));
        d.add_feature("Cut1", extrude(s, -3.0));
    });
    let doc = h.checkout("main", &doc).unwrap();
    let MergeOutcome::Merged { version, document } =
        h.merge("b", &doc, &Resolutions::new(), 3).unwrap()
    else {
        panic!("expected a clean merge");
    };
    assert_eq!(
        names(&document),
        ["Sketch1", "Extrusion1", "Sketch2", "Cut1"]
    );
    assert_eq!(document.features[1].kind, extrude(FeatureId(1), 30.0));
    assert_eq!(h.version(&version).unwrap().parents.len(), 2);
    // Merging again changes nothing.
    assert_eq!(
        h.merge("b", &document, &Resolutions::new(), 4),
        Ok(MergeOutcome::UpToDate)
    );
}

#[test]
fn features_added_on_both_sides_get_distinct_ids() {
    let base = block();
    let mut ours = base.clone();
    let s = ours.add_feature("Sketch2", sketch(5.0));
    ours.add_feature("Boss", extrude(s, 2.0));
    let mut theirs = base.clone();
    let s = theirs.add_feature("Sketch3", sketch(7.0));
    theirs.add_feature("Pocket", extrude(s, -2.0));
    theirs.end_of_part = Some(FeatureId(4));

    let r = merge3(&base, &ours, &theirs, &Resolutions::new());
    assert!(r.is_clean(), "{r:?}");
    assert_eq!(
        r.renumbered,
        [(FeatureId(3), FeatureId(5)), (FeatureId(4), FeatureId(6))]
    );
    assert_eq!(
        names(&r.document),
        [
            "Sketch1",
            "Extrusion1",
            "Sketch2",
            "Boss",
            "Sketch3",
            "Pocket"
        ]
    );
    // Their pocket still uses their sketch, and their marker moved with it.
    assert_eq!(r.document.features[5].kind, extrude(FeatureId(5), -2.0));
    assert_eq!(r.document.end_of_part, Some(FeatureId(6)));
    // New ids continue above everything in use.
    let mut merged = r.document;
    assert_eq!(merged.add_feature("Next", sketch(1.0)), FeatureId(7));
}

#[test]
fn identical_additions_count_once() {
    let base = block();
    let mut ours = base.clone();
    ours.add_feature("Sketch2", sketch(5.0));
    let r = merge3(&base, &ours, &ours.clone(), &Resolutions::new());
    assert!(r.is_clean());
    assert!(r.renumbered.is_empty());
    assert_eq!(r.document.features.len(), 3);
}

#[test]
fn rename_and_redefine_of_one_feature_merge() {
    let base = block();
    let mut ours = base.clone();
    ours.features[1].name = "Base".into();
    let mut theirs = base.clone();
    set_distance(&mut theirs, 2, 12.0);
    theirs.features[1].suppressed = true;
    let r = merge3(&base, &ours, &theirs, &Resolutions::new());
    assert!(r.is_clean());
    let f = &r.document.features[1];
    assert_eq!((f.name.as_str(), f.suppressed), ("Base", true));
    assert_eq!(f.kind, extrude(FeatureId(1), 12.0));
}

#[test]
fn same_field_conflict_is_reported_and_resolvable() {
    let base = block();
    let mut ours = base.clone();
    set_distance(&mut ours, 2, 20.0);
    let mut theirs = base.clone();
    set_distance(&mut theirs, 2, 30.0);

    let r = merge3(&base, &ours, &theirs, &Resolutions::new());
    let key = ConflictKey::Feature {
        id: FeatureId(2),
        field: FeatureField::Definition,
    };
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(r.conflicts[0].key, key);
    assert_eq!(
        r.conflicts[0].theirs.as_ref().unwrap()["extent"]["distance"],
        30.0
    );
    // Unresolved conflicts keep our side.
    assert_eq!(r.document.features[1].kind, extrude(FeatureId(1), 20.0));

    let r = merge3(
        &base,
        &ours,
        &theirs,
        &Resolutions::from([(key, Side::Theirs)]),
    );
    assert!(r.is_clean());
    assert_eq!(r.document.features[1].kind, extrude(FeatureId(1), 30.0));
}

#[test]
fn conflicted_merge_records_nothing_until_resolved() {
    let (mut h, doc) = forked();
    let doc = commit_on(&mut h, &doc, "main", |d| set_distance(d, 2, 20.0));
    let doc = commit_on(&mut h, &doc, "b", |d| set_distance(d, 2, 30.0));
    let doc = h.checkout("main", &doc).unwrap();
    let count = h.versions().len();
    let MergeOutcome::Conflicted(r) = h.merge("b", &doc, &Resolutions::new(), 3).unwrap() else {
        panic!("expected a conflict");
    };
    assert_eq!(h.versions().len(), count);
    let picks: Resolutions = r
        .conflicts
        .iter()
        .map(|c| (c.key.clone(), Side::Theirs))
        .collect();
    let MergeOutcome::Merged { document, .. } = h.merge("b", &doc, &picks, 3).unwrap() else {
        panic!("expected the resolved merge to be recorded");
    };
    assert_eq!(document.features[1].kind, extrude(FeatureId(1), 30.0));
}

#[test]
fn delete_versus_edit_is_a_conflict() {
    let base = block();
    let mut ours = base.clone();
    ours.features.pop();
    let mut theirs = base.clone();
    set_distance(&mut theirs, 2, 30.0);
    let r = merge3(&base, &ours, &theirs, &Resolutions::new());
    assert_eq!(
        r.conflicts[0].key,
        ConflictKey::Feature {
            id: FeatureId(2),
            field: FeatureField::Existence
        }
    );
    assert!(r.conflicts[0].ours.is_none());
    assert_eq!(r.document.features.len(), 1);

    // Deleting an unchanged feature just deletes it.
    let r = merge3(&base, &ours, &base, &Resolutions::new());
    assert!(r.is_clean());
    assert_eq!(r.document.features.len(), 1);
}

#[test]
fn broken_references_make_the_merge_invalid() {
    // We delete the extrusion and its sketch; they add a second extrusion
    // of that sketch. Every field merges, but the result is not a valid part.
    let base = block();
    let mut ours = base.clone();
    ours.features.clear();
    let mut theirs = base.clone();
    theirs.add_feature("Extrusion2", extrude(FeatureId(1), 4.0));
    let r = merge3(&base, &ours, &theirs, &Resolutions::new());
    assert!(r.conflicts.is_empty());
    assert_eq!(r.invalid, Some(DocumentError::MissingFeature(FeatureId(1))));
    assert!(!r.is_clean());
}

#[test]
fn one_sided_reorder_wins_and_new_features_follow_their_anchor() {
    let mut base = block();
    base.add_feature("Sketch2", sketch(5.0)); // id 3
    let mut ours = base.clone();
    // Ours adds a feature after Extrusion1.
    let new = ours.add_feature("Sketch9", sketch(9.0)); // id 4
    let new = ours.features.pop().map(|f| (new, f)).unwrap().1;
    ours.features.insert(2, new);
    // Theirs moves Sketch2 to the top.
    let mut theirs = base.clone();
    let s2 = theirs.features.pop().unwrap();
    theirs.features.insert(0, s2);

    let r = merge3(&base, &ours, &theirs, &Resolutions::new());
    assert!(r.is_clean(), "{r:?}");
    assert_eq!(
        names(&r.document),
        ["Sketch2", "Sketch1", "Extrusion1", "Sketch9"]
    );
}

#[test]
fn conflicting_reorders_are_reported() {
    let mut base = block();
    base.add_feature("Sketch2", sketch(5.0));
    let mut ours = base.clone();
    ours.features.swap(0, 2); // Sketch2, Extrusion1, Sketch1: invalid but fine here
    let mut theirs = base.clone();
    theirs.features.swap(1, 2);
    let r = merge3(&base, &ours, &theirs, &Resolutions::new());
    assert!(r.conflicts.iter().any(|c| c.key == ConflictKey::Order));
}

#[test]
fn parameters_merge_by_name() {
    let p = |name: &str, value| Parameter {
        name: name.into(),
        value,
        comment: String::new(),
    };
    let mut base = block();
    base.parameters = vec![p("width", 40.0), p("depth", 10.0)];
    let mut ours = base.clone();
    ours.parameters[0].value = 50.0;
    ours.parameters.push(p("hole", 3.0));
    let mut theirs = base.clone();
    theirs.parameters.remove(1);
    theirs.parameters.push(p("fillet", 1.0));
    let r = merge3(&base, &ours, &theirs, &Resolutions::new());
    assert!(r.is_clean());
    assert_eq!(
        r.document.parameters,
        [p("width", 50.0), p("hole", 3.0), p("fillet", 1.0)]
    );
}

#[test]
fn fast_forward_moves_the_branch() {
    let (mut h, doc) = forked();
    let edited = commit_on(&mut h, &doc, "b", |d| set_distance(d, 2, 7.0));
    let doc = h.checkout("main", &edited).unwrap();
    assert_eq!(
        h.merge("b", &doc, &Resolutions::new(), 3),
        Ok(MergeOutcome::FastForward(edited))
    );
    assert_eq!(h.branches()["main"], h.branches()["b"]);
}

#[test]
fn resolves_names_and_id_prefixes() {
    let (mut h, doc) = forked();
    commit_on(&mut h, &doc, "main", |d| set_distance(d, 2, 7.0));
    let id = h.branches()["main"].clone();
    assert_eq!(h.resolve(&id.as_str()[..6]), Ok(id.clone()));
    assert_eq!(h.resolve(id.as_str()), Ok(id));
    assert!(h.resolve("abc").is_err()); // too short to be a prefix
    assert!(h.resolve("nope").is_err());
}

#[test]
fn json_round_trip_and_validation() {
    let (mut h, doc) = forked();
    commit_on(&mut h, &doc, "b", |d| set_distance(d, 2, 7.0));
    h.create_tag("v1", "b").unwrap();
    let json = serde_json::to_value(&h).unwrap();
    let back: History = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back, h);

    let broken = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut j = json.clone();
        edit(&mut j);
        serde_json::from_value::<History>(j).is_err()
    };
    assert!(broken(
        &|j| j["head"] = serde_json::json!({"branch": "gone"})
    ));
    assert!(broken(&|j| j["branches"]["x"] = "0000".into()));
    assert!(broken(&|j| j["tags"]["b"] = j["branches"]["b"].clone()));
    assert!(broken(
        &|j| j["versions"][0]["parents"] = serde_json::json!(["ffff"])
    ));
    assert!(broken(&|j| {
        let first = j["versions"][0].clone();
        j["versions"].as_array_mut().unwrap().push(first);
    }));
    assert!(broken(&|j| j["versions"] = serde_json::json!([])));
    // A child listed before its parent (a cycle can't be written otherwise).
    assert!(broken(&|j| j["versions"].as_array_mut().unwrap().reverse()));
}

#[test]
fn diff_reports_every_kind_of_change() {
    let p = |name: &str, value| Parameter {
        name: name.into(),
        value,
        comment: String::new(),
    };
    let mut old = block();
    old.add_feature("Sketch2", sketch(3.0));
    old.parameters = vec![p("a", 1.0), p("b", 2.0)];
    let mut new = old.clone();
    new.name = "Renamed".into();
    new.units = LengthUnit::In;
    new.parameters = vec![p("a", 5.0), p("c", 3.0)];
    new.features[0].suppressed = true;
    new.features.swap(1, 2);
    new.features.remove(1); // Sketch2
    new.add_feature("Sketch4", sketch(1.0));
    new.end_of_part = Some(FeatureId(2));

    let changes: Vec<String> = diff(&old, &new).iter().map(|c| c.to_string()).collect();
    assert_eq!(
        changes,
        [
            "renamed part \"Block\" to \"Renamed\"",
            "changed units",
            "changed parameter a",
            "added parameter c",
            "removed parameter b",
            "changed Sketch1 (suppressed)",
            "added Sketch4",
            "removed Sketch2",
            "moved end of part",
        ]
    );
    assert!(diff(&old, &old).is_empty());

    let mut reordered = block();
    reordered.add_feature("Sketch2", sketch(3.0));
    reordered.features.swap(0, 2);
    assert_eq!(
        diff(&old_features_only(&old), &reordered),
        [Change::Reordered]
    );
}

/// `old` without the parameters, for comparisons about features only.
fn old_features_only(doc: &PartDocument) -> PartDocument {
    let mut doc = doc.clone();
    doc.parameters.clear();
    doc
}

#[test]
fn repeated_merges_use_the_latest_common_version() {
    let (mut h, doc) = forked();
    let doc = commit_on(&mut h, &doc, "b", |d| set_distance(d, 2, 20.0));
    let doc = commit_on(&mut h, &doc, "main", |d| d.features[0].name = "Base".into());
    let doc = h.checkout("main", &doc).unwrap();
    let MergeOutcome::Merged { document: doc, .. } =
        h.merge("b", &doc, &Resolutions::new(), 3).unwrap()
    else {
        panic!("expected a clean merge");
    };
    // b changes the same field again. Against the original base this would
    // look like a conflict with the merged 20; against the latest common
    // version (b's previous tip) it's a clean change.
    let doc = commit_on(&mut h, &doc, "b", |d| set_distance(d, 2, 40.0));
    let doc = h.checkout("main", &doc).unwrap();
    let MergeOutcome::Merged { document, .. } = h.merge("b", &doc, &Resolutions::new(), 4).unwrap()
    else {
        panic!("expected a clean merge");
    };
    assert_eq!(document.features[0].name, "Base");
    assert_eq!(document.features[1].kind, extrude(FeatureId(1), 40.0));
}

#[test]
fn merge_refuses_uncommitted_changes_and_unknown_targets() {
    let (mut h, doc) = forked();
    let mut edited = doc.clone();
    set_distance(&mut edited, 2, 1.0);
    assert_eq!(
        h.merge("b", &edited, &Resolutions::new(), 2),
        Err(HistoryError::UncommittedChanges)
    );
    assert_eq!(
        h.merge("nope", &doc, &Resolutions::new(), 2),
        Err(HistoryError::UnknownRef("nope".into()))
    );
}

#[test]
fn unrelated_histories_have_no_merge_base() {
    let (mut h, doc) = forked();
    let json = serde_json::to_value(&h).unwrap();
    // Graft a second root version onto the stored history.
    let mut j = json.clone();
    let mut root = j["versions"][0].clone();
    root["id"] = "00000000000000ff".into();
    root["message"] = "Other root".into();
    j["versions"].as_array_mut().unwrap().push(root);
    j["branches"]["other"] = "00000000000000ff".into();
    h = serde_json::from_value(j).unwrap();
    assert_eq!(
        h.merge("other", &doc, &Resolutions::new(), 2),
        Err(HistoryError::NoCommonAncestor)
    );
}

#[test]
fn branch_management() {
    let (mut h, _) = forked();
    assert_eq!(
        h.delete_branch("main"),
        Err(HistoryError::DeleteCurrentBranch)
    );
    assert_eq!(h.delete_branch("b"), Ok(()));
    assert_eq!(
        h.delete_branch("b"),
        Err(HistoryError::UnknownRef("b".into()))
    );
    assert_eq!(
        h.create_branch("x", "missing"),
        Err(HistoryError::UnknownRef("missing".into()))
    );
    for bad in [
        "",
        " lead",
        "trail ",
        "tab\there",
        &"x".repeat(MAX_NAME_CHARS + 1),
    ] {
        assert_eq!(
            h.create_branch(bad, "main"),
            Err(HistoryError::BadName(bad.into()))
        );
    }
    assert_eq!(h.create_branch(&"x".repeat(MAX_NAME_CHARS), "main"), Ok(()));
    assert_eq!(h.create_branch("Révision B", "main"), Ok(()));
}

#[test]
fn detached_commits_move_the_head_not_a_branch() {
    let (mut h, doc) = forked();
    let first = h.head_version().id.clone();
    let doc = h.checkout(first.as_str(), &doc).unwrap();
    let mut edited = doc.clone();
    set_distance(&mut edited, 2, 3.0);
    let id = h.commit(&edited, "detached", 2).unwrap();
    assert_eq!(h.head(), &Head::Version(id.clone()));
    assert_eq!(h.branches()["main"], first);
    // Keep the work by naming it.
    h.create_branch("kept", id.as_str()).unwrap();
    assert_eq!(h.resolve("kept"), Ok(id));
}

#[test]
fn rejects_invalid_documents_and_odd_ids() {
    let mut bad = block();
    bad.features.swap(0, 1);
    assert!(matches!(
        History::new(&bad, "x", 1),
        Err(HistoryError::Document(_))
    ));
    let (mut h, _) = forked();
    assert!(matches!(
        h.commit(&bad, "x", 1),
        Err(HistoryError::Document(_))
    ));

    let json = serde_json::to_value(&h).unwrap();
    for id in ["", "has space", "\u{1b}[2J", &"a".repeat(65)] {
        let mut j = json.clone();
        j["versions"][0]["id"] = id.into();
        j["branches"]["main"] = id.into();
        j["branches"]["b"] = id.into();
        assert!(serde_json::from_value::<History>(j).is_err(), "{id:?}");
    }
    // An invalid document inside a stored version is rejected too.
    let mut j = json.clone();
    j["versions"][0]["document"]["features"][1]["kind"]["sketch"] = 99.into();
    assert!(serde_json::from_value::<History>(j).is_err());
}
