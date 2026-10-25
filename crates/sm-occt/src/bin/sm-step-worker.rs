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
    match std::io::stdout()
        .write_all(&out)
        .and_then(|()| std::io::stdout().flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

fn run() -> Result<Vec<u8>, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--sandbox-self-test") {
        return self_test();
    }
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

/// Locks down, then checks that what the sandbox must forbid fails and
/// what the reader needs still works. Used by the test suite.
fn self_test() -> Result<Vec<u8>, String> {
    sm_occt::step::worker_warm_up().map_err(|e| format!("warm-up: {e}"))?;
    sm_occt::sandbox::lock_down(&WorkerLimits::default()).map_err(|e| format!("sandbox: {e}"))?;
    let mut report = Vec::new();
    let mut check = |name: &str, allowed: bool| {
        report.push(format!(
            "{name}={}",
            if allowed { "allowed" } else { "denied" }
        ))
    };
    check(
        "read_file",
        std::fs::read("/etc/hostname").is_ok() || std::fs::read("/proc/self/status").is_ok(),
    );
    check("list_dir", std::fs::read_dir("/").is_ok());
    check(
        "write_file",
        std::fs::write("/tmp/sm-step-worker-self-test", b"x").is_ok(),
    );
    check(
        "exec",
        std::process::Command::new("/bin/true").status().is_ok(),
    );
    check(
        "socket",
        std::net::TcpStream::connect("127.0.0.1:9").is_ok()
            || std::net::UdpSocket::bind("127.0.0.1:0").is_ok(),
    );
    check("thread", std::thread::spawn(|| 1).join().is_ok());
    check("allocate", {
        let v = vec![1u8; 64 << 20];
        v[v.len() - 1] == 1
    });
    let cube = sm_occt::make_box(1.0, 1.0, 1.0)
        .and_then(|b| b.to_brep())
        .is_ok();
    check("occt", cube);
    Ok(report.join("\n").into_bytes())
}
