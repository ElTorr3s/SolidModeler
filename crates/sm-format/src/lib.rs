//! The native `.smp` part file. See `docs/smp-format.md` for the layout.
//!
//! An `.smp` is a zip archive:
//!
//! ```text
//! manifest.json      format id, version, and the hash of document.json
//! document.json      the PartDocument (parameters + feature history)
//! cache/model.step   optional geometry cache, valid only if its hash matches
//! ```
//!
//! Files may come from anyone, so every entry is read with a size limit and
//! the cache is only trusted when it matches the stored document.

use serde::{Deserialize, Serialize};
use sm_document::PartDocument;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, Write};
use std::path::Path;
use thiserror::Error;
use zip::write::SimpleFileOptions;

pub const FORMAT_ID: &str = "smp";
/// Bumped whenever `document.json` changes incompatibly. Readers upgrade
/// older versions through [`migrate`] and refuse newer ones.
pub const FORMAT_VERSION: u32 = 1;

const MANIFEST: &str = "manifest.json";
const DOCUMENT: &str = "document.json";
const CACHE_STEP: &str = "cache/model.step";

/// Largest uncompressed size accepted for each entry. Zip entries can
/// decompress about 1000:1, so these stop small files from exhausting memory.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
pub const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_CACHE_BYTES: u64 = 1024 * 1024 * 1024;

