//! Three-way merge of two versions of a part at the feature level.
//!
//! Features are matched by id, parameters by name. Each feature merges field
//! by field (name, suppressed, definition), so renaming a feature on one
//! branch and changing its distance on the other combines cleanly. A field
//! changed differently on both sides, or a feature deleted on one side and
//! changed on the other, is a conflict: the merge keeps our side and reports
//! it, and the caller can pick the other side through [`Resolutions`].

use serde::{Deserialize, Serialize};
use sm_document::{DocumentError, Feature, FeatureId, PartDocument};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use crate::Change;

/// Which side of a merge to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Ours,
    Theirs,
}

/// The part of a feature a conflict is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureField {
    /// Deleted on one side and changed on the other.
    Existence,
    Name,
    Suppressed,
    /// The feature's definition (sketch geometry, extrude extent, ...).
    Definition,
}

/// Names one decision in a merge. Conflicts are reported and resolved by key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "on", rename_all = "snake_case")]
pub enum ConflictKey {
    DocumentName,
    Units,
    EndOfPart,
    /// Both sides reordered the features they share, differently.
    Order,
    Parameter {
        name: String,
    },
    /// `id` is the feature's id in the merged document.
    Feature {
        id: FeatureId,
        field: FeatureField,
    },
}

impl fmt::Display for ConflictKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConflictKey::DocumentName => write!(f, "part name"),
            ConflictKey::Units => write!(f, "units"),
            ConflictKey::EndOfPart => write!(f, "end of part"),
            ConflictKey::Order => write!(f, "feature order"),
            ConflictKey::Parameter { name } => write!(f, "parameter {name}"),
            ConflictKey::Feature { id, field } => {
                let field = match field {
                    FeatureField::Existence => "deleted on one side, changed on the other",
                    FeatureField::Name => "name",
                    FeatureField::Suppressed => "suppressed",
                    FeatureField::Definition => "definition",
                };
                write!(f, "feature {} {field}", id.0)
            }
        }
    }
}

/// Picks for conflicts, by key. Keys that aren't conflicts are ignored.
pub type Resolutions = BTreeMap<ConflictKey, Side>;

/// One unresolved conflict, with each side's value as JSON for display.
/// `None` means the item doesn't exist on that side.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Conflict {
    pub key: ConflictKey,
    pub base: Option<serde_json::Value>,
    pub ours: Option<serde_json::Value>,
    pub theirs: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MergeResult {
    /// The merged part. Unresolved conflicts take our side, or the side
    /// named in the resolutions.
    pub document: PartDocument,
    /// Conflicts that the resolutions didn't settle.
    pub conflicts: Vec<Conflict>,
    /// Features both sides added under the same id: their side's feature was
    /// given a new id, `(old, new)`. Their references were updated to match.
    pub renumbered: Vec<(FeatureId, FeatureId)>,
    /// Set when the merged part is not a valid document even though every
    /// field merged, for example their new extrude uses a sketch we deleted.
    /// It has to be fixed by hand before the merge can be committed.
    pub invalid: Option<DocumentError>,
}

impl MergeResult {
    /// True if the result can be committed as is.
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty() && self.invalid.is_none()
    }
}

/// Merges `ours` and `theirs`, two descendants of `base`.
pub fn merge3(
    base: &PartDocument,
    ours: &PartDocument,
    theirs: &PartDocument,
    resolutions: &Resolutions,
) -> MergeResult {
    merge3_with_bases(base, &[], ours, theirs, resolutions)
}

