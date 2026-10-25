# Topological naming

A history-based modeler lets a later feature point at earlier geometry, for
example "fillet this edge", "sketch on this face", or "extrude up to this
face". The model is rebuilt from the feature tree on every edit, and a
rebuild produces brand-new B-rep entities. The reference therefore has to be
a *name* that can be looked up again in the new model. It cannot be a
pointer or an index into the face list. This is the topological naming
problem. It decides whether an Inventor-style workflow (edit an early sketch
and everything downstream follows) works at all. It also decides whether
Onshape-style branches and merges (roadmap step 5) can be made to work,
because a merged feature tree is a model the user never rebuilt themselves.

This document covers the approach used in `crates/sm-naming`, what it
borrows from other systems, and what is still open.

## How other systems do it

**Index-based naming (the naive approach, FreeCAD before 1.0).** References
are stored as `Face6` or `Edge12`: positions in the kernel's entity list. Any
upstream edit that adds or removes a face renumbers everything after it, and
downstream features silently attach to the wrong face. This is the failure
users mean when they say "the topological naming problem". We never store an
index; `EntityRef` indices are only valid for the rebuild that produced them.

**Generative / history-based naming (Kripac 1997; Capoyleas, Chen and
Hoffmann 1996; Marcheix and Pierra 2002).** An entity is named by *how it was
made*: the feature that created it, plus the input (a sketch curve, an edge
of the profile) it was generated from. Entities that are modified, split or
merged by later operations take their names from their ancestors, plus extra
information to tell the pieces apart. Kripac's "topological ID system" and
Hoffmann's "generic naming" both add topological context, such as
neighbouring faces, to disambiguate split pieces, with geometric criteria as
a last resort. Our scheme belongs to this family.

**OpenCASCADE OCAF (`TNaming`).** OCAF records an *evolution* for each
operation in a data framework: `PRIMITIVE` (new shape), `GENERATED` (made
from an input shape, e.g. a sweep face from a profile edge), `MODIFY` (same
entity, changed geometry), `DELETE`, and `SELECTED`. A `TNaming_Selector`
stores a selection as a naming structure that it re-solves after
regeneration, using the recorded evolutions and, where needed, intersection
of neighbouring named shapes. Its weakness is that every modelling algorithm
has to report its history accurately, and when one doesn't, names break.

