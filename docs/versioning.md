# Versions, branches and merges

SolidModeller aims to be Inventor for modeling, with Onshape's file and
version management, running locally on Omarchy instead of in the cloud. This
document covers the version history of a part (implemented in the
`sm-history` crate) and the plan for growing it into a local document and
project manager for parts and assemblies.

The direction was settled overnight on 2026-10-24 (ballot 1, recorded in
`docs/decisions.md` in PR #7, decided by the design-decisions thread under the owner's
delegation): the working format becomes a local **project store**, and `.smp`
becomes the single-document package. This PR is the first layer: the version
graph, branches, tags and the feature-level merge, stored today in the `.smp`
package. The same engine moves onto the project store next (see
[Toward a local Onshape-style document manager](#toward-a-local-onshape-style-document-manager)).

## How it works today

### Versions are content-addressed snapshots

A part's history is a graph of versions. Each version holds:

- the `PartDocument` as a list of object hashes: one for the header (name,
  units, parameters, end-of-part marker) and one per feature, in order. No
  geometry; geometry is always rebuilt from the feature tree.
- its parents: none for the first version, one for an ordinary version, two
  for a merge (ours first).
- a message and a time (seconds since the Unix epoch).
- an id: the BLAKE3 hash of all of the above (64 hex digits; `smcli log`
  shows the first 8).

Objects are stored once, named by the BLAKE3 hash of their canonical JSON
(compact, keys sorted explicitly by the writer, so the result doesn't depend
on serde_json build features). A golden-hash test pins the hash of a fixed
document, so any change to field names, float formatting or ordering is
caught before it silently breaks deduplication. A version that changes one extrusion depth adds one
small object; the header and every other feature are shared with its parent.
This is git's tree structure, and the project store uses the same objects.

Every snapshot is the whole truth for its version. Change lists ("changed
Extrusion2 (definition)", "added Fillet1") are computed by `sm_history::diff`
between a version and its parent. The project store adds a microversion log
on top for undo and audit, but nothing is ever rebuilt from that log.

### Branches, tags and the head

- **Branches** are movable names, like git branches or Onshape branches.
  Every history starts on `main`. Recording a version on a branch moves the
  branch forward.
- **Tags** are fixed names for a version: Onshape's named versions, or a
  release such as `Rev A`. They never move.
- The **head** is what the open document was checked out from: a branch, or
  a specific version (when a tag or an old version is checked out). Versions
  recorded on a detached version are kept, and a branch can be created at
  them to keep working there.
- Names may contain letters in any script (so `Révision B` works), up to
  100 characters, with no leading or trailing spaces, and no control or
  invisible format characters (such as the bidirectional overrides that can
  make `main\u{202e}` look like `main`). A name can't be both a branch and a
  tag.
- A target is resolved as a branch name, then a tag name, then a version id
  or an id prefix of at least 4 digits that matches exactly one version.

### The open document and uncommitted changes

The open part (`document.json`) is separate from the history, like a git
working tree. It can differ from the head version; `smcli log` says so.
Checkout and merge refuse to run while it has unrecorded changes, so work is
never silently thrown away. Only what the user can see counts as a change:
adding a feature and deleting it again leaves nothing to record.

### Merging

Merging is three-way, at the feature level: the merge base is the nearest
version both sides descend from, and each side's changes since then are
combined. After criss-cross merges (each branch merged into the other) there
can be several equally near ones. The one recorded last is used as the base,
and anything the nearest bases disagree on is a conflict whenever ours and
theirs still differ there. Otherwise a conflict each branch settled its own
way would be decided silently by which branch happened to commit first.

- **Features** are matched by their stable id. Each feature merges field by
  field: name, suppressed, and definition (sketch geometry, extrude extent,
  and so on). Renaming Extrusion2 on one branch and changing its depth on the
  other merges cleanly.
- **Parameters** are matched by name and merge as a whole (value and comment).
  A renamed parameter is a deletion plus an addition.
- **Part name and units** merge as single values. Values are stored in the
  document's unit today, so a unit change on one side and any changed or new
  number on the other is a conflict, even though the unit itself merged
  cleanly. (Decision D3 moves stored geometry to millimetres, after which a
  unit change is display-only and this conflict goes away.)
- **The end-of-part marker** merges as a single value. It is stored as the
  feature below it and moves with that feature. If the merge deletes that
  feature, the marker moves down to the next surviving feature, or to the end
  of the part. If the merged marker would roll back a feature one side added
  while it was active there, or roll in one that side had left rolled back,
  that is a conflict, so new work never silently disappears or appears.
- **Order:** if only one side reordered the features both sides share, its
  order wins. Features only one side has are placed after the feature they
  followed on that side. When both sides added features at the same place,
  the side whose order was used comes first (ours, unless only they
  reordered).
- **Feature ids across branches (interim):** every branch of a history hands out
  feature ids from one shared counter (the history raises the open
  document's counter on checkout and merge), so two branches never give
  different features the same id, and merging the same branch again later
  stays correct. A part edited outside its history (for example two copies of
  a file) can still collide; then their side's feature gets a fresh id and
  their references are rewritten, and the result lists every renumbering.
  The counter is raised on load to the highest one any version recorded,
  and the open document is raised to it when a file is read. This is only
  safe within one history: two copies of a file (or of a
  project store) each continue the counter and reuse ids. Decision D1
  (schema foundations) replaces counter ids with random UUIDv7 ids, after
  which the shared counter and renumbering are removed entirely. Two identical additions count as one.

A **conflict** is a value changed differently on both sides, a feature
deleted on one side and changed on the other, both sides reordering
differently, a unit change against value edits, or a marker that would roll
back new work. The merge keeps our side for each conflict and reports it with
the base, ours and theirs values. Nothing is recorded until every conflict is
settled: the caller passes a pick (ours or theirs) per conflict and merges
again. `smcli merge <file> <branch> theirs` settles all of them one way.

A merge can also be **invalid** without any conflict, for example when we
deleted a sketch that their new extrusion uses. The merged document is
validated, and an invalid result is reported and never recorded. Today the
fix is to edit one branch and merge again; see [Next steps](#next-steps).

If their version is already part of ours, the merge does nothing. If ours
has no changes of its own, the head simply moves to theirs (fast-forward).

**Reversible:** the merge granularity (three fields per feature) is the
coarsest that handles the common cases. Finer merges (one sketch point, one
extrude option) can be added without changing the stored format.

### Storage in `.smp`

The history is an optional `history.json` entry in the `.smp` zip:

```json
{
  "head": { "branch": "main" },
  "branches": { "main": "6759db85…", "deeper-pocket": "cc96e917…" },
  "tags": { "Rev A": "8e0327a3…" },
  "next_feature_id": 7,
  "versions": [
    { "id": "8e0327a3…", "parents": [], "message": "Initial", "time": 1792885200,
      "tree": { "header": "41c2…", "features": ["9a0e…", "77b1…"] } }
  ],
  "objects": {
    "41c2…": {"end_of_part":null,"features":[],"name":"Block","next_id":7,…},
    "9a0e…": {"id":1,"kind":{…},"name":"Sketch1","suppressed":false}
  }
}
```

- The version list is append-only and in parent-before-child order, so a new
  version only adds lines at the end of the list. Each object is one line.
- **Lazy loading:** opening a file checks the structure and hashes but parses
  only the head version's document. Other versions are parsed and validated
  when they are used (checkout, merge, log).
- **Verified on load:** every object must hash to its name and every version
  id to its record, so nothing in the file can stand in for something else.
  Version ids, parents, branches and tags must exist and parents must come
  earlier in the list (which also rules out cycles). Objects are kept byte
  for byte as written, so reformatting `history.json` by hand invalidates it.
- **Limits:** at most 100,000 versions and 64 MiB of history text. The limit
  is enforced when saving too, so a part never becomes too large to reopen.
  When reading, the history may expand to at most 100 times the size of the
  whole file (beyond a 1 MiB allowance), which stops small files built to
  expand.
- A file without `history.json` is a part with no versions yet. `smcli commit`
  starts one.
- A damaged history is an error when opening, not silently dropped, because
  saving the file again would otherwise throw away every version. For the
  same reason, saving without a history over a file that has one is refused.
- **Upgrades:** when a file from an older format version is opened, every
  stored version is upgraded along with the open document, not only the
  head. Their ids are hashes, so they change, and parents, branches, tags and
  the head follow. (The project store instead records each object's schema
  version and upgrades on read, never rewriting objects in place.)

### Try it

```sh
smcli demo part.smp
smcli commit part.smp "Initial"
smcli branch part.smp deeper-pocket
smcli tag part.smp "Rev A"
smcli set-distance part.smp Extrusion3 6 && smcli commit part.smp "Taller notch"
smcli checkout part.smp deeper-pocket
smcli set-distance part.smp Extrusion2 -8 && smcli commit part.smp "Deeper pocket"
smcli checkout part.smp main
smcli merge part.smp deeper-pocket      # clean: different features changed
smcli log part.smp
```

## Toward a local Onshape-style document manager

Onshape's file management, without the server: everything lives in local
files and folders that the user owns, backs up and syncs however they like.
The design below follows ballot 1 and ADR 4 in `docs/decisions.md`.

### The project store

A project is a folder (Inventor's project, Onshape's document set) with:

- a project file naming the project and its settings (the role of Inventor's
  `.ipj`);
- an object store: the same canonical JSON objects as above, named by their
  BLAKE3 hash and written once (temp file, fsync, rename). Object ids are
  checked as exact-length lowercase hex before they become paths, every
  object is re-hashed when read, and objects, log records and the project as
  a whole have size limits;
- refs (branches, versions, releases) as the only mutable state, updated
  atomically under a lock;
- an append-only microversion log of edits, for undo and audit.

`.smp` (part), `.sma` (assembly) and `.smd` (drawing) become single-document
packages for exchange and Pack and Go, optionally carrying that document's
history in the `history.json` above. Opening a loose `.smp` still works and
opens it as a one-document project.

### Versions, branches and releases across documents

- A project branch spans every document in the project, so an engineering
  change can touch parts and assemblies and merge back as one.
- Named versions are immutable. Releases have the states Work in progress,
  In review, Released and Obsolete. Released versions are read-only forever
  (their hashes enforce it); branching from a release is allowed and is how
  an engineering change starts. Locally, the user can release.

### References and where-used

References inside a project are live on the current branch; references to
other projects pin a version. A where-used and search index (SQLite) may be
kept as a cache rebuilt from the objects, never as the source of truth.

### History browsing

The GUI's version manager shows the version graph with branches, versions
and releases, the change list for each version, side-by-side comparison of
two versions (feature tree diff now, geometry diff later), restore of an old
version as a new one, and a merge dialog that lists each conflict with base,
ours and theirs values.

### Git

The layout is plain canonical JSON and the export is deterministic, so a
project can be put in git by the user. The app never needs git and never runs
it on a project folder, because a hostile `.git/config` can run code.

## Interaction with topological naming

Today features refer to each other only by feature id (an extrusion names
its sketch), and the rare merge that renumbers a feature rewrites those
references (`FeatureKind::remap_references`). When features start referring
to faces and edges, two things must hold for merges to stay correct:

1. Persistent face and edge names are built from feature ids and sketch
   entity ids (the naming thread's `TopoName::remap_features`), so any
   renumbering rewrites them with the same id map. With UUIDv7 ids (D1) no
   renumbering happens at all.
2. A merged document must be rebuilt before it is trusted: a reference can
   be valid as data but no longer find its face (the other branch changed
   the geometry it pointed at). The rebuild's sick-feature report (roadmap
   step 3) is the place to show that, as a merge warning.

## Next steps

1. Move the engine onto the project store: object store, refs under a lock,
   microversion log, project file, releases. Built on the foundations PR
   (UUIDv7 ids, values with units), which also removes renumbering.
2. Resolve invalid merges in the merge dialog, by editing the merged part
   before recording it.
3. Rebuild after every merge and report references that no longer resolve
   as sick features.
4. Finer merges of sketches (per entity, using sketch entity ids) and of
   duplicate value names (rename the incoming one, rewrite its expressions by
   the value's id).
5. Version manager UI.
