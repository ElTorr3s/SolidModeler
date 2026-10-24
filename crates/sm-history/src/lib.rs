//! Versions, branches and merges of a part's feature history, in the style
//! of Onshape but stored locally inside the `.smp` file.
//!
//! A [`History`] is a graph of [`Version`]s. Each version is a snapshot of
//! the [`PartDocument`] (parameters and feature tree, no geometry) plus its
//! parent versions, a message and a time. Branches and tags are names that
//! point at versions; the head is the branch (or version) the open document
//! was checked out from. Merges work on the feature tree, see [`merge`].
//!
//! The open document itself lives outside the history (in `document.json`),
//! so it can have uncommitted changes, like a git working tree.

mod diff;
pub mod merge;

pub use diff::{diff, Change};
pub use merge::{merge3, Conflict, ConflictKey, FeatureField, MergeResult, Resolutions, Side};

use serde::{Deserialize, Serialize};
use sm_document::{DocumentError, PartDocument};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use thiserror::Error;

/// Branch every new history starts on.
pub const DEFAULT_BRANCH: &str = "main";
/// Longest branch or tag name, in characters.
pub const MAX_NAME_CHARS: usize = 100;

/// Identifies a version: 16 hex digits hashed from its contents. It's a
/// label, not an integrity check; uniqueness is enforced on load.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VersionId(String);

impl VersionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The first 8 digits, enough to tell versions apart in a log.
    pub fn short(&self) -> &str {
        &self.0[..self.0.len().min(8)]
    }
}

impl fmt::Display for VersionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Version {
    pub id: VersionId,
    /// Empty for the first version, two for a merge (ours first).
    #[serde(default)]
    pub parents: Vec<VersionId>,
    pub message: String,
    /// Seconds since the Unix epoch.
    pub time: u64,
    pub document: PartDocument,
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
    #[error("the merge still has conflicts or is invalid")]
    MergeNotClean,
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
}

