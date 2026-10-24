# Versions, branches and merges

SolidModeller aims to be Inventor for modeling, with Onshape's file and
version management, running locally on Omarchy instead of in the cloud. This
document covers the version history of a part (implemented in the
`sm-history` crate) and the plan for growing it into a local document and
project manager for parts and assemblies.

Every choice marked **Reversible** below was picked because it is the easiest
to change later. Choices that change the `.smp` format or the product's
direction are listed under [Decisions for the owner](#decisions-for-the-owner).

## How it works today

### Versions are snapshots of the feature tree

A part's history is a graph of versions. Each version holds:

- a full snapshot of the `PartDocument`: parameters, features, end-of-part
  marker. No geometry; geometry is always rebuilt from the feature tree.
- its parents: none for the first version, one for an ordinary version, two
  for a merge (ours first).
- a message and a time (seconds since the Unix epoch).
- an id: 16 hex digits hashed from the above. It is a label, not a security
  check. Ids must be unique and parents must appear earlier in the file,
  which is checked on load and also rules out cycles.

**Reversible:** versions store snapshots rather than an operation log. The
change list for any version ("changed Extrusion2 (definition)", "added
Fillet1") is computed by `sm_history::diff` between a version and its
parent, so nothing is lost. Going from snapshots to an operation log later is
a mechanical conversion; the other way round depends on the operations'
exact semantics never changing. Snapshots cost space (see
[Size](#size)).

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
- Names may contain any printable characters (so `Révision B` works), up to
  100 characters, with no leading or trailing spaces. A name can't be both a
  branch and a tag.
- A target is resolved as a branch name, then a tag name, then a full
  version id, then an id prefix of at least 4 digits that matches exactly
  one version.

### The open document and uncommitted changes

The open part (`document.json`) is separate from the history, like a git
working tree. It can differ from the head version; `smcli log` says so.
Checkout and merge refuse to run while it has unrecorded changes, so work is
never silently thrown away.

### Merging

Merging is three-way, at the feature level: the merge base is the newest
version both sides descend from, and each side's changes since then are
combined.

- **Features** are matched by their stable id. Each feature merges field by
  field: name, suppressed, and definition (sketch geometry, extrude extent,
  and so on). Renaming Extrusion2 on one branch and changing its depth on the
  other merges cleanly.
- **Parameters** are matched by name and merge as a whole (value and comment).
  A renamed parameter is a deletion plus an addition.
- **Part name, units and the end-of-part marker** merge as single values.
- **Order:** if only one side reordered the features both sides share, its
  order wins. Features only one side has are placed after the feature they
  followed on that side. When both sides added features at the same place,
  the side whose order was used comes first (ours, unless only they
  reordered).
- **New features with the same id:** ids are handed out per document, so two
  branches that each add a feature usually give it the same id. Their side's
  feature gets a fresh id, and every reference to it on their side (and their
  end-of-part marker) is rewritten. The merge result lists every renumbering.
  Two identical additions count as one.

A **conflict** is a value changed differently on both sides, a feature
deleted on one side and changed on the other, or both sides reordering
differently. The merge keeps our side for each conflict and reports it with
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

The history is an optional `history.json` entry in the `.smp` zip,
pretty-printed JSON:

```json
{
  "head": { "branch": "main" },
  "branches": { "main": "6759db85d1f0a3b2", "deeper-pocket": "cc96e9170a4e6b81" },
  "tags": { "Rev A": "8e0327a3c5d2e190" },
  "versions": [
    { "id": "8e0327a3c5d2e190", "parents": [], "message": "Initial",
      "time": 1792885200, "document": { "name": "Block", "...": "..." } }
  ]
}
```

- The version list is append-only and in parent-before-child order, so a new
  version only adds lines at the end of the file.
- A file without `history.json` is a part with no versions yet. `smcli commit`
  starts one.
- A damaged history is an error when opening, not silently dropped, because
  saving the file again would otherwise throw away every version.
- Size limit: 256 MiB uncompressed, enforced while decompressing like the
  other entries. Every stored document is validated on load.

**Reversible:** adding an optional entry doesn't change `document.json`, so
the format version stays 1 and files without history are unaffected.

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

## Size

Every version stores the whole feature tree. Feature trees are small (a
complex part is a few hundred KB of JSON) and the zip compresses repeated
snapshots well, so hundreds of versions are fine. For thousands, the planned
fix is content-addressed storage: each feature and parameter stored once by
hash, and versions listing hashes. That changes `history.json` only, and is
the same structure as a git tree, which keeps the git option below open.

## Toward a local Onshape-style document manager

Onshape's file management, without the server: everything lives in local
files and folders that the user owns, backs up and syncs however they like.

### Projects

A project is a folder with a small `project.json` (name, settings, the list
of documents) next to its `.smp` parts and, later, `.sma` assemblies. This
is Inventor's project file, made simple. Opening a project indexes its
documents.

### Versions across documents

- **Per-document history** (today): each part carries its own versions,
  branches and tags, so a single file can be emailed with its history.
- **Project versions and releases** (next): a project-level tag records one
  version of every document at once, so `Release 3` means a consistent set of
  parts and assemblies. A release can carry a state (work in progress, in
  review, released, obsolete) and released versions are read-only, as in
  Onshape's release management and Inventor's Vault lifecycle states.
- **Branches across documents:** a project branch is a branch of the same
  name in each document it touches, so an engineering change can span parts
  and assemblies and merge back together.

### References and where-used

Assemblies (and derived parts) refer to a part by document id plus a version
selector: a branch (follows the latest), a tag, or a fixed version id. The
project index keeps a reverse map, which answers where-used ("which
assemblies use this bracket, and at which version") and shows which
assemblies need updating when a part moves forward, like Onshape's
"update to latest" and Inventor's where-used in Vault. Inside a part, stable
references to faces and edges depend on the topological naming work (see
below).

### History browsing

The GUI's version manager shows the version graph with branches and tags,
the change list for each version, side-by-side comparison of two versions
(feature tree diff now, geometry diff later), restore of an old version as a
new one, and a merge dialog that lists each conflict with base, ours and
theirs values.

### Optional git compatibility

Because the feature tree is plain JSON, a project could also be a git
repository, with the documents' JSON committed directly instead of zipped.
This is optional: the built-in history needs no git. Whether to offer it is
a decision for the owner (below).

## Interaction with topological naming

Today features refer to each other only by feature id (an extrusion names
its sketch), and merges rewrite those references when they renumber
features (`FeatureKind::remap_references`). When features start referring to
faces and edges, two things must hold for merges to stay correct:

1. Persistent face and edge names must be built from feature ids and the
   sketch entities that created them, so renumbering a feature can rewrite
   them with the same id map, and `remap_references` must cover them.
2. A merged document must be rebuilt before it is trusted: a reference can
   be valid as data but no longer find its face (the other branch changed
   the geometry it pointed at). The rebuild's sick-feature report (roadmap
   step 3) is the place to show that, as a merge warning.

## Next steps

1. Resolve invalid merges in the merge dialog, by editing the merged part
   before recording it (a merge with a hand-edited result).
2. Content-addressed storage for large histories.
3. Rebuild-on-merge with sick-feature reporting, once the rebuilder exists.
4. Project folders, project-level versions and releases, where-used.
5. Version manager UI.

## Decisions for the owner

These change the format or the product's direction, so they need your call.
Each has a default that is already in place or easy to reverse.

1. **Snapshots or operation log.** Default: snapshots, with the change log
   computed. Onshape stores every edit (microversions). Recording each edit
   would give a finer history, at the cost of more storage and a format that
   is harder to change.
2. **History inside the part file, or next to it.** Default: inside, so a
   part carries its history when copied or emailed. The alternative is a
   project-level store, which handles cross-document versions more naturally
   and keeps part files small.
3. **Git compatibility.** Default: none. Offering it means storing documents
   as unzipped JSON in project folders as an option.
4. **Release states and read-only releases.** Proposed above; needs your
   rules (who can release, whether released versions can be branched from).
