//! The editable part model: user parameters plus an ordered feature history.
//!
//! This is the source of truth that gets saved into `.smp` files. Geometry is
//! always derived from it by replaying the features (see `sm-kernel`).

use serde::{Deserialize, Serialize};

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
    /// Index into `features` of the end-of-part marker. Features at or after
    /// this index are suppressed by rollback. `None` means the whole history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_of_part: Option<usize>,
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
    pub profiles: Vec<Profile>,
}

/// A closed loop of points in sketch coordinates. The closing segment from
/// the last point back to the first is implicit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub points: Vec<[f64; 2]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extrude {
    pub sketch: FeatureId,
    pub extent: ExtrudeExtent,
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
    #[error("feature {0:?} is not a sketch")]
    NotASketch(FeatureId),
    #[error("feature {referrer:?} refers to {target:?}, which comes later in the history")]
    ForwardReference {
        referrer: FeatureId,
        target: FeatureId,
    },
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
        let id = FeatureId(self.next_id);
        self.next_id += 1;
        self.features.push(Feature {
            id,
            name: name.into(),
            suppressed: false,
            kind,
        });
        id
    }

    pub fn feature(&self, id: FeatureId) -> Option<&Feature> {
        self.features.iter().find(|f| f.id == id)
    }

    /// Features that take part in a rebuild: before the end-of-part marker
    /// and not suppressed.
    pub fn active_features(&self) -> impl Iterator<Item = &Feature> {
        let end = self
            .end_of_part
            .unwrap_or(self.features.len())
            .min(self.features.len());
        self.features[..end].iter().filter(|f| !f.suppressed)
    }

    /// Checks that every reference points at an earlier sketch.
    pub fn validate(&self) -> Result<(), DocumentError> {
        for (index, feature) in self.features.iter().enumerate() {
            if let FeatureKind::Extrude(extrude) = &feature.kind {
                let target_index = self
                    .features
                    .iter()
                    .position(|f| f.id == extrude.sketch)
                    .ok_or(DocumentError::MissingFeature(extrude.sketch))?;
                if target_index >= index {
                    return Err(DocumentError::ForwardReference {
                        referrer: feature.id,
                        target: extrude.sketch,
                    });
                }
                if !matches!(self.features[target_index].kind, FeatureKind::Sketch(_)) {
                    return Err(DocumentError::NotASketch(extrude.sketch));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        doc.end_of_part = Some(1);
        assert_eq!(doc.active_features().count(), 1);
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
    fn json_round_trip() {
        let doc = block();
        let json = serde_json::to_string(&doc).unwrap();
        let back: PartDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(doc, back);
    }
}
