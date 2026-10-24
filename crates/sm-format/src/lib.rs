//! The native `.smp` part file. See `docs/smp-format.md` for the layout.
//!
//! An `.smp` is a zip archive:
//!
//! ```text
//! manifest.json      format id, version, and the hash of document.json
//! document.json      the PartDocument (parameters + feature history)
//! cache/model.step   optional geometry cache, valid only if its hash matches
//! ```

use serde::{Deserialize, Serialize};
use sm_document::PartDocument;
use std::io::{Read, Seek, Write};
use thiserror::Error;
use zip::write::SimpleFileOptions;

pub const FORMAT_ID: &str = "smp";
/// Bumped whenever `document.json` changes incompatibly. Readers upgrade
/// older versions and refuse newer ones.
pub const FORMAT_VERSION: u32 = 1;

const MANIFEST: &str = "manifest.json";
const DOCUMENT: &str = "document.json";
const CACHE_STEP: &str = "cache/model.step";

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
    #[error("not an .smp file (format id {0:?})")]
    WrongFormat(String),
    #[error("file version {found} is newer than this build supports ({FORMAT_VERSION})")]
    TooNew { found: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub generator: String,
    /// FNV-1a hash of `document.json`, used to tell whether the cache is stale.
    pub document_hash: String,
    /// Hash of the document the cache was built from, if a cache is present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hash: Option<String>,
}

/// What [`read`] returns.
#[derive(Debug)]
pub struct SmpFile {
    pub manifest: Manifest,
    pub document: PartDocument,
    /// Cached STEP geometry, present only if it matches the document.
    pub cached_step: Option<String>,
}

/// Writes `doc` as an `.smp` archive. `cached_step` should be the STEP export
/// of a rebuild of exactly this document, or `None` to skip the cache.
pub fn write<W: Write + Seek>(
    out: W,
    doc: &PartDocument,
    cached_step: Option<&str>,
) -> Result<(), FormatError> {
    let document = serde_json::to_vec_pretty(doc).map_err(|source| FormatError::Json {
        file: DOCUMENT,
        source,
    })?;
    let hash = fnv1a(&document);
    let manifest = Manifest {
        format: FORMAT_ID.into(),
        version: FORMAT_VERSION,
        generator: concat!("SolidModeller ", env!("CARGO_PKG_VERSION")).into(),
        document_hash: hash.clone(),
        cache_hash: cached_step.map(|_| hash),
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
        serde_json::from_slice(&read_entry(&mut zip, MANIFEST)?).map_err(|source| {
            FormatError::Json {
                file: MANIFEST,
                source,
            }
        })?;
    if manifest.format != FORMAT_ID {
        return Err(FormatError::WrongFormat(manifest.format));
    }
    if manifest.version > FORMAT_VERSION {
        return Err(FormatError::TooNew {
            found: manifest.version,
        });
    }

    let document_bytes = read_entry(&mut zip, DOCUMENT)?;
    let document = serde_json::from_slice(&document_bytes).map_err(|source| FormatError::Json {
        file: DOCUMENT,
        source,
    })?;

    // Trust the cache only if it was built from the document actually stored,
    // so hand-edited or partially rewritten files still rebuild correctly.
    let actual_hash = fnv1a(&document_bytes);
    let cached_step = match &manifest.cache_hash {
        Some(h) if *h == actual_hash => {
            let bytes = read_entry(&mut zip, CACHE_STEP)?;
            Some(String::from_utf8_lossy(&bytes).into_owned())
        }
        _ => None,
    };
    Ok(SmpFile {
        manifest,
        document,
        cached_step,
    })
}

fn read_entry<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>, FormatError> {
    let mut buf = Vec::new();
    zip.by_name(name)?.read_to_end(&mut buf)?;
    Ok(buf)
}

fn fnv1a(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sm_document::LengthUnit;
    use std::io::Cursor;

    #[test]
    fn round_trip_with_cache() {
        let doc = PartDocument::new("Part1", LengthUnit::Mm);
        let mut buf = Cursor::new(Vec::new());
        write(&mut buf, &doc, Some("ISO-10303-21;")).unwrap();
        buf.set_position(0);
        let file = read(buf).unwrap();
        assert_eq!(file.document, doc);
        assert_eq!(file.cached_step.as_deref(), Some("ISO-10303-21;"));
    }

    #[test]
    fn stale_cache_is_ignored() {
        let doc = PartDocument::new("Part1", LengthUnit::Mm);
        let mut buf = Cursor::new(Vec::new());
        write(&mut buf, &doc, Some("old geometry")).unwrap();

        // Rewrite the archive with an edited document but the old manifest.
        buf.set_position(0);
        let mut src = zip::ZipArchive::new(&mut buf).unwrap();
        let manifest = read_entry(&mut src, MANIFEST).unwrap();
        let step = read_entry(&mut src, CACHE_STEP).unwrap();
        let edited = serde_json::to_vec(&PartDocument::new("Renamed", LengthUnit::Mm)).unwrap();
        let mut out = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        for (name, data) in [
            (MANIFEST, &manifest),
            (DOCUMENT, &edited),
            (CACHE_STEP, &step),
        ] {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();

        out.set_position(0);
        let file = read(out).unwrap();
        assert_eq!(file.document.name, "Renamed");
        assert!(file.cached_step.is_none());
    }

    #[test]
    fn rejects_newer_version() {
        let mut out = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        zip.start_file(MANIFEST, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"format":"smp","version":99,"generator":"x","document_hash":"0"}"#)
            .unwrap();
        zip.finish().unwrap();
        out.set_position(0);
        assert!(matches!(read(out), Err(FormatError::TooNew { found: 99 })));
    }
}
