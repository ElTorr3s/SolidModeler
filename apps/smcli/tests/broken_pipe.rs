//! `smcli log part.smp | head` must stop quietly when the reader closes the
//! pipe, not panic.

use sm_document::{LengthUnit, PartDocument};
use sm_history::History;
use std::io::Read;
use std::process::{Command, Stdio};

#[test]
fn log_into_a_closed_pipe_exits_quietly() {
    let dir = std::env::temp_dir().join(format!("smcli-pipe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("part.smp");

    // Far more log than a pipe buffers (64 KiB on Linux), so smcli is still
    // writing when the pipe closes.
    let mut doc = PartDocument::new("Part0", LengthUnit::Mm);
    let mut history = History::new(&doc, "Initial", 1).unwrap();
    for i in 1..2000 {
        doc.name = format!("Part{i}");
        history.commit(&doc, &format!("Rename {i}"), 1 + i).unwrap();
    }
    sm_format::save_with_history(&path, &doc, Some(&history), None).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_smcli"))
        .args(["log", path.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Read one line's worth, then close the pipe like `head -1`.
    let mut first = [0u8; 15];
    child.stdout.take().unwrap().read_exact(&mut first).unwrap();
    let output = child.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(&first, b"on branch main\n");
    assert!(output.status.success(), "{:?}: {stderr}", output.status);
    assert!(stderr.is_empty(), "{stderr}");
}
