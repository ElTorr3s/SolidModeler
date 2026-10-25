//! Versions, branches and merges of a part's feature history, in the style
//! of Onshape but stored locally.
//!
//! A [`History`] is a graph of [`Version`]s. Each version records the
//! [`PartDocument`] (parameters and feature tree, no geometry) as a list of
//! content-addressed objects, plus its parent versions, a message and a
//! time. Branches and tags are names that point at versions; the head is the
//! branch (or version) the open document was checked out from. Merges work
//! on the feature tree, see [`merge`].
//!
//! The open document itself lives outside the history (in `document.json`),
//! so it can have uncommitted changes, like a git working tree.

mod diff;
pub mod merge;
mod objects;

pub use diff::{diff, Change};
pub use merge::{
    merge3, merge3_with_bases, Conflict, ConflictKey, FeatureField, MergeResult, Resolutions, Side,
};
pub use objects::Hash;

use objects::{Objects, ObjectsData};
use serde::{Deserialize, Serialize};
use sm_document::{is_unsafe_text_char, DocumentError, Feature, FeatureId, PartDocument};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use thiserror::Error;

/// Branch every new history starts on.
pub const DEFAULT_BRANCH: &str = "main";
/// Longest branch or tag name, in characters.
pub const MAX_NAME_CHARS: usize = 100;
/// Most versions one history may hold.
pub const MAX_VERSIONS: usize = 100_000;

/// Identifies a version: the BLAKE3 hash of its record (parents, message,
/// time and the hashes of its contents).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VersionId(objects::Hash);

impl VersionId {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The first 8 digits, enough to tell versions apart in a log.
    pub fn short(&self) -> &str {
        &self.as_str()[..8]
    }
}

impl fmt::Display for VersionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// One recorded state of the part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Version {
    pub id: VersionId,
    /// Empty for the first version, two for a merge (ours first).
    #[serde(default)]
    pub parents: Vec<VersionId>,
    pub message: String,
    /// Seconds since the Unix epoch.
    pub time: u64,
    tree: Tree,
}

/// A document as object hashes: the header (the document without its
/// features) and each feature in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Tree {
    header: Hash,
    features: Vec<Hash>,
}

/// What a version id is hashed from.
#[derive(Serialize)]
struct VersionRecord<'a> {
    parents: &'a [VersionId],
    message: &'a str,
    time: u64,
    tree: &'a Tree,
}

/// What the open document was checked out from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Head {
    /// On a branch: new versions move the branch forward.
    Branch(String),
    /// On a specific version (a tag or an old version). New versions are
    /// still recorded but no branch points at them until one is created.
    Version(VersionId),
}

/// Errors. Text that came from a file is shown with `{:?}`, which escapes
/// anything that could act on a terminal.
#[derive(Debug, Error, PartialEq)]
pub enum HistoryError {
    #[error("no branch, tag or version called {0:?}")]
    UnknownRef(String),
    #[error("{0:?} is not a valid branch or tag name")]
    BadName(String),
    #[error("a branch or tag called {0:?} already exists")]
    NameTaken(String),
    #[error("the open document has changes that aren't saved as a version")]
    UncommittedChanges,
    #[error("nothing changed since the last version")]
    NoChanges,
    #[error("can't delete the branch that is checked out")]
    DeleteCurrentBranch,
    #[error("the two versions have no common ancestor")]
    NoCommonAncestor,
    #[error("a history can hold at most {MAX_VERSIONS} versions")]
    TooManyVersions,
    #[error("invalid history: {0}")]
    Invalid(String),
    #[error("invalid document: {0}")]
    Document(#[from] DocumentError),
}

/// What [`History::merge`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum MergeOutcome {
    /// Their version is already part of ours; nothing to do.
    UpToDate,
    /// Ours had no changes of its own, so the head simply moved to theirs.
    /// Load this document.
    FastForward(PartDocument),
    /// A merge version was recorded. Load this document.
    Merged {
        version: VersionId,
        document: PartDocument,
    },
    /// Nothing was recorded: resolve the conflicts (or fix the invalid
    /// document) and merge again with resolutions.
    Conflicted(MergeResult),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "HistoryData", into = "HistoryData")]
