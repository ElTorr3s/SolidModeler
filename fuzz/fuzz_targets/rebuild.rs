//! Rebuilds generated, always-valid histories of any size up to the kernel's
//! limits. A crash is a panic that escaped the kernel; a `-timeout` hit is a
//! document within every limit that still ties up the machine.
//! Every document that rebuilds must also survive a save and reopen.

#![no_main]

use libfuzzer_sys::fuzz_target;
use sm_fuzz::FuzzDocument;
use std::io::Cursor;

fuzz_target!(init: sm_fuzz::allow_caught_panics(), |doc: FuzzDocument| {
    let doc = doc.0;
    assert_eq!(doc.validate(), Ok(()));
    let mut buf = Cursor::new(Vec::new());
    sm_format::write(&mut buf, &doc, None).unwrap();
    buf.set_position(0);
    assert_eq!(sm_format::read(buf).unwrap().document, doc);
    let _ = sm_kernel::rebuild(&doc);
});
