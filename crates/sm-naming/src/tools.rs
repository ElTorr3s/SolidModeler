//! Names the faces of a freshly built tool solid from the sketch it came
//! from. This is the only place names are invented; booleans only pass them
//! on (see `track.rs`).
//!
//! Faces are classified by geometry, not by the order truck's sweeps emit
//! them in, so a change inside truck can't silently shuffle names.

use crate::name::{FaceOrigin, FaceRole, SketchEdge};
use sm_document::{FeatureId, Profile};
use sm_kernel::profile::{Point2, Region};
use sm_kernel::{Frame, LINEAR_TOLERANCE};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use truck_modeling::{
    BoundedCurve, Face, InnerSpace, ParametricCurve, Point3, Solid, Surface, Vector3,
};

/// Identifies a face's underlying surface. truck's booleans copy surfaces
/// unchanged into the result and only rebuild trimming loops, so a face of
/// a boolean result has the same key as the input face it was cut from.
pub(crate) type SurfaceKey = u64;

pub(crate) fn surface_key(face: &Face) -> SurfaceKey {
    // Debug output of floats round-trips exactly, so equal keys mean
    // bit-identical surfaces. Orientation is not part of the key: a cut
    // inverts the tool's faces but keeps their surfaces.
    let mut hasher = DefaultHasher::new();
    format!("{:?}", face.surface()).hash(&mut hasher);
    hasher.finish()
}

/// The origin of every face of one solid, keyed by surface.
#[derive(Debug, Clone, Default)]
pub struct FaceOrigins {
    by_surface: HashMap<SurfaceKey, Vec<FaceOrigin>>,
}

impl FaceOrigins {
    pub(crate) fn insert(&mut self, key: SurfaceKey, origin: FaceOrigin) {
        let origins = self.by_surface.entry(key).or_default();
        if !origins.contains(&origin) {
            origins.push(origin);
            origins.sort();
        }
    }

    /// The origin of a face with this surface. Two origins sharing one
    /// surface would take bit-identical construction; the older feature wins
    /// and [`FaceOrigins::count`] lets callers report the conflict.
    pub(crate) fn get(&self, key: SurfaceKey) -> Option<FaceOrigin> {
        self.by_surface.get(&key).and_then(|o| o.first()).copied()
    }

    /// How many origins claim this surface; anything but 1 is untraced.
    pub(crate) fn count(&self, key: SurfaceKey) -> usize {
        self.by_surface.get(&key).map_or(0, Vec::len)
    }

    /// Origins for the faces of `solid`, taken from `sources`.
    pub(crate) fn carried(solid: &Solid, sources: &[&FaceOrigins]) -> FaceOrigins {
        let mut out = FaceOrigins::default();
        for face in faces(solid) {
            let key = surface_key(&face);
            for source in sources {
                for &origin in source.by_surface.get(&key).into_iter().flatten() {
                    out.insert(key, origin);
                }
            }
        }
        out
    }
}

pub(crate) fn faces(solid: &Solid) -> impl Iterator<Item = Face> + '_ {
    solid
        .boundaries()
        .iter()
        .flat_map(|shell| shell.face_iter().cloned())
}

/// Finds the document segment a cleaned region segment came from. The
/// kernel drops repeated points and may reverse loops, so match endpoints
/// in either direction within tolerance.
fn sketch_edge(profiles: &[Profile], a: Point2, b: Point2) -> Option<SketchEdge> {
    let near = |p: Point2, q: Point2| (p[0] - q[0]).hypot(p[1] - q[1]) <= LINEAR_TOLERANCE;
    profiles.iter().enumerate().find_map(|(i, profile)| {
        let n = profile.points.len();
        (0..n).find_map(|j| {
            let (p, q) = (profile.points[j], profile.points[(j + 1) % n]);
            ((near(p, a) && near(q, b)) || (near(p, b) && near(q, a))).then_some(SketchEdge {
                profile: i as u32,
                segment: j as u32,
            })
        })
    })
}

fn outer_profile(profiles: &[Profile], region: &Region) -> Option<u32> {
    sketch_edge(profiles, region.outer[0], region.outer[1]).map(|e| e.profile)
}

