//! Checks a serialized B-rep in Rust before OpenCascade reads it.
//!
//! The STEP worker sends its result as OpenCascade binary B-rep
//! (`BinTools`, format version 3, no triangulation). The worker parses
//! untrusted files, so the parent treats these bytes as hostile too.
//! OpenCascade's reader trusts them: indices into its tables are not
//! range-checked, and distribution builds compile the geometric
//! constructors' argument checks out. So the whole stream is parsed here
//! first, and OpenCascade only sees bytes that pass:
//!
//! * the exact layout the writer produces, ending exactly at the last byte;
//! * every count fits in the bytes that remain, so nothing allocates more
//!   than the input's size;
//! * every index (location, curve, surface, shape) points at an entry that
//!   exists and, for shapes, was read earlier;
//! * every enum and flag byte has a value the reader handles;
//! * every real is finite and within bounds, directions are unit length
//!   and axes orthonormal, transforms are a rotation times a scale;
//! * curves and surfaces satisfy their constructors' rules (degrees, knot
//!   order, multiplicities, pole counts, weights, trims inside the basis
//!   range, offset bases at least C1), nesting is bounded;
//! * locations are in the canonical form the reader stores one-to-one, so
//!   its table cannot come out shorter than the indices assume;
//! * shapes only contain the sub-shape types `TopoDS_Builder` accepts.
//!
//! Anything else, including valid B-reps the writer would never produce,
//! is rejected.

use std::collections::HashSet;
use std::f64::consts::FRAC_PI_2;

const HEADER: &[u8] = b"\nOpen CASCADE Topology V3 (c)\n";
/// Largest coordinate or length accepted (mm): 1000 km.
const COORD_MAX: f64 = 1e9;
/// Largest curve or surface parameter, knot or weight accepted.
const PARAM_MAX: f64 = 1e15;
/// Largest tolerance accepted (mm).
const TOL_MAX: f64 = 1e3;
/// How far a direction's length may be from 1, and how far apart two
/// axes may be from perpendicular.
const UNIT_TOL: f64 = 1e-6;
/// OpenCascade's `Precision::PConfusion`, used for trims at range ends.
const P_CONFUSION: f64 = 1e-9;
/// `Geom_BSplineCurve::MaxDegree`.
const MAX_DEGREE: u16 = 25;
/// Trimmed and offset curves and surfaces nest their basis inline.
const MAX_NESTING: u32 = 8;
/// Largest power in a composed location.
const MAX_POWER: u32 = 64;

/// What a screened B-rep holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrepSummary {
    pub root: ShapeKind,
    /// Shape records; one record can be placed several times.
    pub shapes: usize,
    /// Face records.
    pub faces: usize,
}

/// `TopAbs_ShapeEnum`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    Compound,
    CompSolid,
    Solid,
    Shell,
    Face,
    Wire,
    Edge,
    Vertex,
}

impl ShapeKind {
    fn from_u8(v: u8) -> Option<ShapeKind> {
        use ShapeKind::*;
        Some(match v {
            0 => Compound,
            1 => CompSolid,
            2 => Solid,
            3 => Shell,
            4 => Face,
            5 => Wire,
            6 => Edge,
            7 => Vertex,
            _ => return None,
        })
    }

    /// `TopoDS_Builder::Add`'s table of which parents take which children.
    fn may_contain(self, child: ShapeKind) -> bool {
        use ShapeKind::*;
        match child {
            Compound | CompSolid => self == Compound,
            Solid => matches!(self, Compound | CompSolid),
            Shell => matches!(self, Compound | Solid),
            Face => matches!(self, Compound | Shell),
            Wire => matches!(self, Compound | Face),
            Edge => matches!(self, Compound | Solid | Wire),
            Vertex => matches!(self, Compound | Solid | Face | Edge),
        }
    }
}

pub type Result<T> = std::result::Result<T, String>;

fn fail<T>(at: usize, what: impl std::fmt::Display) -> Result<T> {
    Err(format!("worker B-rep rejected at byte {at}: {what}"))
}

/// Parameter range of a curve, or of one direction of a surface.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Range {
    Periodic,
    Bounded(f64, f64),
}

impl Range {
    const INFINITE: Range = Range::Bounded(f64::NEG_INFINITY, f64::INFINITY);

    /// The check `Geom_TrimmedCurve` / `Geom_RectangularTrimmedSurface`
    /// make on their trim parameters.
    fn trim(self, a: f64, b: f64) -> std::result::Result<Range, &'static str> {
        if a == b {
            return Err("trim parameters are equal");
        }
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        if let Range::Bounded(first, last) = self {
            if lo < first - P_CONFUSION || hi > last + P_CONFUSION {
                return Err("trim outside the basis range");
            }
        }
        Ok(Range::Bounded(lo, hi))
    }
}

#[derive(Debug, Clone, Copy)]
struct CurveInfo {
    range: Range,
    /// At least C1, as an offset basis must be.
    smooth: bool,
}

#[derive(Debug, Clone, Copy)]
struct SurfaceInfo {
    u: Range,
    v: Range,
    smooth: bool,
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.b.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return fail(self.pos, "unexpected end of data");
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn literal(&mut self, text: &[u8]) -> Result<()> {
        let at = self.pos;
        if self.take(text.len()).ok() != Some(text) {
            return fail(at, format!("expected {:?}", String::from_utf8_lossy(text)));
        }
        Ok(())
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn bool(&mut self) -> Result<bool> {
        let at = self.pos;
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            v => fail(at, format!("flag byte {v}")),
        }
    }

