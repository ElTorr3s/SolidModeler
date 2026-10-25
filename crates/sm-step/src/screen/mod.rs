//! Screening of untrusted STEP files before any C++ code sees them.
//!
//! OpenCascade's STEP reader is large C++ that was never hardened against
//! hostile input, so import runs in two stages: this pure-Rust pass parses
//! the whole exchange structure with hard limits on size, entity count and
//! nesting, rejects anything malformed (bad syntax, dangling or duplicate
//! instance references, no B-rep solids), and summarises the file. Only a
//! file that passes is handed to OpenCascade, and even then in a sandboxed
//! worker process (see the `sm-occt` crate).

pub mod part21;
mod units;

pub use units::Units;

use part21::{Entity, Exchange, Param, Record};
use std::collections::BTreeSet;
use std::fmt;
use thiserror::Error;

/// Caps that keep a hostile or corrupt file from exhausting memory or time.
/// Real files are far below these; raise them for unusually large models.
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_entities: usize,
    /// Deepest list nesting in a parameter.
    pub max_nesting: usize,
    pub max_complex_parts: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_input_bytes: 512 << 20,
            max_entities: 10_000_000,
            max_nesting: 64,
            max_complex_parts: 64,
        }
    }
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("not a STEP file (missing ISO-10303-21 header)")]
    NotStep,
    #[error("line {line}: {message}")]
    Syntax { line: usize, message: String },
    #[error("file too large: {0}")]
    TooLarge(String),
    #[error("#{from} refers to #{to}, which is not defined")]
    DanglingReference { from: u64, to: u64 },
    #[error("the file contains no B-rep solids")]
    NoSolids,
}

/// What screening found out about a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Screened {
    /// The first schema named in the header, upper-cased.
    pub schema: String,
    /// Length unit of the first shape representation, e.g. "millimetre".
    pub length_unit: String,
    pub entities: usize,
    /// B-rep solid items (`MANIFOLD_SOLID_BREP`, `BREP_WITH_VOIDS`,
    /// `FACETED_BREP`) before assembly instancing.
    pub solids: usize,
    /// Product names, in file order, without duplicates.
    pub products: Vec<String>,
    pub warnings: Vec<String>,
}

pub const SOLID_ITEMS: [&str; 3] = ["MANIFOLD_SOLID_BREP", "BREP_WITH_VOIDS", "FACETED_BREP"];

/// Screens a STEP file with the default [`Limits`].
pub fn screen(text: &[u8]) -> Result<Screened, ImportError> {
    screen_with_limits(text, &Limits::default())
}

pub fn screen_with_limits(text: &[u8], limits: &Limits) -> Result<Screened, ImportError> {
    let exchange = part21::parse(text, limits)?;
    let mut warnings = Vec::new();
    let schema = exchange.schemas().into_iter().next().unwrap_or_default();
    let known = [
        "AP242",
        "AUTOMOTIVE_DESIGN",
        "CONFIG_CONTROL_DESIGN",
        "AP203",
        "AP214",
    ];
    if !known.iter().any(|k| schema.contains(k)) {
        warnings.push(format!("unrecognised schema {schema:?}"));
    }

    let mut ids: Vec<u64> = exchange.entities.keys().copied().collect();
    ids.sort_unstable();
    let mut solids = 0;
    let mut length_unit = None;
    let mut products = Vec::new();
    let mut seen_products = BTreeSet::new();
    let graph = Graph { ex: &exchange };
    for &id in &ids {
        let entity = &exchange.entities[&id];
        for rec in entity.records() {
            check_refs(&exchange, id, &rec.params)?;
        }
        if SOLID_ITEMS.iter().any(|n| entity.has(n)) {
            solids += 1;
        }
        if let Some(rec) = entity.record("PRODUCT") {
            let name = Args::new(id, rec).string(1);
            if !name.is_empty() && seen_products.insert(name.clone()) {
                products.push(name);
            }
        }
        if length_unit.is_none() && entity.has("GLOBAL_UNIT_ASSIGNED_CONTEXT") {
            match units::context_units(&graph, id) {
                Ok(u) => length_unit = Some(u.length_name),
                Err(e) => warnings.push(format!("units: {e}")),
            }
        }
    }
    if solids == 0 {
        return Err(ImportError::NoSolids);
    }
    Ok(Screened {
        schema,
        length_unit: length_unit.unwrap_or_else(|| {
            warnings.push("no length unit declared; OpenCascade assumes millimetres".into());
            "millimetre".into()
        }),
        entities: exchange.entities.len(),
        solids,
        products,
        warnings,
    })
}

/// Every `#n` used as a parameter must be defined. Iterative, so deeply
/// nested lists can't overflow the stack.
fn check_refs(ex: &Exchange, from: u64, params: &[Param]) -> Result<(), ImportError> {
    let mut stack: Vec<&[Param]> = vec![params];
    while let Some(list) = stack.pop() {
        for p in list {
            match p {
                Param::Ref(to) if !ex.entities.contains_key(to) => {
                    return Err(ImportError::DanglingReference { from, to: *to })
                }
                Param::List(items) => stack.push(items),
                Param::Typed(_, inner) => stack.push(std::slice::from_ref(inner)),
                _ => {}
            }
        }
    }
    Ok(())
}

/// Why one item couldn't be converted. Carries the entity it is about, so
/// warnings point at the offending line of the file.
#[derive(Debug, Clone, PartialEq)]
pub struct ConvertError {
    pub entity: u64,
    pub message: String,
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}: {}", self.entity, self.message)
    }
}

