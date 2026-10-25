use super::*;

/// Saves a history to JSON text and loads it back, as `.smp` does. The
/// error is returned as text to check what a user would see.
fn reload(json: &serde_json::Value) -> Result<History, String> {
    serde_json::from_str(&json.to_string()).map_err(|e| e.to_string())
}
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
    let changes: Vec<String> = h
        .changes(log[0])
        .unwrap()
        .iter()
        .map(|c| c.to_string())
        .collect();
    assert_eq!(
        changes,
        [
            "changed Base sketch (name)",
            "changed Extrusion1 (definition)"
        ]
    );
    assert_eq!(h.changes(log[1]).unwrap().len(), 2); // both features added
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
    assert_eq!(reload(&json), Ok(h));

    let broken = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut j = json.clone();
        edit(&mut j);
        reload(&j).is_err()
    };
    assert!(broken(
        &|j| j["head"] = serde_json::json!({"branch": "gone"})
    ));
    assert!(broken(&|j| j["branches"]["x"] = "0000".into()));
    assert!(broken(&|j| j["branches"]["x"] = "f".repeat(64).into()));
    // Tampering with a recorded version or object is caught by its hash.
    assert!(broken(&|j| j["versions"][0]["message"] = "rewritten".into()));
    assert!(broken(&|j| {
        let objects = j["objects"].as_object_mut().unwrap();
        let (_, first) = objects.iter_mut().next().unwrap();
        first["name"] = "tampered".into();
    }));
    assert!(broken(&|j| {
        let objects = j["objects"].as_object_mut().unwrap();
        let key = objects.keys().next().unwrap().clone();
        objects.remove(&key);
    }));
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
    // A second root version, as an import of an unrelated part would add.
    let other = h.record(Vec::new(), &doc, "Other root", 5).unwrap();
    h.branches.insert("other".into(), other);
    let mut h = reload(&serde_json::to_value(&h).unwrap()).unwrap();
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
    for id in [
        "",
        "has space",
        "\u{1b}[2J",
        &"a".repeat(63),
        &"A".repeat(64),
        &"a".repeat(65),
    ] {
        let mut j = json.clone();
        j["versions"][0]["id"] = id.into();
        j["branches"]["main"] = id.into();
        j["branches"]["b"] = id.into();
        let err = reload(&j).unwrap_err();
        // Nothing from the file reaches the message unescaped.
        assert!(
            !err.chars().any(sm_document::is_unsafe_text_char),
            "{err:?}"
        );
    }

    // A stored version with an invalid document is rejected when used. Old
    // versions are parsed lazily; a broken head is rejected on load.
    let old = h.record(Vec::new(), &bad, "bad", 3).unwrap();
    assert!(matches!(h.document(&old), Err(HistoryError::Document(_))));
    let json = serde_json::to_value(&h).unwrap();
    let loaded = reload(&json).unwrap();
    assert!(loaded.document(&old).is_err());
    let mut j = json.clone();
    j["head"] = serde_json::json!({ "version": old.as_str() });
    assert!(reload(&j).is_err());
}

#[test]
fn version_ids_are_blake3_hashes() {
    let (h, _) = forked();
    let id = &h.head_version().id;
    assert_eq!(id.as_str().len(), 64);
    assert!(id.as_str().bytes().all(|b| b.is_ascii_hexdigit()));
}

#[test]
fn criss_cross_merges_use_the_latest_nearest_base() {
    let (mut h, doc) = forked();
    let v0 = h.head_version().id.clone();
    // main renames the sketch, b deepens the extrusion.
    let doc = commit_on(&mut h, &doc, "main", |d| d.features[0].name = "Base".into());
    let m1 = h.branches()["main"].clone();
    let doc = commit_on(&mut h, &doc, "b", |d| set_distance(d, 2, 20.0));
    let b1 = h.branches()["b"].clone();
    // Merge each way: b1 into main, and m1 (not main's merge) into b.
    let doc = h.checkout("main", &doc).unwrap();
    let Ok(MergeOutcome::Merged { document: doc, .. }) =
        h.merge(b1.as_str(), &doc, &Resolutions::new(), 3)
    else {
        panic!("expected a clean merge");
    };
    let doc = h.checkout("b", &doc).unwrap();
    let Ok(MergeOutcome::Merged { document: doc, .. }) =
        h.merge(m1.as_str(), &doc, &Resolutions::new(), 4)
    else {
        panic!("expected a clean merge");
    };
    // Both m1 and b1 are nearest common ancestors of the two merges; the
    // one recorded last (b1) is the base, never the older v0.
    let (main, b) = (h.branches()["main"].clone(), h.branches()["b"].clone());
    let base = h.merge_base(&main, &b).unwrap();
    assert_eq!(base, b1);
    assert_ne!(base, v0);
    // The merges agree, so merging them again is clean and changes nothing.
    let Ok(MergeOutcome::Merged { document, .. }) = h.merge("main", &doc, &Resolutions::new(), 5)
    else {
        panic!("expected a clean merge");
    };
    assert_eq!(document.features[0].name, "Base");
    assert_eq!(document.features[1].kind, extrude(FeatureId(1), 20.0));
}