/// [`merge3`] when `ours` and `theirs` have several equally near common
/// ancestors, as after criss-cross merges: `base` is the one used, and
/// `other_bases` are the rest. Anything the bases disagree on was settled
/// differently on each side, so if ours and theirs still differ there it
/// is a conflict, even where comparing with `base` alone would pick a side
/// silently.
pub fn merge3_with_bases(
    base: &PartDocument,
    other_bases: &[PartDocument],
    ours: &PartDocument,
    theirs: &PartDocument,
    resolutions: &Resolutions,
) -> MergeResult {
    let (theirs, renumbered) = renumber_colliding(base, ours, theirs);
    let mut m = Merger {
        resolutions,
        forced: other_bases
            .iter()
            .flat_map(|other| changed_keys(base, other))
            .collect(),
        conflicts: Vec::new(),
    };

    let name = m.pick(
        ConflictKey::DocumentName,
        &base.name,
        &ours.name,
        &theirs.name,
    );
    // Values are stored in the document's unit, so a unit change on one
    // side reinterprets every number the other side entered. That needs a
    // person to decide, even though the unit itself merges cleanly.
    let units = if (ours.units != base.units
        && theirs.units == base.units
        && values_changed(base, &theirs))
        || (theirs.units != base.units && ours.units == base.units && values_changed(base, ours))
    {
        m.conflict(
            ConflictKey::Units,
            Some(&base.units),
            Some(&ours.units),
            Some(&theirs.units),
            &ours.units,
            &theirs.units,
        )
    } else {
        m.pick(ConflictKey::Units, &base.units, &ours.units, &theirs.units)
    };
    let parameters = m.parameters(base, ours, &theirs);
    let features = m.features(base, ours, &theirs);
    // A pick for the marker applies even when the marker itself merged
    // cleanly, since it can also settle the rollback check below.
    let end_of_part = match resolutions.get(&ConflictKey::EndOfPart) {
        Some(Side::Ours) => ours.end_of_part,
        Some(Side::Theirs) => theirs.end_of_part,
        None => m.pick(
            ConflictKey::EndOfPart,
            &base.end_of_part,
            &ours.end_of_part,
            &theirs.end_of_part,
        ),
    };

    // Start from ours so private bookkeeping (the id counter) carries over,
    // then make sure the counter is above every id either side has used.
    let mut document = ours.clone();
    document.name = name;
    document.units = units;
    document.parameters = parameters;
    document.features = features;
    document.end_of_part =
        end_of_part.and_then(|id| reanchor(id, &document, [ours, &theirs, base]));
    document.repair_ids();
    if !resolutions.contains_key(&ConflictKey::EndOfPart)
        && !m.conflicts.iter().any(|c| c.key == ConflictKey::EndOfPart)
        && (changes_new_activity(base, ours, &document)
            || changes_new_activity(base, &theirs, &document))
    {
        // A pick for the marker settles this on the next merge (handled
        // above), so the value returned here is not used.
        m.conflict(
            ConflictKey::EndOfPart,
            Some(&base.end_of_part),
            Some(&ours.end_of_part),
            Some(&theirs.end_of_part),
            &ours.end_of_part,
            &theirs.end_of_part,
        );
    }
    let invalid = document.validate().err();

    MergeResult {
        document,
        conflicts: m.conflicts,
        renumbered,
        invalid,
    }
}

/// True if `side` changed any number relative to `base`: a parameter value,
/// a feature definition, or a new feature.
fn values_changed(base: &PartDocument, side: &PartDocument) -> bool {
    crate::diff(base, side).iter().any(|c| match c {
        Change::ParameterAdded(_) | Change::ParameterChanged(_) | Change::FeatureAdded { .. } => {
            true
        }
        Change::FeatureChanged { fields, .. } => fields.contains(&"definition"),
        _ => false,
    })
}