fn segments(region: &Region) -> impl Iterator<Item = (Point2, Point2)> + '_ {
    std::iter::once(&region.outer)
        .chain(&region.holes)
        .flat_map(|points| (0..points.len()).map(move |i| sm_kernel::profile::edge(points, i)))
}

fn plane_normal(face: &Face) -> Option<Vector3> {
    match face.oriented_surface() {
        Surface::Plane(plane) => Some(plane.normal()),
        _ => None,
    }
}

/// Names the faces of an extrude tool built from `region` of a sketch with
/// `profiles`, on `frame`. `reversed` is true when the extrude runs against
/// the sketch normal; the start cap is always the one on the sketch side, so
/// flipping an extrude's direction keeps its names.
pub fn extrude_tool(
    feature: FeatureId,
    profiles: &[Profile],
    region: &Region,
    frame: &Frame,
    reversed: bool,
    solid: &Solid,
) -> FaceOrigins {
    // Without a known outer loop, caps stay unnamed rather than colliding
    // with another region's caps.
    let profile = outer_profile(profiles, region);
    let start = profile.map(|profile| FaceRole::StartCap { profile });
    let end = profile.map(|profile| FaceRole::EndCap { profile });
    let (below, above) = if reversed { (end, start) } else { (start, end) };
    let to_2d = |p: Point3| {
        let d = p - frame.origin;
        [d.dot(frame.u), d.dot(frame.v)]
    };
    let mut origins = FaceOrigins::default();
    let mut unknown = 0;
    for face in faces(solid) {
        let normal = plane_normal(&face);
        let role = match normal.map(|n| n.dot(frame.normal)) {
            Some(d) if d < -1.0 + 1e-9 => below,
            Some(d) if d > 1.0 - 1e-9 => above,
            Some(_) => {
                let n = normal.unwrap();
                let points: Vec<Point2> = face.vertex_iter().map(|v| to_2d(v.point())).collect();
                side_segment(region, [n.dot(frame.u), n.dot(frame.v)], &points)
                    .and_then(|(a, b)| sketch_edge(profiles, a, b))
                    .map(|edge| FaceRole::Side { edge, piece: 0 })
            }
            None => None,
        };
        let role = role.unwrap_or_else(|| {
            unknown += 1;
            FaceRole::Unknown { ordinal: unknown }
        });
        origins.insert(surface_key(&face), FaceOrigin { feature, role });
    }
    origins
}

/// The region segment a side face was swept from: same outward direction,
/// nearest line (flush handling may have moved the face slightly), and a
/// midpoint within the face's extent so collinear segments stay apart.
fn side_segment(region: &Region, normal: Point2, face: &[Point2]) -> Option<(Point2, Point2)> {
    let len = normal[0].hypot(normal[1]);
    let normal = [normal[0] / len, normal[1] / len];
    let dir = [-normal[1], normal[0]];
    let dot = |p: Point2, d: Point2| p[0] * d[0] + p[1] * d[1];
    let along: Vec<f64> = face.iter().map(|&p| dot(p, dir)).collect();
    let (lo, hi) = along
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &t| (lo.min(t), hi.max(t)));
    let face_offset = dot(face[0], normal);
    segments(region)
        .filter(|&(a, b)| {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = dx.hypot(dy);
            let side = [dy / l, -dx / l];
            let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            dot(side, normal) > 1.0 - 1e-9 && (lo..=hi).contains(&dot(mid, dir))
        })
        .min_by(|&(a, _), &(c, _)| {
            let da = (dot(a, normal) - face_offset).abs();
            let dc = (dot(c, normal) - face_offset).abs();
            da.total_cmp(&dc)
        })
}

/// A revolve's axis in model space and its sweep angle in radians
/// (negative sweeps clockwise about `direction`).
#[derive(Debug, Clone, Copy)]
pub struct RevolveAxis {
    pub origin: Point3,
    pub direction: Vector3,
    pub angle: f64,
}

