//! What happened to each argument sub-shape in one modelling operation.

use super::ffi::ffi::HistEntry;
use super::{OcctError, Result, Shape, SubKind, SubShape};
use std::collections::HashMap;

/// How a result sub-shape relates to an argument sub-shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Relation {
    /// The same entity came through unchanged.
    Kept,
    /// The entity was changed (trimmed, split, moved); it may map to
    /// several result entities.
    Modified,
    /// A new entity was made from it, e.g. the side face of a prism from a
    /// profile edge, or a fillet face from a filleted edge.
    Generated,
}

/// One origin of a result sub-shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Source {
    /// Which argument of the operation (0 for the first).
    pub arg: usize,
    pub sub: SubShape,
    pub relation: Relation,
}

#[derive(Debug, Clone, Default)]
pub struct History {
    forward: HashMap<(usize, SubShape), Vec<(Relation, SubShape)>>,
    deleted: HashMap<(usize, SubShape), ()>,
    backward: HashMap<SubShape, Vec<Source>>,
}

impl History {
    pub(crate) fn new(entries: &[HistEntry], args: usize, result: &Shape) -> Result<History> {
        let mut h = History::default();
        let bad = || OcctError::Kernel("operation returned malformed history".into());
        for e in entries {
            let arg = usize::from(e.arg);
            if arg >= args {
                return Err(bad());
            }
            let kind = SubKind::from_u8(e.kind).ok_or_else(bad)?;
            let input = SubShape {
                kind,
                index: e.index,
            };
            let relation = match e.relation {
                0 => Relation::Kept,
                1 => Relation::Modified,
                2 => Relation::Generated,
                3 => {
                    h.deleted.insert((arg, input), ());
                    continue;
                }
                _ => return Err(bad()),
            };
            let out = SubShape {
                kind: SubKind::from_u8(e.result_kind).ok_or_else(bad)?,
                index: e.result_index,
            };
            if out.index >= result.count(out.kind) {
                return Err(bad());
            }
            h.forward
                .entry((arg, input))
                .or_default()
                .push((relation, out));
            h.backward.entry(out).or_default().push(Source {
                arg,
                sub: input,
                relation,
            });
        }
        Ok(h)
    }

    fn related(&self, arg: usize, sub: SubShape, want: Relation) -> Vec<SubShape> {
        self.forward
            .get(&(arg, sub))
            .into_iter()
            .flatten()
            .filter(|(r, _)| *r == want)
            .map(|(_, s)| *s)
            .collect()
    }

    /// Result entities that argument `arg`'s `sub` became when changed.
    pub fn modified(&self, arg: usize, sub: SubShape) -> Vec<SubShape> {
        self.related(arg, sub, Relation::Modified)
    }

    /// Result entities created from argument `arg`'s `sub`.
    pub fn generated(&self, arg: usize, sub: SubShape) -> Vec<SubShape> {
        self.related(arg, sub, Relation::Generated)
    }

    /// The result entity that is argument `arg`'s `sub`, unchanged.
    pub fn kept(&self, arg: usize, sub: SubShape) -> Option<SubShape> {
        self.related(arg, sub, Relation::Kept).into_iter().next()
    }

    pub fn is_deleted(&self, arg: usize, sub: SubShape) -> bool {
        self.deleted.contains_key(&(arg, sub))
    }

    /// Where a result entity came from. Empty for entities with no
    /// recorded origin (e.g. a brand-new seam edge).
    pub fn source(&self, result: SubShape) -> &[Source] {
        self.backward.get(&result).map_or(&[], Vec::as_slice)
    }
}
