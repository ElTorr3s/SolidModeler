//! Property tests for the `.smp` reader and writer. The reader takes files
//! from anyone, so beyond round trips it must never panic on any input.

use proptest::prelude::*;
use sm_testkit::strategies::document;
use std::io::Cursor;

fn encode(doc: &sm_document::PartDocument, step: Option<&str>) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    sm_format::write(&mut buf, doc, step).unwrap();
    buf.into_inner()
}

proptest! {
    /// Saving and reopening gives back exactly the same document and cache.
    #[test]
    fn write_read_round_trip(doc in document(), step in prop::option::of(".{0,64}")) {
        let file = sm_format::read(Cursor::new(encode(&doc, step.as_deref()))).unwrap();
        prop_assert_eq!(file.document, doc);
        prop_assert_eq!(file.cached_step, step);
    }

    /// Arbitrary bytes are rejected with an error, never a panic.
    #[test]
    fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        let _ = sm_format::read(Cursor::new(bytes));
    }

    /// Corrupting bytes of a valid file is rejected with an error or still
    /// opens to a valid document, never a panic.
    #[test]
    fn corrupted_files_never_panic(
        doc in document(),
        edits in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..16),
    ) {
        let mut bytes = encode(&doc, Some("ISO-10303-21;"));
        for (i, b) in edits {
            let i = i.index(bytes.len());
            bytes[i] = b;
        }
        if let Ok(file) = sm_format::read(Cursor::new(bytes)) {
            prop_assert_eq!(file.document.validate(), Ok(()));
        }
    }

    /// Truncated files (an interrupted copy or download) never panic.
    #[test]
    fn truncated_files_never_panic(doc in document(), cut in any::<prop::sample::Index>()) {
        let bytes = encode(&doc, None);
        let n = cut.index(bytes.len());
        prop_assert!(sm_format::read(Cursor::new(&bytes[..n])).is_err());
    }
}