const GENERATOR: &str = concat!("SolidModeller ", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Error)]
pub enum FormatError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("archive error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("invalid json in {file}: {source}")]
    Json {
        file: &'static str,
        source: serde_json::Error,
    },
    #[error("{file} is larger than the {limit}-byte limit")]
    TooLarge { file: &'static str, limit: u64 },
    #[error("not an .smp file (format id {0:?})")]
    WrongFormat(String),
    #[error("file version {found} is newer than this build supports ({FORMAT_VERSION})")]
    TooNew { found: u32 },
    #[error("file version {0} is not a valid .smp version")]
    BadVersion(u32),
    #[error("invalid document: {0}")]
    Invalid(#[from] sm_document::DocumentError),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub generator: String,
    /// FNV-1a hash of `document.json`. Detects changes; it is not an
    /// integrity check, since whoever writes the file controls it.
    pub document_hash: String,
    /// Hash of the generator plus document the cache was built from, if a
    /// cache is present. Including the generator invalidates caches written
    /// by a different build, whose kernel may build different geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hash: Option<String>,
}

/// What [`read`] returns.
#[derive(Debug)]
pub struct SmpFile {
    pub manifest: Manifest,
    pub document: PartDocument,
    /// Cached STEP geometry, present only if it matches the document and
    /// this build. Treat it as untrusted input like any other STEP file.
    pub cached_step: Option<String>,
}

/// Saves `doc` to `path` without risking the existing file: the archive is
/// written to a temporary file next to it, flushed to disk, and then renamed
/// over the original in one step.
pub fn save(path: &Path, doc: &PartDocument, cached_step: Option<&str>) -> Result<(), FormatError> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map_or_else(Default::default, |n| n.to_string_lossy());
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let file = File::create(&tmp)?;
        let mut out = BufWriter::new(file);
        write(&mut out, doc, cached_step)?;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        std::fs::rename(&tmp, path)?;
        // Make the rename itself durable.
        #[cfg(unix)]
        File::open(dir)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

pub fn open(path: &Path) -> Result<SmpFile, FormatError> {
    read(File::open(path)?)
}

/// Writes `doc` as an `.smp` archive. `cached_step` should be the STEP export
/// of a rebuild of exactly this document, or `None` to skip the cache.
pub fn write<W: Write + Seek>(
    out: W,
    doc: &PartDocument,
    cached_step: Option<&str>,
) -> Result<(), FormatError> {
    // Never write a file that can't be read back.
    doc.validate()?;
    let document = serde_json::to_vec_pretty(doc).map_err(|source| FormatError::Json {
        file: DOCUMENT,
        source,
    })?;
    let manifest = Manifest {
        format: FORMAT_ID.into(),
        version: FORMAT_VERSION,
        generator: GENERATOR.into(),
        document_hash: fnv1a(&[&document]),
        cache_hash: cached_step.map(|_| cache_key(GENERATOR, &document)),
    };
    let manifest = serde_json::to_vec_pretty(&manifest).map_err(|source| FormatError::Json {
        file: MANIFEST,
        source,
    })?;

    let mut zip = zip::ZipWriter::new(out);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    // Manifest first so tools can identify the file from the first entry.
    zip.start_file(MANIFEST, options)?;
    zip.write_all(&manifest)?;
    zip.start_file(DOCUMENT, options)?;
    zip.write_all(&document)?;
    if let Some(step) = cached_step {
        zip.start_file(CACHE_STEP, options)?;
        zip.write_all(step.as_bytes())?;
    }
    zip.finish()?;
    Ok(())
}

pub fn read<R: Read + Seek>(input: R) -> Result<SmpFile, FormatError> {
    let mut zip = zip::ZipArchive::new(input)?;
    let manifest: Manifest =
        serde_json::from_slice(&read_entry(&mut zip, MANIFEST, MAX_MANIFEST_BYTES)?).map_err(
            |source| FormatError::Json {
                file: MANIFEST,
                source,
            },
        )?;
    if manifest.format != FORMAT_ID {
        return Err(FormatError::WrongFormat(manifest.format));
    }
    if manifest.version > FORMAT_VERSION {
        return Err(FormatError::TooNew {
            found: manifest.version,
        });
    }

    let document_bytes = read_entry(&mut zip, DOCUMENT, MAX_DOCUMENT_BYTES)?;
    let json: serde_json::Value =
        serde_json::from_slice(&document_bytes).map_err(|source| FormatError::Json {
            file: DOCUMENT,
            source,
        })?;
    let mut document: PartDocument = serde_json::from_value(migrate(manifest.version, json)?)
        .map_err(|source| FormatError::Json {
            file: DOCUMENT,
            source,
        })?;
    document.repair_ids();
    document.validate()?;

    // Trust the cache only if it was built by this build from the document
    // actually stored. Anything else (stale, missing, unreadable, not UTF-8)
    // just means a rebuild.
    let cached_step = match &manifest.cache_hash {
        Some(h) if *h == cache_key(GENERATOR, &document_bytes) => {
            read_entry(&mut zip, CACHE_STEP, MAX_CACHE_BYTES)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
        }
        _ => None,
    };
    Ok(SmpFile {
        manifest,
        document,
        cached_step,
    })
}

/// Upgrades a `document.json` written by an older format version to the
/// current one. Version 1 is the first, so there is nothing to upgrade yet;
/// each future version adds a step here.
pub fn migrate(version: u32, json: serde_json::Value) -> Result<serde_json::Value, FormatError> {
    match version {
        1 => Ok(json),
        v => Err(FormatError::BadVersion(v)),
    }
}

fn read_entry<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &'static str,
    limit: u64,
) -> Result<Vec<u8>, FormatError> {
    let entry = zip.by_name(name)?;
    let too_large = FormatError::TooLarge { file: name, limit };
    // The declared size is attacker-controlled, so it only allows an early
    // exit; the `take` below is the real limit.
    if entry.size() > limit {
        return Err(too_large);
    }
    let mut buf = Vec::new();
    entry.take(limit.saturating_add(1)).read_to_end(&mut buf)?;
    if buf.len() as u64 > limit {
        return Err(too_large);
    }
    Ok(buf)
}

fn cache_key(generator: &str, document: &[u8]) -> String {
    fnv1a(&[generator.as_bytes(), b"\0", document])
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

#[cfg(test)]
mod tests {
    use super::*;
    use sm_document::{
        BodyOperation, Extrude, ExtrudeExtent, FeatureId, FeatureKind, LengthUnit, OriginPlane,
        Parameter, Profile, Sketch,
    };
    use std::io::Cursor;

    fn part() -> PartDocument {
        let mut doc = PartDocument::new("Part1", LengthUnit::In);
        doc.parameters.push(Parameter {
            name: "depth".into(),
            value: 0.5,
            comment: String::new(),
        });
        let sketch = doc.add_feature(
            "Sketch1",
            FeatureKind::Sketch(Sketch {
                plane: OriginPlane::XZ,
                offset: 1.0,
                profiles: vec![Profile {
                    points: vec![[0.0, 0.0], [2.0, 0.0], [2.0, 1.0]],
                }],
            }),
        );
        for (name, operation) in [
            ("Extrusion1", BodyOperation::Join),
            ("Cut1", BodyOperation::Cut),
        ] {
            doc.add_feature(
                name,
                FeatureKind::Extrude(Extrude {
                    sketch,
                    extent: ExtrudeExtent::Symmetric { distance: 0.5 },
                    operation,
                }),
            );
        }
        doc.features[2].suppressed = true;
        doc.end_of_part = Some(FeatureId(3));
        doc
    }

    fn to_bytes(doc: &PartDocument, step: Option<&str>) -> Cursor<Vec<u8>> {
        let mut buf = Cursor::new(Vec::new());
        write(&mut buf, doc, step).unwrap();
        buf.set_position(0);
        buf
    }

    /// Builds an archive by hand from (name, bytes) entries.
    fn archive(entries: &[(&str, &[u8])]) -> Cursor<Vec<u8>> {
        let mut out = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        out.set_position(0);
        out
    }

    fn entry(buf: &mut Cursor<Vec<u8>>, name: &'static str) -> Vec<u8> {
        buf.set_position(0);
        let bytes = read_entry(
            &mut zip::ZipArchive::new(&mut *buf).unwrap(),
            name,
            u64::MAX,
        );
        buf.set_position(0);
        bytes.unwrap()
    }

    #[test]
    fn round_trip_with_cache() {
        let doc = part();
        let file = read(to_bytes(&doc, Some("ISO-10303-21;"))).unwrap();
        assert_eq!(file.document, doc);
        assert_eq!(file.cached_step.as_deref(), Some("ISO-10303-21;"));
    }

    #[test]
    fn stale_or_foreign_cache_is_ignored() {
        let mut buf = to_bytes(&part(), Some("old geometry"));
        let manifest = entry(&mut buf, MANIFEST);
        let step = entry(&mut buf, CACHE_STEP);

        // Edited document, old manifest.
        let mut renamed = part();
        renamed.name = "Renamed".into();
        let edited = serde_json::to_vec(&renamed).unwrap();
        let file = read(archive(&[
            (MANIFEST, &manifest),
            (DOCUMENT, &edited),
            (CACHE_STEP, &step),
        ]))
        .unwrap();
        assert_eq!(file.document.name, "Renamed");
        assert!(file.cached_step.is_none());

        // Same document, but the cache was written by another build.
        let document = entry(&mut buf, DOCUMENT);
        let mut m: Manifest = serde_json::from_slice(&manifest).unwrap();
        m.cache_hash = Some(cache_key("SolidModeller 0.0.1", &document));
        let m = serde_json::to_vec(&m).unwrap();
        let file = read(archive(&[
            (MANIFEST, &m),
            (DOCUMENT, &document),
            (CACHE_STEP, &step),
        ]))
        .unwrap();
        assert!(file.cached_step.is_none());
    }

    #[test]
    fn missing_or_invalid_cache_entry_still_opens() {
        let mut buf = to_bytes(&part(), Some("geometry"));
        let manifest = entry(&mut buf, MANIFEST);
        let document = entry(&mut buf, DOCUMENT);
        let file = read(archive(&[(MANIFEST, &manifest), (DOCUMENT, &document)])).unwrap();
        assert!(file.cached_step.is_none());

        let file = read(archive(&[
            (MANIFEST, &manifest),
            (DOCUMENT, &document),
            (CACHE_STEP, b"\xff\xfe not utf-8"),
        ]))
        .unwrap();
        assert!(file.cached_step.is_none());
    }

    #[test]
    fn rejects_bad_versions() {
        for (version, ok) in [(99, false), (0, false)] {
            let manifest = format!(
                r#"{{"format":"smp","version":{version},"generator":"x","document_hash":"0"}}"#
            );
            let doc = serde_json::to_vec(&part()).unwrap();
            let result = read(archive(&[
                (MANIFEST, manifest.as_bytes()),
                (DOCUMENT, &doc),
            ]));
            assert_eq!(result.is_ok(), ok);
            match version {
                99 => assert!(matches!(result, Err(FormatError::TooNew { found: 99 }))),
                _ => assert!(matches!(result, Err(FormatError::BadVersion(0)))),
            }
        }
    }

    #[test]
    fn rejects_decompression_bombs() {
        // 1 MiB of spaces compresses to about 1 KiB.
        let bomb = vec![b' '; (MAX_MANIFEST_BYTES * 16) as usize];
        let result = read(archive(&[(MANIFEST, &bomb)]));
        assert!(matches!(
            result,
            Err(FormatError::TooLarge { file: MANIFEST, .. })
        ));
    }

    #[test]
    fn refuses_to_write_invalid_documents() {
        let mut doc = part();
        doc.parameters[0].value = f64::NAN;
        let mut buf = Cursor::new(Vec::new());
        assert!(matches!(
            write(&mut buf, &doc, None),
            Err(FormatError::Invalid(_))
        ));
    }

    #[test]
    fn save_replaces_file_atomically() {
        let dir = std::env::temp_dir().join(format!("smp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("part.smp");

        save(&path, &part(), None).unwrap();
        // A failed save (invalid document) leaves the old file intact.
        let mut bad = part();
        bad.parameters[0].value = f64::INFINITY;
        assert!(save(&path, &bad, None).is_err());
        assert_eq!(open(&path).unwrap().document, part());
        // No temp files left behind.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
