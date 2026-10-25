//! Reads one STEP file with OpenCascade inside a sandbox.
//!
//! Started only by `sm_occt::step::import_step`, never by hand:
//! `sm-step-worker <memory bytes> <cpu seconds> <max bodies>`, with the
//! file on stdin. Writes the framed result (see `step::encode`) to stdout
//! and exits non-zero on failure.

use sm_occt::sandbox::WorkerLimits;
use std::io::{Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let out = match run() {
        Ok(bytes) => bytes,
        Err(msg) => {
            let _ = std::io::stdout().write_all(&sm_occt::step::worker_error(&msg));
            return ExitCode::FAILURE;
        }
    };
    match std::io::stdout().write_all(&out).and_then(|()| std::io::stdout().flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

fn run() -> Result<Vec<u8>, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [memory, cpu, max_bodies] = args.as_slice() else {
        return Err("usage: sm-step-worker <memory bytes> <cpu seconds> <max bodies>".into());
    };
    let parse = |s: &String| s.parse::<u64>().map_err(|_| format!("bad argument {s:?}"));
    let limits = WorkerLimits {
        memory_bytes: parse(memory)?,
        cpu_seconds: parse(cpu)?,
    };
    let max_bodies = u32::try_from(parse(max_bodies)?).map_err(|_| "bad body limit".to_string())?;

    // Load OpenCascade's resources while files can still be opened.
    sm_occt::step::worker_warm_up().map_err(|e| format!("warm-up: {e}"))?;
    sm_occt::sandbox::lock_down(&limits).map_err(|e| format!("sandbox: {e}"))?;

    let mut input = Vec::new();
    let max = sm_step::screen::Limits::default().max_input_bytes as u64;
    std::io::stdin()
        .take(max + 1)
        .read_to_end(&mut input)
        .map_err(|e| format!("reading input: {e}"))?;
    if input.len() as u64 > max {
        return Err("input too large".into());
    }
    sm_occt::step::worker_read(&input, max_bodies).map_err(|e| e.to_string())
}