#[test]
fn end_of_part_marker_follows_its_feature_through_merges() {
    // Base: Sketch1, Extrusion1, Sketch2, marker above Extrusion1.
    let mut base = block();
    base.add_feature("Sketch2", sketch(5.0));
    base.end_of_part = Some(FeatureId(2));

    // They move Extrusion1 to the end: the marker moves with it.
    let mut theirs = base.clone();
    let e = theirs.features.remove(1);
    theirs.features.push(e);
    let r = merge3(&base, &base, &theirs, &Resolutions::new());
    assert!(r.is_clean(), "{r:?}");
    assert_eq!(r.document.end_of_part, Some(FeatureId(2)));
    assert_eq!(r.document.active_features().count(), 2);

    // We delete Extrusion1 (unchanged on their side): the marker moves down
    // to the next surviving feature instead of dangling.
    let mut ours = base.clone();
    ours.features.remove(1);
    ours.end_of_part = None; // what an editor might do; base still has it
    let mut theirs = base.clone();
    theirs.features[0].name = "Renamed".into();
    let r = merge3(&base, &ours, &theirs, &Resolutions::new());
    assert!(r.is_clean(), "{r:?}");
    assert_eq!(r.document.end_of_part, None);

    let mut ours = base.clone();
    ours.features.remove(1); // marker still names the deleted feature
    let r = merge3(&base, &ours, &theirs, &Resolutions::new());
    assert!(r.is_clean(), "{r:?}");
    assert_eq!(r.document.end_of_part, Some(FeatureId(3)));
    assert_eq!(r.document.active_features().count(), 1);

    // Deleting the last feature, with the marker above it, moves the
    // marker to the end of the part.
    let mut base = block();
    base.end_of_part = Some(FeatureId(2));
    let mut ours = base.clone();
    ours.features.pop();
    let r = merge3(&base, &ours, &base, &Resolutions::new());
    assert!(r.is_clean(), "{r:?}");
    assert_eq!(r.document.end_of_part, None);
}

#[test]
fn unchanged_features_are_stored_once() {
    let mut doc = block();
    for i in 0..20 {
        let s = doc.add_feature(format!("Sketch{i}"), sketch(i as f64 + 1.0));
        doc.add_feature(format!("Extrusion{i}"), extrude(s, 1.0));
    }
    let mut h = History::new(&doc, "Initial", 1).unwrap();
    let before = h.objects.len();
    for depth in 20..30 {
        set_distance(&mut doc, 2, depth as f64);
        h.commit(&doc, "deeper", 2).unwrap();
    }
    // Ten versions that each change one number add one feature each; the
    // header and the other 41 features are shared.
    assert_eq!(h.objects.len(), before + 10);
}