/// The stored form of a [`History`], validated on load.
#[derive(Serialize, Deserialize)]
struct HistoryData {
    head: Head,
    branches: BTreeMap<String, VersionId>,
    #[serde(default)]
    tags: BTreeMap<String, VersionId>,
    versions: Vec<Version>,
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
        };
        let id = history.record(Vec::new(), doc, message, time);
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

    /// Finds a version by branch name, tag name, full id, or an id prefix of
    /// at least 4 digits that matches exactly one version, in that order.
    pub fn resolve(&self, name: &str) -> Result<VersionId, HistoryError> {
        if let Some(id) = self.branches.get(name).or_else(|| self.tags.get(name)) {
            return Ok(id.clone());
        }
        let id = VersionId(name.into());
        if self.index.contains_key(&id) {
            return Ok(id);
        }
        if name.len() >= 4 {
            let mut matches = self.versions.iter().filter(|v| v.id.0.starts_with(name));
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
        let head = self.head_version();
        if head.document == *doc {
            return Err(HistoryError::NoChanges);
        }
        let parents = vec![head.id.clone()];
        let id = self.record(parents, doc, message, time);
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
        self.head = if self.branches.contains_key(target) {
            Head::Branch(target.into())
        } else {
            Head::Version(id.clone())
        };
        Ok(self.version(&id).expect("resolved").document.clone())
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
        let base_id = self
            .merge_base(&ours_id, &theirs_id)
            .ok_or(HistoryError::NoCommonAncestor)?;
        if base_id == theirs_id {
            return Ok(MergeOutcome::UpToDate);
        }
        if base_id == ours_id {
            self.advance_head(theirs_id.clone());
            return Ok(MergeOutcome::FastForward(
                self.version(&theirs_id).unwrap().document.clone(),
            ));
        }

        let doc = |id: &VersionId| &self.version(id).unwrap().document;
        let result = merge3(doc(&base_id), doc(&ours_id), doc(&theirs_id), resolutions);
        if !result.is_clean() {
            return Ok(MergeOutcome::Conflicted(result));
        }
        let into = self
            .current_branch()
            .unwrap_or("detached version")
            .to_owned();
        let message = format!("Merge {theirs} into {into}");
        let id = self.record(vec![ours_id, theirs_id], &result.document, &message, time);
        self.advance_head(id.clone());
        Ok(MergeOutcome::Merged {
            version: id,
            document: result.document,
        })
    }

    /// The nearest version both `a` and `b` descend from.
    pub fn merge_base(&self, a: &VersionId, b: &VersionId) -> Option<VersionId> {
        let of_a = self.ancestors(a);
        // Versions are in topological order, so the common ancestor with the
        // highest index can't be an ancestor of another common ancestor.
        self.ancestors(b)
            .intersection(&of_a)
            .max()
            .map(|&i| self.versions[i].id.clone())
    }

    /// `id` and every version it descends from, newest first.
    pub fn log(&self, id: &VersionId) -> Vec<&Version> {
        let mut found: Vec<usize> = self.ancestors(id).into_iter().collect();
        found.sort_unstable_by(|a, b| b.cmp(a));
        found.into_iter().map(|i| &self.versions[i]).collect()
    }

    /// What changed in a version compared with its first parent.
    pub fn changes(&self, version: &Version) -> Vec<Change> {
        match version.parents.first().and_then(|p| self.version(p)) {
            Some(parent) => diff(&parent.document, &version.document),
            None => diff(
                &PartDocument::new(version.document.name.clone(), version.document.units),
                &version.document,
            ),
        }
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

    fn require_clean(&self, working: &PartDocument) -> Result<(), HistoryError> {
        if self.head_version().document == *working {
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

    /// Appends a version and returns its id. Documents must already be valid.
    fn record(
        &mut self,
        parents: Vec<VersionId>,
        doc: &PartDocument,
        message: &str,
        time: u64,
    ) -> VersionId {
        let mut version = Version {
            id: VersionId(String::new()),
            parents,
            message: message.into(),
            time,
            document: doc.clone(),
        };
        let content = serde_json::to_vec(&version).expect("serializable");
        // A clash with a different version is astronomically unlikely, but
        // costs nothing to rule out.
        let mut salt = 0u64;
        loop {
            let id = VersionId(fnv1a(&[&content, &salt.to_le_bytes()]));
            match self.version(&id) {
                None => {
                    version.id = id.clone();
                    self.index.insert(id.clone(), self.versions.len());
                    self.versions.push(version);
                    return id;
                }
                // Recording exactly the same version twice is a no-op.
                Some(v)
                    if v.parents == version.parents
                        && v.document == version.document
                        && v.message == version.message
                        && v.time == version.time =>
                {
                    return id;
                }
                Some(_) => salt += 1,
            }
        }
    }
}

impl TryFrom<HistoryData> for History {
    type Error = HistoryError;

    fn try_from(data: HistoryData) -> Result<Self, HistoryError> {
        let invalid = |msg: String| HistoryError::Invalid(msg);
        let mut index = HashMap::with_capacity(data.versions.len());
        let mut versions = data.versions;
        for (i, v) in versions.iter_mut().enumerate() {
            // Ids are shown in logs, so only plain characters are allowed.
            if v.id.0.is_empty()
                || v.id.0.len() > 64
                || !v.id.0.bytes().all(|b| b.is_ascii_alphanumeric())
            {
                return Err(invalid(format!("bad version id {:?}", v.id.0)));
            }
            // Parents must come earlier, which also rules out cycles.
            if let Some(p) = v.parents.iter().find(|p| !index.contains_key(*p)) {
                return Err(invalid(format!(
                    "version {} has unknown or later parent {p}",
                    v.id
                )));
            }
            if index.insert(v.id.clone(), i).is_some() {
                return Err(invalid(format!("version id {} is used twice", v.id)));
            }
            v.document.repair_ids();
            v.document.validate()?;
        }
        if versions.is_empty() {
            return Err(invalid("no versions".into()));
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
        Ok(History {
            head: data.head,
            branches: data.branches,
            tags: data.tags,
            versions,
            index,
        })
    }
}

impl From<History> for HistoryData {
    fn from(h: History) -> Self {
        HistoryData {
            head: h.head,
            branches: h.branches,
            tags: h.tags,
            versions: h.versions,
        }
    }
}

/// Branch and tag names: printable, no surrounding spaces, not too long.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(char::is_control)
}

fn fnv1a(parts: &[&[u8]]) -> String {
    let hash = parts
        .iter()
        .flat_map(|p| p.iter())
        .fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ *b as u64).wrapping_mul(0x100000001b3)
        });
    format!("{hash:016x}")
}

/// The current time in seconds since the Unix epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests;