/// The conflict keys for everything that differs between two documents.
fn changed_keys(a: &PartDocument, b: &PartDocument) -> Vec<ConflictKey> {
    let feature = |id, field| ConflictKey::Feature { id, field };
    let mut keys = Vec::new();
    for change in crate::diff(a, b) {
        match change {
            Change::Renamed { .. } => keys.push(ConflictKey::DocumentName),
            Change::UnitsChanged => keys.push(ConflictKey::Units),
            Change::ParameterAdded(name)
            | Change::ParameterRemoved(name)
            | Change::ParameterChanged(name) => keys.push(ConflictKey::Parameter { name }),
            Change::FeatureAdded { id, .. } | Change::FeatureRemoved { id, .. } => {
                keys.push(feature(id, FeatureField::Existence))
            }
            Change::FeatureChanged { id, fields, .. } => keys.extend(fields.iter().map(|f| {
                feature(
                    id,
                    match *f {
                        "name" => FeatureField::Name,
                        "suppressed" => FeatureField::Suppressed,
                        _ => FeatureField::Definition,
                    },
                )
            })),
            Change::Reordered => keys.push(ConflictKey::Order),
            Change::EndOfPartMoved => keys.push(ConflictKey::EndOfPart),
        }
    }
    keys
}

/// True if a feature `side` added ends up active in `merged` while it was
/// rolled back on that side, or the other way round. Either would silently
/// add or drop that side's work.
fn changes_new_activity(base: &PartDocument, side: &PartDocument, merged: &PartDocument) -> bool {
    let above_marker = |doc: &PartDocument| -> HashSet<FeatureId> {
        let end = doc
            .end_of_part
            .and_then(|id| doc.features.iter().position(|f| f.id == id))
            .unwrap_or(doc.features.len());
        doc.features[..end].iter().map(|f| f.id).collect()
    };
    let (active_there, active_here) = (above_marker(side), above_marker(merged));
    side.features.iter().any(|f| {
        base.feature(f.id).is_none()
            && merged.feature(f.id).is_some()
            && active_there.contains(&f.id) != active_here.contains(&f.id)
    })
}

/// The end-of-part marker is stored as the feature below it. If the merge
/// deleted that feature, the marker moves down to the next feature that
/// survived (as it would when deleting the feature in the editor), or to the
/// end of the part. `sides` are searched in order for where `id` was.
fn reanchor(id: FeatureId, merged: &PartDocument, sides: [&PartDocument; 3]) -> Option<FeatureId> {
    if merged.feature(id).is_some() {
        return Some(id);
    }
    let side = sides.into_iter().find(|d| d.feature(id).is_some())?;
    side.features
        .iter()
        .skip_while(|f| f.id != id)
        .map(|f| f.id)
        .find(|f| merged.feature(*f).is_some())
}

struct Merger<'a> {
    resolutions: &'a Resolutions,
    /// Keys that are conflicts whenever ours and theirs differ.
    forced: HashSet<ConflictKey>,
    conflicts: Vec<Conflict>,
}

