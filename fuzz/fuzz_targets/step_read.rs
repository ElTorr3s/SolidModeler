//! Parses arbitrary text as STEP (ISO 10303-21) with truck-stepio and
//! converts every shell it finds, which is the path STEP import and the
//! `.smp` geometry cache will take. The parser is a dependency, but files
//! from other CAD tools reach it directly, so its crashes are ours.

#![no_main]

use libfuzzer_sys::fuzz_target;
use truck_stepio::r#in::Table;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Some(table) = Table::from_step(text) else {
        return;
    };
    for shell in table.shell.values() {
        let _ = table.to_compressed_shell(shell);
    }
});
