//! Closed loops of sketch curves, for features such as extrude.
//!
//! Each loop lists the curves it is made of, so a feature can name the
//! faces it creates after the [`CurveId`] that produced them. Curve ids are
//! stable, which keeps those names valid while the sketch is edited.

use crate::{CurveGeom, CurveId, PointId, Sketch};
use thiserror::Error;

/// Points closer than this join up when finding loops, in model units.
/// Endpoints held together by a coincident constraint solve to well within
/// it.
pub const JOIN_TOLERANCE: f64 = 1e-6;

/// One curve in a loop, in the direction the loop runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileSegment {
    pub curve: CurveId,
    /// `true` when the loop runs from the curve's end to its start. Arcs
    /// run counter-clockwise from start to end, so a reversed arc runs
    /// clockwise.
    pub reversed: bool,
}

/// A closed chain of curves. Segment `i` ends where segment `i + 1` starts,
/// and the last ends where the first starts. A full circle is a loop of one
/// segment. The loop runs whichever way round it was found: callers that
/// need counter-clockwise outer loops orient them from the geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileLoop {
    pub segments: Vec<ProfileSegment>,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ProfileError {
    /// The listed endpoints join one curve or three or more. Profiles need
    /// every endpoint shared by exactly two curves.
    #[error("sketch has open or branching ends at {0:?}")]
    OpenOrBranching(Vec<PointId>),
    /// A line or arc whose two ends are in the same place.
    #[error("curve {0:?} has zero length")]
    ZeroLength(CurveId),
}

impl Sketch {
    /// Closed loops formed by the sketch's non-construction curves.
    pub fn profile_loops(&self) -> Result<Vec<ProfileLoop>, ProfileError> {
        let mut loops = Vec::new();
        // (curve, start, end) for every open curve.
        let mut edges: Vec<(CurveId, PointId, PointId)> = Vec::new();
        for (&id, c) in &self.curves {
            if c.construction {
                continue;
            }
            match c.geom {
                CurveGeom::Circle { .. } => loops.push(ProfileLoop {
                    segments: vec![ProfileSegment {
                        curve: id,
                        reversed: false,
                    }],
                }),
                CurveGeom::Line { start, end } | CurveGeom::Arc { start, end, .. } => {
                    edges.push((id, start, end))
                }
            }
        }

        let node = self.join_points(edges.iter().flat_map(|&(_, s, e)| [s, e]));
        let node_of = |p: PointId| {
            node[node
                .binary_search_by_key(&p, |&(q, _)| q)
                .expect("collected")]
            .1
        };

        // Incident edges per node.
        let nodes = node.iter().map(|&(_, n)| n + 1).max().unwrap_or(0);
        let mut incident: Vec<Vec<usize>> = vec![Vec::new(); nodes];
        for (k, &(curve, s, e)) in edges.iter().enumerate() {
            let (ns, ne) = (node_of(s), node_of(e));
            if ns == ne {
                return Err(ProfileError::ZeroLength(curve));
            }
            incident[ns].push(k);
            incident[ne].push(k);
        }
        let mut bad: Vec<PointId> = node
            .iter()
            .filter(|&&(_, n)| incident[n].len() != 2)
            .map(|&(p, _)| p)
            .collect();
        if !bad.is_empty() {
            bad.sort();
            return Err(ProfileError::OpenOrBranching(bad));
        }

        let mut used = vec![false; edges.len()];
        for first in 0..edges.len() {
            if used[first] {
                continue;
            }
            let mut segments = Vec::new();
            let start_node = node_of(edges[first].1);
            let (mut k, mut at) = (first, start_node);
            loop {
                used[k] = true;
                let (curve, s, e) = edges[k];
                let reversed = node_of(s) != at;
                segments.push(ProfileSegment { curve, reversed });
                at = node_of(if reversed { s } else { e });
                if at == start_node {
                    break;
                }
                // Every node has exactly two edges; take the one not just used.
                k = incident[at]
                    .iter()
                    .copied()
                    .find(|&j| j != k)
                    .expect("degree two");
            }
            loops.push(ProfileLoop { segments });
        }
        Ok(loops)
    }

