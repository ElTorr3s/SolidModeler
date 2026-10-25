# Design decisions

Architecture decision records (ADRs) for SolidModeller. Each one says what was decided,
why, what else was considered, and how hard it is to undo.

The first batch below was decided overnight on 2026-10-24/25 by the design-decision
thread ("Oracle"). Master was asleep and had delegated the pending decisions so that
work could continue ("make a decision agent with extra effort to try and make best
decision and continue working"). Hard calls went to a vote, as Master asked: five
independent agents (a CAD kernel engineer, an Inventor/Vault/Onshape power user, a Rust
systems architect, a local-first data expert, and a devil's advocate), plus the four
standing reviewers (Sentinel for security, Bolt for performance, Hawk for bugs, Sage
for the global review). Oracle made the final call. **Master can override any of
these.** The ones most worth a look are marked ⚑.

The yardstick for every decision is Master's vision: *Inventor-style modeling, with
Onshape's file and version management, running locally on Omarchy instead of in the
cloud*, for professional use, with no mistakes.

## Summary

| # | Decision | Vote | Undo cost |
|---|---|---|---|
| 1 ⚑ | Kernel: OpenCASCADE behind our own Rust kernel facade, replacing truck | 9-0 | Medium. The facade keeps the rest of the code backend-neutral |
| 2 | Working format: a local project store, not one zip per part | 9-0 | High after real projects exist; free today |
| 3 | `.smp`, `.sma`, `.smd` become single-document packages for exchange and Pack and Go | 8-1 on per-kind extensions | Low |
| 4 | Store layout: content-addressed canonical JSON with an edit log; SQLite only as a cache | 8-1 over SQLite | Medium |
| 5 | History: microversion log plus immutable snapshots; Vault-style releases | 9-0 | Medium |
| 6 | Ids: UUIDv7 for every object | not put to a vote | High after the format freezes |
| 7 | Units: millimetres and radians inside; document units are display only | not put to a vote | High after the freeze |
| 8 | Every user-entered number is an expression with units and an Inventor-style dN name | not put to a vote | High after the freeze |
| 9 | Sketches store entities and constraints with ids; profiles are selected regions | not put to a vote | High after the freeze |
| 10 | Rebuild keeps going past a failed feature and reports per-feature status | not put to a vote | Low (API only) |
| 11 | Geometry cache: key on geometry inputs only (renames keep it); a binary mesh instead of STEP | not put to a vote | Low (caches are disposable) |
| 12 | Format stays a draft until the step 3 schema lands, then freezes with fixtures | not put to a vote | Low |
| 13 ⚑ | Origin-plane sketch frames stay as they are, locked by a test, pending Master's Inventor check | not put to a vote | Free until the freeze |
| 14 | Roadmap: versions and the project store become step 5; the parity steps move to 6-11 | not put to a vote | Low |
| 15 | Stacked PRs on new branches are allowed; only Master merges into main | not put to a vote | Low |
| 16 | Build order: schema foundations, then the kernel facade, then everything else | not put to a vote | Low |

Votes for decisions 1-5 come from ballot 1 (the evidence and full ballot are in the
project files, `decisions/ballot-1-kernel-and-storage.md`). Decisions 6-16 were made
without a vote because the four reviewers had already converged on them, or because they
are cheap to reverse.

---

## 1. Kernel: OpenCASCADE behind our own Rust facade ⚑

**Decision.** sm-kernel's public API exposes only SolidModeller's own types: bodies,
face, edge and vertex handles that carry persistent names, meshes, and mass properties.
No truck or OpenCASCADE type appears in any public signature. Underneath it, one backend
crate (`sm-occt`) links the system OpenCASCADE (OCCT) library through a thin C++ shim
built with `cxx`:

- Every shim entry point catches `Standard_Failure` and returns a `Result`.
- Only opaque handles cross the boundary.
- It targets OCCT 7.6 or later, tested on Ubuntu's 7.6 and Arch's current package.

The document model, history, versioning, sketch solver, UI and file formats stay in
Rust.

- **Untrusted files.** OCCT never parses untrusted bytes in the app process (Sentinel's
  condition). STEP import has two stages. First, a bounded Rust Part 21 parser screens
  the file. Then OCCT reads it in a sandboxed worker subprocess (landlock, seccomp,
  rlimits, timeout). The sandbox is mandatory in every build, with no switch to turn it
  off. The worker takes the file's bytes on stdin and returns only a serialized B-rep,
  which the parent validates.
- **Persistent naming** lives in our own layer. OCCT's per-operation history
  (Modified, Generated, IsDeleted) feeds it.
- **Reproducibility.** The kernel backend and version are recorded with cached geometry
  and in the store, because two OCCT versions can rebuild a part slightly differently.
- **Transition.** New truck-specific work stopped tonight. The facade and the OCCT
  backend are being built now, and new features target the facade. truck stays as the
  working backend until OCCT passes the regression suite and Master confirms. Then truck
  is removed.

**Why.** Inventor parity needs fillets, chamfers, shells, drafts and booleans that are
exact when faces are coplanar. truck has none of the blends in any release, and its last
release was on 2024-09-20. Its booleans fail on the flush cases Inventor users build
every day:

- A boss flush with a side wall.
- Two overlapping sketches extruded to the same height.
- A cut flush with an earlier pocket floor.
- Any part in metres.

PR #3's workaround nudges faces by 0.04 mm, and it silently changed a volume: 9004 mm³
where 9000 mm³ was correct (Sage). Hawk also found truck cuts that return Ok with
nothing left, or with the wrong body. "No mistakes" rules out geometry that is quietly
wrong.

Oracle probed system OCCT 7.6.3 in the build container, installed with
`apt-get install libocct-*-dev` in under a minute:

| Case (all from a 40 × 20 × 10 mm block unless stated) | truck | OpenCASCADE 7.6 |
|---|---|---|
| Two overlapping same-height sketches, joined | Boolean failed | Valid, exact 16000 mm³ |
| Flush pocket, boss on top face, flush end notch | Needs the nudge | Valid, exact, no nudge |
| 10 mm hole through a 20 × 20 × 10 block | 646 ms, polyline edges, 3212.08 mm³ | 4.8 ms, exact cylinder, 3214.602 mm³ |
| 36 holes in a plate in one cut | 215 ms (Bolt) | 64 ms, exact |
| Fillet r = 2 on one edge, on all 12 edges, and on every edge of an L-shaped join | Not available | Valid, exact cylindrical fillet faces |
| Chamfer, 1 mm shell with an open face, 3° draft | Not available | Valid (shell volume exact) |
| AP242 export in inches with name "Bob's 1in cube" and a colour, then re-import | AP214, millimetres only, no names | AP242 header, INCH unit, escaped name, colour, exact volume |
| Which input face became which output face | No history | Modified, Generated and IsDeleted per face and edge |

**Vote: 9-0 for this option.** Confidence ran from 3 to 5. The split was over timing.
Three voters (power user, kernel engineer, architect) said to build the OCCT backend
tonight. Four (local-first expert, devil's advocate, Sentinel, Sage) said only to spike
it and leave the switch to Master. Hawk and Bolt didn't address timing. The decision
takes the common ground: freeze truck work, build the facade and the OCCT backend, and
leave removing truck to Master.

**Dissent and risks.**

- It reverses Master's own words. Offered OpenCascade on day one, he said "code it in
  rust like the cool kids do… start from scratch", and later "truck is ok". Every voter
  named this as the strongest argument against.
- It adds a C++ dependency with crash risk, slower clean builds, version drift (Ubuntu
  7.6 against Arch 7.8/7.9), and an LGPL-2.1 library. LGPL is fine with dynamic linking:
  our code stays MIT/Apache.
- OCCT's fillets are good, but not Parasolid-grade.
- Bolt: crossing the FFI on every feature makes per-feature caching harder, so cache keys
  must come from our own input hashes, not from OCCT state.

**Alternatives.**

- **Stay pure Rust on truck.** Fork it, then write coplanar booleans, blends, shell,
  draft and hidden-line removal ourselves. That is years of kernel work before the CAD
  app.
- **Defer.** Keep truck and accept the failures until step 3. But six threads were
  building on truck tonight, so the switch only gets more expensive.

**What would reverse it.** Master preferring pure Rust after seeing this table, or the
OCCT shim crashing or diverging on Arch's version in the regression suite. Because of
the facade, reversing it means writing a different backend, not rewriting the app.

**This also settles the fillet card** in "Plan the STEP-based solid modeler". We take the
"own kernel types" half of option A (the facade), and fillets come from OCCT instead of
truck master's experimental fillet.

## 2. The working format is a local project store

**Decision.** A project is a folder, the local equivalent of an Onshape document set and
of an Inventor project. It holds every part, assembly and drawing, with their whole
history, in one store:

- Each document has a UUID.
- References between documents in the same project are live on the current branch.
- References to documents in other projects (libraries, standard parts) pin a version,
  as Onshape's linked documents do. That is why Onshape has no "Resolve Link".
- An index answers where-used questions.
- A project-level version or release captures a consistent set of documents at once.
- A loose `.smp` can still be opened directly, as Inventor users expect. It opens as a
  one-document project, and saving it into a project imports it.

**Why.** Master asked "is .smp really the best?" For the working format, no. A zip per
part is opaque to diffs and merges. It also cannot give the Onshape half of the vision:
version-pinned references between parts, assemblies and drawings, plus atomic releases
across several documents. Per-file history (PR #4) gives versions of one part only. Bolt
measured another limit: PR #4's snapshot history hit the 256 MiB read limit at about 600
versions of a 527 KB part, because Deflate cannot deduplicate whole snapshots.

**Vote: 9-0, confidence 3-4.** The strongest argument against: users lose "the file is
the part", so emailing a part needs an export step. PR #4 already works, and the store
is more code before assemblies exist.

**Alternatives.** Keep one `.smp` zip per part with history inside (PR #4 as it is).
Use unzipped JSON folders per document. Use a plain git repository driven by the app
(see decision 4 for why not).

## 3. `.smp` stays as the name of the exchange package

**Decision.** `.smp` (part), `.sma` (assembly) and `.smd` (drawing) become the
single-document packages used for sending files to people, Pack and Go and archiving,
like Inventor's `.ipt`, `.iam` and `.idw`. Each one is a zip with:

- a manifest,
- the canonical document JSON,
- that document's history (optional; PR #4's `history.json` is reused for this),
- a mesh cache (optional).

Each package carries a `kind` field that the reader checks. The project file, which
plays the role of Inventor's `.ipj`, is named by the versioning thread.

**Vote: 8-1 for separate extensions per kind.** The architect voted for one extension
with a `kind` field (one loader, one MIME type).

## 4. Store layout: content-addressed canonical JSON

**Decision.**

- **Objects** are canonical JSON, named by their BLAKE3 hash and written once: write to a
  temp file, fsync, then rename.
- **Refs** (branches, versions, releases) are the only mutable state, updated atomically
  under a lock.
- **Edit log:** an append-only log records every recorded edit.
- **Packing:** objects are packed so that large projects don't open slowly from many
  small files (Bolt).
- **SQLite** is allowed only as a disposable index for where-used queries and search,
  always rebuildable from the objects. Any SQLite use runs in defensive mode with
  `trusted_schema` off and no extensions (Sentinel).
- **Git:** the layout is git-compatible, as plain canonical JSON files with a
  deterministic export. The app never needs git and never runs it on a project folder,
  because a hostile `.git/config` can execute code (Sentinel).
- **Untrusted stores** (Sentinel, on review of this record):
  - Object ids must be exact-length lowercase hex before they are used as paths, and
    paths are never built from names inside objects.
  - Every object and pack entry is re-hashed and verified on read.
  - Pack entries, objects, log records and per-project totals have size caps. Counts in
    pack headers are never trusted.

**Vote: 8-1 for an object store as the source of truth.** The local-first expert voted
for SQLite, in Fossil's style: a single file with transactions, crash safety and fast
queries for free. Against that: the database is binary, not diffable, and unsafe to
live-sync with file tools. Hawk added two conditions: fault-injection tests for crash
safety, and migration of every stored object on read, not only the newest one.

## 5. History: microversions, snapshots and releases

These answer the versioning thread's four open questions.

1. **Snapshots or operation log: both.** Every recorded edit is a microversion in the
   log, pointing at an immutable, content-addressed snapshot. Unchanged features dedupe
   through hashing. Replay length is capped. Vote 9-0; Sage would start with snapshots
   and add the log later.
2. **Where history lives:** in the project store. A package carries one document's
   history only when it is exported.
3. **Git compatibility:** through the layout and a deterministic export, never by making
   git the store.
4. **Releases** use Vault-style states: Work in progress, In review, Released, Obsolete.
   - A released version is read-only forever, enforced by its hash.
   - Branching from a release is allowed, since that is how engineering changes start.
   - On a local single-user machine the user can release. Roles come later.

PR #4's three-way merge engine, branches, tags and diff are kept and moved onto the
store. Once decision 6 lands, the renumbering on merge goes away. Hawk confirmed that
counter ids make a second merge from the same branch edit the wrong feature. A unit
change must conflict with value edits when merging (Hawk).

## 6. Ids: UUIDv7 for every persistent object

**Decision.** Every persistent object gets a random UUIDv7 when it is created: features,
sketch points, curves and constraints, parameters, bodies, work features, and the
document itself.

- Ids are never reused or renumbered.
- They are serialized as lowercase hyphenated strings.
- All objects in a document share one namespace, and validation rejects duplicates.
- Typed newtypes (`FeatureId`, `EntityId`, `ConstraintId`, `ParamId`, `BodyId`,
  `DocumentId`) wrap one representation.
- Tests inject a seeded generator, so they stay deterministic.
- `next_id` is removed.
- Display names are separate from ids. Feature and parameter names stay unique within a
  document, as in Inventor, and a merge renames an incoming duplicate instead of failing.
- Solvers and kernels may use dense internal indices; only the stored form uses UUIDs.

**Why.** Branching and merging is approved. With a per-document counter, two branches
hand out the same id, which breaks merges (Sage, Hawk). Random ids make that impossible
without coordination, offline. UUIDv7 sorts by creation time, which helps debugging and
deterministic tie-breaks. Alternatives were ULIDs (shorter, but not an RFC) and random
64-bit ids (shorter, but too small once ids are referenced across projects).

## 7. Units: millimetres and radians inside

**Decision.**

- **Inside:** all stored geometry, kernel tolerances, caches and STEP export use
  millimetres; angles are radians.
- **Document units** become the display unit and the default unit for input. Changing
  them never moves geometry.

**Why.** This answers the global reviewer's question "which internal unit?" Millimetres
are what STEP files and OpenCASCADE overwhelmingly use, and what most mechanical parts
are sized in. Before this, a metre part made the kernel's 0.01 tolerance mean 10 mm, and
booleans failed (Sage). Inventor stores centimetres internally, but users never see
that, so matching it buys nothing.

## 8. Every number is an expression with units

**Decision.** Every user-entered number is a `Value { name, expr }`. That covers sketch
dimensions, extrude distances, angles, counts, sketch-plane offsets and parameter
values.

- **`expr`** is the text as entered: `40 mm`, `Length / 2`, `30 deg`. Literals are saved
  with an explicit unit, which the UI appends as Inventor does.
- **`name`** is its Inventor-style model parameter name (`d0`, `d1`, …). The next number
  is the highest in use plus one, never a stored counter. Renaming a value rewrites the
  expressions that use it.
- **Dimensions:** each field declares one (length, angle, unitless or integer count).
  Evaluation checks them, so `Length * Width` in a length field is an error on that
  feature.
- **Functions** follow Inventor's.
- **Evaluated numbers** are never stored.
- **User parameters** are numeric for now. Text and true/false types are in the schema
  already; the UI for them comes later.
- **Parsing and evaluation** live in a small `sm-expr` crate.

**Why.** This is how Inventor works, and the owner will expect `d3 * 2`. Changing `f64`
fields to expressions after the freeze would touch every feature in every saved file
(Sage). Referencing parameters by name keeps diffs readable.

## 9. Sketches store entities and constraints

**Decision.** A sketch is `{ placement, entities, constraints }`.

- **Placement:** an origin plane, a work plane or a face, plus an offset value.
- **Entities:** points, lines, circles and arcs for now; splines later. Each has an id,
  geometry in millimetres (solver state, not expressions) and a construction flag.
- **Constraints:** each has an id, one of the solver's 22 kinds (PR #5), entity
  references by id, a driven flag, and a `Value` for dimensions.
- **Profiles:** extrudes and other profile features select regions, named by the entity
  ids of their boundary loops.
- **Face references:** features that point at faces or edges use
  `GeomRef { feature, name }`, with names defined by the topological naming work.

The sketch solver keeps its own internal handles and maps them to these ids.

**Why.** Today a sketch is a list of point loops, so no sketch segment has an identity.
Persistent face names have to start from stable sketch entity ids (Sage).

## 10. Rebuild reports status per feature

**Decision.** Rebuild keeps going past a failed feature, as Inventor does with sick
features.

- **Status per feature:** ok, warning, failed (with a coded, user-facing message),
  skipped because something it depends on failed, suppressed, or rolled back.
- **A failed feature** contributes nothing, and later features that don't depend on it
  still run.
- **Hard errors** are only for unreadable documents. A resource limit fails the feature
  that hits it.
- **Threading:** rebuild can be cancelled and runs off the UI thread.
- **Incremental rebuild** with a per-feature input-hash cache arrives in step 3.

Changing the API now is cheap. Changing it later touches every caller.

## 11. Geometry cache

**Decision.**

- **Cache key:** only the inputs that affect geometry, hashed with BLAKE3, plus a
  `GEOMETRY_VERSION` constant that the kernel bumps whenever it builds geometry
  differently, plus the kernel backend and its version. Renaming a part or a feature, or
  editing a comment, keeps the cache.
- **Contents:** a binary mesh: little-endian positions, normals and indices; per-face
  ranges keyed by persistent face names, so picking works; edge polylines; and a bounding
  box. An exact B-rep cache may come later.
- **STEP** leaves the cache and is used only for export.
- **Rules:** caches are disposable and never versioned.

**Why.** This answers the two open questions in `docs/smp-format.md`. A STEP cache must
still be parsed and tessellated before it can be shown, which on large parts can be
slower than replaying the history (Bolt).

## 12. When the format freezes

**Decision.** The document schema stays a draft, with no migration code, until decisions
6-9 and the naming references have landed (target: step 3). Until then, old development
files may stop loading, with a clear error. At the freeze:

- golden fixture files that every later build must open,
- a `min_reader_version` in the manifest, so older builds open newer files read-only
  instead of silently dropping fields they don't know,
- migration on read from then on, for every stored object.

This answers the global reviewer's question "OK to treat .smp as unstable until the step
3 schema lands?" with yes.

## 13. Origin-plane sketch frames ⚑

**Decision.** The current frames stay, documented in the spec and locked by a test:

| Plane | Sketch x | Sketch y | Normal |
|---|---|---|---|
| XY | +X | +Y | +Z |
| YZ | +Y | +Z | +X |
| XZ | +X | +Z | −Y |

**Master, please check this against Inventor.** This is the global reviewer's open
question: does a sketch on Inventor's XZ plane map sketch y to +Z with normal −Y?
Oracle could not confirm it from sources. It takes ten seconds in Inventor, and changing
it before the freeze costs nothing.

## 14. Roadmap order

**Decision.** Steps 1-4 stay as they are. **Step 5 becomes versions, branches, merges and
the local project store**, the Onshape half of the vision, which is already being built.
The parity checklist's proposed steps 5-10 (PR #2) move to 6-11:

| Step | Contents |
|---|---|
| 6 | Part completeness |
| 7 | Assemblies (they need naming and the project store's references) |
| 8 | Drawings (OpenCASCADE has hidden-line removal) |
| 9 | Sheet metal |
| 10 | Advanced modeling |
| 11 | Add-ins and analysis |

The global reviewer's pull-forwards are accepted:

- **Now:** merge-safe ids, document ids, atomic save, the parameter type system.
- **Step 3:** model states schema hooks (UI in 10), work planes, an undo/command layer
  (`sm-app`), and STEP names and colours.
- **Kernel track:** with OpenCASCADE, it becomes "wrap and test" (exact coplanar
  booleans, fillet and chamfer, shell and draft, hidden lines, unfolding) instead of
  research.

PR #2's step column needs renumbering (5-10 → 6-11) before it merges.

## 15. Branches and merging

New branches for stacked PRs are allowed, so step 3 doesn't have to pile into PR #3.
Each PR states its base and is retargeted to `main` when its base merges. Only Master
merges into `main`.

## 16. Build order for the parallel threads

Several threads change the same schema, so the work is sequenced to avoid writing ids
and schemas twice (Sage):

1. **Schema foundations** (decisions 6-10, owned by the main coder): one small PR stacked
   on PR #3. The sketch solver thread supplies the constraint kinds, and the naming
   thread supplies the persistent name type.
2. **Kernel facade** (owned by the naming thread), with the `sm-occt` backend (owned by
   the STEP import thread) under it.
3. **Then in parallel:** wiring in the solver, the project store, the viewer's mesh cache
   and picking by name, and STEP import.
