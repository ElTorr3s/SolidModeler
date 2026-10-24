//! The editable part model: user parameters plus an ordered feature history.
//!
//! This is the source of truth that gets saved into `.smp` files. Geometry is
//! always derived from it by replaying the features (see `sm-kernel`).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Stable identifier for a feature. Later features refer to earlier ones by id,
/// so ids survive renames and reordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FeatureId(pub u32);

/// A part document, the equivalent of an Inventor `.ipt`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartDocument {
    pub name: String,
    /// Model length unit. Geometry values are stored in this unit.
    pub units: LengthUnit,
    /// Named user parameters (Inventor's "User Parameters").
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    /// Features in history order.
    #[serde(default)]
    pub features: Vec<Feature>,
    /// The end-of-part marker, stored as the first feature *below* it. That
    /// feature and everything after it are rolled back. `None` means the
    /// marker is at the end of the history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_of_part: Option<FeatureId>,
    /// Next id to hand out. Repaired before use by [`PartDocument::repair_ids`].
    #[serde(default)]
    next_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LengthUnit {
    Mm,
    Cm,
    M,
    In,
}

impl LengthUnit {
    /// Size of one unit in millimetres.
    pub fn to_mm(self) -> f64 {
        match self {
            LengthUnit::Mm => 1.0,
            LengthUnit::Cm => 10.0,
            LengthUnit::M => 1000.0,
            LengthUnit::In => 25.4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub value: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    pub id: FeatureId,
    pub name: String,
    #[serde(default)]
    pub suppressed: bool,
    pub kind: FeatureKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FeatureKind {
    Sketch(Sketch),
    Extrude(Extrude),
}

/// The three origin planes every part starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OriginPlane {
    XY,
    YZ,
    XZ,
}

/// A 2D sketch. For now profiles are closed polylines; arcs, splines and
/// constraints arrive with the sketcher in step 3 of the roadmap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sketch {
    pub plane: OriginPlane,
    /// Offset of the sketch plane along the plane normal.
    #[serde(default)]
    pub offset: f64,
    /// Closed loops. A loop inside another loop is a hole in it, so a
    /// rectangle with a smaller rectangle inside extrudes to a frame.
    pub profiles: Vec<Profile>,
}

/// A closed loop of points in sketch coordinates. The closing segment from
/// the last point back to the first is implicit; repeating the first point
/// at the end is allowed and ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub points: Vec<[f64; 2]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extrude {
    pub sketch: FeatureId,
    pub extent: ExtrudeExtent,
    #[serde(default)]
    pub operation: BodyOperation,
}