impl Merger<'_> {
    /// True if `key` (or, for a feature field, the feature as a whole) is
    /// one the merge bases disagree on.
    fn is_forced(&self, key: &ConflictKey) -> bool {
        self.forced.contains(key)
            || matches!(key, ConflictKey::Feature { id, .. } if self.forced.contains(
                &ConflictKey::Feature { id: *id, field: FeatureField::Existence }))
    }

    /// Standard three-way pick of one value.
    fn pick<T: Clone + PartialEq + Serialize>(
        &mut self,
        key: ConflictKey,
        base: &T,
        ours: &T,
        theirs: &T,
    ) -> T {
        if ours == theirs || (theirs == base && !self.is_forced(&key)) {
            return ours.clone();
        }
        if ours == base && !self.is_forced(&key) {
            return theirs.clone();
        }
        self.conflict(key, Some(base), Some(ours), Some(theirs), ours, theirs)
    }

    /// Three-way pick of something that may not exist on every side.
    fn pick_opt<T: Clone + PartialEq + Serialize>(
        &mut self,
        key: ConflictKey,
        base: Option<&T>,
        ours: Option<&T>,
        theirs: Option<&T>,
    ) -> Option<T> {
        if ours == theirs || (theirs == base && !self.is_forced(&key)) {
            return ours.cloned();
        }
        if ours == base && !self.is_forced(&key) {
            return theirs.cloned();
        }
        self.conflict(key, base, ours, theirs, &ours, &theirs)
            .cloned()
    }

    fn conflict<V: Serialize, T: Clone>(
        &mut self,
        key: ConflictKey,
        base: Option<&V>,
        ours: Option<&V>,
        theirs: Option<&V>,
        ours_value: &T,
        theirs_value: &T,
    ) -> T {
        if let Some(side) = self.resolutions.get(&key) {
            return match side {
                Side::Ours => ours_value.clone(),
                Side::Theirs => theirs_value.clone(),
            };
        }
        let json = |v: Option<&V>| v.map(|v| serde_json::to_value(v).expect("serializable"));
        self.conflicts.push(Conflict {
            key,
            base: json(base),
            ours: json(ours),
            theirs: json(theirs),
        });
        ours_value.clone()
    }

    /// Parameters merge by name, each as a whole (value and comment). Order
    /// is ours, with their new parameters appended in their order.
    fn parameters(
        &mut self,
        base: &PartDocument,
        ours: &PartDocument,
        theirs: &PartDocument,
    ) -> Vec<sm_document::Parameter> {
        fn find<'a>(doc: &'a PartDocument, name: &str) -> Option<&'a sm_document::Parameter> {
            doc.parameters.iter().find(|p| p.name == name)
        }
        let mut names: Vec<&str> = Vec::new();
        for p in ours
            .parameters
            .iter()
            .chain(&theirs.parameters)
            .chain(&base.parameters)
        {
            if !names.contains(&p.name.as_str()) {
                names.push(&p.name);
            }
        }
        names
            .into_iter()
            .filter_map(|name| {
                self.pick_opt(
                    ConflictKey::Parameter { name: name.into() },
                    find(base, name),
                    find(ours, name),
                    find(theirs, name),
                )
            })
            .collect()
    }

    fn features(
        &mut self,
        base: &PartDocument,
        ours: &PartDocument,
        theirs: &PartDocument,
    ) -> Vec<Feature> {
        fn index(doc: &PartDocument) -> HashMap<FeatureId, &Feature> {
            doc.features.iter().map(|f| (f.id, f)).collect()
        }
        let (b, o, t) = (index(base), index(ours), index(theirs));

        let mut merged: HashMap<FeatureId, Feature> = HashMap::new();
        let mut ids: Vec<FeatureId> = b.keys().chain(o.keys()).chain(t.keys()).copied().collect();
        ids.sort();
        ids.dedup();
        for id in ids {
            let (fb, fo, ft) = (
                b.get(&id).copied(),
                o.get(&id).copied(),
                t.get(&id).copied(),
            );
            let feature = match (fb, fo, ft) {
                (Some(fb), Some(fo), Some(ft)) => Some(self.feature_fields(fb, fo, ft)),
                _ => self.pick_opt(
                    ConflictKey::Feature {
                        id,
                        field: FeatureField::Existence,
                    },
                    fb,
                    fo,
                    ft,
                ),
            };
            if let Some(feature) = feature {
                merged.insert(id, feature);
            }
        }

        let order = self.order(base, ours, theirs, &merged);
        order
            .into_iter()
            .map(|id| merged.remove(&id).expect("ordered ids are merged ids"))
            .collect()
    }

    fn feature_fields(&mut self, b: &Feature, o: &Feature, t: &Feature) -> Feature {
        let key = |field| ConflictKey::Feature { id: o.id, field };
        Feature {
            id: o.id,
            name: self.pick(key(FeatureField::Name), &b.name, &o.name, &t.name),
            suppressed: self.pick(
                key(FeatureField::Suppressed),
                &b.suppressed,
                &o.suppressed,
                &t.suppressed,
            ),
            kind: self.pick(key(FeatureField::Definition), &b.kind, &o.kind, &t.kind),
        }
    }

    /// Orders the merged features. If only one side reordered the features
    /// all three versions share, its order wins; if both did, differently,
    /// that's a conflict. Features only one side has are then inserted after
    /// the feature they followed on that side.
    fn order(
        &mut self,
        base: &PartDocument,
        ours: &PartDocument,
        theirs: &PartDocument,
        merged: &HashMap<FeatureId, Feature>,
    ) -> Vec<FeatureId> {
        let ids = |doc: &PartDocument| -> HashSet<FeatureId> {
            doc.features.iter().map(|f| f.id).collect()
        };
        let (ib, io, it) = (ids(base), ids(ours), ids(theirs));
        let shared = |doc: &PartDocument| -> Vec<FeatureId> {
            doc.features
                .iter()
                .map(|f| f.id)
                .filter(|id| ib.contains(id) && io.contains(id) && it.contains(id))
                .collect()
        };
        let (sb, so, st) = (shared(base), shared(ours), shared(theirs));
        let forced = self.is_forced(&ConflictKey::Order);
        let primary_is_theirs = if so == st || (st == sb && !forced) {
            false
        } else if so == sb && !forced {
            true
        } else {
            self.conflict(
                ConflictKey::Order,
                Some(&sb),
                Some(&so),
                Some(&st),
                &false,
                &true,
            )
        };
        let (primary, other) = if primary_is_theirs {
            (theirs, ours)
        } else {
            (ours, theirs)
        };

        let mut order: Vec<FeatureId> = primary
            .features
            .iter()
            .map(|f| f.id)
            .filter(|id| merged.contains_key(id))
            .collect();
        // When both sides added features at the same spot, the primary
        // side's come first, so skip past features the other side lacks.
        let in_other: HashSet<FeatureId> = other.features.iter().map(|f| f.id).collect();
        let mut placed: HashSet<FeatureId> = order.iter().copied().collect();
        let mut anchor: Option<FeatureId> = None;
        for id in other.features.iter().map(|f| f.id) {
            if !merged.contains_key(&id) {
                continue;
            }
            if !placed.contains(&id) {
                let mut at = anchor.map_or(0, |a| order.iter().position(|x| *x == a).unwrap() + 1);
                while at < order.len() && !in_other.contains(&order[at]) {
                    at += 1;
                }
                order.insert(at, id);
                placed.insert(id);
            }
            anchor = Some(id);
        }
        // Every merged feature exists on at least one side, so all are placed.
        debug_assert_eq!(order.len(), merged.len());
        order
    }
}

