# The `.smp` part file (version 1, draft)

`.smp` is SolidModeller's native, editable part format, the equivalent of an
Inventor `.ipt`. It stores the feature history so a part can be reopened and
edited. STEP is used only for exchange with other CAD tools, because no
mainstream CAD system reads parametric history from STEP.

Version 1 is not frozen yet. Open questions are listed at the end.

## Container

An `.smp` file is a zip archive (Deflate) with these entries:

| Entry | Required | Size limit | Contents |
|---|---|---|---|
| `manifest.json` | yes | 64 KiB | Format id, version, generator, hashes. Always the first entry. |
| `document.json` | yes | 64 MiB | The `PartDocument`: units, parameters, feature history, end-of-part marker |
| `history.json` | no | 256 MiB | Versions, branches and tags of the part. See `docs/versioning.md`. |
| `cache/model.step` | no | 1 GiB | STEP of the rebuilt geometry, so a file can be shown without a rebuild |

Readers enforce the size limits while decompressing (not from the declared
size, which the file's author controls), so a small file can't expand into
gigabytes of memory. Future versions will add entries (thumbnails,
appearances, attached files) without changing the ones above.

Writers save atomically: the archive is written to a temporary file in the
same folder, flushed to disk, and renamed over the old file, so a crash or a
full disk never destroys the previous version.

## manifest.json

```json
{
  "format": "smp",
  "version": 1,
  "generator": "SolidModeller 0.1.0",
  "document_hash": "a1b2c3d4e5f60718",
  "cache_hash": "0f1e2d3c4b5a6978"
}
```

- `version` is bumped on any incompatible change to `document.json`. Readers
  upgrade older versions through `sm_format::migrate` and refuse newer ones.
  Version 0 is invalid.
- `document_hash` is the 64-bit FNV-1a hash of the stored `document.json`.
  It detects changes; it is not an integrity or authenticity check.
- `cache_hash` is the hash of the generator string plus the `document.json`
  the cache was built from. A reader uses the cache only if it matches the
  document it actually read *and* its own generator, so hand-edited files and
  files from other builds (whose kernel may build different geometry) are
  rebuilt. A missing, oversized or non-UTF-8 cache also just means a rebuild.

## document.json

The serde serialization of `sm_document::PartDocument`:

- Features refer to each other by stable numeric `id`, never by name or
  position, so renaming and reordering don't break references. A feature may
  only refer to features earlier in the history. Ids must be unique.
- `end_of_part` is the rollback marker, stored as the id of the first feature
  *below* it, so it moves with that feature when the history is reordered.
- All numbers must be finite. Parameter names must be unique identifiers.
- `next_id` is a hint; readers raise it above the largest id in use.

## Open questions before freezing version 1

These change the format, so they are decisions for the project owner:

1. **Cache key.** Today the cache is invalidated by any change to
   `document.json`, including renames. Hashing only the inputs that affect
   geometry would keep the cache valid across renames and comments.
2. **Cache contents.** A STEP cache still has to be parsed and tessellated to
   be shown. A binary mesh cache (positions, normals, indices, per-face
   ranges) would open much faster; STEP would remain export-only.
3. **Branching and merging.** The first version is implemented: an optional
   `history.json` holds snapshots of the feature tree and merges work at the
   feature level. Its own open decisions (snapshots or an operation log,
   history inside the file or in a project store) are listed in
   `docs/versioning.md`. The feature tree should stay plain, stable,
   diff-friendly JSON, which argues for keeping all derived data in `cache/`
   and treating it as disposable.
