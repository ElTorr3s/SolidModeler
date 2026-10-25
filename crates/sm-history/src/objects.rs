//! Content-addressed storage of the pieces of a part: each feature and each
//! document header (name, units, parameters, marker) is stored once, named
//! by the BLAKE3 hash of its canonical JSON. Versions list hashes, so a
//! version that changes one number costs one small object, not a copy of
//! the whole part.
//!
//! Objects are kept as raw JSON text and parsed only when a version is
//! opened, so loading a long history costs little more than reading it.
//! Their text is canonical (compact, keys sorted) and must stay byte for
//! byte as written: an object whose text doesn't hash to its name is
//! rejected.

use crate::HistoryError;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::fmt;

/// A BLAKE3 hash as 64 lowercase hex digits. Used for objects and versions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Hash(String);

impl Hash {
    pub fn of(bytes: &[u8]) -> Self {
        Hash(blake3::hash(bytes).to_hex().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Hash {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        if s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            Ok(Hash(s))
        } else {
            // Debug formatting escapes anything that could reach a terminal.
            Err(format!("{s:?} is not a 64-digit lowercase hex hash"))
        }
    }
}

impl From<Hash> for String {
    fn from(h: Hash) -> String {
        h.0
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The object store: hash to canonical JSON text.
#[derive(Debug, Clone, Default)]
pub(crate) struct Objects(BTreeMap<Hash, Box<RawValue>>);

impl PartialEq for Objects {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .zip(&other.0)
                .all(|((h, a), (k, b))| h == k && a.get() == b.get())
    }
}

/// Serialized form: a map from hash to the object, written verbatim.
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct ObjectsData(BTreeMap<Hash, RawText>);

/// An object's JSON text, kept verbatim.
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct RawText(Box<RawValue>);

impl Objects {
    /// Stores `value` and returns its hash. Storing the same value twice
    /// stores it once.
    pub fn put<T: Serialize>(&mut self, value: &T) -> Result<Hash, HistoryError> {
        // Canonical text: compact, with object keys sorted (serde_json's
        // Value orders them), so equal values always hash the same.
        let canonical = serde_json::to_value(value).expect("documents serialize");
        let text = canonical.to_string();
        let hash = Hash::of(text.as_bytes());
        match self.0.get(&hash) {
            None => {
                let raw = RawValue::from_string(text).expect("serde_json output is JSON");
                self.0.insert(hash.clone(), raw);
            }
            Some(existing) if existing.get() == text => {}
            // Two different texts with one BLAKE3 hash: never in practice,
            // but never silently treat them as the same object.
            Some(_) => return Err(HistoryError::Invalid(format!("hash collision on {hash}"))),
        }
        Ok(hash)
    }

    pub fn get<T: DeserializeOwned>(&self, hash: &Hash) -> Result<T, HistoryError> {
        let raw = self
            .0
            .get(hash)
            .ok_or_else(|| HistoryError::Invalid(format!("missing object {hash}")))?;
        serde_json::from_str(raw.get())
            .map_err(|e| HistoryError::Invalid(format!("object {hash}: {e}")))
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn contains(&self, hash: &Hash) -> bool {
        self.0.contains_key(hash)
    }

    pub fn to_data(&self) -> ObjectsData {
        ObjectsData(
            self.0
                .iter()
                .map(|(h, raw)| (h.clone(), RawText(raw.clone())))
                .collect(),
        )
    }

    /// Checks that every object is named by the hash of its exact text, so
    /// a stored object can't stand in for a different one.
    pub fn from_data(data: ObjectsData) -> Result<Self, HistoryError> {
        let mut objects = BTreeMap::new();
        for (hash, RawText(raw)) in data.0 {
            if Hash::of(raw.get().as_bytes()) != hash {
                return Err(HistoryError::Invalid(format!(
                    "object {hash} does not match its hash"
                )));
            }
            objects.insert(hash, raw);
        }
        Ok(Objects(objects))
    }
}
