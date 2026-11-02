//! Binary encoding of a [`Mesh`] for the geometry cache.
//!
//! The layout is specified in `docs/mesh-cache.md`. Cache files are
//! disposable: anything that fails to decode, carries another layout
//! version, or was built for a different cache key is rebuilt from the
//! history rather than migrated. The decoder treats its input as untrusted:
//! it never panics, checks every count against the bytes actually present
//! before allocating, and only returns meshes that pass
//! [`Mesh::validate`].

use crate::{EdgeRun, FaceRange, Mesh, MeshError, MAX_NAME_BYTES};

pub const MAGIC: [u8; 8] = *b"SMMESH\0\0";
/// Bumped whenever the byte layout changes. Old caches are simply rebuilt.
pub const LAYOUT_VERSION: u32 = 1;
/// Longest backend description accepted, in bytes.
const MAX_BACKEND_BYTES: usize = 256;
const CHECKSUM_BYTES: usize = 32;

/// What the cache was built from, so a stale cache is never shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheHeader {
    /// BLAKE3 of the geometry inputs (computed by the kernel side; opaque here).
    pub key: [u8; 32],
    /// Kernel backend and version, e.g. `"occt 7.8.1"` or `"truck 0.6"`.
    pub backend: String,
}

/// Size limits for decoding untrusted cache bytes.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_bytes: usize,
}

impl Default for Limits {
    /// 512 MiB: about 10 million triangles.
    fn default() -> Self {
        Limits {
            max_bytes: 512 << 20,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("not a mesh cache")]
    BadMagic,
    #[error("mesh cache layout {0} is not supported (expected {LAYOUT_VERSION})")]
    Version(u32),
    #[error("mesh cache uses unknown flags {0:#x}")]
    Flags(u32),
    #[error("mesh cache is {0} bytes, over the limit")]
    TooLarge(usize),
    #[error("mesh cache is truncated")]
    Truncated,
    #[error("mesh cache has {0} unexpected trailing bytes")]
    Trailing(usize),
    #[error("mesh cache checksum does not match its contents")]
    Checksum,
    #[error("mesh cache has a name that is not UTF-8 or is too long")]
    BadString,
    #[error("mesh cache bounding box does not match its points")]
    Bounds,
    #[error("mesh cache holds an invalid mesh: {0}")]
    Invalid(#[from] MeshError),
}

/// Encodes `mesh`. The mesh must be valid (see [`Mesh::validate`]).
pub fn encode(mesh: &Mesh, header: &CacheHeader) -> Result<Vec<u8>, MeshError> {
    mesh.validate()?;
    if header.backend.len() > MAX_BACKEND_BYTES {
        return Err(MeshError::BadName(header.backend.clone()));
    }
    let mut out = Vec::with_capacity(
        64 + mesh.positions.len() * 24 + mesh.indices.len() * 4 + mesh.edge_points.len() * 12,
    );
    out.extend_from_slice(&MAGIC);
    put_u32(&mut out, LAYOUT_VERSION);
    put_u32(&mut out, 0); // flags
    out.extend_from_slice(&header.key);
    put_str(&mut out, &header.backend);
    for n in [
        mesh.bodies.len(),
        mesh.positions.len(),
        mesh.indices.len(),
        mesh.faces.len(),
        mesh.edge_points.len(),
        mesh.edges.len(),
    ] {
        // validate() guarantees these fit in u32.
        put_u32(&mut out, n as u32);
    }
    let (lo, hi) = mesh.bounds().unwrap_or(([0.0; 3], [0.0; 3]));
    lo.iter().chain(&hi).for_each(|&c| put_f32(&mut out, c));
    for body in &mesh.bodies {
        put_str(&mut out, body);
    }
    put_points(&mut out, &mesh.positions);
    put_points(&mut out, &mesh.normals);
    for &i in &mesh.indices {
        put_u32(&mut out, i);
    }
    for f in &mesh.faces {
        for n in [f.body, f.first_index, f.index_count] {
            put_u32(&mut out, n);
        }
        put_str(&mut out, &f.name);
    }
    put_points(&mut out, &mesh.edge_points);
    for e in &mesh.edges {
        for n in [e.body, e.first_point, e.point_count] {
            put_u32(&mut out, n);
        }
        put_str(&mut out, &e.name);
    }
    let checksum = blake3::hash(&out);
    out.extend_from_slice(checksum.as_bytes());
    Ok(out)
}

/// Decodes and validates a mesh cache.
pub fn decode(bytes: &[u8], limits: Limits) -> Result<(CacheHeader, Mesh), DecodeError> {
    if bytes.len() > limits.max_bytes {
        return Err(DecodeError::TooLarge(bytes.len()));
    }
    if bytes.len() < MAGIC.len() || bytes[..MAGIC.len()] != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let mut r = Reader {
        bytes,
        pos: MAGIC.len(),
    };
    // Version before checksum, so a newer layout reports as such.
    let version = r.u32()?;
    if version != LAYOUT_VERSION {
        return Err(DecodeError::Version(version));
    }
    let body_len = bytes
        .len()
        .checked_sub(CHECKSUM_BYTES)
        .ok_or(DecodeError::Truncated)?;
    if body_len < r.pos || blake3::hash(&bytes[..body_len]).as_bytes() != &bytes[body_len..] {
        return Err(DecodeError::Checksum);
    }
    let mut r = Reader {
        bytes: &bytes[..body_len],
        pos: r.pos,
    };

    let flags = r.u32()?;
    if flags != 0 {
        return Err(DecodeError::Flags(flags));
    }
    let key: [u8; 32] = r.take(32)?.try_into().expect("took 32 bytes");
    let backend = r.str(MAX_BACKEND_BYTES)?;
    let [bodies, positions, indices, faces, edge_points, edges] =
        [(); 6].map(|_| r.u32().map(|n| n as usize));
    let (bodies, positions, indices, faces, edge_points, edges) =
        (bodies?, positions?, indices?, faces?, edge_points?, edges?);
    let mut bounds = [0f32; 6];
    for b in &mut bounds {
        *b = r.f32()?;
    }

    // Every element takes at least this many bytes, so a count larger than
    // the remaining input is rejected before anything is allocated.
    let mut mesh = Mesh {
        bodies: r.vec(bodies, 2, |r| r.str(MAX_NAME_BYTES))?,
        positions: r.vec(positions, 12, Reader::point)?,
        normals: r.vec(positions, 12, Reader::point)?,
        indices: r.vec(indices, 4, Reader::u32)?,
        faces: Vec::new(),
        edge_points: Vec::new(),
        edges: Vec::new(),
    };
    mesh.faces = r.vec(faces, 14, |r| {
        let [body, first_index, index_count] = [r.u32()?, r.u32()?, r.u32()?];
        Ok(FaceRange {
            body,
            first_index,
            index_count,
            name: r.str(MAX_NAME_BYTES)?,
        })
    })?;
    mesh.edge_points = r.vec(edge_points, 12, Reader::point)?;
    mesh.edges = r.vec(edges, 14, |r| {
        let [body, first_point, point_count] = [r.u32()?, r.u32()?, r.u32()?];
        Ok(EdgeRun {
            body,
            first_point,
            point_count,
            name: r.str(MAX_NAME_BYTES)?,
        })
    })?;
    if r.pos != r.bytes.len() {
        return Err(DecodeError::Trailing(r.bytes.len() - r.pos));
    }
    mesh.validate()?;
    let expected = mesh.bounds().unwrap_or(([0.0; 3], [0.0; 3]));
    let stored = (
        [bounds[0], bounds[1], bounds[2]],
        [bounds[3], bounds[4], bounds[5]],
    );
    if stored != expected {
        return Err(DecodeError::Bounds);
    }
    Ok((CacheHeader { key, backend }, mesh))
}

fn put_u32(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_le_bytes());
}
fn put_f32(out: &mut Vec<u8>, x: f32) {
    out.extend_from_slice(&x.to_le_bytes());
}
fn put_points(out: &mut Vec<u8>, points: &[[f32; 3]]) {
    points.iter().flatten().for_each(|&c| put_f32(out, c));
}
/// Length-prefixed UTF-8. Callers keep strings under u16::MAX bytes.
fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if n > self.remaining() {
            return Err(DecodeError::Truncated);
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }

    fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }

    fn point(&mut self) -> Result<[f32; 3], DecodeError> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    fn str(&mut self, max: usize) -> Result<String, DecodeError> {
        let len = u16::from_le_bytes(self.take(2)?.try_into().expect("2 bytes")) as usize;
        if len > max {
            return Err(DecodeError::BadString);
        }
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| DecodeError::BadString)
    }

    /// Reads `count` elements of at least `min_size` bytes each.
    fn vec<T>(
        &mut self,
        count: usize,
        min_size: usize,
        mut read: impl FnMut(&mut Self) -> Result<T, DecodeError>,
    ) -> Result<Vec<T>, DecodeError> {
        if count.saturating_mul(min_size) > self.remaining() {
            return Err(DecodeError::Truncated);
        }
        let mut v = Vec::with_capacity(count);
        for _ in 0..count {
            v.push(read(self)?);
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::cube;

    fn header() -> CacheHeader {
        CacheHeader {
            key: [7; 32],
            backend: "truck 0.6".into(),
        }
    }

    #[test]
    fn round_trips() {
        let mesh = cube();
        let bytes = encode(&mesh, &header()).unwrap();
        let (h, back) = decode(&bytes, Limits::default()).unwrap();
        assert_eq!(h, header());
        assert_eq!(back, mesh);

        let empty = encode(&Mesh::default(), &header()).unwrap();
        assert_eq!(
            decode(&empty, Limits::default()).unwrap().1,
            Mesh::default()
        );
    }

    #[test]
    fn layout_is_little_endian_and_fixed() {
        let bytes = encode(&cube(), &header()).unwrap();
        assert_eq!(&bytes[..8], b"SMMESH\0\0");
        assert_eq!(&bytes[8..12], &[1, 0, 0, 0]);
        assert_eq!(&bytes[12..16], &[0; 4]);
        assert_eq!(&bytes[16..48], &[7; 32]);
        assert_eq!(&bytes[48..50], &[9, 0]);
        assert_eq!(&bytes[50..59], b"truck 0.6");
        // Counts: 1 body, 24 positions, 36 indices, 6 faces, 24 points, 12 edges.
        let counts: Vec<u32> = bytes[59..83]
            .chunks(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        assert_eq!(counts, [1, 24, 36, 6, 24, 12]);
    }

    #[test]
    fn refuses_invalid_meshes_to_encode() {
        let mut mesh = cube();
        mesh.indices[0] = 99;
        assert!(encode(&mesh, &header()).is_err());
        let long = CacheHeader {
            backend: "x".repeat(MAX_BACKEND_BYTES + 1),
            ..header()
        };
        assert!(encode(&cube(), &long).is_err());
    }

    #[test]
    fn rejects_bad_headers() {
        let bytes = encode(&cube(), &header()).unwrap();
        let limits = Limits::default();
        assert_eq!(decode(b"", limits), Err(DecodeError::BadMagic));
        assert_eq!(decode(b"SMMESH", limits), Err(DecodeError::BadMagic));
        let mut v2 = bytes.clone();
        v2[8] = 2;
        assert_eq!(decode(&v2, limits), Err(DecodeError::Version(2)));
        assert_eq!(
            decode(&bytes, Limits { max_bytes: 100 }),
            Err(DecodeError::TooLarge(bytes.len()))
        );
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(decode(&trailing, limits), Err(DecodeError::Checksum));
    }

    /// Re-signs a tampered body so the checks behind the checksum run.
    fn resign(mut bytes: Vec<u8>) -> Vec<u8> {
        let n = bytes.len() - CHECKSUM_BYTES;
        let sum = blake3::hash(&bytes[..n]);
        bytes[n..].copy_from_slice(sum.as_bytes());
        bytes
    }

    #[test]
    fn rejects_well_signed_garbage() {
        let bytes = encode(&cube(), &header()).unwrap();
        let limits = Limits::default();
        let mut flags = bytes.clone();
        flags[12] = 1;
        assert_eq!(decode(&resign(flags), limits), Err(DecodeError::Flags(1)));

        // Huge counts are refused before allocating.
        let mut huge = bytes.clone();
        huge[63..67].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&resign(huge), limits), Err(DecodeError::Truncated));

        // Wrong bounding box.
        let mut bounds = bytes.clone();
        bounds[83..87].copy_from_slice(&(-1.0f32).to_le_bytes());
        assert_eq!(decode(&resign(bounds), limits), Err(DecodeError::Bounds));

        // An index past the positions surfaces as an invalid mesh.
        let first_index = 83 + 24 + 2 + 6 + 24 * 24;
        let mut index = bytes.clone();
        index[first_index..first_index + 4].copy_from_slice(&500u32.to_le_bytes());
        assert!(matches!(
            decode(&resign(index), limits),
            Err(DecodeError::Invalid(MeshError::IndexOutOfRange { .. }))
        ));

        // Invalid UTF-8 in the body name.
        let mut utf8 = bytes.clone();
        utf8[109] = 0xff;
        assert_eq!(decode(&resign(utf8), limits), Err(DecodeError::BadString));
    }

    /// Every truncation and every single-byte corruption is an error, never
    /// a panic or a silently different mesh.
    #[test]
    fn survives_truncation_and_corruption() {
        let bytes = encode(&cube(), &header()).unwrap();
        let limits = Limits::default();
        for n in 0..bytes.len() {
            assert!(decode(&bytes[..n], limits).is_err(), "truncated at {n}");
        }
        for i in 0..bytes.len() {
            let mut b = bytes.clone();
            b[i] ^= 0x5a;
            assert!(decode(&b, limits).is_err(), "corrupted byte {i}");
            // With a valid checksum, decoding may succeed only on a valid mesh.
            if let Ok((_, mesh)) = decode(&resign(b), limits) {
                assert_eq!(mesh.validate(), Ok(()));
            }
        }
    }

    /// Random bytes behind a correct magic, version and checksum.
    #[test]
    fn random_input_never_panics() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..2000 {
            let len = (next() % 400) as usize;
            let mut b = MAGIC.to_vec();
            b.extend_from_slice(&LAYOUT_VERSION.to_le_bytes());
            b.extend((0..len).map(|_| next() as u8));
            b.extend_from_slice(&[0; CHECKSUM_BYTES]);
            let _ = decode(&resign(b), Limits::default());
        }
    }
}