/// Names the faces of a revolve tool. The kernel has no revolve feature yet;
/// this is ready for when it does, and is tested against truck's `rsweep`.
pub fn revolve_tool(
    feature: FeatureId,
    profiles: &[Profile],
    region: &Region,
    frame: &Frame,
    axis: RevolveAxis,
    solid: &Solid,
) -> FaceOrigins {
    let profile = outer_profile(profiles, region);
    let (dir, angle) = if axis.angle < 0.0 {
        (-axis.direction.normalize(), -axis.angle)
    } else {
        (axis.direction.normalize(), axis.angle)
    };
    let whole = angle >= std::f64::consts::TAU - 1e-9;
    let at = |p: Point2| frame.origin + frame.u * p[0] + frame.v * p[1];
    // Angle 0 is the half-plane holding the profile.
    let radial = |p: Point3| {
        let d = p - axis.origin;
        d - dir * d.dot(dir)
    };
    let e1 = segments(region)
        .map(|(a, _)| radial(at(a)))
        .find(|r| r.magnitude() > LINEAR_TOLERANCE)
        .map(|r| r.normalize());
    let Some(e1) = e1 else {
        return FaceOrigins::default();
    };
    let e2 = dir.cross(e1);
    let phi = |p: Point3| {
        let r = radial(p);
        (r.magnitude() > LINEAR_TOLERANCE)
            .then(|| r.dot(e2).atan2(r.dot(e1)).rem_euclid(std::f64::consts::TAU))
    };
    let rh = |p: Point3| {
        let d = p - axis.origin;
        [radial(p).magnitude(), d.dot(dir)]
    };
    let same_angle = |a: f64, b: f64| {
        let d = (a - b).rem_euclid(std::f64::consts::TAU);
        !(1e-6..=std::f64::consts::TAU - 1e-6).contains(&d)
    };

    let mut sides: Vec<(SketchEdge, f64, SurfaceKey)> = Vec::new();
    let mut origins = FaceOrigins::default();
    let mut unknown = 0;
    for face in faces(solid) {
        let key = surface_key(&face);
        let points: Vec<Point3> = face.vertex_iter().map(|v| v.point()).collect();
        let angles: Vec<f64> = points.iter().filter_map(|&p| phi(p)).collect();
        let cap = !whole && !angles.is_empty() && angles.iter().all(|&a| same_angle(a, angles[0]));
        let role = if cap && same_angle(angles[0], 0.0) {
            profile.map(|profile| FaceRole::StartCap { profile })
        } else if cap && same_angle(angles[0], angle) {
            profile.map(|profile| FaceRole::EndCap { profile })
        } else {
            let near = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]) <= 1e-6;
            let segment = segments(region).find(|&(a, b)| {
                let (a, b) = (rh(at(a)), rh(at(b)));
                points.iter().all(|&p| near(rh(p), a) || near(rh(p), b))
            });
            if let Some(edge) = segment.and_then(|(a, b)| sketch_edge(profiles, a, b)) {
                // Named below, once all pieces of the face are known.
                sides.push((edge, mid_angle(&face, &phi), key));
                continue;
            }
            None
        };
        let role = role.unwrap_or_else(|| {
            unknown += 1;
            FaceRole::Unknown { ordinal: unknown }
        });
        origins.insert(key, FaceOrigin { feature, role });
    }
    // Number the pieces of each swept face by angle.
    sides.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut piece = 0;
    for i in 0..sides.len() {
        piece = if i > 0 && sides[i - 1].0 == sides[i].0 {
            piece + 1
        } else {
            0
        };
        let (edge, _, key) = sides[i];
        let role = FaceRole::Side { edge, piece };
        origins.insert(key, FaceOrigin { feature, role });
    }
    origins
}