pub struct History {
    head: Head,
    branches: BTreeMap<String, VersionId>,
    tags: BTreeMap<String, VersionId>,
    /// Append-only, parents before children, so the list is in topological
    /// order and new versions only ever add lines at the end.
    versions: Vec<Version>,
    index: HashMap<VersionId, usize>,
    objects: Objects,
    /// Feature ids are handed out from this counter on every branch, so two
    /// branches never give different features the same id.
    next_feature_id: u32,
}

/// The stored form of a [`History`], validated on load.
#[derive(Serialize, Deserialize)]
struct HistoryData {
    head: Head,
    branches: BTreeMap<String, VersionId>,
    #[serde(default)]
    tags: BTreeMap<String, VersionId>,
    #[serde(default)]
    next_feature_id: u32,
    versions: Vec<Version>,
    objects: ObjectsData,
}

impl History {
    /// Starts a history whose first version is `doc`, on [`DEFAULT_BRANCH`].
    pub fn new(doc: &PartDocument, message: &str, time: u64) -> Result<Self, HistoryError> {
        doc.validate()?;
        let mut history = History {
            head: Head::Branch(DEFAULT_BRANCH.into()),
            branches: BTreeMap::new(),
            tags: BTreeMap::new(),
            versions: Vec::new(),
            index: HashMap::new(),
            objects: Objects::default(),
            next_feature_id: 0,
        };
        let id = history.record(Vec::new(), doc, message, time)?;
        history.branches.insert(DEFAULT_BRANCH.into(), id);
        Ok(history)
    }

    pub fn head(&self) -> &Head {
        &self.head
    }

    pub fn current_branch(&self) -> Option<&str> {
        match &self.head {
            Head::Branch(name) => Some(name),
            Head::Version(_) => None,
        }
    }

    pub fn head_version(&self) -> &Version {
        let id = match &self.head {
            Head::Branch(name) => &self.branches[name],
            Head::Version(id) => id,
        };
        self.version(id).expect("head is valid")
    }

    pub fn version(&self, id: &VersionId) -> Option<&Version> {
        self.index.get(id).map(|&i| &self.versions[i])
    }

    pub fn versions(&self) -> &[Version] {
        &self.versions
    }

    pub fn branches(&self) -> &BTreeMap<String, VersionId> {
        &self.branches
    }

    pub fn tags(&self) -> &BTreeMap<String, VersionId> {
        &self.tags
    }

    /// The part as recorded in version `id`. Stored versions are parsed and
    /// validated here, when they are used, not when the history is loaded.
    pub fn document(&self, id: &VersionId) -> Result<PartDocument, HistoryError> {
        let version = self
            .version(id)
            .ok_or_else(|| HistoryError::UnknownRef(id.to_string()))?;
        let mut doc: PartDocument = self.objects.get(&version.tree.header)?;
        if !doc.features.is_empty() {
            return Err(HistoryError::Invalid(format!(
                "header of version {id} contains features"
            )));
        }
        doc.features = version
            .tree
            .features
            .iter()
            .map(|h| self.objects.get::<Feature>(h))
            .collect::<Result<_, _>>()?;
        doc.repair_ids();
        doc.validate()?;
        Ok(doc)
    }

    /// The part as recorded in the head version.
    pub fn head_document(&self) -> Result<PartDocument, HistoryError> {
        self.document(&self.head_version().id)
    }

    /// Finds a version by branch name, tag name, full id, or an id prefix of
    /// at least 4 digits that matches exactly one version, in that order.
    pub fn resolve(&self, name: &str) -> Result<VersionId, HistoryError> {
        if let Some(id) = self.branches.get(name).or_else(|| self.tags.get(name)) {
            return Ok(id.clone());
        }
        if name.len() >= 4 {
            let mut matches = self
                .versions
                .iter()
                .filter(|v| v.id.as_str().starts_with(name));
            if let (Some(v), None) = (matches.next(), matches.next()) {
                return Ok(v.id.clone());
            }
        }
        Err(HistoryError::UnknownRef(name.into()))
    }

