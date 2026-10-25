//! STEP import (two stages, sandboxed) and export.
//!
//! Import never lets OpenCascade parse untrusted bytes in this process:
//!
//! 1. [`sm_step::screen`] parses the file in Rust with hard limits and
//!    rejects anything malformed.
//! 2. The `sm-step-worker` executable reads it with OpenCascade inside the
//!    sandbox from [`crate::sandbox`], with a wall-clock timeout, and sends
//!    back each solid as OpenCascade binary B-rep in a small framed format.
//! 3. This process checks the framing in Rust, then loads each B-rep and
//!    accepts it only if it is a valid solid of sane size.
//!
//! Export runs in-process: it only writes, from shapes we built.

use super::sandbox::WorkerLimits;
use super::{OcctError, Result, Shape, ShapeType};
use crate::ffi::ffi as sys;
use sm_step::screen::{self, Screened};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// OpenCascade's STEP translator keeps its settings in process-wide
/// statics, so translations must not overlap.
static STEP_LOCK: Mutex<()> = Mutex::new(());

const MAGIC: &[u8; 4] = b"SMSW";
const VERSION: u8 = 1;
/// No body name is longer than this; longer means a corrupt stream.
const MAX_NAME: usize = 4096;
/// Coordinates beyond this many millimetres (10 km) are not a part.
const MAX_COORD: f64 = 1e7;

/// A solid read from a STEP file, placed in world coordinates, in
/// millimetres.
#[derive(Debug, Clone)]
pub struct ImportedBody {
    pub name: String,
    /// sRGB, 0..1.
    pub color: Option<[f64; 3]>,
    pub shape: Shape,
}

#[derive(Debug, Clone)]
pub struct StepImport {
    pub bodies: Vec<ImportedBody>,
    /// What screening found (schema, units, products).
    pub screened: Screened,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Path to `sm-step-worker`. `None` looks next to the running
    /// executable (and its parent, for `target/debug/deps` test binaries).
    pub worker: Option<PathBuf>,
    pub limits: WorkerLimits,
    pub timeout: Duration,
    pub max_bodies: u32,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions {
            worker: None,
            limits: WorkerLimits::default(),
            timeout: Duration::from_secs(300),
            max_bodies: 100_000,
        }
    }
}

fn step_err(msg: impl Into<String>) -> OcctError {
    OcctError::Step(msg.into())
}

fn find_worker(opts: &ImportOptions) -> Result<PathBuf> {
    if let Some(p) = &opts.worker {
        return Ok(p.clone());
    }
    let exe =
        std::env::current_exe().map_err(|e| step_err(format!("cannot locate executable: {e}")))?;
    let name = format!("sm-step-worker{}", std::env::consts::EXE_SUFFIX);
    let mut dir = exe.parent();
    for _ in 0..2 {
        let Some(d) = dir else { break };
        let candidate = d.join(&name);
        if candidate.is_file() {
            return Ok(candidate);
        }
        dir = d.parent();
    }
    Err(step_err(format!(
        "{name} not found next to {}",
        exe.display()
    )))
}

/// Reads every solid from a STEP file. See the module docs for the stages.
pub fn import_step(bytes: &[u8], opts: &ImportOptions) -> Result<StepImport> {
    let screened = screen::screen(bytes).map_err(|e| step_err(e.to_string()))?;
    let output = run_worker(bytes, opts)?;
    let raw = decode(&output, opts.max_bodies)?;
    let mut bodies = Vec::with_capacity(raw.len());
    let mut warnings = screened.warnings.clone();
    for (i, body) in raw.into_iter().enumerate() {
        match accept(&body.brep) {
            Ok(shape) => bodies.push(ImportedBody {
                name: body.name,
                color: body.color,
                shape,
            }),
            Err(e) => warnings.push(format!("body {} ({}): {e}; skipped", i + 1, body.name)),
        }
    }
    if bodies.is_empty() {
        return Err(step_err(match warnings.last() {
            Some(w) => format!("no usable solids ({w})"),
            None => "no usable solids".into(),
        }));
    }
    Ok(StepImport {
        bodies,
        screened,
        warnings,
    })
}

/// Loads one B-rep from the worker and checks it before anyone uses it.
fn accept(brep: &[u8]) -> Result<Shape> {
    let shape = Shape::from_brep(brep)?;
    if shape.shape_type()? != ShapeType::Solid {
        return Err(step_err("not a solid"));
    }
    if !shape.is_valid() {
        return Err(step_err("OpenCascade reports the solid as invalid"));
    }
    let b = shape.bounding_box()?;
    if !b
        .min
        .iter()
        .chain(&b.max)
        .all(|v| v.is_finite() && v.abs() <= MAX_COORD)
    {
        return Err(step_err("solid lies outside the 10 km modelling range"));
    }
    Ok(shape)
}