/// The angle in the middle of a revolved face: the circular mean of the
/// midpoints of its boundary edges.
fn mid_angle(face: &Face, phi: &impl Fn(Point3) -> Option<f64>) -> f64 {
    let (mut x, mut y) = (0.0, 0.0);
    for edge in face.edge_iter() {
        let curve = edge.curve();
        let (t0, t1) = curve.range_tuple();
        if let Some(a) = phi(curve.subs((t0 + t1) / 2.0)) {
            x += a.cos();
            y += a.sin();
        }
    }
    y.atan2(x).rem_euclid(std::f64::consts::TAU)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use truck_modeling::{builder, Rad, Wire};

    /// Rectangle x in [5, 8], height in [0, 4] on the XZ plane, which holds
    /// the Z axis it is revolved about.
    fn setup() -> (Vec<Profile>, Region, Frame, Face) {
        let points = vec![[5.0, 0.0], [8.0, 0.0], [8.0, 4.0], [5.0, 4.0]];
        let profiles = vec![Profile {
            points: points.clone(),
        }];
        let region = Region {
            outer: points,
            holes: Vec::new(),
        };
        let frame = Frame {
            origin: Point3::new(0.0, 0.0, 0.0),
            u: Vector3::unit_x(),
            v: Vector3::unit_z(),
            normal: -Vector3::unit_y(),
        };
        let vertices: Vec<_> = region
            .outer
            .iter()
            .map(|p| builder::vertex(frame.origin + frame.u * p[0] + frame.v * p[1]))
            .collect();
        let wire: Wire = (0..4)
            .map(|i| builder::line(&vertices[i], &vertices[(i + 1) % 4]))
            .collect();
        let face = builder::try_attach_plane(&[wire]).unwrap();
        (profiles, region, frame, face)
    }

    fn roles(origins: &FaceOrigins, solid: &Solid) -> Vec<FaceRole> {
        faces(solid)
            .map(|f| origins.get(surface_key(&f)).unwrap().role)
            .collect()
    }

    fn side(segment: u32, piece: u32) -> FaceRole {
        FaceRole::Side {
            edge: SketchEdge {
                profile: 0,
                segment,
            },
            piece,
        }
    }

    #[test]
    fn partial_revolve_has_caps_and_pieces_per_segment() {
        let (profiles, region, frame, face) = setup();
        // truck splits each swept face in 2 below half a turn, 3 above.
        let quarter = std::f64::consts::FRAC_PI_2;
        for (angle, pieces) in [(quarter, 2), (-quarter, 2), (3.5, 3), (-3.5, 3)] {
            let solid = builder::rsweep(
                &face,
                Point3::new(0.0, 0.0, 0.0),
                Vector3::unit_z(),
                Rad(angle),
            );
            let axis = RevolveAxis {
                origin: Point3::new(0.0, 0.0, 0.0),
                direction: Vector3::unit_z(),
                angle,
            };
            let origins = revolve_tool(FeatureId(3), &profiles, &region, &frame, axis, &solid);
            let got: BTreeSet<FaceRole> = roles(&origins, &solid).into_iter().collect();
            let mut want: BTreeSet<FaceRole> = (0..4)
                .flat_map(|s| (0..pieces).map(move |p| side(s, p)))
                .collect();
            want.insert(FaceRole::StartCap { profile: 0 });
            want.insert(FaceRole::EndCap { profile: 0 });
            assert_eq!(got, want, "angle {angle}");
            assert_eq!(roles(&origins, &solid).len(), want.len());

            // The start cap is the profile itself, in the XZ plane.
            let start = faces(&solid)
                .find(|f| {
                    origins.get(surface_key(f)).unwrap().role == FaceRole::StartCap { profile: 0 }
                })
                .unwrap();
            assert!(start.vertex_iter().all(|v| v.point().y.abs() < 1e-9));
        }
    }

    #[test]
    fn full_revolve_numbers_pieces_by_angle() {
        let (profiles, region, frame, face) = setup();
        let solid = builder::rsweep(
            &face,
            Point3::new(0.0, 0.0, 0.0),
            Vector3::unit_z(),
            Rad(std::f64::consts::TAU),
        );
        let axis = RevolveAxis {
            origin: Point3::new(0.0, 0.0, 0.0),
            direction: Vector3::unit_z(),
            angle: std::f64::consts::TAU,
        };
        let origins = revolve_tool(FeatureId(3), &profiles, &region, &frame, axis, &solid);
        let all = roles(&origins, &solid);
        let unique: BTreeSet<FaceRole> = all.iter().copied().collect();
        assert_eq!(all.len(), unique.len(), "{all:?}");
        let pieces = all.len() as u32 / 4;
        assert!(pieces >= 2, "{all:?}");
        let want: BTreeSet<FaceRole> = (0..4)
            .flat_map(|s| (0..pieces).map(move |p| side(s, p)))
            .collect();
        assert_eq!(unique, want);

        // Piece 0 of the outer wall starts at the sketch plane and runs
        // counter-clockwise about +Z, so it holds points with y > 0.
        let outer0 = faces(&solid)
            .find(|f| origins.get(surface_key(f)).unwrap().role == side(1, 0))
            .unwrap();
        let ys: Vec<f64> = outer0.vertex_iter().map(|v| v.point().y).collect();
        assert!(ys.iter().any(|y| y.abs() < 1e-9), "{ys:?}");
        assert!(ys.iter().all(|&y| y > -1e-9), "{ys:?}");
    }

    #[test]
    fn revolve_axis_on_a_profile_point_does_not_panic() {
        // Profile touching the axis: no crash, every face gets some name.
        let (_, _, frame, _) = setup();
        let points = vec![[0.0, 0.0], [3.0, 0.0], [3.0, 2.0], [0.0, 2.0]];
        let profiles = vec![Profile {
            points: points.clone(),
        }];
        let region = Region {
            outer: points,
            holes: Vec::new(),
        };
        let vertices: Vec<_> = region
            .outer
            .iter()
            .map(|p| builder::vertex(frame.origin + frame.u * p[0] + frame.v * p[1]))
            .collect();
        let wire: Wire = (0..4)
            .map(|i| builder::line(&vertices[i], &vertices[(i + 1) % 4]))
            .collect();
        let face = builder::try_attach_plane(&[wire]).unwrap();
        let angle = std::f64::consts::FRAC_PI_2;
        let solid = builder::rsweep(
            &face,
            Point3::new(0.0, 0.0, 0.0),
            Vector3::unit_z(),
            Rad(angle),
        );
        let axis = RevolveAxis {
            origin: Point3::new(0.0, 0.0, 0.0),
            direction: Vector3::unit_z(),
            angle,
        };
        let origins = revolve_tool(FeatureId(1), &profiles, &region, &frame, axis, &solid);
        for f in faces(&solid) {
            assert!(origins.get(surface_key(&f)).is_some());
        }
        // A profile lying entirely on the axis has no angle reference.
        let on_axis = Region {
            outer: vec![[0.0, 0.0], [0.0, 1.0], [0.0, 2.0]],
            holes: Vec::new(),
        };
        let none = revolve_tool(FeatureId(1), &profiles, &on_axis, &frame, axis, &solid);
        assert!(faces(&solid).all(|f| none.get(surface_key(&f)).is_none()));
    }

    #[test]
    fn caps_of_an_untraceable_region_stay_unknown() {
        // Profiles that don't contain the region: nothing may be named
        // after a guessed loop, so caps (and sides) are unknown and unique.
        let (_, region, frame, face) = setup();
        let solid = builder::tsweep(&face, frame.normal * 2.0);
        let origins = extrude_tool(FeatureId(1), &[], &region, &frame, false, &solid);
        let all = roles(&origins, &solid);
        assert!(
            all.iter().all(|r| matches!(r, FaceRole::Unknown { .. })),
            "{all:?}"
        );
        let unique: BTreeSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len());
    }

    #[test]
    fn sketch_edges_match_cleaned_and_reversed_segments() {
        let profiles = vec![
            Profile {
                points: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]],
            },
            Profile {
                points: vec![[5.0, 5.0], [6.0, 5.0], [6.0, 6.0]],
            },
        ];
        let e = |profile, segment| SketchEdge { profile, segment };
        assert_eq!(
            sketch_edge(&profiles, [1.0, 0.0], [0.0, 0.0]),
            Some(e(0, 0))
        );
        // After the repeated point is dropped, (1,0)-(1,1) is segment 2.
        assert_eq!(
            sketch_edge(&profiles, [1.0, 0.0], [1.0, 1.0]),
            Some(e(0, 2))
        );
        // Closing segment, implicit in profile 1.
        assert_eq!(
            sketch_edge(&profiles, [5.0, 5.0], [6.0, 6.0]),
            Some(e(1, 2))
        );
        assert_eq!(sketch_edge(&profiles, [9.0, 9.0], [6.0, 6.0]), None);
        assert_eq!(sketch_edge(&[], [0.0, 0.0], [1.0, 0.0]), None);
    }
}
