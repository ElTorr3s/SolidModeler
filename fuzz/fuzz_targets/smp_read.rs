//! Opens arbitrary bytes as an `.smp` file. Covers the zip container and the
//! per-entry size limits (decompression bombs): run with `-rss_limit_mb` so
//! a small input that expands into gigabytes is reported as a crash.

#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    if let Ok(file) = sm_format::read(Cursor::new(data)) {
        // Whatever the reader accepts must be a valid document.
        assert_eq!(file.document.validate(), Ok(()));
    }
});