fn run_worker(bytes: &[u8], opts: &ImportOptions) -> Result<Vec<u8>> {
    let worker = find_worker(opts)?;
    let mut child = Command::new(&worker)
        .arg(opts.limits.memory_bytes.to_string())
        .arg(opts.limits.cpu_seconds.to_string())
        .arg(opts.max_bodies.to_string())
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| step_err(format!("cannot start {}: {e}", worker.display())))?;

    let max_output = (bytes.len() as u64)
        .saturating_mul(16)
        .saturating_add(64 << 20);
    let mut stdin = child.stdin.take().expect("piped");
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let input = bytes.to_vec();
    let writer = std::thread::spawn(move || {
        // A worker that dies early closes the pipe; that's reported below.
        let _ = stdin.write_all(&input);
    });
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let r = (&mut stdout).take(max_output + 1).read_to_end(&mut out);
        r.map(|_| out)
    });
    let err_reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = (&mut stderr).take(64 << 10).read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    });

    let deadline = Instant::now() + opts.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(step_err(format!(
                    "reading took longer than {} s and was stopped",
                    opts.timeout.as_secs()
                )));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => return Err(step_err(format!("waiting for the worker: {e}"))),
        }
    };
    let _ = writer.join();
    let output = reader
        .join()
        .map_err(|_| step_err("output reader panicked"))?
        .map_err(|e| step_err(format!("reading worker output: {e}")))?;
    let log = err_reader.join().unwrap_or_default();
    if output.len() as u64 > max_output {
        return Err(step_err("worker output too large"));
    }
    if !status.success() {
        let detail = decode_error(&output).unwrap_or_else(|| log.trim().to_string());
        return Err(step_err(if detail.is_empty() {
            format!("worker failed ({status})")
        } else {
            detail
        }));
    }
    Ok(output)
}

pub(crate) struct RawBody {
    pub name: String,
    pub color: Option<[f64; 3]>,
    pub brep: Vec<u8>,
}

struct Cursor<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.buf.len());
        let Some(end) = end else {
            return Err(step_err("worker output is truncated"));
        };
        let s = &self.buf[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }
    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }
    fn string(&mut self, max: usize) -> Result<String> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(step_err("worker output has an oversized string"));
        }
        String::from_utf8(self.take(n)?.to_vec())
            .map_err(|_| step_err("worker output is not UTF-8"))
    }
}

/// Worker output: `SMSW`, version, status (0 = bodies, 1 = error), then
/// either a count and per body (name, colour flag, rgb, B-rep length and
/// bytes), or an error message. Little-endian throughout.
pub(crate) fn encode(bodies: &[RawBody]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(0);
    out.extend_from_slice(&(bodies.len() as u32).to_le_bytes());
    for b in bodies {
        let name: &str = if b.name.len() > MAX_NAME {
            let mut end = MAX_NAME;
            while !b.name.is_char_boundary(end) {
                end -= 1;
            }
            &b.name[..end]
        } else {
            &b.name
        };
        out.extend_from_slice(&(name.len() as u32).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.push(u8::from(b.color.is_some()));
        for c in b.color.unwrap_or_default() {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out.extend_from_slice(&(b.brep.len() as u64).to_le_bytes());
        out.extend_from_slice(&b.brep);
    }
    out
}

pub(crate) fn encode_error(message: &str) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(1);
    let msg = &message.as_bytes()[..message.len().min(MAX_NAME)];
    out.extend_from_slice(&(msg.len() as u32).to_le_bytes());
    out.extend_from_slice(msg);
    out
}

fn header(c: &mut Cursor) -> Result<u8> {
    if c.take(4)? != MAGIC || c.u8()? != VERSION {
        return Err(step_err("worker output has a bad header"));
    }
    c.u8()
}

fn decode_error(buf: &[u8]) -> Option<String> {
    let mut c = Cursor { buf, at: 0 };
    if header(&mut c).ok()? != 1 {
        return None;
    }
    // Lossy: the message is only shown to the user.
    let n = c.u32().ok()? as usize;
    let bytes = c.take(n.min(MAX_NAME)).ok()?;
    Some(String::from_utf8_lossy(bytes).into_owned())
}

pub(crate) fn decode(buf: &[u8], max_bodies: u32) -> Result<Vec<RawBody>> {
    let mut c = Cursor { buf, at: 0 };
    match header(&mut c)? {
        0 => {}
        1 => return Err(step_err(decode_error(buf).unwrap_or_default())),
        _ => return Err(step_err("worker output has a bad status")),
    }
    let n = c.u32()?;
    if n > max_bodies {
        return Err(step_err("worker reported too many bodies"));
    }
    let mut bodies = Vec::new();
    for _ in 0..n {
        let name = c.string(MAX_NAME)?;
        let has_color = match c.u8()? {
            0 => false,
            1 => true,
            _ => return Err(step_err("worker output has a bad colour flag")),
        };
        let rgb = [c.f64()?, c.f64()?, c.f64()?];
        if has_color && !rgb.iter().all(|v| (0.0..=1.0).contains(v)) {
            return Err(step_err("worker output has a colour out of range"));
        }
        let len = usize::try_from(c.u64()?).map_err(|_| step_err("B-rep too large"))?;
        let brep = c.take(len)?.to_vec();
        bodies.push(RawBody {
            name,
            color: has_color.then_some(rgb),
            brep,
        });
    }
    if c.at != buf.len() {
        return Err(step_err("worker output has trailing bytes"));
    }
    Ok(bodies)
}

/// Runs inside the sandboxed worker: OpenCascade reads the file.
pub(crate) fn read_in_worker(bytes: &[u8], max_bodies: u32) -> Result<Vec<RawBody>> {
    let _guard = STEP_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let bodies = sys::read_step(bytes, max_bodies).map_err(|e| step_err(e.what()))?;
    Ok(bodies
        .into_iter()
        .map(|b| RawBody {
            name: b.name,
            color: b.has_color.then_some([b.r, b.g, b.b]),
            brep: b.brep,
        })
        .collect())
}

/// The worker's whole job once sandboxed: read and frame the bodies.
#[doc(hidden)]
pub fn worker_read(bytes: &[u8], max_bodies: u32) -> Result<Vec<u8>> {
    Ok(encode(&read_in_worker(bytes, max_bodies)?))
}

#[doc(hidden)]
pub fn worker_error(message: &str) -> Vec<u8> {
    encode_error(message)
}

/// Called once by the worker before the sandbox closes, so OpenCascade
/// loads its resource files while it still may.
#[doc(hidden)]
pub fn worker_warm_up() -> Result<()> {
    const BOX: &[u8] = include_bytes!("../../sm-step/tests/corpus/occt/box.step");
    read_in_worker(BOX, 10).map(|_| ())
}

/// A body to export.
pub struct ExportBody<'a> {
    pub name: &'a str,
    pub color: Option<[f64; 3]>,
    pub shape: &'a Shape,
}

