//! Persistent identifiers for everything a document stores.
//!
//! Every object (document, feature, sketch point and curve, constraint, value,
//! parameter, body) gets a random UUIDv7 when it is created. Ids are never
//! reused or renumbered, so two branches of a document can be merged without
//! remapping, and references survive renames and reordering. One namespace
//! covers all kinds of object; the typed wrappers only stop one kind being
//! passed where another is expected.
//!
//! Ids come from an [`IdSource`], so tests can use [`IdSource::seeded`] and
//! get the same ids on every run.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// A new id from `ids`.
            pub fn new(ids: &mut IdSource) -> Self {
                Self(ids.new_id())
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_type!(
    /// A document. Kept by every version of it; "Save As" makes a new one.
    DocumentId
);
id_type!(
    /// A feature in a part's history.
    FeatureId
);
id_type!(
    /// A sketch point.
    PointId
);
id_type!(
    /// A sketch curve (line, circle, arc). Features name the faces a curve
    /// sweeps out after it.
    CurveId
);
id_type!(
    /// A sketch constraint or dimension.
    ConstraintId
);
id_type!(
    /// A user-entered value (dimension, distance, angle, parameter value).
    ValueId
);
id_type!(
    /// A user parameter.
    ParamId
);
id_type!(
    /// A solid body.
    BodyId
);

/// Where new ids come from: random UUIDv7s in the application, a seeded
/// sequence in tests.
#[derive(Debug, Clone)]
pub struct IdSource(Kind);

#[derive(Debug, Clone)]
enum Kind {
    Random,
    Seeded { state: u64, count: u64 },
}

impl Default for IdSource {
    fn default() -> Self {
        Self::random()
    }
}

impl IdSource {
    /// Random UUIDv7s: time-ordered, with 74 random bits.
    pub fn random() -> Self {
        Self(Kind::Random)
    }

    /// A deterministic sequence of valid UUIDv7s for tests. Each id's
    /// timestamp is its position in the sequence, so they also sort in
    /// creation order.
    pub fn seeded(seed: u64) -> Self {
        Self(Kind::Seeded {
            state: seed,
            count: 0,
        })
    }

    pub fn new_id(&mut self) -> Uuid {
        match &mut self.0 {
            Kind::Random => Uuid::now_v7(),
            Kind::Seeded { state, count } => {
                *count += 1;
                let mut random = [0u8; 10];
                for chunk in random.chunks_mut(8) {
                    let bytes = splitmix64(state).to_le_bytes();
                    chunk.copy_from_slice(&bytes[..chunk.len()]);
                }
                uuid::Builder::from_unix_timestamp_millis(*count, &random).into_uuid()
            }
        }
    }
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_v7_and_unique() {
        for mut ids in [IdSource::random(), IdSource::seeded(1)] {
            let all: HashSet<_> = (0..10_000).map(|_| ids.new_id()).collect();
            assert_eq!(all.len(), 10_000);
            assert!(all.iter().all(|u| u.get_version_num() == 7));
            assert!(all
                .iter()
                .all(|u| u.get_variant() == uuid::Variant::RFC4122));
        }
    }

    #[test]
    fn seeded_ids_repeat_and_sort_in_creation_order() {
        let a: Vec<_> = {
            let mut ids = IdSource::seeded(42);
            (0..100).map(|_| ids.new_id()).collect()
        };
        let mut ids = IdSource::seeded(42);
        let b: Vec<_> = (0..100).map(|_| ids.new_id()).collect();
        assert_eq!(a, b);
        assert!(a.windows(2).all(|w| w[0] < w[1]));
        assert_ne!(a[0], IdSource::seeded(43).new_id());
    }

    #[test]
    fn serializes_as_lowercase_hyphenated_string() {
        let id = FeatureId::new(&mut IdSource::seeded(7));
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{}\"", id.0.hyphenated()));
        assert_eq!(json, json.to_lowercase());
        assert_eq!(serde_json::from_str::<FeatureId>(&json).unwrap(), id);
        assert!(serde_json::from_str::<FeatureId>("7").is_err());
        assert!(serde_json::from_str::<FeatureId>("\"not-a-uuid\"").is_err());
    }
}