    fn u16(&mut self) -> Result<u16> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }

    fn i32(&mut self) -> Result<i32> {
        let s = self.take(4)?;
        Ok(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn real(&mut self) -> Result<f64> {
        let at = self.pos;
        let s = self.take(8)?;
        let v = f64::from_le_bytes(s.try_into().expect("8 bytes"));
        if !v.is_finite() {
            return fail(at, "non-finite number");
        }
        Ok(v)
    }

    fn bounded(&mut self, lo: f64, hi: f64, what: &str) -> Result<f64> {
        let at = self.pos;
        let v = self.real()?;
        if v < lo || v > hi {
            return fail(at, format!("{what} {v} outside {lo}..={hi}"));
        }
        Ok(v)
    }

    fn coord(&mut self) -> Result<f64> {
        self.bounded(-COORD_MAX, COORD_MAX, "coordinate")
    }

    fn length(&mut self) -> Result<f64> {
        self.bounded(0.0, COORD_MAX, "length")
    }

    fn param(&mut self) -> Result<f64> {
        self.bounded(-PARAM_MAX, PARAM_MAX, "parameter")
    }

    fn tolerance(&mut self) -> Result<f64> {
        self.bounded(0.0, TOL_MAX, "tolerance")
    }

    fn point<const N: usize>(&mut self) -> Result<[f64; N]> {
        let mut p = [0.0; N];
        for c in &mut p {
            *c = self.coord()?;
        }
        Ok(p)
    }

    fn dir<const N: usize>(&mut self) -> Result<[f64; N]> {
        let at = self.pos;
        let mut d = [0.0; N];
        for c in &mut d {
            *c = self.real()?;
        }
        if (dot(&d, &d).sqrt() - 1.0).abs() > UNIT_TOL {
            return fail(at, "direction is not unit length");
        }
        Ok(d)
    }

    /// Two or three mutually perpendicular unit directions.
    fn frame<const N: usize>(&mut self, count: usize) -> Result<()> {
        let at = self.pos;
        let dirs: Vec<[f64; N]> = (0..count).map(|_| self.dir()).collect::<Result<_>>()?;
        for i in 0..count {
            for j in i + 1..count {
                if dot(&dirs[i], &dirs[j]).abs() > UNIT_TOL {
                    return fail(at, "axes are not perpendicular");
                }
            }
        }
        Ok(())
    }

    /// A non-negative index of at most `max`. 0 means "none" where the
    /// format allows it.
    fn index(&mut self, max: usize, what: &str) -> Result<usize> {
        let at = self.pos;
        let v = self.i32()?;
        if v < 0 || v as usize > max {
            return fail(at, format!("{what} index {v} outside 0..={max}"));
        }
        Ok(v as usize)
    }

    fn some_index(&mut self, max: usize, what: &str) -> Result<usize> {
        let at = self.pos;
        match self.index(max, what)? {
            0 => fail(at, format!("{what} index 0")),
            v => Ok(v),
        }
    }

    /// `<name> <count>\n`, where each entry takes at least `min_size`
    /// bytes, so the count cannot promise more than the data holds.
    fn table(&mut self, name: &str, min_size: usize) -> Result<usize> {
        self.literal(name.as_bytes())?;
        self.literal(b" ")?;
        let at = self.pos;
        let mut n: usize = 0;
        let mut digits = 0;
        while let Some(&c) = self.b.get(self.pos) {
            if !c.is_ascii_digit() {
                break;
            }
            if digits > 0 && n == 0 {
                return fail(at, "count has a leading zero");
            }
            n = n * 10 + usize::from(c - b'0');
            digits += 1;
            self.pos += 1;
            if digits > 10 || n > i32::MAX as usize {
                return fail(at, "count too large");
            }
        }
        if digits == 0 {
            return fail(at, "missing count");
        }
        self.literal(b"\n")?;
        if n.saturating_mul(min_size) > self.remaining() {
            return fail(at, format!("{name} count {n} exceeds the data"));
        }
        Ok(n)
    }

    /// `n` entries of `size` bytes each must fit before anything is read.
    fn fits(&self, n: u64, size: u64) -> Result<()> {
        if n.saturating_mul(size) > self.remaining() as u64 {
            return fail(self.pos, "array exceeds the data");
        }
        Ok(())
    }

    fn count(&mut self, min: i32, what: &str) -> Result<usize> {
        let at = self.pos;
        let v = self.i32()?;
        if v < min {
            return fail(at, format!("{what} {v} below {min}"));
        }
        Ok(v as usize)
    }

    fn degree(&mut self) -> Result<u16> {
        let at = self.pos;
        let d = self.u16()?;
        if !(1..=MAX_DEGREE).contains(&d) {
            return fail(at, format!("degree {d}"));
        }
        Ok(d)
    }

    fn weight(&mut self) -> Result<()> {
        let at = self.pos;
        if self.param()? <= 0.0 {
            return fail(at, "weight is not positive");
        }
        Ok(())
    }

    fn continuity(&mut self) -> Result<()> {
        let at = self.pos;
        // GeomAbs_C0 .. GeomAbs_CN
        if self.u8()? > 6 {
            return fail(at, "continuity out of range");
        }
        Ok(())
    }
}

fn dot<const N: usize>(a: &[f64; N], b: &[f64; N]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Knots and multiplicities of one B-spline direction: checks
/// `Geom_BSplineCurve`'s rules and returns the range and whether it is C1.
fn knots(
    r: &mut Reader,
    n: usize,
    degree: u16,
    periodic: bool,
    poles: usize,
) -> Result<(Range, bool)> {
    let at = r.pos;
    r.fits(n as u64, 12)?;
    let mut ks = Vec::with_capacity(n);
    let mut ms = Vec::with_capacity(n);
    for _ in 0..n {
        let k = r.param()?;
        let m = r.i32()?;
        if let Some(&prev) = ks.last() {
            let prev: f64 = prev;
            // Stricter than OpenCascade's "more than one ulp apart".
            if k - prev <= 1e-12 * prev.abs().max(1.0) {
                return fail(at, "knots are not increasing");
            }
        }
        if m < 1 {
            return fail(at, "multiplicity below 1");
        }
        ks.push(k);
        ms.push(m as i64);
    }
    // BSplCLib::NbPoles
    let deg = i64::from(degree);
    let (first, last) = (ms[0], ms[n - 1]);
    if first > deg + 1 || last > deg + 1 {
        return fail(at, "end multiplicity above degree + 1");
    }
    let interior = &ms[1..n - 1];
    if interior.iter().any(|&m| m > deg) {
        return fail(at, "interior multiplicity above degree");
    }
    let expected = if periodic {
        if first > deg || first != last {
            return fail(at, "periodic end multiplicities");
        }
        first + interior.iter().sum::<i64>()
    } else {
        first + last + interior.iter().sum::<i64>() - (deg + 1)
    };
    if expected != poles as i64 {
        return fail(at, "pole count does not match the knots");
    }
    // Periodic splines are C(degree - m) across the seam too.
    let worst = if periodic {
        interior.iter().copied().chain([first]).max().unwrap_or(0)
    } else {
        interior.iter().copied().max().unwrap_or(0)
    };
    let range = if periodic {
        Range::Periodic
    } else {
        Range::Bounded(ks[0], ks[n - 1])
    };
    Ok((range, deg - worst >= 1))
}

struct Screener<'a> {
    r: Reader<'a>,
    curves2d: usize,
    curves: usize,
    surfaces: usize,
    locations: usize,
}

impl<'a> Screener<'a> {
    fn locations(&mut self) -> Result<()> {
        let n = self.r.table("Locations", 1)?;
        // Which entries are elementary (a single transform).
        let mut elementary = Vec::with_capacity(n);
        let mut composed: HashSet<Vec<(usize, i32)>> = HashSet::new();
        for i in 1..=n {
            let at = self.r.pos;
            match self.r.u8()? {
                1 => {
                    self.transform()?;
                    elementary.push(true);
                }
                2 => {
                    // (elementary index, power) pairs ending at index 0.
                    // OpenCascade multiplies them together and stores the
                    // result only if it is new and not the identity, so
                    // the pairs must be in the writer's canonical form:
                    // earlier elementary entries, non-zero powers, no two
                    // neighbours on the same entry (they would merge or
                    // cancel), not a single entry to the power 1 (that is
                    // the entry itself), and no repeat of an earlier list.
                    let mut items: Vec<(usize, i32)> = Vec::new();
                    loop {
                        let idx = self.r.index(i - 1, "location")?;
                        if idx == 0 {
                            break;
                        }
                        if !elementary[idx - 1] {
                            return fail(at, "location refers to a composed location");
                        }
                        let p = self.r.i32()?;
                        if p == 0 || p.unsigned_abs() > MAX_POWER {
                            return fail(at, format!("location power {p}"));
                        }
                        if items.last().is_some_and(|&(prev, _)| prev == idx) {
                            return fail(at, "location repeats its neighbour");
                        }
                        items.push((idx, p));
                    }
                    if items.is_empty() || (items.len() == 1 && items[0].1 == 1) {
                        return fail(at, "location is not in canonical form");
                    }
                    if !composed.insert(items) {
                        return fail(at, "location repeats an earlier one");
                    }
                    elementary.push(false);
                }
                t => return fail(at, format!("location type {t}")),
            }
        }
        self.locations = n;
        Ok(())
    }

    /// A 3x4 matrix that must be a rotation times a non-zero scale plus
    /// a translation, as `gp_Trsf::SetValues` assumes.
    fn transform(&mut self) -> Result<()> {
        let at = self.r.pos;
        let mut m = [[0.0; 3]; 3];
        for row in &mut m {
            for c in row.iter_mut() {
                *c = self.r.param()?;
            }
            self.r.coord()?;
        }
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        let s = det.cbrt();
        if !(1e-6..=1e6).contains(&s.abs()) {
            return fail(at, "transform scale out of range");
        }
        for i in 0..3 {
            for j in 0..3 {
                let rr: f64 = (0..3).map(|k| m[i][k] * m[j][k]).sum::<f64>() / (s * s);
                let want = if i == j { 1.0 } else { 0.0 };
                if (rr - want).abs() > UNIT_TOL {
                    return fail(at, "transform is not a rotation and scale");
                }
            }
        }
        Ok(())
    }

    fn curve2d(&mut self, depth: u32) -> Result<CurveInfo> {
        let at = self.r.pos;
        if depth > MAX_NESTING {
            return fail(at, "curves nested too deeply");
        }
        let r = &mut self.r;
        let smooth_conic = |range| {
            Ok(CurveInfo {
                range,
                smooth: true,
            })
        };
        match r.u8()? {
            1 => {
                r.point::<2>()?;
                r.dir::<2>()?;
                smooth_conic(Range::INFINITE)
            }
            2 => {
                r.point::<2>()?;
                r.frame::<2>(2)?;
                r.length()?;
                smooth_conic(Range::Periodic)
            }
            3 => {
                r.point::<2>()?;
                r.frame::<2>(2)?;
                let (major, minor) = (r.length()?, r.length()?);
                if minor > major {
                    return fail(at, "ellipse minor radius above major");
                }
                smooth_conic(Range::Periodic)
            }
            4 => {
                r.point::<2>()?;
                r.frame::<2>(2)?;
                r.length()?;
                smooth_conic(Range::INFINITE)
            }
            5 => {
                r.point::<2>()?;
                r.frame::<2>(2)?;
                r.length()?;
                r.length()?;
                smooth_conic(Range::INFINITE)
            }
            6 => {
                let rational = r.bool()?;
                let degree = r.degree()?;
                for _ in 0..=degree {
                    r.point::<2>()?;
                    if rational {
                        r.weight()?;
                    }
                }
                smooth_conic(Range::Bounded(0.0, 1.0))
            }
            7 => {
                let rational = r.bool()?;
                let periodic = r.bool()?;
                let degree = r.degree()?;
                let poles = r.count(2, "pole count")?;
                let nknots = r.count(2, "knot count")?;
                r.fits(poles as u64, 16)?;
                for _ in 0..poles {
                    r.point::<2>()?;
                    if rational {
                        r.weight()?;
                    }
                }
                let (range, smooth) = knots(r, nknots, degree, periodic, poles)?;
                Ok(CurveInfo { range, smooth })
            }
            8 => {
                let (a, b) = (r.param()?, r.param()?);
                let basis = self.basis2d(depth, 8)?;
                let range = basis.range.trim(a, b).or_else(|e| fail(at, e))?;
                Ok(CurveInfo {
                    range,
                    smooth: basis.smooth,
                })
            }
            9 => {
                r.coord()?;
                let basis = self.basis2d(depth, 9)?;
                if !basis.smooth {
                    return fail(at, "offset of a C0 curve");
                }
                Ok(basis)
            }
            t => fail(at, format!("2D curve type {t}")),
        }
    }

    /// The basis of a trimmed (8) or offset (9) curve. OpenCascade folds a
    /// basis of the same kind into the outer curve, so the writer never
    /// nests one directly in the other.
    fn basis2d(&mut self, depth: u32, outer: u8) -> Result<CurveInfo> {
        if self.r.b.get(self.r.pos) == Some(&outer) {
            return fail(self.r.pos, "curve nests its own kind");
        }
        self.curve2d(depth + 1)
    }

    fn curve(&mut self, depth: u32) -> Result<CurveInfo> {
        let at = self.r.pos;
        if depth > MAX_NESTING {
            return fail(at, "curves nested too deeply");
        }
        let r = &mut self.r;
        let smooth_conic = |range| {
            Ok(CurveInfo {
                range,
                smooth: true,
            })
        };
        match r.u8()? {
            1 => {
                r.point::<3>()?;
                r.dir::<3>()?;
                smooth_conic(Range::INFINITE)
            }
            2 => {
                r.point::<3>()?;
                r.frame::<3>(3)?;
                r.length()?;
                smooth_conic(Range::Periodic)
            }
            3 => {
                r.point::<3>()?;
                r.frame::<3>(3)?;
                let (major, minor) = (r.length()?, r.length()?);
                if minor > major {
                    return fail(at, "ellipse minor radius above major");
                }
                smooth_conic(Range::Periodic)
            }
            4 => {
                r.point::<3>()?;
                r.frame::<3>(3)?;
                r.length()?;
                smooth_conic(Range::INFINITE)
            }
            5 => {
                r.point::<3>()?;
                r.frame::<3>(3)?;
                r.length()?;
                r.length()?;
                smooth_conic(Range::INFINITE)
            }
            6 => {
                let rational = r.bool()?;
                let degree = r.degree()?;
                for _ in 0..=degree {
                    r.point::<3>()?;
                    if rational {
                        r.weight()?;
                    }
                }
                smooth_conic(Range::Bounded(0.0, 1.0))
            }
            7 => {
                let rational = r.bool()?;
                let periodic = r.bool()?;
                let degree = r.degree()?;
                let poles = r.count(2, "pole count")?;
                let nknots = r.count(2, "knot count")?;
                r.fits(poles as u64, 24)?;
                for _ in 0..poles {
                    r.point::<3>()?;
                    if rational {
                        r.weight()?;
                    }
                }
                let (range, smooth) = knots(r, nknots, degree, periodic, poles)?;
                Ok(CurveInfo { range, smooth })
            }
            8 => {
                let (a, b) = (r.param()?, r.param()?);
                let basis = self.basis(depth, 8)?;
                let range = basis.range.trim(a, b).or_else(|e| fail(at, e))?;
                Ok(CurveInfo {
                    range,
                    smooth: basis.smooth,
                })
            }
            9 => {
                r.coord()?;
                r.dir::<3>()?;
                let basis = self.basis(depth, 9)?;
                if !basis.smooth {
                    return fail(at, "offset of a C0 curve");
                }
                Ok(basis)
            }
            t => fail(at, format!("curve type {t}")),
        }
    }

    fn basis(&mut self, depth: u32, outer: u8) -> Result<CurveInfo> {
        if self.r.b.get(self.r.pos) == Some(&outer) {
            return fail(self.r.pos, "curve nests its own kind");
        }
        self.curve(depth + 1)
    }

    fn surface(&mut self, depth: u32) -> Result<SurfaceInfo> {
        let at = self.r.pos;
        if depth > MAX_NESTING {
            return fail(at, "surfaces nested too deeply");
        }
        let analytic = |u, v| Ok(SurfaceInfo { u, v, smooth: true });
        let t = self.r.u8()?;
        let r = &mut self.r;
        match t {
            1 => {
                r.point::<3>()?;
                r.frame::<3>(3)?;
                analytic(Range::INFINITE, Range::INFINITE)
            }
            2 => {
                r.point::<3>()?;
                r.frame::<3>(3)?;
                r.length()?;
                analytic(Range::Periodic, Range::INFINITE)
            }
            3 => {
                r.point::<3>()?;
                r.frame::<3>(3)?;
                r.length()?;
                let a = r.real()?;
                // gp_Cone: Resolution < |angle| < pi/2 - Resolution.
                if !(1e-9..=FRAC_PI_2 - 1e-9).contains(&a.abs()) {
                    return fail(at, "cone semi-angle out of range");
                }
                analytic(Range::Periodic, Range::INFINITE)
            }
            4 => {
                r.point::<3>()?;
                r.frame::<3>(3)?;
                r.length()?;
                analytic(Range::Periodic, Range::Bounded(-FRAC_PI_2, FRAC_PI_2))
            }
            5 => {
                r.point::<3>()?;
                r.frame::<3>(3)?;
                r.length()?;
                r.length()?;
                analytic(Range::Periodic, Range::Periodic)
            }
            6 => {
                r.dir::<3>()?;
                let c = self.curve(depth + 1)?;
                Ok(SurfaceInfo {
                    u: c.range,
                    v: Range::INFINITE,
                    smooth: c.smooth,
                })
            }
            7 => {
                r.point::<3>()?;
                r.dir::<3>()?;
                let c = self.curve(depth + 1)?;
                Ok(SurfaceInfo {
                    u: Range::Periodic,
                    v: c.range,
                    smooth: c.smooth,
                })
            }
            8 => {
                let rational = r.bool()? | r.bool()?;
                let (ud, vd) = (r.degree()?, r.degree()?);
                for _ in 0..(usize::from(ud) + 1) * (usize::from(vd) + 1) {
                    r.point::<3>()?;
                    if rational {
                        r.weight()?;
                    }
                }
                analytic(Range::Bounded(0.0, 1.0), Range::Bounded(0.0, 1.0))
            }
            9 => {
                let rational = r.bool()? | r.bool()?;
                let (up, vp) = (r.bool()?, r.bool()?);
                let (ud, vd) = (r.degree()?, r.degree()?);
                let nup = r.count(2, "pole count")?;
                let nvp = r.count(2, "pole count")?;
                let nuk = r.count(2, "knot count")?;
                let nvk = r.count(2, "knot count")?;
                r.fits((nup as u64).saturating_mul(nvp as u64), 24)?;
                for _ in 0..nup * nvp {
                    r.point::<3>()?;
                    if rational {
                        r.weight()?;
                    }
                }
                let (u, us) = knots(r, nuk, ud, up, nup)?;
                let (v, vs) = knots(r, nvk, vd, vp, nvp)?;
                Ok(SurfaceInfo {
                    u,
                    v,
                    smooth: us && vs,
                })
            }
            10 => {
                let (u1, u2, v1, v2) = (r.param()?, r.param()?, r.param()?, r.param()?);
                if self.r.b.get(self.r.pos) == Some(&10) {
                    return fail(self.r.pos, "surface nests its own kind");
                }
                let s = self.surface(depth + 1)?;
                let u = s.u.trim(u1, u2).or_else(|e| fail(at, e))?;
                let v = s.v.trim(v1, v2).or_else(|e| fail(at, e))?;
                Ok(SurfaceInfo {
                    u,
                    v,
                    smooth: s.smooth,
                })
            }
            11 => {
                r.coord()?;
                if self.r.b.get(self.r.pos) == Some(&11) {
                    return fail(self.r.pos, "surface nests its own kind");
                }
                let s = self.surface(depth + 1)?;
                if !s.smooth {
                    return fail(at, "offset of a C0 surface");
                }
                Ok(s)
            }
            t => fail(at, format!("surface type {t}")),
        }
    }

    fn geometry(&mut self) -> Result<()> {
        let n = self.r.table("Curve2ds", 1)?;
        for _ in 0..n {
            self.curve2d(0)?;
        }
        self.curves2d = n;
        let n = self.r.table("Curves", 1)?;
        for _ in 0..n {
            self.curve(0)?;
        }
        self.curves = n;
        // Written without triangulation: these tables are empty.
        self.r.literal(b"Polygon3D 0\n")?;
        self.r.literal(b"PolygonOnTriangulations 0\n")?;
        let n = self.r.table("Surfaces", 1)?;
        for _ in 0..n {
            self.surface(0)?;
        }
        self.surfaces = n;
        self.r.literal(b"Triangulations 0\n")?;
        Ok(())
    }

    fn location(&mut self) -> Result<usize> {
        self.r.index(self.locations, "location")
    }

    fn vertex(&mut self) -> Result<()> {
        self.r.tolerance()?;
        self.r.point::<3>()?;
        loop {
            let at = self.r.pos;
            match self.r.u8()? {
                0 => return Ok(()),
                1 => {
                    self.r.param()?;
                    self.r.some_index(self.curves, "curve")?;
                }
                2 => {
                    self.r.param()?;
                    self.r.some_index(self.curves2d, "2D curve")?;
                    self.r.some_index(self.surfaces, "surface")?;
                }
                3 => {
                    self.r.param()?;
                    self.r.param()?;
                    self.r.some_index(self.surfaces, "surface")?;
                }
                t => return fail(at, format!("vertex representation {t}")),
            }
            self.location()?;
        }
    }

    fn edge(&mut self) -> Result<()> {
        self.r.tolerance()?;
        for _ in 0..3 {
            self.r.bool()?;
        }
        loop {
            let at = self.r.pos;
            match self.r.u8()? {
                0 => return Ok(()),
                1 => {
                    self.r.some_index(self.curves, "curve")?;
                    self.location()?;
                    self.r.param()?;
                    self.r.param()?;
                }
                t @ (2 | 3) => {
                    self.r.some_index(self.curves2d, "2D curve")?;
                    if t == 3 {
                        self.r.some_index(self.curves2d, "2D curve")?;
                        self.r.continuity()?;
                    }
                    self.r.some_index(self.surfaces, "surface")?;
                    self.location()?;
                    self.r.param()?;
                    self.r.param()?;
                    for _ in 0..4 {
                        self.r.param()?;
                    }
                }
                4 => {
                    self.r.continuity()?;
                    self.r.some_index(self.surfaces, "surface")?;
                    self.location()?;
                    self.r.some_index(self.surfaces, "surface")?;
                    self.location()?;
                }
                t => return fail(at, format!("edge representation {t}")),
            }
        }
    }

    fn face(&mut self) -> Result<()> {
        self.r.bool()?;
        self.r.tolerance()?;
        self.r.some_index(self.surfaces, "surface")?;
        self.location()?;
        let at = self.r.pos;
        if self.r.u8()? != 0 {
            return fail(at, "face has a triangulation");
        }
        Ok(())
    }

    /// A reference to shape `n - index + 1`, which must already be read
    /// (`< limit`, 1-based).
    fn reference(
        &mut self,
        n: usize,
        limit: usize,
        kinds: &[ShapeKind],
    ) -> Result<Option<ShapeKind>> {
        let at = self.r.pos;
        let o = self.r.u8()?;
        if o == b'*' {
            return Ok(None);
        }
        if o > 3 {
            return fail(at, format!("orientation {o}"));
        }
        let idx = self.r.i32()?;
        let k = (n as i64) - i64::from(idx) + 1;
        if k < 1 || k >= limit as i64 {
            return fail(at, format!("shape reference {idx}"));
        }
        self.location()?;
        Ok(Some(kinds[k as usize - 1]))
    }

    fn shapes(&mut self) -> Result<BrepSummary> {
        self.r.literal(b"\n")?;
        // Smallest shape: type, 7 flags, end marker.
        let n = self.r.table("TShapes", 9)?;
        let mut kinds = Vec::with_capacity(n);
        let mut faces = 0;
        for i in 1..=n {
            let at = self.r.pos;
            let kind = ShapeKind::from_u8(self.r.u8()?)
                .ok_or_else(|| format!("worker B-rep rejected at byte {at}: shape type"))?;
            match kind {
                ShapeKind::Vertex => self.vertex()?,
                ShapeKind::Edge => self.edge()?,
                ShapeKind::Face => {
                    faces += 1;
                    self.face()?
                }
                _ => {}
            }
            for _ in 0..7 {
                self.r.bool()?;
            }
            loop {
                let at = self.r.pos;
                match self.reference(n, i, &kinds)? {
                    None => break,
                    Some(child) if kind.may_contain(child) => {}
                    Some(child) => return fail(at, format!("{kind:?} cannot contain {child:?}")),
                }
            }
            kinds.push(kind);
        }
        let at = self.r.pos;
        let root = match self.reference(n, n + 1, &kinds)? {
            Some(k) => k,
            None => return fail(at, "no root shape"),
        };
        if self.r.remaining() != 0 {
            return fail(self.r.pos, "trailing data");
        }
        Ok(BrepSummary {
            root,
            shapes: n,
            faces,
        })
    }
}

/// Parses a whole serialized B-rep; see the module documentation.
pub fn screen(bytes: &[u8]) -> Result<BrepSummary> {
    let mut s = Screener {
        r: Reader { b: bytes, pos: 0 },
        curves2d: 0,
        curves: 0,
        surfaces: 0,
        locations: 0,
    };
    s.r.literal(HEADER)?;
    s.locations()?;
    s.geometry()?;
    s.shapes()
}

/// Builds B-rep streams in the writer's layout, for tests here and in
/// `crate::tests` (which checks OpenCascade reads them the same way).
#[cfg(test)]
pub(crate) mod build {
    #[derive(Default, Clone)]
    pub struct Brep {
        pub locations: Vec<Vec<u8>>,
        pub curves2d: Vec<Vec<u8>>,
        pub curves: Vec<Vec<u8>>,
        pub surfaces: Vec<Vec<u8>>,
        pub shapes: Vec<Vec<u8>>,
        /// Orientation, shape index (1-based, in `shapes` order), location.
        pub root: (u8, usize, i32),
    }

    pub fn real(v: f64) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }

    pub fn int(v: i32) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }

    pub fn reals(vs: &[f64]) -> Vec<u8> {
        vs.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    pub const X: [f64; 3] = [1.0, 0.0, 0.0];
    pub const Y: [f64; 3] = [0.0, 1.0, 0.0];
    pub const Z: [f64; 3] = [0.0, 0.0, 1.0];

    pub fn identity_location() -> Vec<u8> {
        transform(1.0, [0.0; 3])
    }

    pub fn transform(scale: f64, t: [f64; 3]) -> Vec<u8> {
        let mut v = vec![1];
        for (i, ti) in t.iter().enumerate() {
            let mut row = [0.0; 3];
            row[i] = scale;
            v.extend(reals(&row));
            v.extend(real(*ti));
        }
        v
    }

    /// A composed location: (elementary index, power) pairs.
    pub fn composed(items: &[(i32, i32)]) -> Vec<u8> {
        let mut v = vec![2];
        for &(i, p) in items {
            v.extend(int(i));
            v.extend(int(p));
        }
        v.extend(int(0));
        v
    }

    pub fn line(p: [f64; 3], d: [f64; 3]) -> Vec<u8> {
        [vec![1], reals(&p), reals(&d)].concat()
    }

    pub fn line2d() -> Vec<u8> {
        [vec![1], reals(&[0.0, 0.0, 1.0, 0.0])].concat()
    }

    pub fn circle(r: f64) -> Vec<u8> {
        [
            vec![2],
            reals(&[0.0; 3]),
            reals(&Z),
            reals(&X),
            reals(&Y),
            real(r),
        ]
        .concat()
    }

    /// Non-rational, non-periodic B-spline curve.
    pub fn bspline(degree: u16, poles: usize, knots: &[(f64, i32)]) -> Vec<u8> {
        let mut v = vec![7, 0, 0];
        v.extend(degree.to_le_bytes());
        v.extend(int(poles as i32));
        v.extend(int(knots.len() as i32));
        for i in 0..poles {
            v.extend(reals(&[i as f64, (i % 2) as f64, 0.0]));
        }
        for &(k, m) in knots {
            v.extend(real(k));
            v.extend(int(m));
        }
        v
    }

    pub fn trimmed(a: f64, b: f64, basis: Vec<u8>) -> Vec<u8> {
        [vec![8], real(a), real(b), basis].concat()
    }

    pub fn offset(d: f64, basis: Vec<u8>) -> Vec<u8> {
        [vec![9], real(d), reals(&Z), basis].concat()
    }

    pub fn plane() -> Vec<u8> {
        [vec![1], reals(&[0.0; 3]), reals(&Z), reals(&X), reals(&Y)].concat()
    }

    pub fn sphere(r: f64) -> Vec<u8> {
        [
            vec![4],
            reals(&[0.0; 3]),
            reals(&Z),
            reals(&X),
            reals(&Y),
            real(r),
        ]
        .concat()
    }

    pub fn cone(angle: f64) -> Vec<u8> {
        [
            vec![3],
            reals(&[0.0; 3]),
            reals(&Z),
            reals(&X),
            reals(&Y),
            real(1.0),
            real(angle),
        ]
        .concat()
    }

    pub fn rect_trimmed(b: [f64; 4], basis: Vec<u8>) -> Vec<u8> {
        [vec![10], reals(&b), basis].concat()
    }

    const FLAGS: [u8; 7] = [0, 0, 1, 1, 0, 0, 0];

    /// A shape record: type byte, its data, flags, sub-shape references
    /// (orientation, 1-based index in `shapes`, location) and the end mark.
    /// `total` is the final shape count, which references are relative to.
    pub fn shape(kind: u8, data: Vec<u8>, subs: &[(u8, usize, i32)], total: usize) -> Vec<u8> {
        let mut v = vec![kind];
        v.extend(data);
        v.extend(FLAGS);
        for &(o, k, l) in subs {
            v.push(o);
            v.extend(int((total as i64 - k as i64 + 1) as i32));
            v.extend(int(l));
        }
        v.push(b'*');
        v
    }

    pub fn vertex_data(p: [f64; 3]) -> Vec<u8> {
        [real(1e-7), reals(&p), vec![0]].concat()
    }

    /// An edge on 3D curve `curve` over `[a, b]`.
    pub fn edge_data(curve: i32, a: f64, b: f64) -> Vec<u8> {
        [
            real(1e-7),
            vec![1, 1, 0, 1],
            int(curve),
            int(0),
            real(a),
            real(b),
            vec![0],
        ]
        .concat()
    }

    pub fn face_data(surface: i32) -> Vec<u8> {
        [vec![0], real(1e-7), int(surface), int(0), vec![0]].concat()
    }

    fn table(out: &mut Vec<u8>, name: &str, items: &[Vec<u8>]) {
        out.extend(format!("{name} {}\n", items.len()).bytes());
        for i in items {
            out.extend(i);
        }
    }

    impl Brep {
        pub fn bytes(&self) -> Vec<u8> {
            let mut out = super::HEADER.to_vec();
            table(&mut out, "Locations", &self.locations);
            table(&mut out, "Curve2ds", &self.curves2d);
            table(&mut out, "Curves", &self.curves);
            out.extend(b"Polygon3D 0\nPolygonOnTriangulations 0\n");
            table(&mut out, "Surfaces", &self.surfaces);
            out.extend(b"Triangulations 0\n\n");
            table(&mut out, "TShapes", &self.shapes);
            let (o, k, l) = self.root;
            out.push(o);
            out.extend(int((self.shapes.len() as i64 - k as i64 + 1) as i32));
            out.extend(int(l));
            out
        }
    }

    /// One vertex.
    pub fn vertex() -> Brep {
        Brep {
            shapes: vec![shape(7, vertex_data([1.0, 2.0, 3.0]), &[], 1)],
            root: (0, 1, 0),
            ..Brep::default()
        }
    }

    /// An edge along `curve` (3D curve 1) between two vertices, in a
    /// compound, so geometry tests have a shape that uses the curve.
    pub fn edge_on(curve: Vec<u8>, a: f64, b: f64) -> Brep {
        let n = 4;
        Brep {
            curves: vec![curve],
            shapes: vec![
                shape(7, vertex_data([0.0; 3]), &[], n),
                shape(7, vertex_data([1.0, 0.0, 0.0]), &[], n),
                shape(6, edge_data(1, a, b), &[(0, 1, 0), (1, 2, 0)], n),
                shape(0, vec![], &[(0, 3, 0)], n),
            ],
            root: (0, 4, 0),
            ..Brep::default()
        }
    }

    /// A face on `surface` (surface 1) with no boundary, in a compound.
    pub fn face_on(surface: Vec<u8>) -> Brep {
        Brep {
            surfaces: vec![surface],
            shapes: vec![
                shape(4, face_data(1), &[], 2),
                shape(0, vec![], &[(0, 1, 0)], 2),
            ],
            root: (0, 2, 0),
            ..Brep::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    fn ok(b: &Brep) -> BrepSummary {
        screen(&b.bytes()).unwrap_or_else(|e| panic!("{e}"))
    }

    fn rejected(b: &Brep, why: &str) {
        match screen(&b.bytes()) {
            Ok(s) => panic!("accepted ({why}): {s:?}"),
            Err(e) => assert!(e.contains(why), "{e} (wanted {why:?})"),
        }
    }

    #[test]
    fn minimal_streams_pass() {
        let s = ok(&vertex());
        assert_eq!((s.root, s.shapes, s.faces), (ShapeKind::Vertex, 1, 0));
        let s = ok(&edge_on(line([0.0; 3], X), 0.0, 1.0));
        assert_eq!(s.root, ShapeKind::Compound);
        assert_eq!(ok(&face_on(plane())).faces, 1);
    }

    #[test]
    fn framing_must_be_exact() {
        let good = vertex().bytes();
        for cut in 0..good.len() {
            assert!(screen(&good[..cut]).is_err(), "accepted {cut} bytes");
        }
        let mut extra = good.clone();
        extra.push(0);
        assert!(screen(&extra).unwrap_err().contains("trailing"));
        // Another format version.
        let v4 = [
            b"\nOpen CASCADE Topology V4, (c) Open Cascade\n".as_slice(),
            &good[HEADER.len()..],
        ]
        .concat();
        assert!(screen(&v4).is_err());
        // Counts: leading zero, sign, too large for the data, too many digits.
        let text = String::from_utf8_lossy(&good).into_owned();
        for (from, to) in [
            ("Locations 0\n", "Locations 00\n"),
            ("Locations 0\n", "Locations -0\n"),
            ("Locations 0\n", "Locations 9\n"),
            ("Curves 0\n", "Curves 99999999999\n"),
            ("Polygon3D 0\n", "Polygon3D 1\n"),
            ("Triangulations 0\n", "Triangulations 1\n"),
        ] {
            let bad = good_with(&good, &text, from, to);
            assert!(screen(&bad).is_err(), "accepted {to:?}");
        }
    }

    /// `good` with the first `from` replaced by `to`, byte for byte.
    fn good_with(good: &[u8], text: &str, from: &str, to: &str) -> Vec<u8> {
        let at = text.find(from).unwrap();
        [&good[..at], to.as_bytes(), &good[at + from.len()..]].concat()
    }

    #[test]
    fn shape_references_must_point_backwards_at_allowed_types() {
        // A compound that refers to itself, to a later shape, or past the end.
        for k in [2usize, 3] {
            let mut b = vertex();
            b.shapes = vec![
                shape(0, vec![], &[(0, k, 0)], 2),
                shape(7, vertex_data([0.0; 3]), &[], 2),
            ];
            b.root = (0, 1, 0);
            rejected(&b, "shape reference");
        }
        let mut b = vertex();
        b.root = (0, 2, 0);
        rejected(&b, "shape reference");
        // Orientation beyond INTERNAL/EXTERNAL; location past the table.
        let mut b = vertex();
        b.root = (4, 1, 0);
        rejected(&b, "orientation");
        let mut b = vertex();
        b.root = (0, 1, 1);
        rejected(&b, "location index");
        // A wire cannot hold a face; a shell cannot hold an edge.
        for (parent, child, child_data) in
            [(5u8, 4u8, face_data(1)), (3, 6, edge_data(1, 0.0, 1.0))]
        {
            let mut b = edge_on(line([0.0; 3], X), 0.0, 1.0);
            b.surfaces = vec![plane()];
            b.shapes = vec![
                shape(child, child_data, &[], 2),
                shape(parent, vec![], &[(0, 1, 0)], 2),
            ];
            b.root = (0, 2, 0);
            rejected(&b, "cannot contain");
        }
        // Unknown shape type.
        let mut b = vertex();
        b.shapes[0][0] = 8;
        rejected(&b, "shape type");
    }

    #[test]
    fn locations_must_be_canonical() {
        let with = |locs: Vec<Vec<u8>>| {
            let mut b = vertex();
            let n = locs.len() as i32;
            b.locations = locs;
            b.root = (0, 1, n);
            b
        };
        let t = || transform(1.0, [5.0, 0.0, 0.0]);
        ok(&with(vec![identity_location()]));
        ok(&with(vec![transform(25.4, [1.0, 2.0, 3.0])]));
        ok(&with(vec![t(), t(), composed(&[(1, 2)])]));
        ok(&with(vec![
            t(),
            t(),
            composed(&[(1, 1), (2, -1)]),
            composed(&[(2, 1), (1, 1)]),
        ]));
        rejected(&with(vec![t(), composed(&[(1, 1)])]), "canonical");
        rejected(&with(vec![t(), composed(&[])]), "canonical");
        rejected(&with(vec![t(), composed(&[(1, 0)])]), "power");
        rejected(&with(vec![t(), composed(&[(1, 65)])]), "power");
        // i32::MIN has no positive counterpart (found by fuzzing).
        rejected(&with(vec![t(), composed(&[(1, i32::MIN)])]), "power");
        rejected(&with(vec![t(), composed(&[(1, 1), (1, 1)])]), "neighbour");
        rejected(
            &with(vec![
                t(),
                t(),
                composed(&[(1, 1), (2, 1)]),
                composed(&[(1, 1), (2, 1)]),
            ]),
            "repeats an earlier",
        );
        rejected(
            &with(vec![
                t(),
                t(),
                composed(&[(1, 2)]),
                composed(&[(3, 1), (1, 1)]),
            ]),
            "composed location",
        );
        rejected(&with(vec![t(), composed(&[(2, 2)])]), "location index");
        rejected(&with(vec![t(), composed(&[(-1, 2)])]), "location index");
        rejected(&with(vec![vec![3]]), "location type");
        // Shear, zero scale, non-finite translation.
        let mut shear = identity_location();
        shear[1 + 8..1 + 16].copy_from_slice(&0.5f64.to_le_bytes());
        rejected(&with(vec![shear]), "rotation");
        rejected(&with(vec![transform(0.0, [0.0; 3])]), "scale");
        rejected(
            &with(vec![transform(1.0, [f64::NAN, 0.0, 0.0])]),
            "non-finite",
        );
        rejected(&with(vec![transform(1.0, [1e10, 0.0, 0.0])]), "coordinate");
    }

    #[test]
    fn curves_must_satisfy_their_constructors() {
        let cubic = || bspline(3, 4, &[(0.0, 4), (1.0, 4)]);
        ok(&edge_on(cubic(), 0.0, 1.0));
        ok(&edge_on(
            bspline(2, 5, &[(0.0, 3), (1.0, 1), (2.0, 1), (3.0, 3)]),
            0.0,
            3.0,
        ));
        ok(&edge_on(circle(2.0), 0.0, 1.0));
        ok(&edge_on(trimmed(0.25, 0.75, cubic()), 0.25, 0.75));
        ok(&edge_on(trimmed(-5.0, 5.0, line([0.0; 3], X)), -5.0, 5.0));
        ok(&edge_on(offset(1.0, circle(2.0)), 0.0, 1.0));
        ok(&edge_on(
            trimmed(0.0, 1.0, offset(1.0, circle(2.0))),
            0.0,
            1.0,
        ));

        rejected(
            &edge_on(bspline(0, 2, &[(0.0, 1), (1.0, 1)]), 0.0, 1.0),
            "degree",
        );
        rejected(
            &edge_on(bspline(26, 27, &[(0.0, 27), (1.0, 27)]), 0.0, 1.0),
            "degree",
        );
        rejected(
            &edge_on(bspline(3, 4, &[(1.0, 4), (0.0, 4)]), 0.0, 1.0),
            "increasing",
        );
        rejected(
            &edge_on(bspline(3, 4, &[(0.0, 4), (0.0, 4)]), 0.0, 1.0),
            "increasing",
        );
        rejected(
            &edge_on(bspline(3, 5, &[(0.0, 4), (1.0, 4)]), 0.0, 1.0),
            "pole count",
        );
        rejected(
            &edge_on(bspline(3, 4, &[(0.0, 5), (1.0, 3)]), 0.0, 1.0),
            "end multiplicity",
        );
        rejected(
            &edge_on(bspline(2, 4, &[(0.0, 3), (1.0, 3), (2.0, 3)]), 0.0, 1.0),
            "interior multiplicity",
        );
        rejected(
            &edge_on(bspline(3, 4, &[(0.0, 0), (1.0, 4)]), 0.0, 1.0),
            "multiplicity below",
        );
        // Counts that promise more data than there is never allocate.
        let mut huge = cubic();
        huge[5..9].copy_from_slice(&i32::MAX.to_le_bytes());
        rejected(&edge_on(huge, 0.0, 1.0), "exceeds the data");
        let mut negative = cubic();
        negative[9..13].copy_from_slice(&(-1i32).to_le_bytes());
        rejected(&edge_on(negative, 0.0, 1.0), "knot count");
        // Rational with a zero weight.
        let mut rational = vec![6, 1];
        rational.extend(1u16.to_le_bytes());
        rational.extend(reals(&[0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0]));
        rejected(&edge_on(rational, 0.0, 1.0), "weight");

        rejected(&edge_on(trimmed(0.5, 0.5, cubic()), 0.5, 0.5), "equal");
        rejected(
            &edge_on(trimmed(-0.5, 0.5, cubic()), 0.0, 0.5),
            "outside the basis",
        );
        rejected(
            &edge_on(trimmed(0.0, 0.5, trimmed(0.0, 1.0, cubic())), 0.0, 0.5),
            "its own kind",
        );
        rejected(
            &edge_on(offset(1.0, offset(1.0, circle(1.0))), 0.0, 1.0),
            "its own kind",
        );
        // C0 basis: a degree-1 spline with an interior knot.
        let c0 = bspline(1, 3, &[(0.0, 2), (1.0, 1), (2.0, 2)]);
        rejected(&edge_on(offset(1.0, c0), 0.0, 1.0), "C0");
        // Deep nesting, alternating trims and offsets.
        let mut deep = circle(1.0);
        for i in 0..10 {
            deep = if i % 2 == 0 {
                offset(1.0, deep)
            } else {
                trimmed(0.0, 1.0, deep)
            };
        }
        rejected(&edge_on(deep, 0.0, 1.0), "nested too deeply");

        rejected(
            &edge_on(line([0.0; 3], [1.0, 1.0, 0.0]), 0.0, 1.0),
            "unit length",
        );
        rejected(
            &edge_on(line([f64::INFINITY, 0.0, 0.0], X), 0.0, 1.0),
            "non-finite",
        );
        let mut skew = circle(1.0);
        skew[1 + 48..1 + 72].copy_from_slice(&reals(&Z));
        rejected(&edge_on(skew, 0.0, 1.0), "perpendicular");
        rejected(&edge_on(circle(-1.0), 0.0, 1.0), "length");
        rejected(&edge_on(vec![10], 0.0, 1.0), "curve type");
        // An ellipse whose minor radius exceeds the major.
        let ellipse = [
            vec![3],
            reals(&[0.0; 3]),
            reals(&Z),
            reals(&X),
            reals(&Y),
            real(1.0),
            real(2.0),
        ]
        .concat();
        rejected(&edge_on(ellipse, 0.0, 1.0), "minor radius");
    }

    #[test]
    fn surfaces_must_satisfy_their_constructors() {
        ok(&face_on(sphere(2.0)));
        ok(&face_on(rect_trimmed([0.0, 1.0, -1.0, 1.0], sphere(2.0))));
        ok(&face_on(cone(0.3)));
        rejected(
            &face_on(rect_trimmed([0.0, 1.0, -2.0, 1.0], sphere(2.0))),
            "outside the basis",
        );
        rejected(
            &face_on(rect_trimmed(
                [0.0, 1.0, 0.0, 1.0],
                rect_trimmed([0.0, 1.0, 0.0, 1.0], plane()),
            )),
            "its own kind",
        );
        rejected(&face_on(cone(0.0)), "semi-angle");
        rejected(&face_on(cone(FRAC_PI_2)), "semi-angle");
        rejected(&face_on(vec![12]), "surface type");
        // B-spline surface whose pole grid overflows or exceeds the data.
        let mut s = vec![9, 0, 0, 0, 0];
        s.extend(1u16.to_le_bytes());
        s.extend(1u16.to_le_bytes());
        for n in [i32::MAX, i32::MAX, 2, 2] {
            s.extend(int(n));
        }
        rejected(&face_on(s), "exceeds the data");
        // Faces must have a surface, and no triangulation.
        let mut b = face_on(plane());
        b.shapes[0] = shape(
            4,
            [vec![0], real(1e-7), int(0), int(0), vec![0]].concat(),
            &[],
            2,
        );
        rejected(&b, "surface index 0");
        let mut b = face_on(plane());
        b.shapes[0] = shape(
            4,
            [vec![0], real(1e-7), int(1), int(0), vec![2]].concat(),
            &[],
            2,
        );
        rejected(&b, "triangulation");
        let mut b = face_on(plane());
        b.shapes[0] = shape(
            4,
            [vec![0], real(-1.0), int(1), int(0), vec![0]].concat(),
            &[],
            2,
        );
        rejected(&b, "tolerance");
    }

    #[test]
    fn edge_and_vertex_representations_are_checked() {
        let base = || edge_on(line([0.0; 3], X), 0.0, 1.0);
        let with_edge = |data: Vec<u8>| {
            let mut b = base();
            b.curves2d = vec![line2d()];
            b.surfaces = vec![plane()];
            b.shapes[2] = shape(6, data, &[(0, 1, 0), (1, 2, 0)], 4);
            b
        };
        let head = || [real(1e-7), vec![1, 1, 0]].concat();
        let on_surface = |closed: bool, cont: u8| {
            let mut d = head();
            d.push(if closed { 3 } else { 2 });
            d.extend(int(1));
            if closed {
                d.extend(int(1));
                d.push(cont);
            }
            d.extend([int(1), int(0)].concat());
            d.extend(reals(&[0.0, 1.0, 0.0, 0.0, 1.0, 0.0]));
            d.push(0);
            d
        };
        ok(&with_edge(on_surface(false, 0)));
        ok(&with_edge(on_surface(true, 6)));
        rejected(&with_edge(on_surface(true, 7)), "continuity");
        // Polygon and triangulation representations.
        rejected(
            &with_edge([head(), vec![5], int(1), int(0), vec![0]].concat()),
            "edge representation",
        );
        // Index past the curve table, and index 0.
        rejected(
            &with_edge([head(), vec![1], int(2), int(0), reals(&[0.0, 1.0]), vec![0]].concat()),
            "curve index",
        );
        rejected(
            &with_edge([head(), vec![1], int(0), int(0), reals(&[0.0, 1.0]), vec![0]].concat()),
            "curve index 0",
        );
        // Flag bytes other than 0 and 1.
        rejected(&with_edge([real(1e-7), vec![2, 1, 0, 0]].concat()), "flag");
        // Vertex point representations.
        let with_vertex = |reps: Vec<u8>| {
            let mut b = base();
            b.surfaces = vec![plane()];
            b.shapes[0] = shape(
                7,
                [real(1e-7), reals(&[0.0; 3]), reps, vec![0]].concat(),
                &[],
                4,
            );
            b
        };
        ok(&with_vertex([vec![1], real(0.0), int(1), int(0)].concat()));
        ok(&with_vertex(
            [vec![3], real(0.0), real(0.0), int(1), int(0)].concat(),
        ));
        rejected(
            &with_vertex([vec![2], real(0.0), int(1), int(1), int(0)].concat()),
            "2D curve index",
        );
        rejected(&with_vertex([vec![4]].concat()), "vertex representation");
    }

    #[test]
    fn arbitrary_bytes_never_panic() {
        // Prefixes and single-byte changes of every builder stream.
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let streams = [
            vertex().bytes(),
            edge_on(
                bspline(2, 5, &[(0.0, 3), (1.0, 1), (2.0, 1), (3.0, 3)]),
                0.0,
                3.0,
            )
            .bytes(),
            edge_on(trimmed(0.0, 1.0, offset(1.0, circle(2.0))), 0.0, 1.0).bytes(),
            face_on(rect_trimmed([0.0, 1.0, -1.0, 1.0], sphere(2.0))).bytes(),
        ];
        for s in &streams {
            for cut in 0..s.len() {
                let _ = screen(&s[..cut]);
            }
            for _ in 0..20_000 {
                let mut m = s.clone();
                let i = next() as usize % m.len();
                m[i] = next() as u8;
                let _ = screen(&m);
            }
        }
        for len in 0..2000 {
            let junk: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            assert!(screen(&junk).is_err());
        }
    }
}