    /// Records `doc` as a new version on top of the head.
    pub fn commit(
        &mut self,
        doc: &PartDocument,
        message: &str,
        time: u64,
    ) -> Result<VersionId, HistoryError> {
        doc.validate()?;
        if same_content(&self.head_document()?, doc) {
            return Err(HistoryError::NoChanges);
        }
        let parents = vec![self.head_version().id.clone()];
        let id = self.record(parents, doc, message, time)?;
        self.advance_head(id.clone());
        Ok(id)
    }

    /// Creates a branch pointing at `at` (a branch, tag or version).
    pub fn create_branch(&mut self, name: &str, at: &str) -> Result<(), HistoryError> {
        let id = self.resolve(at)?;
        self.check_new_name(name)?;
        self.branches.insert(name.into(), id);
        Ok(())
    }

    pub fn delete_branch(&mut self, name: &str) -> Result<(), HistoryError> {
        if self.current_branch() == Some(name) {
            return Err(HistoryError::DeleteCurrentBranch);
        }
        self.branches
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| HistoryError::UnknownRef(name.into()))
    }

    /// Names a version, like Onshape's named versions or a release. Tags
    /// never move.
    pub fn create_tag(&mut self, name: &str, at: &str) -> Result<(), HistoryError> {
        let id = self.resolve(at)?;
        self.check_new_name(name)?;
        self.tags.insert(name.into(), id);
        Ok(())
    }

    /// Switches to a branch, tag or version and returns the document to
    /// open. Refuses if `working` (the open document) has changes that
    /// aren't recorded, so nothing is lost.
    pub fn checkout(
        &mut self,
        target: &str,
        working: &PartDocument,
    ) -> Result<PartDocument, HistoryError> {
        self.require_clean(working)?;
        let id = self.resolve(target)?;
        let doc = self.open(&id)?;
        self.head = if self.branches.contains_key(target) {
            Head::Branch(target.into())
        } else {
            Head::Version(id)
        };
        Ok(doc)
    }

    /// Merges `theirs` (a branch, tag or version) into the head. `working`
    /// must have no uncommitted changes. Conflicts are settled by
    /// `resolutions`; any left over return [`MergeOutcome::Conflicted`]
    /// without recording anything.
    pub fn merge(
        &mut self,
        theirs: &str,
        working: &PartDocument,
        resolutions: &Resolutions,
        time: u64,
    ) -> Result<MergeOutcome, HistoryError> {
        self.require_clean(working)?;
        let ours_id = self.head_version().id.clone();
        let theirs_id = self.resolve(theirs)?;
        let bases = self.merge_bases(&ours_id, &theirs_id);
        let base_id = bases
            .first()
            .cloned()
            .ok_or(HistoryError::NoCommonAncestor)?;
        if base_id == theirs_id {
            return Ok(MergeOutcome::UpToDate);
        }
        if base_id == ours_id {
            let doc = self.open(&theirs_id)?;
            self.advance_head(theirs_id);
            return Ok(MergeOutcome::FastForward(doc));
        }

        let other_bases = bases[1..]
            .iter()
            .map(|id| self.document(id))
            .collect::<Result<Vec<_>, _>>()?;
        let result = merge3_with_bases(
            &self.document(&base_id)?,
            &other_bases,
            &self.document(&ours_id)?,
            &self.document(&theirs_id)?,
            resolutions,
        );
        if !result.is_clean() {
            return Ok(MergeOutcome::Conflicted(result));
        }
        let into = self
            .current_branch()
            .unwrap_or("detached version")
            .to_owned();
        let message = format!("Merge {theirs} into {into}");
        let id = self.record(vec![ours_id, theirs_id], &result.document, &message, time)?;
        self.advance_head(id.clone());
        Ok(MergeOutcome::Merged {
            version: id.clone(),
            document: self.open(&id)?,
        })
    }

    /// The nearest version both `a` and `b` descend from. After criss-cross
    /// merges there can be several equally near ones; this is the one
    /// recorded last, and [`History::merge_bases`] lists them all. Merging
    /// uses this one as the base and makes everything the others disagree
    /// on a conflict (see [`merge3_with_bases`]).
    pub fn merge_base(&self, a: &VersionId, b: &VersionId) -> Option<VersionId> {
        self.merge_bases(a, b).into_iter().next()
    }

    /// Every nearest common ancestor of `a` and `b` (none is an ancestor of
    /// another), the one recorded last first.
    pub fn merge_bases(&self, a: &VersionId, b: &VersionId) -> Vec<VersionId> {
        let of_a = self.ancestors(a);
        let mut common: Vec<usize> = self.ancestors(b).intersection(&of_a).copied().collect();
        // Versions are in topological order: walking from the newest, each
        // nearest ancestor is reached before its own ancestors are dropped.
        common.sort_unstable_by(|x, y| y.cmp(x));
        let mut dropped = HashSet::new();
        let mut nearest = Vec::new();
        for i in common {
            if dropped.contains(&i) {
                continue;
            }
            nearest.push(self.versions[i].id.clone());
            let mut older = self.ancestors(&self.versions[i].id);
            older.remove(&i);
            dropped.extend(older);
        }
        nearest
    }

    /// The next feature id no branch has used. Documents opened from the
    /// history are raised to it; callers that build the open document some
    /// other way should do the same with [`PartDocument::reserve_ids`].
    pub fn next_feature_id(&self) -> FeatureId {
        FeatureId(self.next_feature_id)
    }

    /// `id` and every version it descends from, newest first.
    pub fn log(&self, id: &VersionId) -> Vec<&Version> {
        let mut found: Vec<usize> = self.ancestors(id).into_iter().collect();
        found.sort_unstable_by(|a, b| b.cmp(a));
        found.into_iter().map(|i| &self.versions[i]).collect()
    }

    /// What changed in a version compared with its first parent.
    pub fn changes(&self, version: &Version) -> Result<Vec<Change>, HistoryError> {
        let doc = self.document(&version.id)?;
        let parent = match version.parents.first() {
            Some(p) => self.document(p)?,
            None => PartDocument::new(doc.name.clone(), doc.units),
        };
        Ok(diff(&parent, &doc))
    }

    /// Rewrites every stored version through `upgrade`, which receives each
    /// document as JSON in the form it was stored and returns it in the
    /// current form. Used when a file from an older format version is
    /// opened, so old versions upgrade along with the open document. Version
    /// ids are hashes of their contents, so they change; parents, branches,
    /// tags and the head follow.
    pub fn migrate_documents<E: fmt::Display>(
        &self,
        mut upgrade: impl FnMut(serde_json::Value) -> Result<serde_json::Value, E>,
    ) -> Result<History, HistoryError> {
        let mut new = History {
            head: self.head.clone(),
            branches: BTreeMap::new(),
            tags: BTreeMap::new(),
            versions: Vec::new(),
            index: HashMap::new(),
            objects: Objects::default(),
            next_feature_id: self.next_feature_id,
        };
        let mut ids: HashMap<&VersionId, VersionId> = HashMap::new();
        for v in &self.versions {
            let invalid = |e: &dyn fmt::Display| {
                HistoryError::Invalid(format!("upgrading version {}: {e}", v.id))
            };
            let mut json: serde_json::Value = self.objects.get(&v.tree.header)?;
            let features = v
                .tree
                .features
                .iter()
                .map(|h| self.objects.get(h))
                .collect::<Result<Vec<serde_json::Value>, _>>()?;
            json.as_object_mut()
                .ok_or_else(|| invalid(&"header is not an object"))?
                .insert("features".into(), features.into());
            let json = upgrade(json).map_err(|e| invalid(&e))?;
            let mut doc: PartDocument = serde_json::from_value(json).map_err(|e| invalid(&e))?;
            doc.repair_ids();
            doc.validate()?;
            let parents = v.parents.iter().map(|p| ids[p].clone()).collect();
            let id = new.record(parents, &doc, &v.message, v.time)?;
            ids.insert(&v.id, id);
        }
        new.branches = self
            .branches
            .iter()
            .map(|(n, id)| (n.clone(), ids[id].clone()))
            .collect();
        new.tags = self
            .tags
            .iter()
            .map(|(n, id)| (n.clone(), ids[id].clone()))
            .collect();
        if let Head::Version(id) = &self.head {
            new.head = Head::Version(ids[id].clone());
        }
        Ok(new)
    }

    /// Opens a version for editing: its document, with the id counter raised
    /// so new features get ids no branch has used.
    fn open(&self, id: &VersionId) -> Result<PartDocument, HistoryError> {
        let mut doc = self.document(id)?;
        doc.reserve_ids(FeatureId(self.next_feature_id));
        Ok(doc)
    }

    fn ancestors(&self, id: &VersionId) -> HashSet<usize> {
        let mut seen = HashSet::new();
        let mut queue: VecDeque<usize> = self.index.get(id).copied().into_iter().collect();
        while let Some(i) = queue.pop_front() {
            if seen.insert(i) {
                queue.extend(self.versions[i].parents.iter().map(|p| self.index[p]));
            }
        }
        seen
    }

    /// True if `working` has nothing that isn't recorded in the head version.
    pub fn is_clean(&self, working: &PartDocument) -> Result<bool, HistoryError> {
        Ok(same_content(&self.head_document()?, working))
    }

    fn require_clean(&self, working: &PartDocument) -> Result<(), HistoryError> {
        if self.is_clean(working)? {
            Ok(())
        } else {
            Err(HistoryError::UncommittedChanges)
        }
    }

    fn check_new_name(&self, name: &str) -> Result<(), HistoryError> {
        if !valid_name(name) {
            return Err(HistoryError::BadName(name.into()));
        }
        if self.branches.contains_key(name) || self.tags.contains_key(name) {
            return Err(HistoryError::NameTaken(name.into()));
        }
        Ok(())
    }

    fn advance_head(&mut self, id: VersionId) {
        match &self.head {
            Head::Branch(name) => {
                self.branches.insert(name.clone(), id);
            }
            Head::Version(_) => self.head = Head::Version(id),
        }
    }

    /// Stores a version and returns its id. Documents must already be valid.
    fn record(
        &mut self,
        parents: Vec<VersionId>,
        doc: &PartDocument,
        message: &str,
        time: u64,
    ) -> Result<VersionId, HistoryError> {
        let mut header = doc.clone();
        let features = std::mem::take(&mut header.features);
        let tree = Tree {
            header: self.objects.put(&header)?,
            features: features
                .iter()
                .map(|f| self.objects.put(f))
                .collect::<Result<_, _>>()?,
        };
        let id = record_id(&parents, message, time, &tree);
        self.next_feature_id = self.next_feature_id.max(doc.next_id().0);
        // The same record twice is the same version.
        if !self.index.contains_key(&id) {
            if self.versions.len() >= MAX_VERSIONS {
                return Err(HistoryError::TooManyVersions);
            }
            self.index.insert(id.clone(), self.versions.len());
            self.versions.push(Version {
                id: id.clone(),
                parents,
                message: message.into(),
                time,
                tree,
            });
        }
        Ok(id)
    }
}

