//! What changed between two versions of a part, at the feature level. This
//! is the change log shown for each version; versions themselves are stored
//! as whole snapshots (see `docs/versioning.md`).

use sm_document::{FeatureId, PartDocument};
use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Renamed {
        from: String,
        to: String,
    },
    UnitsChanged,
    ParameterAdded(String),
    ParameterRemoved(String),
    ParameterChanged(String),
    FeatureAdded {
        id: FeatureId,
        name: String,
    },
    FeatureRemoved {
        id: FeatureId,
        name: String,
    },
    /// `fields` lists what changed: "name", "suppressed", "definition".
    FeatureChanged {
        id: FeatureId,
        name: String,
        fields: Vec<&'static str>,
    },
    /// The features both versions have are in a different order.
    Reordered,
    EndOfPartMoved,
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Change::Renamed { from, to } => write!(f, "renamed part {from:?} to {to:?}"),
            Change::UnitsChanged => write!(f, "changed units"),
            Change::ParameterAdded(n) => write!(f, "added parameter {n}"),
            Change::ParameterRemoved(n) => write!(f, "removed parameter {n}"),
            Change::ParameterChanged(n) => write!(f, "changed parameter {n}"),
            Change::FeatureAdded { name, .. } => write!(f, "added {name}"),
            Change::FeatureRemoved { name, .. } => write!(f, "removed {name}"),
            Change::FeatureChanged { name, fields, .. } => {
                write!(f, "changed {name} ({})", fields.join(", "))
            }
            Change::Reordered => write!(f, "reordered features"),
            Change::EndOfPartMoved => write!(f, "moved end of part"),
        }
    }
}

/// Changes that turn `old` into `new`, in a stable order: part, parameters,
/// then features in `new`'s order, then removals.
pub fn diff(old: &PartDocument, new: &PartDocument) -> Vec<Change> {
    let index = |doc: &PartDocument| -> HashMap<FeatureId, usize> {
        doc.features
            .iter()
            .enumerate()
            .map(|(i, f)| (f.id, i))
            .collect()
    };
    let (old_index, new_index) = (index(old), index(new));
    let mut changes = Vec::new();
    if old.name != new.name {
        changes.push(Change::Renamed {
            from: old.name.clone(),
            to: new.name.clone(),
        });
    }
    if old.units != new.units {
        changes.push(Change::UnitsChanged);
    }

    for p in &new.parameters {
        match old.parameters.iter().find(|q| q.name == p.name) {
            None => changes.push(Change::ParameterAdded(p.name.clone())),
            Some(q) if q != p => changes.push(Change::ParameterChanged(p.name.clone())),
            Some(_) => {}
        }
    }
    for q in &old.parameters {
        if !new.parameters.iter().any(|p| p.name == q.name) {
            changes.push(Change::ParameterRemoved(q.name.clone()));
        }
    }

    for f in &new.features {
        match old_index.get(&f.id).map(|&i| &old.features[i]) {
            None => changes.push(Change::FeatureAdded {
                id: f.id,
                name: f.name.clone(),
            }),
            Some(g) => {
                let mut fields = Vec::new();
                if g.name != f.name {
                    fields.push("name");
                }
                if g.suppressed != f.suppressed {
                    fields.push("suppressed");
                }
                if g.kind != f.kind {
                    fields.push("definition");
                }
                if !fields.is_empty() {
                    changes.push(Change::FeatureChanged {
                        id: f.id,
                        name: f.name.clone(),
                        fields,
                    });
                }
            }
        }
    }
    for g in &old.features {
        if !new_index.contains_key(&g.id) {
            changes.push(Change::FeatureRemoved {
                id: g.id,
                name: g.name.clone(),
            });
        }
    }

    let shared = |a: &PartDocument, b: &HashMap<FeatureId, usize>| -> Vec<FeatureId> {
        a.features
            .iter()
            .map(|f| f.id)
            .filter(|id| b.contains_key(id))
            .collect()
    };
    if shared(old, &new_index) != shared(new, &old_index) {
        changes.push(Change::Reordered);
    }
    if old.end_of_part != new.end_of_part {
        changes.push(Change::EndOfPartMoved);
    }
    changes
}
