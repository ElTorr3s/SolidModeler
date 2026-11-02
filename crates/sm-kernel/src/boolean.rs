//! Applies a feature's tool solid to the part's bodies.
//!
//! truck's booleans are only reliable on two single-shell solids whose
//! surfaces actually cross. When nothing crosses, its result is meaningless
//! (a cut that misses returns nothing, a cut fully inside returns the
//! cut-out piece), and multi-shell solids (separate lumps, or hollows) break
//! its face classification. So this module:
//!
//! - keeps every body as one outer shell plus zero or more hollows ("voids"),
//! - classifies how two shells relate with triangle meshes before calling
//!   truck, and only calls truck when their surfaces cross, and
//! - splits truck's results back into separate bodies when a cut breaks a
//!   body in two.

use crate::{curves, Body, Budget, Model, RebuildError, Tolerance, MAX_BODIES, MAX_FACES};
use sm_document::{BodyOperation, FeatureId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use truck_meshalgo::prelude::*;
use truck_modeling::{Point3, Solid, Surface};

/// How a tool shell sits relative to a body shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// Their boundaries cross: a real boolean is needed.
    Crossing,
    /// No shared volume.
    Disjoint,
    /// The tool lies entirely inside the body.
    ToolInside,
    /// The body lies entirely inside the tool.
    BodyInside,
}

/// A body as an outer solid (one outward shell) minus hollows (each also a
/// one-shell outward solid, lying inside `outer` and apart from each other).
#[derive(Debug, Clone)]
struct Piece {
    outer: Solid,
    voids: Vec<Solid>,
}

impl Piece {
    fn solid(&self) -> Solid {
        let mut shells = self.outer.boundaries().clone();
        for void in &self.voids {
            let mut inverted = void.clone();
            inverted.not();
            shells.extend(inverted.boundaries().iter().cloned());
        }
        Solid::new(shells)
    }
}

/// What every operation in one feature needs: who to blame, the part's
/// tolerances, the rebuild's work budget, and the feature's mesh cache.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub id: FeatureId,
    pub tol: Tolerance,
    pub budget: &'a Budget,
    pub meshes: &'a MeshCache,
}

struct Meshed {
    mesh: PolygonMesh,
    min: Point3,
    max: Point3,
}

/// Faces (by identity) and their orientations: two solids with the same key
/// have the same boundary, so they mesh the same.
type SolidKey = Vec<(truck_topology::FaceID<Surface>, bool)>;

/// Meshes of the solids one feature has already classified. The same body
/// is compared against every region and slab of a feature, and meshing it
/// is the expensive part, so each solid is meshed once. Entries keep their
/// solid alive, so a face's identity can't be reused while it is cached.
#[derive(Default)]
pub struct MeshCache(RefCell<HashMap<SolidKey, (Solid, Rc<Meshed>)>>);

fn key(solid: &Solid) -> SolidKey {
    solid
        .boundaries()
        .iter()
        .flat_map(|shell| shell.face_iter())
        .map(|f| (f.id(), f.orientation()))
        .collect()
}