/// Writes the bodies to an AP242 STEP file in millimetres, with names and
/// colours.
pub fn export_step(bodies: &[ExportBody], path: &Path) -> Result<()> {
    let path = path
        .to_str()
        .ok_or_else(|| OcctError::Input("STEP path is not valid UTF-8".into()))?;
    let _guard = STEP_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut writer = sys::step_writer_new().map_err(super::kernel)?;
    for b in bodies {
        let [r, g, bl] = b.color.unwrap_or_default();
        if b.color.is_some() && ![r, g, bl].iter().all(|v| (0.0..=1.0).contains(v)) {
            return Err(OcctError::Input(
                "colour components must be within 0..1".into(),
            ));
        }
        sys::step_writer_add(
            writer.pin_mut(),
            b.shape.inner(),
            b.name,
            b.color.is_some(),
            r,
            g,
            bl,
        )
        .map_err(super::kernel)?;
    }
    sys::step_writer_write(writer.pin_mut(), path).map_err(super::kernel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(name: &str, color: Option<[f64; 3]>, brep: &[u8]) -> RawBody {
        RawBody {
            name: name.into(),
            color,
            brep: brep.to_vec(),
        }
    }

    #[test]
    fn framing_round_trips() {
        let bodies = vec![
            body("bracket", Some([1.0, 0.5, 0.0]), b"abc"),
            body("ø pin", None, b""),
        ];
        let out = decode(&encode(&bodies), 10).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "bracket");
        assert_eq!(out[0].color, Some([1.0, 0.5, 0.0]));
        assert_eq!(out[0].brep, b"abc");
        assert_eq!(out[1].name, "ø pin");
        assert_eq!(out[1].color, None);
        let err = decode(&encode_error("boom"), 10).err().unwrap();
        assert_eq!(err, OcctError::Step("boom".into()));
        // Long names are cut at a character boundary.
        let long = "é".repeat(MAX_NAME);
        let out = decode(&encode(&[body(&long, None, b"x")]), 10).unwrap();
        assert!(out[0].name.len() <= MAX_NAME);
    }

    #[test]
    fn malformed_worker_output_is_rejected() {
        let good = encode(&[body("a", Some([0.1, 0.2, 0.3]), b"brep")]);
        // Every truncation fails cleanly.
        for cut in 0..good.len() {
            assert!(decode(&good[..cut], 10).is_err(), "accepted {cut} bytes");
        }
        // Trailing junk.
        let mut extra = good.clone();
        extra.push(0);
        assert!(decode(&extra, 10).is_err());
        // Body count over the cap.
        assert!(decode(&good, 0).is_err());
        // Bad magic, version, status, colour flag, colour range, length.
        let mut bad = good.clone();
        bad[0] = b'X';
        assert!(decode(&bad, 10).is_err());
        let mut bad = good.clone();
        bad[4] = 9;
        assert!(decode(&bad, 10).is_err());
        let mut bad = good.clone();
        bad[5] = 7;
        assert!(decode(&bad, 10).is_err());
        let flag = 4 + 1 + 1 + 4 + 4 + 1;
        let mut bad = good.clone();
        bad[flag] = 2;
        assert!(decode(&bad, 10).is_err());
        let mut bad = good.clone();
        bad[flag + 1..flag + 9].copy_from_slice(&2.0f64.to_le_bytes());
        assert!(decode(&bad, 10).is_err());
        let mut bad = good.clone();
        let len_at = flag + 1 + 24;
        bad[len_at..len_at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode(&bad, 10).is_err());
        // A huge name length.
        let mut bad = good;
        bad[10..14].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&bad, 10).is_err());
    }
}
