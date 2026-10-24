# The `.smp` part file (version 1)

`.smp` is SolidModeller's native, editable part format, the equivalent of an
Inventor `.ipt`. It stores the feature history so a part can be reopened and
edited. STEP is used only for exchange with other CAD tools, because no
mainstream CAD system reads parametric history from STEP.

## Container

An `.smp` file is a zip archive (Deflate) with these entries:

| Entry | Required | Contents |
|---|---|---|
| `manifest.json` | yes | Format id, version, generator, hashes. Always the first entry. |
| `document.json` | yes | The `PartDocument`: units, parameters, feature history, end-of-part marker |
| `cache/model.step` | no | STEP of the rebuilt geometry, so a file can be shown without a rebuild |

Future versions will add entries (thumbnails, appearances, attached files)
without changing the ones above.

## manifest.json

```json
{
  "format": "smp",
  "version": 1,
  "generator": "SolidModeller 0.1.0",
  "document_hash": "a1b2c3d4e5f60718",
  "cache_hash": "a1b2c3d4e5f60718"
}
```

- `version` is bumped on any incompatible change to `document.json`. Readers
  upgrade older versions and refuse newer ones.
- `document_hash` is the 64-bit FNV-1a hash of the stored `document.json` bytes.
- `cache_hash` is the document hash the cache was built from. A reader uses the
  cache only if it equals the hash of the `document.json` it actually read, so
  a hand-edited document never shows stale geometry. Otherwise it rebuilds.

## document.json

The serde serialization of `sm_document::PartDocument`. Features refer to each
other by stable numeric `id`, never by name or position, so renaming and
reordering do not break references. A feature may only refer to features
earlier in the history. `end_of_part` is the rollback marker: features at or
after that index are not rebuilt.