pub type CResult<T> = Result<T, ConvertError>;

pub fn err<T>(entity: u64, message: impl Into<String>) -> CResult<T> {
    Err(ConvertError {
        entity,
        message: message.into(),
    })
}

/// Read access to the instance table with typed parameter accessors.
pub struct Graph<'a> {
    pub ex: &'a Exchange,
}

impl<'a> Graph<'a> {
    pub fn get(&self, id: u64) -> CResult<&'a Entity> {
        match self.ex.entities.get(&id) {
            Some(e) => Ok(e),
            None => err(id, "referenced but not defined"),
        }
    }

    /// The record `name` of entity `id`, which must be of that type (as a
    /// simple instance or a part of a complex one).
    pub fn record(&self, id: u64, name: &str) -> CResult<&'a Record> {
        match self.get(id)?.record(name) {
            Some(r) => Ok(r),
            None => err(
                id,
                format!("expected {name}, found {}", type_name(self.get(id)?)),
            ),
        }
    }

    /// The first of `names` that entity `id` is, with its record.
    pub fn record_any(
        &self,
        id: u64,
        names: &[&'static str],
    ) -> CResult<(&'static str, &'a Record)> {
        let entity = self.get(id)?;
        for name in names {
            if let Some(r) = entity.record(name) {
                return Ok((name, r));
            }
        }
        err(
            id,
            format!(
                "expected one of {}, found {}",
                names.join(", "),
                type_name(entity)
            ),
        )
    }
}

pub fn type_name(entity: &Entity) -> String {
    match entity {
        Entity::Simple(r) => r.name.clone(),
        Entity::Complex(parts) => {
            let names: Vec<&str> = parts.iter().map(|r| r.name.as_str()).collect();
            format!("({})", names.join(" "))
        }
    }
}

/// Typed access to one record's parameters; every accessor names the
/// entity in its error.
pub struct Args<'a> {
    pub id: u64,
    pub rec: &'a Record,
}

impl<'a> Args<'a> {
    pub fn new(id: u64, rec: &'a Record) -> Self {
        Args { id, rec }
    }

    pub fn param(&self, i: usize) -> CResult<&'a Param> {
        match self.rec.params.get(i) {
            Some(p) => Ok(p),
            None => err(
                self.id,
                format!("{} has no parameter {}", self.rec.name, i + 1),
            ),
        }
    }

    fn wrong<T>(&self, i: usize, want: &str) -> CResult<T> {
        err(
            self.id,
            format!("{} parameter {}: expected {want}", self.rec.name, i + 1),
        )
    }

    pub fn real(&self, i: usize) -> CResult<f64> {
        match real_of(self.param(i)?) {
            Some(v) => Ok(v),
            None => self.wrong(i, "a number"),
        }
    }

    pub fn int(&self, i: usize) -> CResult<i64> {
        match self.param(i)? {
            Param::Int(v) => Ok(*v),
            _ => self.wrong(i, "an integer"),
        }
    }

    pub fn reference(&self, i: usize) -> CResult<u64> {
        match self.param(i)? {
            Param::Ref(id) => Ok(*id),
            _ => self.wrong(i, "a reference"),
        }
    }

    pub fn opt_reference(&self, i: usize) -> CResult<Option<u64>> {
        match self.rec.params.get(i) {
            None | Some(Param::Unset) | Some(Param::Derived) => Ok(None),
            Some(Param::Ref(id)) => Ok(Some(*id)),
            Some(_) => self.wrong(i, "a reference"),
        }
    }

    pub fn logical(&self, i: usize) -> CResult<bool> {
        match self.param(i)? {
            Param::Enum(e) if e == "T" => Ok(true),
            Param::Enum(e) if e == "F" => Ok(false),
            _ => self.wrong(i, ".T. or .F."),
        }
    }

    pub fn string(&self, i: usize) -> String {
        match self.rec.params.get(i) {
            Some(Param::Str(s)) => s.clone(),
            _ => String::new(),
        }
    }

    pub fn list(&self, i: usize) -> CResult<&'a [Param]> {
        match self.param(i)? {
            Param::List(items) => Ok(items),
            _ => self.wrong(i, "a list"),
        }
    }

    pub fn refs(&self, i: usize) -> CResult<Vec<u64>> {
        self.list(i)?
            .iter()
            .map(|p| match p {
                Param::Ref(id) => Ok(*id),
                _ => self.wrong(i, "a list of references"),
            })
            .collect()
    }

    pub fn reals(&self, i: usize) -> CResult<Vec<f64>> {
        self.list(i)?
            .iter()
            .map(|p| real_of(p).map_or_else(|| self.wrong(i, "a list of numbers"), Ok))
            .collect()
    }

    pub fn ints(&self, i: usize) -> CResult<Vec<i64>> {
        self.list(i)?
            .iter()
            .map(|p| match p {
                Param::Int(v) => Ok(*v),
                _ => self.wrong(i, "a list of integers"),
            })
            .collect()
    }
}

/// A number, possibly wrapped in a type such as `LENGTH_MEASURE(...)`.
pub fn real_of(p: &Param) -> Option<f64> {
    match p {
        Param::Real(v) => Some(*v),
        Param::Int(v) => Some(*v as f64),
        Param::Typed(_, inner) => real_of(inner),
        _ => None,
    }
}