fn meshed(cx: Ctx, solid: &Solid) -> Result<Rc<Meshed>, RebuildError> {
    let key = key(solid);
    if let Some((_, m)) = cx.meshes.0.borrow().get(&key) {
        return Ok(m.clone());
    }
    cx.budget.spend(cx.id, key.len() as u64)?;
    let mesh = solid.triangulation(cx.tol.boolean).to_polygon();
    let mut min = Point3::new(f64::MAX, f64::MAX, f64::MAX);
    let mut max = Point3::new(f64::MIN, f64::MIN, f64::MIN);
    for p in mesh.positions() {
        for k in 0..3 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    let m = Rc::new(Meshed { mesh, min, max });
    cx.meshes
        .0
        .borrow_mut()
        .insert(key, (solid.clone(), m.clone()));
    Ok(m)
}

fn relation(cx: Ctx, body: &Solid, tool: &Solid) -> Result<Relation, RebuildError> {
    let tol = cx.tol;
    let (body, tool) = (meshed(cx, body)?, meshed(cx, tool)?);
    let apart = (0..3)
        .any(|k| body.max[k] < tool.min[k] - tol.linear || tool.max[k] < body.min[k] - tol.linear);
    if apart {
        return Ok(Relation::Disjoint);
    }
    if body.mesh.collide_with(&tool.mesh).is_some() {
        return Ok(Relation::Crossing);
    }
    // Nothing crosses, so any one vertex decides containment.
    let inside = |outer: &Meshed, inner: &Meshed| {
        inner
            .mesh
            .positions()
            .first()
            .is_some_and(|&p| outer.mesh.inside(p))
    };
    Ok(if inside(&body, &tool) {
        Relation::ToolInside
    } else if inside(&tool, &body) {
        Relation::BodyInside
    } else {
        Relation::Disjoint
    })
}

/// Whether moving a flush tool face through `slab` leaves the result
/// unchanged: for Join the slab must lie entirely in one body's material,
/// for Cut entirely outside every body.
pub fn slab_ok(
    cx: Ctx,
    bodies: &[Body],
    slab: &Solid,
    op: BodyOperation,
) -> Result<bool, RebuildError> {
    for body in bodies {
        let pieces = pieces_of(cx, body)?;
        for piece in &pieces {
            let rel = relation(cx, &piece.outer, slab)?;
            // A slab touching a hollow is neither safely solid nor empty.
            let mut in_hollow = false;
            for v in &piece.voids {
                if relation(cx, v, slab)? != Relation::Disjoint {
                    in_hollow = true;
                }
            }
            match op {
                BodyOperation::Join if rel == Relation::ToolInside && !in_hollow => {
                    return Ok(true)
                }
                BodyOperation::Cut
                    if rel != Relation::Disjoint
                        && !(rel == Relation::ToolInside && in_hollow_only(cx, piece, slab)?) =>
                {
                    return Ok(false)
                }
                _ => {}
            }
        }
    }
    Ok(op == BodyOperation::Cut)
}

/// True if `slab`, inside `piece.outer`, lies entirely within one hollow
/// (empty space for a Cut).
fn in_hollow_only(cx: Ctx, piece: &Piece, slab: &Solid) -> Result<bool, RebuildError> {
    for v in &piece.voids {
        if relation(cx, v, slab)? == Relation::ToolInside {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Runs a truck boolean on two single-shell solids and splits the result
/// into pieces. Failures and panics become `BooleanFailed`: truck asserts on
/// some degenerate inputs instead of returning `None`.
fn truck_op(
    cx: Ctx,
    a: &Solid,
    b: &Solid,
    op: impl FnOnce(&Solid, &Solid, f64) -> Option<Solid>,
) -> Result<Vec<Piece>, RebuildError> {
    let faces = |s: &Solid| s.boundaries().iter().map(|s| s.len() as u64).sum::<u64>();
    // truck intersects every face of one solid with every face of the other.
    cx.budget
        .spend(cx.id, faces(a).saturating_mul(faces(b)).max(1))?;
    let solid = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| op(a, b, cx.tol.boolean)))
        .ok()
        .flatten()
        .ok_or(RebuildError::BooleanFailed(cx.id))?;
    split(cx, &curves::straighten(&solid, cx.tol))?.ok_or(RebuildError::BooleanFailed(cx.id))
}

/// Splits a solid into pieces: each outward shell is a separate body, and
/// each inward shell is a hollow in the outward shell that contains it.
/// Returns `None` if an inward shell has no container (a broken result).
fn split(cx: Ctx, solid: &Solid) -> Result<Option<Vec<Piece>>, RebuildError> {
    let mut outers = Vec::new();
    let mut voids = Vec::new();
    for shell in solid.boundaries() {
        let one = Solid::new(vec![shell.clone()]);
        if meshed(cx, &one)?.mesh.volume() >= 0.0 {
            outers.push(one);
        } else {
            let mut positive = one;
            positive.not();
            voids.push(positive);
        }
    }
    let mut pieces: Vec<Piece> = outers
        .into_iter()
        .map(|outer| Piece {
            outer,
            voids: Vec::new(),
        })
        .collect();
    for void in voids {
        let mut host = None;
        for (i, p) in pieces.iter().enumerate() {
            if relation(cx, &p.outer, &void)? == Relation::ToolInside {
                host = Some(i);
                break;
            }
        }
        let Some(i) = host else { return Ok(None) };
        pieces[i].voids.push(void);
    }
    Ok(Some(pieces))
}

/// Removes `cutters` from a piece. Each cutter either misses (no change),
/// swallows the piece (gone), sits inside it (becomes a hollow), or crosses
/// it (truck boolean, after which the existing hollows are re-applied to
/// each resulting piece).
fn cut_piece(cx: Ctx, piece: Piece, cutters: &[Solid]) -> Result<Vec<Piece>, RebuildError> {
    let mut pieces = vec![piece];
    for cutter in cutters {
        let mut next = Vec::new();
        for p in pieces {
            match relation(cx, &p.outer, cutter)? {
                Relation::Disjoint => next.push(p),
                Relation::BodyInside => {}
                Relation::ToolInside => next.push(add_hollow(cx, p, cutter.clone())?),
                Relation::Crossing => {
                    // A - B is A ∩ (complement of B); inverting B gives the complement.
                    let mut complement = cutter.clone();
                    complement.not();
                    let parts = truck_op(cx, &p.outer, &complement, truck_shapeops::and)?;
                    for q in parts {
                        let hollows: Vec<Solid> =
                            q.voids.into_iter().chain(p.voids.iter().cloned()).collect();
                        let bare = Piece {
                            outer: q.outer,
                            voids: Vec::new(),
                        };
                        next.extend(cut_piece(cx, bare, &hollows)?);
                    }
                }
            }
        }
        pieces = next;
    }
    Ok(pieces)
}

/// Adds a hollow that lies inside `piece`, merging it with any hollow it
/// overlaps so hollows always stay apart.
fn add_hollow(cx: Ctx, mut piece: Piece, mut hollow: Solid) -> Result<Piece, RebuildError> {
    let mut others = Vec::new();
    for v in std::mem::take(&mut piece.voids) {
        match relation(cx, &hollow, &v)? {
            Relation::Disjoint => others.push(v),
            Relation::ToolInside => {}
            Relation::BodyInside => hollow = v,
            Relation::Crossing => {
                hollow = single(cx, truck_op(cx, &hollow, &v, truck_shapeops::or)?)?;
            }
        }
    }
    others.push(hollow);
    piece.voids = others;
    Ok(piece)
}

/// The one solid a union of two touching shells must produce.
fn single(cx: Ctx, pieces: Vec<Piece>) -> Result<Solid, RebuildError> {
    match <[Piece; 1]>::try_from(pieces) {
        Ok([p]) => Ok(p.solid()),
        Err(_) => Err(RebuildError::BooleanFailed(cx.id)),
    }
}

fn pieces_of(cx: Ctx, body: &Body) -> Result<Vec<Piece>, RebuildError> {
    // Bodies are only ever built from pieces, so splitting can't fail.
    split(cx, &body.solid)?.ok_or(RebuildError::BooleanFailed(cx.id))
}

fn push_pieces(
    model: &mut Model,
    id: FeatureId,
    feature: FeatureId,
    pieces: Vec<Piece>,
) -> Result<(), RebuildError> {
    for piece in pieces {
        if model.bodies.len() >= MAX_BODIES {
            return Err(RebuildError::TooManyBodies(id));
        }
        let faces = |s: &Solid| s.boundaries().iter().map(|sh| sh.len()).sum::<usize>();
        let total = model.bodies.iter().map(|b| faces(&b.solid)).sum::<usize>()
            + faces(&piece.outer)
            + piece.voids.iter().map(faces).sum::<usize>();
        if total > MAX_FACES {
            return Err(RebuildError::TooManyFaces(id));
        }
        model.bodies.push(Body {
            feature,
            solid: piece.solid(),
        });
    }
    Ok(())
}

/// Adds a body for a New Solid feature.
pub fn add_body(model: &mut Model, id: FeatureId, tool: Solid) -> Result<(), RebuildError> {
    push_pieces(
        model,
        id,
        id,
        vec![Piece {
            outer: tool,
            voids: Vec::new(),
        }],
    )
}

/// Join: merges the tool with every body it touches into one body. A tool
/// that touches nothing becomes a new body.
pub fn join(cx: Ctx, model: &mut Model, tool: Solid) -> Result<(), RebuildError> {
    let id = cx.id;
    let mut merged = tool.clone();
    let mut voids = Vec::new();
    let mut first_feature = None;
    let mut kept = Vec::with_capacity(model.bodies.len());
    let mut islands = Vec::new();
    for body in std::mem::take(&mut model.bodies) {
        let pieces = pieces_of(cx, &body)?;
        let outer = &pieces[0].outer;
        match relation(cx, outer, &merged)? {
            Relation::Disjoint => {
                kept.push(body);
                continue;
            }
            // The tool (and whatever it already merged) is inside this body.
            Relation::ToolInside => merged = outer.clone(),
            // This body is inside the tool: the tool already covers it.
            Relation::BodyInside => {}
            Relation::Crossing => {
                merged = single(cx, truck_op(cx, outer, &merged, truck_shapeops::or)?)?;
            }
        }
        first_feature.get_or_insert(body.feature);
        // The tool fills whatever part of the body's hollows it covers; a
        // tool floating inside a hollow is a separate island.
        for void in pieces.into_iter().flat_map(|p| p.voids) {
            let bare = Piece {
                outer: void,
                voids: Vec::new(),
            };
            for remaining in cut_piece(cx, bare, std::slice::from_ref(&tool))? {
                voids.push(remaining.outer);
                islands.extend(remaining.voids);
            }
        }
    }
    model.bodies = kept;
    let pieces = cut_piece(
        cx,
        Piece {
            outer: merged,
            voids: Vec::new(),
        },
        &voids,
    )?;
    push_pieces(model, id, first_feature.unwrap_or(id), pieces)?;
    for island in islands {
        add_body(model, id, island)?;
    }
    Ok(())
}

/// Cut: removes the tools' volume (a feature's separate regions) from every
/// body. Bodies the tools miss are unchanged, bodies they swallow disappear,
/// a tool fully inside a body leaves a hollow, and a body cut in two becomes
/// two bodies. Fails if nothing is left.
pub fn cut(cx: Ctx, model: &mut Model, tools: &[Solid]) -> Result<(), RebuildError> {
    let id = cx.id;
    if model.bodies.is_empty() {
        return Err(RebuildError::NoTargetBody(id, BodyOperation::Cut));
    }
    for body in std::mem::take(&mut model.bodies) {
        let feature = body.feature;
        for piece in pieces_of(cx, &body)? {
            let pieces = cut_piece(cx, piece, tools)?;
            push_pieces(model, id, feature, pieces)?;
        }
    }
    // As in Inventor, a cut may remove some bodies but not the whole part.
    if model.bodies.is_empty() {
        return Err(RebuildError::EmptyResult(id));
    }
    Ok(())
}

/// Intersect: keeps only the volume each body shares with any of the tools.
/// The tools are the feature's separate regions. Fails if nothing is left.
pub fn intersect(cx: Ctx, model: &mut Model, tools: &[Solid]) -> Result<(), RebuildError> {
    let id = cx.id;
    if model.bodies.is_empty() {
        return Err(RebuildError::NoTargetBody(id, BodyOperation::Intersect));
    }
    for body in std::mem::take(&mut model.bodies) {
        for piece in pieces_of(cx, &body)? {
            for tool in tools {
                let common = match relation(cx, &piece.outer, tool)? {
                    Relation::Disjoint => Vec::new(),
                    Relation::ToolInside => vec![Piece {
                        outer: tool.clone(),
                        voids: Vec::new(),
                    }],
                    Relation::BodyInside => vec![Piece {
                        outer: piece.outer.clone(),
                        voids: Vec::new(),
                    }],
                    Relation::Crossing => truck_op(cx, &piece.outer, tool, truck_shapeops::and)?,
                };
                for c in common {
                    let pieces = cut_piece(cx, c, &piece.voids)?;
                    push_pieces(model, id, body.feature, pieces)?;
                }
            }
        }
    }
    if model.bodies.is_empty() {
        return Err(RebuildError::EmptyResult(id));
    }
    Ok(())
}