impl TryFrom<HistoryData> for History {
    type Error = HistoryError;

    fn try_from(data: HistoryData) -> Result<Self, HistoryError> {
        let invalid = |msg: String| HistoryError::Invalid(msg);
        if data.versions.is_empty() {
            return Err(invalid("no versions".into()));
        }
        if data.versions.len() > MAX_VERSIONS {
            return Err(HistoryError::TooManyVersions);
        }
        let objects = Objects::from_data(data.objects)?;
        let mut index = HashMap::with_capacity(data.versions.len());
        for (i, v) in data.versions.iter().enumerate() {
            // Parents must come earlier, which also rules out cycles.
            if let Some(p) = v.parents.iter().find(|p| !index.contains_key(*p)) {
                return Err(invalid(format!(
                    "version {} has unknown or later parent {p}",
                    v.id
                )));
            }
            if v.id != record_id(&v.parents, &v.message, v.time, &v.tree) {
                return Err(invalid(format!("version {} does not match its hash", v.id)));
            }
            if index.insert(v.id.clone(), i).is_some() {
                return Err(invalid(format!("version id {} is used twice", v.id)));
            }
            // Contents are parsed when used, but must all be present.
            if let Some(h) = std::iter::once(&v.tree.header)
                .chain(&v.tree.features)
                .find(|h| !objects.contains(h))
            {
                return Err(invalid(format!(
                    "version {} needs missing object {h}",
                    v.id
                )));
            }
        }
        for (name, id) in data.branches.iter().chain(&data.tags) {
            if !valid_name(name) {
                return Err(HistoryError::BadName(name.clone()));
            }
            if !index.contains_key(id) {
                return Err(invalid(format!("{name:?} points at unknown version {id}")));
            }
        }
        if let Some(name) = data.branches.keys().find(|n| data.tags.contains_key(*n)) {
            return Err(HistoryError::NameTaken(name.clone()));
        }
        match &data.head {
            Head::Branch(name) if !data.branches.contains_key(name) => {
                return Err(invalid(format!("head is unknown branch {name:?}")))
            }
            Head::Version(id) if !index.contains_key(id) => {
                return Err(invalid(format!("head is unknown version {id}")))
            }
            _ => {}
        }
        let history = History {
            head: data.head,
            branches: data.branches,
            tags: data.tags,
            versions: data.versions,
            index,
            objects,
            next_feature_id: data.next_feature_id,
        };
        // The counter must be above every id any version handed out, or a
        // branch could reuse one. Each header records its document's counter,
        // so raise the stored value to the largest (cheap: headers are small
        // and shared between versions).
        let mut history = history;
        let headers: HashSet<&Hash> = history.versions.iter().map(|v| &v.tree.header).collect();
        for header in headers {
            #[derive(Deserialize)]
            struct Counter {
                #[serde(default)]
                next_id: u32,
            }
            let counter: Counter = history.objects.get(header)?;
            history.next_feature_id = history.next_feature_id.max(counter.next_id);
        }
        // The open document is compared with the head on every checkout, so
        // a broken head is reported now rather than on first use.
        history.head_document()?;
        Ok(history)
    }
}

impl From<History> for HistoryData {
    fn from(h: History) -> Self {
        HistoryData {
            head: h.head,
            branches: h.branches,
            tags: h.tags,
            next_feature_id: h.next_feature_id,
            versions: h.versions,
            objects: h.objects.to_data(),
        }
    }
}

/// A version's id: the hash of everything recorded about it.
fn record_id(parents: &[VersionId], message: &str, time: u64, tree: &Tree) -> VersionId {
    let record = VersionRecord {
        parents,
        message,
        time,
        tree,
    };
    let text = objects::canonical_json(&serde_json::to_value(&record).expect("serializable"));
    VersionId(Hash::of(text.as_bytes()))
}

/// Equal as far as the user can tell: everything except the internal id
/// counter, so adding and then deleting a feature leaves nothing to record.
fn same_content(a: &PartDocument, b: &PartDocument) -> bool {
    a.name == b.name
        && a.units == b.units
        && a.parameters == b.parameters
        && a.features == b.features
        && a.end_of_part == b.end_of_part
}

/// Branch and tag names: no control or invisible characters, no surrounding
/// spaces, not too long.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(is_unsafe_text_char)
}

/// The current time in seconds since the Unix epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests;