/// How a feature's new volume combines with the existing part body, as in
/// Inventor's Join / Cut / Intersect / New Solid buttons.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyOperation {
    #[default]
    Join,
    Cut,
    Intersect,
    NewBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExtrudeExtent {
    /// Extrude along the sketch normal (negative flips the direction).
    Distance { distance: f64 },
    /// Extrude half the distance to each side of the sketch plane.
    Symmetric { distance: f64 },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DocumentError {
    #[error("feature {0:?} does not exist")]
    MissingFeature(FeatureId),
    #[error("feature id {0:?} is used more than once")]
    DuplicateId(FeatureId),
    #[error("feature {0:?} is not a sketch")]
    NotASketch(FeatureId),
    #[error("feature {referrer:?} refers to {target:?}, which comes later in the history")]
    ForwardReference {
        referrer: FeatureId,
        target: FeatureId,
    },
    #[error("{0} is not a finite number")]
    NonFinite(String),
    #[error("parameter name {0:?} is empty, invalid, or used twice")]
    BadParameterName(String),
}

impl PartDocument {
    pub fn new(name: impl Into<String>, units: LengthUnit) -> Self {
        Self {
            name: name.into(),
            units,
            parameters: Vec::new(),
            features: Vec::new(),
            end_of_part: None,
            next_id: 1,
        }
    }

    /// Appends a feature to the end of the history and returns its id.
    pub fn add_feature(&mut self, name: impl Into<String>, kind: FeatureKind) -> FeatureId {
        self.repair_ids();
        let id = FeatureId(self.next_id);
        self.next_id = self.next_id.checked_add(1).expect("feature ids exhausted");
        self.features.push(Feature {
            id,
            name: name.into(),
            suppressed: false,
            kind,
        });
        id
    }

    /// Makes sure `next_id` is above every id in use, so documents edited or
    /// merged outside this crate never hand out an id twice.
    pub fn repair_ids(&mut self) {
        let max = self.features.iter().map(|f| f.id.0).max().unwrap_or(0);
        self.next_id = self.next_id.max(max.saturating_add(1));
    }

    pub fn feature(&self, id: FeatureId) -> Option<&Feature> {
        self.features.iter().find(|f| f.id == id)
    }

    /// Features that take part in a rebuild: above the end-of-part marker
    /// and not suppressed.
    pub fn active_features(&self) -> impl Iterator<Item = &Feature> {
        let end = self
            .end_of_part
            .and_then(|id| self.features.iter().position(|f| f.id == id))
            .unwrap_or(self.features.len());
        self.features[..end].iter().filter(|f| !f.suppressed)
    }

    /// Checks ids, references, numbers and parameter names.
    pub fn validate(&self) -> Result<(), DocumentError> {
        let mut index = HashMap::with_capacity(self.features.len());
        for (i, feature) in self.features.iter().enumerate() {
            if index.insert(feature.id, i).is_some() {
                return Err(DocumentError::DuplicateId(feature.id));
            }
        }
        if let Some(id) = self.end_of_part {
            if !index.contains_key(&id) {
                return Err(DocumentError::MissingFeature(id));
            }
        }

        let mut names = HashSet::new();
        for p in &self.parameters {
            if !is_identifier(&p.name) || !names.insert(p.name.as_str()) {
                return Err(DocumentError::BadParameterName(p.name.clone()));
            }
            finite(p.value, || format!("parameter {}", p.name))?;
        }

        for (i, feature) in self.features.iter().enumerate() {
            let what = || format!("a value in feature {:?} ({})", feature.id, feature.name);
            match &feature.kind {
                FeatureKind::Sketch(sketch) => {
                    finite(sketch.offset, what)?;
                    for p in sketch.profiles.iter().flat_map(|p| &p.points) {
                        finite(p[0], what)?;
                        finite(p[1], what)?;
                    }
                }
                FeatureKind::Extrude(extrude) => {
                    let (ExtrudeExtent::Distance { distance }
                    | ExtrudeExtent::Symmetric { distance }) = extrude.extent;
                    finite(distance, what)?;
                    let target = *index
                        .get(&extrude.sketch)
                        .ok_or(DocumentError::MissingFeature(extrude.sketch))?;
                    if target >= i {
                        return Err(DocumentError::ForwardReference {
                            referrer: feature.id,
                            target: extrude.sketch,
                        });
                    }
                    if !matches!(self.features[target].kind, FeatureKind::Sketch(_)) {
                        return Err(DocumentError::NotASketch(extrude.sketch));
                    }
                }
            }
        }
        Ok(())
    }
}

fn finite(value: f64, what: impl FnOnce() -> String) -> Result<(), DocumentError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(DocumentError::NonFinite(what()))
    }
}