/// Feature ids are handed out per document, so two branches that each add a
/// feature usually give it the same id. Their side's colliding features get
/// fresh ids, and every reference to them on their side is rewritten.
fn renumber_colliding(
    base: &PartDocument,
    ours: &PartDocument,
    theirs: &PartDocument,
) -> (PartDocument, Vec<(FeatureId, FeatureId)>) {
    let mut next = base
        .next_id()
        .0
        .max(ours.next_id().0)
        .max(theirs.next_id().0);
    let mut map: HashMap<FeatureId, FeatureId> = HashMap::new();
    let mut renumbered = Vec::new();
    for t in &theirs.features {
        if base.feature(t.id).is_some() {
            continue;
        }
        match ours.feature(t.id) {
            // Both added exactly the same feature: treat it as one.
            Some(o) if o == t => {}
            Some(_) => {
                // Out of ids: leave the clash, which validation then reports
                // as a duplicate id instead of a panic.
                let Some(after) = next.checked_add(1) else {
                    break;
                };
                let new = FeatureId(next);
                next = after;
                map.insert(t.id, new);
                renumbered.push((t.id, new));
            }
            None => {}
        }
    }
    if map.is_empty() {
        return (theirs.clone(), renumbered);
    }
    let remap = |id: FeatureId| map.get(&id).copied().unwrap_or(id);
    let mut theirs = theirs.clone();
    for f in &mut theirs.features {
        f.id = remap(f.id);
        f.kind.remap_references(remap);
    }
    theirs.end_of_part = theirs.end_of_part.map(remap);
    (theirs, renumbered)
}