    /// Groups points that lie within [`JOIN_TOLERANCE`] of each other.
    /// Returns `(point, group)` sorted by point.
    fn join_points(&self, points: impl Iterator<Item = PointId>) -> Vec<(PointId, usize)> {
        let mut pts: Vec<PointId> = points.collect();
        pts.sort();
        pts.dedup();
        // Sweep in x order so only nearby points are compared.
        let mut by_x: Vec<(f64, f64, usize)> = pts
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let [x, y] = self.xy(p);
                (x, y, i)
            })
            .collect();
        by_x.sort_by(|a, b| a.0.total_cmp(&b.0));

        let mut parent: Vec<usize> = (0..pts.len()).collect();
        fn find(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for a in 0..by_x.len() {
            for b in a + 1..by_x.len() {
                if by_x[b].0 - by_x[a].0 > JOIN_TOLERANCE {
                    break;
                }
                if (by_x[b].0 - by_x[a].0).hypot(by_x[b].1 - by_x[a].1) <= JOIN_TOLERANCE {
                    let (ra, rb) = (find(&mut parent, by_x[a].2), find(&mut parent, by_x[b].2));
                    parent[ra] = rb;
                }
            }
        }

        let mut group_of_root = vec![usize::MAX; pts.len()];
        let mut groups = 0;
        pts.iter()
            .enumerate()
            .map(|(i, &p)| {
                let root = find(&mut parent, i);
                if group_of_root[root] == usize::MAX {
                    group_of_root[root] = groups;
                    groups += 1;
                }
                (p, group_of_root[root])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Constraint;

    fn seg(curve: CurveId, reversed: bool) -> ProfileSegment {
        ProfileSegment { curve, reversed }
    }

    #[test]
    fn shared_points_form_a_loop() {
        let mut s = Sketch::new();
        let p = [[0.0, 0.0], [10.0, 0.0], [10.0, 5.0], [0.0, 5.0]]
            .map(|[x, y]| s.add_point(x, y).unwrap());
        let a = s.add_line(p[0], p[1]).unwrap();
        let b = s.add_line(p[2], p[1]).unwrap(); // drawn backwards
        let c = s.add_line(p[2], p[3]).unwrap();
        let d = s.add_line(p[3], p[0]).unwrap();
        let loops = s.profile_loops().unwrap();
        assert_eq!(
            loops,
            vec![ProfileLoop {
                segments: vec![seg(a, false), seg(b, true), seg(c, false), seg(d, false)]
            }]
        );
    }

    #[test]
    fn coincident_points_join_and_arcs_and_circles_count() {
        let mut s = Sketch::new();
        // A slot: two lines and two arcs, with separate endpoint points held
        // together by coincident constraints.
        let l1a = s.add_point(0.0, 0.0).unwrap();
        let l1b = s.add_point(10.0, 0.0).unwrap();
        let l2a = s.add_point(10.0, 4.0).unwrap();
        let l2b = s.add_point(0.0, 4.0).unwrap();
        let bottom = s.add_line(l1a, l1b).unwrap();
        let top = s.add_line(l2a, l2b).unwrap();
        let c1 = s.add_point(10.0, 2.0).unwrap();
        let r0 = s.add_point(10.0, 0.1).unwrap();
        let r1 = s.add_point(10.0, 4.1).unwrap();
        let right = s.add_arc(c1, r0, r1).unwrap();
        let c2 = s.add_point(0.0, 2.0).unwrap();
        let q0 = s.add_point(0.0, 4.0).unwrap();
        let q1 = s.add_point(0.0, 0.0).unwrap();
        let left = s.add_arc(c2, q0, q1).unwrap();
        for (x, y) in [(l1b, r0), (r1, l2a), (l2b, q0), (q1, l1a)] {
            s.add_constraint(Constraint::Coincident(x, y)).unwrap();
        }
        let hole_c = s.add_point(5.0, 2.0).unwrap();
        let hole = s.add_circle(hole_c, 1.0).unwrap();
        // Construction geometry is left out even when it dangles.
        let k0 = s.add_point(-5.0, -5.0).unwrap();
        let axis = s.add_line(k0, hole_c).unwrap();
        s.set_construction(axis, true).unwrap();

        let loops = s.profile_loops().unwrap();
        assert_eq!(loops.len(), 2);
        assert_eq!(loops[0].segments, vec![seg(hole, false)]);
        assert_eq!(
            loops[1].segments,
            vec![
                seg(bottom, false),
                seg(right, false),
                seg(top, false),
                seg(left, false)
            ]
        );
    }

    #[test]
    fn open_branching_and_degenerate_sketches_are_reported() {
        let mut s = Sketch::new();
        let a = s.add_point(0.0, 0.0).unwrap();
        let b = s.add_point(5.0, 0.0).unwrap();
        let c = s.add_point(5.0, 5.0).unwrap();
        s.add_line(a, b).unwrap();
        s.add_line(b, c).unwrap();
        assert_eq!(
            s.profile_loops(),
            Err(ProfileError::OpenOrBranching(vec![a, c]))
        );
        s.add_line(c, a).unwrap();
        assert!(s.profile_loops().is_ok());
        let d = s.add_point(9.0, 9.0).unwrap();
        s.add_line(b, d).unwrap();
        assert_eq!(
            s.profile_loops(),
            Err(ProfileError::OpenOrBranching(vec![b, d]))
        );

        let mut s = Sketch::new();
        let a = s.add_point(1.0, 1.0).unwrap();
        let b = s.add_point(1.0, 1.0 + 1e-9).unwrap();
        let line = s.add_line(a, b).unwrap();
        assert_eq!(s.profile_loops(), Err(ProfileError::ZeroLength(line)));
        assert_eq!(Sketch::new().profile_loops(), Ok(Vec::new()));
    }

    #[test]
    fn curve_ids_survive_edits() {
        let mut s = Sketch::new();
        let p = [[0.0, 0.0], [10.0, 0.0], [5.0, 8.0]].map(|[x, y]| s.add_point(x, y).unwrap());
        let lines = [0, 1, 2].map(|i| s.add_line(p[i], p[(i + 1) % 3]).unwrap());
        let before = s.profile_loops().unwrap();
        let dim = s
            .add_constraint(Constraint::Distance(p[0], p[1], 20.0))
            .unwrap();
        s.set_dimension(dim, 14.0).unwrap();
        s.remove_constraint(dim).unwrap();
        assert_eq!(s.profile_loops().unwrap(), before);
        assert_eq!(
            before[0]
                .segments
                .iter()
                .map(|g| g.curve)
                .collect::<Vec<_>>(),
            lines
        );
    }
}