#[test]
fn branches_never_reuse_feature_ids() {
    // The reviewers' case: both branches add a feature, are merged, then
    // the branch edits its feature and is merged again.
    let (mut h, doc) = forked();
    let doc = commit_on(&mut h, &doc, "main", |d| {
        d.add_feature("MainSketch", sketch(3.0));
    });
    let doc = commit_on(&mut h, &doc, "b", |d| {
        let s = d.add_feature("BranchSketch", sketch(5.0));
        assert_eq!(s, FeatureId(4), "b continues after main's id 3");
    });
    let doc = h.checkout("main", &doc).unwrap();
    let MergeOutcome::Merged { document: doc, .. } =
        h.merge("b", &doc, &Resolutions::new(), 3).unwrap()
    else {
        panic!("expected a clean merge");
    };
    assert_eq!(
        names(&doc),
        ["Sketch1", "Extrusion1", "MainSketch", "BranchSketch"]
    );

    let doc = commit_on(&mut h, &doc, "b", |d| {
        let f = d
            .features
            .iter_mut()
            .find(|f| f.id == FeatureId(4))
            .unwrap();
        f.kind = sketch(50.0);
    });
    let doc = h.checkout("main", &doc).unwrap();
    let MergeOutcome::Merged { document, .. } = h.merge("b", &doc, &Resolutions::new(), 4).unwrap()
    else {
        panic!("expected a clean merge");
    };
    assert_eq!(document.feature(FeatureId(3)).unwrap().kind, sketch(3.0));
    assert_eq!(document.feature(FeatureId(4)).unwrap().kind, sketch(50.0));
    // The opened document continues above every id any branch used.
    let mut document = document;
    assert_eq!(document.add_feature("Next", sketch(1.0)), FeatureId(5));
}

#[test]
fn add_then_delete_is_not_a_change() {
    let (mut h, doc) = forked();
    let mut edited = doc.clone();
    edited.add_feature("Temp", sketch(1.0));
    edited.features.pop();
    assert_eq!(
        h.commit(&edited, "nothing", 2),
        Err(HistoryError::NoChanges)
    );
    assert!(h.checkout("b", &edited).is_ok());
}

#[test]
fn unit_change_conflicts_with_value_edits() {
    let base = block();
    let mut ours = base.clone();
    ours.units = LengthUnit::In;
    let mut theirs = base.clone();
    theirs.add_feature("Block2", sketch(25.4));
    let r = merge3(&base, &ours, &theirs, &Resolutions::new());
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(r.conflicts[0].key, ConflictKey::Units);
    // And the other way round.
    let r = merge3(&base, &theirs, &ours, &Resolutions::new());
    assert_eq!(r.conflicts[0].key, ConflictKey::Units);
    let picked = Resolutions::from([(ConflictKey::Units, Side::Ours)]);
    let r = merge3(&base, &theirs, &ours, &picked);
    assert!(r.is_clean());
    assert_eq!(r.document.units, LengthUnit::Mm);

    // A unit change with only renames on the other side merges cleanly.
    let mut renamed = base.clone();
    renamed.features[0].name = "Base".into();
    let r = merge3(&base, &ours, &renamed, &Resolutions::new());
    assert!(r.is_clean());
    assert_eq!(r.document.units, LengthUnit::In);
}

#[test]
fn new_features_are_not_silently_rolled_back() {
    let base = block();
    // We roll the part back above the extrusion; they add a feature at the
    // end, which would land below our marker.
    let mut ours = base.clone();
    ours.end_of_part = Some(FeatureId(2));
    let mut theirs = base.clone();
    theirs.add_feature("Sketch2", sketch(5.0));
    let r = merge3(&base, &ours, &theirs, &Resolutions::new());
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(r.conflicts[0].key, ConflictKey::EndOfPart);

    let pick = |side| Resolutions::from([(ConflictKey::EndOfPart, side)]);
    let r = merge3(&base, &ours, &theirs, &pick(Side::Theirs));
    assert!(r.is_clean());
    assert_eq!(r.document.end_of_part, None);
    let r = merge3(&base, &ours, &theirs, &pick(Side::Ours));
    assert!(r.is_clean());
    assert_eq!(r.document.end_of_part, Some(FeatureId(2)));
    assert_eq!(r.document.active_features().count(), 1);

    // A feature they added below their own marker stays rolled back without
    // a conflict: that was their choice.
    let mut theirs = base.clone();
    theirs.end_of_part = Some(FeatureId(2));
    theirs.add_feature("Sketch2", sketch(5.0));
    let r = merge3(&base, &ours, &theirs, &Resolutions::new());
    assert!(r.is_clean(), "{r:?}");
}

#[test]
fn names_reject_invisible_characters() {
    let (mut h, _) = forked();
    for bad in ["main\u{202e}", "a\u{200b}b", "x\u{feff}", "bell\u{7}"] {
        assert_eq!(
            h.create_tag(bad, "main"),
            Err(HistoryError::BadName(bad.into()))
        );
    }
}