**FreeCAD 1.0 (the "TNP mitigation", from realthunder's LinkStage3 branch).**
Every shape carries an *element map* from kernel indices to persistent
names. Names are strings that encode history, e.g. the source element, the
operation tag and the feature id, built with OCCT's `BRepTools_History`
(Generated/Modified) and extended through booleans. A split element gets a
postfix. The strings get long, so they are hashed. References store the
mapped name, and a lookup goes through the current element map.

**Onshape.** References are stored as *queries* that the regeneration
evaluates: "faces created by feature F from sketch entity E", "the edge
between these two faces", and so on. Queries resolve deterministically
against the regenerated model. When a query can't be resolved, the feature
is flagged and the user repairs it in the UI. Onshape doesn't publish the
details, but the general shape is known: names are tied to feature and
sketch-entity identity, not to geometry indices, and resolution is
regeneration-time and deterministic. That is also what a branch/merge
system needs.

**Inventor and SolidWorks.** These are proprietary. Both kernels (Autodesk
ShapeManager, Parasolid) attach persistent attributes or ids to entities and
propagate them through operations, and both fall back to geometric matching
when an edit changes topology. Both surface failures as sick features
(Inventor) or "missing reference" errors (SolidWorks). They don't guess
silently. We copy that UX contract: resolve if the name is unambiguous,
otherwise flag the feature.

## Our scheme

### 1. Generated names for tool faces

Each feature builds a *tool* solid (an extrude or revolve of sketch regions)
before any boolean. `sm_naming::extrude_tool` and `revolve_tool` name each
tool face by its role relative to the sketch:

| Role | Extrude | Revolve |
| --- | --- | --- |
| `start(pN)` | cap on the sketch side (even for a reversed extrude) | cap at angle 0, the sketch half-plane |
| `end(pN)` | far cap | cap at the end angle (partial revolve only) |
| `side(pN.sM)` | face swept by segment M of loop N | face swept by segment M of loop N, numbered `/k` by angle when truck splits it into pieces |

`pN` names a region by its outer loop. A name such as `#7.side(p0.s2)` is
"the face that feature 7 swept from segment 2 of sketch loop 0".

Faces are classified **by geometry**: plane normals against the sketch
frame, projection back into sketch coordinates, and radius/height/angle about
the revolve axis. They are not classified by the order truck's sweep emits
them in, so a truck upgrade cannot quietly reshuffle names. Side faces are
matched to the nearest segment with the same outward direction, so the
kernel's flush-face nudge (0.04 mm, `extrude.rs`) doesn't confuse them.

### 2. Booleans pass names on

truck's booleans never create surfaces. They trim and split the input faces
and copy their surfaces bit-for-bit into the result. So each face of a
boolean result is matched to its input face by **surface identity**: a hash
of the face's surface. The hash ignores orientation, because a cut inverts
the tool. This is our equivalent of OCAF's `MODIFY` evolution, and it needs
nothing from truck except that it keep copying surfaces. The tests check it in
the pocket, slot, inserted-feature and hole scenarios: no face of the
rebuilt model may be left `unknown`. Faces whose surface matches no input
face, or more than one, are listed in `BodyNaming::untraced`, and every test
rebuild asserts that list is empty. If a kernel change ever merges or
rebuilds surfaces (for example a coplanar-face clean-up), the tests fail
loudly instead of references quietly going wrong.

`sm_kernel::rebuild_observed` reports each tool and each body change to a
`RebuildObserver`. The `sm-naming` tracker carries a `surface → origin` map
per body through the whole history. The kernel's plain `rebuild()` is
unchanged and passes a no-op observer.

### 3. Split faces: told apart by neighbours

When a later feature splits a face, as a through-slot splits a top face in
two, every fragment has the same origin. Each fragment then adds the origins
of the neighbouring faces that **only it** touches:

```
#2.end(p0)[#2.side(p0.s3),#4.side(p0.s3)]   top face, left of the slot
#2.end(p0)[#2.side(p0.s1),#4.side(p0.s1)]   top face, right of the slot
```

This is Kripac's and Hoffmann's topological disambiguation. It survives
dimension changes and moving the slot, because neighbours are named, not
measured. Only when neighbours can't tell fragments apart (true symmetry)
does an ordinal `~k` number them by position. That ordinal is the one part
of a name that is not stable, and it is marked as such.

### 4. Edges and vertices are named by their faces

An edge is the pair of faces that meet at it. A vertex is the set of faces
that meet at it. When two faces meet more than once, the edge adds the
origins of the faces at its ends that only it touches, and then an ordinal.
Faces are the primary names; everything else derives from them, as in
Onshape-style adjacency queries.

### 5. Resolution never guesses

`Naming::resolve(&TopoName)` returns one of four results:

- `Exact`: the name exists unchanged.
- `Healed`: the entity is there but its name changed. For example, a split
  face became whole again, or its neighbours changed. The best match is
  unique: neighbour overlap decides, and matching ordinals only break exact
  ties. A stored fragment only heals onto a face that still touches at
  least one of its distinguishing neighbours. If the fragment was cut away,
  its surviving sibling is not a match, and the reference is `Lost`.
- `Ambiguous`: several candidates fit equally well, or several entities
  carry the exact name. The typical case is a
  reference taken on a whole face that a later edit split. Inventor would
  mark the feature sick. A fillet might choose to use all candidates. The
  feature decides; the naming layer doesn't pick one at random.
- `Lost`: nothing has that origin any more, because the creating feature is
  suppressed or deleted, or the face was cut away.

A feature resolves its references against `NamedRebuild::naming_before(id)`,
the model as it stood just above that feature, which matches Inventor's
rollback semantics.

Resolution is read-only. A `Healed` result never rewrites the document
during a rebuild, because a rebuild that edits the file would show up as a
phantom change in version history. The healed name lives in the rebuild
result. The document is only updated when the user accepts the repair or
next edits that feature, as with any other edit.

### 6. Text, merges and the local-first vision

Names are plain serde data and are written into the `.smp` feature tree as
structured JSON, next to the feature that uses them. The `Display` form
(`#7.side(p0.s2)`) is only for people reading logs; it is never parsed back.
The JSON is versioned with the `.smp` format, and every list in it is capped
at `MAX_NAME_LIST` (1024) while parsing, because names come from untrusted
files. Generated names are truncated to the same cap so they always load
again. They contain only feature ids and
sketch keys. They contain no kernel pointers, no hashes and no machine- or
cloud-specific ids, so the same file rebuilds to the same names on any
machine, fully offline. Two branches that add unrelated features produce
identical names regardless of feature order (tested). When a merge renumbers
features (`docs/versioning.md`), `TopoName::remap_features` rewrites every
name through the same id map (tested: every face, edge and vertex name still
resolves to the same geometry after renumbering). After a merge, the
versioning layer rebuilds with `rebuild_named` and turns any `Ambiguous` or
`Lost` reference into a sick feature for the user to repair.

## Choices made to be easy to reverse

- **Separate crate, opt-in kernel hook.** All naming logic is in
  `sm-naming`. The kernel change is a trait with default no-op methods, and
  `rebuild()` behaves exactly as before. To back the work out, delete the
  crate and the trait.
- **No `.smp` format change yet.** No feature stores a `TopoName` today.
  The first referencing feature (fillet, sketch on face) adds the field, and
  that format change goes to the owner for a decision.
- **Surface hash, not kernel attributes.** Matching by surface identity
  works with truck unmodified. If we later fork truck or move to a kernel
  with attribute propagation, only `FaceOrigins::carried` changes. The names
  stay the same.
- **History as a change log.** `NamedRebuild` logs each body change (one
  per boolean) rather than copying every body after every feature, and
  `naming_before` replays the log. Memory grows with the number of booleans,
  which the kernel already caps (`MAX_TOOLS`). The incremental rebuilder
  should own this cache later.

## Decided since the first draft

These come from the design-decisions thread, working under the owner's
delegation:

- **Ids become UUIDv7.** Feature ids, sketch entity ids and constraint ids
  are UUIDv7, never reused or renumbered (schema-foundations PR). Names will
  be built from them, so `TopoName::remap_features` and merge renumbering go
  away.
- **Sketch curve ids.** The sketch solver (PR #5) gives every profile
  segment a `CurveId`. `SketchEdge` becomes that id, which closes limitation
  1 below.
- **OpenCASCADE behind our own facade.** The kernel moves to OCCT behind a
  Rust facade that exposes only our own types. Naming will then be fed from
  OCCT's Modified/Generated/IsDeleted history, including
  `UnifySameDomain`'s, instead of by matching truck's surfaces. The names
  and the resolution rules above don't change; only the place that produces
  `FaceOrigins` does. truck stays the default backend until OCCT passes the
  test suite and the owner confirms the switch.

## Known limitations and next steps

1. **Sketch segments are keyed by index** (`p0.s2`). Inserting a point into
   a loop shifts the indices of the segments after it. Those side faces
   then resolve to the wrong segment's face, and the result may even be
   reported `Exact`. This is the biggest gap. The fix is stable per-entity
   ids in the sketcher (step 3), which the sketch-solver thread has been
   asked for. `SketchEdge` is the only type that changes.
2. **Revolve pieces follow truck's subdivision.** truck splits a revolved
   face into 2 pieces below half a turn and 3 above, so `/k` can change
   meaning when the angle crosses 180°. When Revolve lands in the kernel, it
   should either merge the pieces or name them by angle range.
3. **Revolve isn't wired in yet.** `revolve_tool` is ready and tested
   against truck's `rsweep`. It gets wired into the tracker when the kernel
   gains a Revolve feature.
4. **Fillets, chamfers and shells will create faces from edges and
   vertices**, not from sketch segments. These need new roles such as
   `blend(edge-name)`, built from the input edge's name, which is how OCAF
   and FreeCAD do it. The `FaceRole` enum is designed to grow.
5. **No geometric fallback.** If a face's surface is not found among the
   inputs (never observed so far), it is named `unknown` and its references
   won't survive. A last-resort geometric match, such as a stored sample
   point with a nearest-face lookup, could be added for `Lost` repair
   suggestions in the UI.
6. **Sketch-on-face and "up to face" references** are where most users will
   meet this system. Their features should store a `TopoName` and resolve it
   through `naming_before`.