/// Parameter names must work in expressions: a letter or `_`, then letters,
/// digits or `_`.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_sketch() -> FeatureKind {
        FeatureKind::Sketch(Sketch {
            plane: OriginPlane::XZ,
            offset: 0.0,
            profiles: vec![],
        })
    }

    fn block() -> PartDocument {
        let mut doc = PartDocument::new("Block", LengthUnit::Mm);
        let sketch = doc.add_feature(
            "Sketch1",
            FeatureKind::Sketch(Sketch {
                plane: OriginPlane::XY,
                offset: 0.0,
                profiles: vec![Profile {
                    points: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 20.0], [0.0, 20.0]],
                }],
            }),
        );
        doc.add_feature(
            "Extrusion1",
            FeatureKind::Extrude(Extrude {
                sketch,
                extent: ExtrudeExtent::Distance { distance: 10.0 },
                operation: BodyOperation::Join,
            }),
        );
        doc
    }

    #[test]
    fn ids_are_sequential_and_valid() {
        let doc = block();
        assert_eq!(doc.features[0].id, FeatureId(1));
        assert_eq!(doc.features[1].id, FeatureId(2));
        assert_eq!(doc.validate(), Ok(()));
    }

    #[test]
    fn end_of_part_rolls_back_history() {
        let mut doc = block();
        doc.end_of_part = Some(FeatureId(2));
        assert_eq!(doc.active_features().count(), 1);
    }

    #[test]
    fn end_of_part_follows_its_feature_when_reordered() {
        let mut doc = block();
        let extra = doc.add_feature("Sketch2", empty_sketch());
        doc.end_of_part = Some(extra);
        assert_eq!(doc.active_features().count(), 2);
        // Move Sketch2 to the top: the marker moves with it.
        let moved = doc.features.pop().unwrap();
        doc.features.insert(0, moved);
        assert_eq!(doc.active_features().count(), 0);
    }

    #[test]
    fn rejects_forward_reference() {
        let mut doc = block();
        doc.features.swap(0, 1);
        assert!(matches!(
            doc.validate(),
            Err(DocumentError::ForwardReference { .. })
        ));
    }

    #[test]
    fn rejects_duplicate_ids_and_missing_marker() {
        let mut doc = block();
        doc.features[1].id = FeatureId(1);
        assert_eq!(
            doc.validate(),
            Err(DocumentError::DuplicateId(FeatureId(1)))
        );

        let mut doc = block();
        doc.end_of_part = Some(FeatureId(99));
        assert_eq!(
            doc.validate(),
            Err(DocumentError::MissingFeature(FeatureId(99)))
        );
    }

    #[test]
    fn rejects_non_finite_values() {
        let mut doc = block();
        if let FeatureKind::Extrude(e) = &mut doc.features[1].kind {
            e.extent = ExtrudeExtent::Distance { distance: f64::NAN };
        }
        assert!(matches!(doc.validate(), Err(DocumentError::NonFinite(_))));

        let mut doc = block();
        if let FeatureKind::Sketch(s) = &mut doc.features[0].kind {
            s.profiles[0].points[1][0] = f64::INFINITY;
        }
        assert!(matches!(doc.validate(), Err(DocumentError::NonFinite(_))));
    }

    #[test]
    fn rejects_bad_parameter_names() {
        for names in [vec![""], vec!["2width"], vec!["a b"], vec!["d0", "d0"]] {
            let mut doc = block();
            doc.parameters = names
                .into_iter()
                .map(|n| Parameter {
                    name: n.into(),
                    value: 1.0,
                    comment: String::new(),
                })
                .collect();
            assert!(matches!(
                doc.validate(),
                Err(DocumentError::BadParameterName(_))
            ));
        }
    }

    #[test]
    fn repairs_next_id_after_load() {
        let json = r#"{"name":"P","units":"mm","features":[
            {"id":7,"name":"S","kind":{"type":"sketch","plane":"XY","profiles":[]}}]}"#;
        let mut doc: PartDocument = serde_json::from_str(json).unwrap();
        assert_eq!(doc.add_feature("S2", empty_sketch()), FeatureId(8));
    }

    #[test]
    fn json_round_trip() {
        let mut doc = block();
        doc.parameters.push(Parameter {
            name: "width".into(),
            value: 40.0,
            comment: "overall".into(),
        });
        doc.features[1].suppressed = true;
        doc.end_of_part = Some(FeatureId(2));
        let json = serde_json::to_string(&doc).unwrap();
        let back: PartDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(doc, back);
    }
}
