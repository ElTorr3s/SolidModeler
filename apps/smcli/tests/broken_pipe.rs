//! `smcli log part.smp | head` must stop quietly when the reader closes the
//! pipe, not panic.

use sm_document::{LengthUnit, PartDocument};
use sm_history::{Head, History};
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

/// Runs smcli with its output going into a pipe whose reader is already
/// closed, so every write fails with a broken pipe.
fn smcli_into_closed_pipe(args: &[&str]) -> std::process::Output {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    Command::new(env!("CARGO_BIN_EXE_smcli"))
        .args(args)
        .stdout(writer)
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

fn smcli(args: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_smcli"))
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
}

#[test]
fn checkout_and_merge_into_a_closed_pipe_still_save() {
    let dir = std::env::temp_dir().join(format!("smcli-pipe-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("part.smp");
    let p = path.to_str().unwrap();
    smcli(&["demo", p]);
    smcli(&["commit", p, "Initial"]);
    smcli(&["branch", p, "b"]);
    smcli(&["set-distance", p, "Extrusion3", "6"]);
    smcli(&["commit", p, "main 6"]);

    let output = smcli_into_closed_pipe(&["checkout", p, "b"]);
    assert!(output.status.success(), "{output:?}");
    let history = sm_format::open(&path).unwrap().history.unwrap();
    assert_eq!(
        history.head(),
        &Head::Branch("b".into()),
        "checkout not saved"
    );

    smcli(&["set-distance", p, "Extrusion2", "-8"]);
    smcli(&["commit", p, "b -8"]);
    smcli(&["checkout", p, "main"]);
    let output = smcli_into_closed_pipe(&["merge", p, "b"]);
    let history = sm_format::open(&path).unwrap().history.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(history.head(), &Head::Branch("main".into()));
    assert_eq!(history.head_version().parents.len(), 2, "merge not saved");
}
