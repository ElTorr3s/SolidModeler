# Mesh cache layout (`sm-mesh`, layout version 1)

The geometry cache holds the display mesh of a rebuilt part so a document
opens with its geometry on screen before the history has been replayed. It
replaces the cached `model.step` (Oracle decision D10). The format is a
cache, not a document: it is disposable and never migrated. A reader that
sees another layout version, a different cache key or backend, a bad
checksum, or anything else it doesn't like, throws the cache away and
rebuilds.

Implementation: `crates/sm-mesh` (`encode`, `decode`, `Mesh::validate`).

## Contents

- Bodies, each with a persistent body id.
- Triangles: positions and per-vertex normals (f32, millimetres) and u32
  indices.
- Faces: contiguous runs of triangles keyed by the face's persistent name
  (the canonical text of sm-naming's `TopoName`), so a pick in the viewport
  resolves to the same reference a feature stores. Faces don't share
  vertices, so normals stay sharp at edges and one face can be highlighted
  alone.
- Edges: polylines keyed by the edge's persistent name.
- The axis-aligned bounding box.
- A header naming what the cache was built from: a 32-byte cache key
  (BLAKE3 of the geometry inputs, per D10) and the kernel backend and
  version, e.g. `occt 7.8.1`.

All numbers are little-endian. Strings are UTF-8 with a u16 byte length.

## Byte layout

| Field | Type | Notes |
|---|---|---|
| magic | 8 bytes | `SMMESH\0\0` |
| layout version | u32 | `1` |
| flags | u32 | must be `0` |
| cache key | 32 bytes | opaque to sm-mesh |
| backend | str | at most 256 bytes |
| counts | 6 × u32 | bodies, positions, indices, faces, edge points, edges |
| bounds | 6 × f32 | min xyz, max xyz; must equal the bounds of all points |
| bodies | str × bodies | persistent body ids |
| positions | 3 × f32 × positions | mm |
| normals | 3 × f32 × positions | |
| indices | u32 × indices | triangles |
| faces | faces × (u32 body, u32 first index, u32 index count, str name) | |
| edge points | 3 × f32 × edge points | mm |
| edges | edges × (u32 body, u32 first point, u32 point count, str name) | |
| checksum | 32 bytes | BLAKE3 of every byte before it |

## Rules a decoded mesh must satisfy

The decoder enforces all of them and returns an error, never a panic:

- The input is at most `Limits::max_bytes` (512 MiB by default). Each count
  is checked against the bytes actually left before anything is allocated.
- Floats are finite. Indices refer to existing positions, and the index
  count is a multiple of 3.
- Faces tile the index buffer in order with no gaps, each a whole number of
  triangles, and every vertex belongs to exactly one face. Edges tile the
  edge points in order, each with at least 2 points.
- Body references are in range. Names are non-empty, at most 4096 bytes,
  valid UTF-8, and unique among faces, among edges and among bodies.
- Nothing follows the checksum, and the stored bounds match the points.

## Who writes it

The kernel facade produces an `sm_mesh::Mesh` after each rebuild. The
viewer draws it, and the save path encodes it into the package with the
cache key. Until the facade lands, the app converts truck solids itself and
uses placeholder names (`b0.f3`) that are stable only between identical
rebuilds.