#[test]
fn ids_exhausted_is_a_conflict_not_a_panic() {
    // Both sides added a different feature under the last usable id, so
    // there is no id left to renumber theirs to.
    let mut base = PartDocument::new("P", LengthUnit::Mm);
    base.add_feature("S", sketch(1.0));
    let mut ours = base.clone();
    let mut theirs = base.clone();
    for doc in [&mut ours, &mut theirs] {
        doc.features.push(sm_document::Feature {
            id: FeatureId(u32::MAX - 1),
            name: "Last".into(),
            suppressed: false,
            kind: sketch(2.0),
        });
    }
    theirs.features[1].name = "Other".into();
    let r = merge3(&base, &ours, &theirs, &Resolutions::new());
    assert!(r.renumbered.is_empty());
    assert_eq!(
        r.conflicts[0].key,
        ConflictKey::Feature {
            id: FeatureId(u32::MAX - 1),
            field: FeatureField::Existence
        }
    );
}

#[test]
fn migration_upgrades_every_version() {
    let (mut h, doc) = forked();
    let doc = commit_on(&mut h, &doc, "b", |d| set_distance(d, 2, 7.0));
    h.create_tag("v1", "b").unwrap();
    let old_doc = h.checkout("v1", &doc).unwrap();

    // A pretend format upgrade that renames every feature.
    let upgraded = h
        .migrate_documents(|mut json| {
            for f in json["features"].as_array_mut().unwrap() {
                let name = f["name"].as_str().unwrap().to_owned();
                f["name"] = format!("{name}*").into();
            }
            Ok::<_, String>(json)
        })
        .unwrap();
    assert_eq!(upgraded.versions().len(), h.versions().len());
    for v in upgraded.versions() {
        let doc = upgraded.document(&v.id).unwrap();
        assert!(doc.features.iter().all(|f| f.name.ends_with('*')));
    }
    // Ids changed, and every name and the detached head follow them.
    assert_ne!(upgraded.branches()["main"], h.branches()["main"]);
    assert_eq!(upgraded.tags()["v1"], upgraded.branches()["b"]);
    assert_eq!(
        upgraded.head(),
        &Head::Version(upgraded.tags()["v1"].clone())
    );
    let head = upgraded.head_document().unwrap();
    assert_eq!(head.features[1].kind, old_doc.features[1].kind);
    assert_eq!(head.features[1].name, "Extrusion1*");
    // Parents are rewritten too.
    let b = upgraded.version(&upgraded.branches()["b"]).unwrap();
    assert_eq!(b.parents, [upgraded.branches()["main"].clone()]);

    // A failing upgrade names the version and leaves nothing half done.
    let err = h.migrate_documents(|_| Err("unsupported")).unwrap_err();
    assert!(err.to_string().contains("unsupported"), "{err}");
}

#[test]
fn canonical_json_sorts_keys_at_every_level() {
    let value =
        serde_json::json!({"b": 1, "a": {"d": [true, null, {"z": 1.5, "y": "x\""}], "c": -0.0}});
    assert_eq!(
        objects::canonical_json(&value),
        r#"{"a":{"c":-0.0,"d":[true,null,{"y":"x\"","z":1.5}]},"b":1}"#
    );
}

/// Pins the hash of a fixed document. If this changes, every stored object
/// and version id changes with it and deduplication against existing files
/// breaks: a field rename, a different float format or key order all show
/// up here. Change the expected value only together with a format version.
#[test]
fn object_hashes_are_stable() {
    let mut doc = block();
    doc.parameters.push(Parameter {
        name: "width".into(),
        value: 40.5,
        comment: "overall".into(),
    });
    let h = History::new(&doc, "Initial", 1_700_000_000).unwrap();
    let v = h.head_version();
    // The exact canonical text of Extrusion1, written out by hand.
    let text = r#"{"id":2,"kind":{"extent":{"distance":10.0,"type":"distance"},"operation":"join","sketch":1,"type":"extrude"},"name":"Extrusion1","suppressed":false}"#;
    assert_eq!(v.tree.features[1], Hash::of(text.as_bytes()));
    assert_eq!(
        v.tree.features[1].as_str(),
        "0dfca8b34eb1065a7bde232b4e9dacf87bb7ab3f16d6984dd5f64cdaf1541b5f"
    );
    assert_eq!(
        v.id.as_str(),
        "dcbebd7639489fe3402a0e057a3f4719f4cb07142ffbc060f227a738d4bd63bf"
    );
}
