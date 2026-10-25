//! Opens an `.smp` whose `document.json` is the fuzz input, then rebuilds
//! it, which is what opening a file in the app does. The container is always
//! valid, so the fuzzer spends its time in the JSON parser, migration,
//! validation and the kernel. Memory blowups while parsing show up under
//! `-rss_limit_mb`, and slow rebuilds under `-timeout`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(init: sm_fuzz::allow_caught_panics(), |data: &[u8]| {
    let bytes = sm_fuzz::smp_with_document(data);
    if let Ok(file) = sm_format::read(Cursor::new(bytes)) {
        assert_eq!(file.document.validate(), Ok(()));
        let _ = sm_kernel::rebuild(&file.document);
    }
});
