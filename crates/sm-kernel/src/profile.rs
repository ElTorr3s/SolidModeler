//! 2D sketch loops: cleaning, validation, and grouping into regions with holes.

use sm_document::Profile;

pub type Point2 = [f64; 2];

/// Why a sketch's loops can't be turned into regions.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProfileError {
    #[error("loop {0} has fewer than 3 distinct points")]
    TooFewPoints(usize),
    #[error("loop {0} encloses no area")]
    ZeroArea(usize),
    #[error("loop {0} crosses itself")]
    SelfIntersecting(usize),
    #[error("loops {0} and {1} cross or touch")]
    LoopsIntersect(usize, usize),
}

/// A face to extrude: an outer loop (counter-clockwise) and any holes in it
/// (clockwise). Both orientations keep the material on the left of each edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub outer: Vec<Point2>,
    pub holes: Vec<Vec<Point2>>,
}

/// Cleans every loop, rejects bad geometry, and groups the loops into regions
/// by nesting depth: a loop inside an odd number of others is a hole in its
/// innermost container, as in Inventor's profile selection.
/// `eps` is the linear tolerance in sketch units.
pub fn regions(profiles: &[Profile], eps: f64) -> Result<Vec<Region>, ProfileError> {
    let loops = profiles
        .iter()
        .enumerate()
        .map(|(i, p)| clean(i, &p.points, eps))
        .collect::<Result<Vec<_>, _>>()?;

    let segments: Vec<_> = loops
        .iter()
        .enumerate()
        .flat_map(|(l, points)| (0..points.len()).map(move |i| (l, edge(points, i))))
        .collect();
    let segs: Vec<_> = segments.iter().map(|&(_, seg)| seg).collect();
    if let Some((a, b)) = touching_pair(&segs, eps, |a, b| segments[a].0 == segments[b].0) {
        let (i, j) = (segments[a].0, segments[b].0);
        return Err(ProfileError::LoopsIntersect(i.min(j), i.max(j)));
    }

    // Loops don't touch, so one vertex decides containment for the whole loop.
    let containers: Vec<Vec<usize>> = (0..loops.len())
        .map(|i| {
            (0..loops.len())
                .filter(|&j| j != i && contains(&loops[j], loops[i][0]))
                .collect()
        })
        .collect();
    let depth = |i: usize| containers[i].len();

    let mut regions = Vec::new();
    let mut region_of = vec![None; loops.len()];
    for i in (0..loops.len()).filter(|&i| depth(i) % 2 == 0) {
        region_of[i] = Some(regions.len());
        regions.push(Region {
            outer: oriented(&loops[i], true),
            holes: Vec::new(),
        });
    }
    for i in (0..loops.len()).filter(|&i| depth(i) % 2 == 1) {
        let parent = containers[i]
            .iter()
            .copied()
            .find(|&j| depth(j) == depth(i) - 1)
            .expect("an odd-depth loop has a container one level up");
        let region = region_of[parent].expect("even-depth loops are regions");
        regions[region].holes.push(oriented(&loops[i], false));
    }
    Ok(regions)
}

/// Drops repeated points (including a closing point equal to the first) and
/// checks that what's left is a simple polygon with area.
fn clean(index: usize, points: &[Point2], eps: f64) -> Result<Vec<Point2>, ProfileError> {
    let mut out: Vec<Point2> = Vec::with_capacity(points.len());
    for &p in points {
        if out.last().is_none_or(|&q| dist(p, q) > eps) {
            out.push(p);
        }
    }
    while out.len() > 1 && dist(out[0], *out.last().unwrap()) <= eps {
        out.pop();
    }
    if out.len() < 3 {
        return Err(ProfileError::TooFewPoints(index));
    }
    let n = out.len();
    let segments: Vec<_> = (0..n).map(|i| edge(&out, i)).collect();
    // Neighbouring edges share an endpoint, so they always "touch".
    let adjacent = |a: usize, b: usize| a.abs_diff(b) == 1 || a.abs_diff(b) == n - 1;
    if touching_pair(&segments, eps, adjacent).is_some() {
        return Err(ProfileError::SelfIntersecting(index));
    }
    if signed_area(&out).abs() <= eps * perimeter(&out) {
        return Err(ProfileError::ZeroArea(index));
    }
    Ok(out)
}

pub fn signed_area(points: &[Point2]) -> f64 {
    (0..points.len())
        .map(|i| {
            let ([x0, y0], [x1, y1]) = edge(points, i);
            x0 * y1 - x1 * y0
        })
        .sum::<f64>()
        / 2.0
}

fn oriented(points: &[Point2], counter_clockwise: bool) -> Vec<Point2> {
    let mut points = points.to_vec();
    if (signed_area(&points) > 0.0) != counter_clockwise {
        points.reverse();
    }
    points
}

pub fn edge(points: &[Point2], i: usize) -> (Point2, Point2) {
    (points[i], points[(i + 1) % points.len()])
}

fn perimeter(points: &[Point2]) -> f64 {
    (0..points.len())
        .map(|i| {
            let (a, b) = edge(points, i);
            dist(a, b)
        })
        .sum()
}

fn dist(a: Point2, b: Point2) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Finds two segments that touch, ignoring pairs for which `skip` is true.
/// A sweep over x-sorted bounding boxes keeps this close to O(n log n) for
/// typical sketches instead of comparing every pair.
fn touching_pair(
    segments: &[(Point2, Point2)],
    tol: f64,
    skip: impl Fn(usize, usize) -> bool,
) -> Option<(usize, usize)> {
    let bounds = |(a, b): (Point2, Point2)| {
        (
            a[0].min(b[0]),
            a[0].max(b[0]),
            a[1].min(b[1]),
            a[1].max(b[1]),
        )
    };
    let mut order: Vec<usize> = (0..segments.len()).collect();
    order.sort_by(|&i, &j| bounds(segments[i]).0.total_cmp(&bounds(segments[j]).0));
    for (k, &i) in order.iter().enumerate() {
        let (_, x1, y0, y1) = bounds(segments[i]);
        for &j in &order[k + 1..] {
            let (jx0, _, jy0, jy1) = bounds(segments[j]);
            if jx0 > x1 + tol {
                break;
            }
            if jy0 > y1 + tol || y0 > jy1 + tol || skip(i, j) {
                continue;
            }
            if segments_touch(segments[i], segments[j], tol) {
                return Some((i, j));
            }
        }
    }
    None
}

/// Even-odd point-in-polygon test.
fn contains(polygon: &[Point2], [x, y]: Point2) -> bool {
    let mut inside = false;
    for i in 0..polygon.len() {
        let ([x0, y0], [x1, y1]) = edge(polygon, i);
        if (y0 > y) != (y1 > y) && x < x0 + (y - y0) / (y1 - y0) * (x1 - x0) {
            inside = !inside;
        }
    }
    inside
}

/// True if two closed segments intersect or come within tolerance.
fn segments_touch((a, b): (Point2, Point2), (c, d): (Point2, Point2), eps: f64) -> bool {
    let cross = |o: Point2, p: Point2, q: Point2| {
        (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0])
    };
    let (d1, d2) = (cross(a, b, c), cross(a, b, d));
    let (d3, d4) = (cross(c, d, a), cross(c, d, b));
    if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
        return true;
    }
    point_segment_dist(c, a, b) <= eps
        || point_segment_dist(d, a, b) <= eps
        || point_segment_dist(a, c, d) <= eps
        || point_segment_dist(b, c, d) <= eps
}

fn point_segment_dist(p: Point2, a: Point2, b: Point2) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0)
    };
    dist(p, [a[0] + ab[0] * t, a[1] + ab[1] * t])
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-6;

    fn square(x: f64, y: f64, size: f64) -> Profile {
        Profile {
            points: vec![[x, y], [x + size, y], [x + size, y + size], [x, y + size]],
        }
    }

    #[test]
    fn closing_and_repeated_points_are_dropped() {
        let p = Profile {
            points: vec![
                [0.0, 0.0],
                [1.0, 0.0],
                [1.0, 0.0],
                [1.0, 1.0],
                [0.0, 1.0],
                [0.0, 0.0],
            ],
        };
        let r = regions(&[p], EPS).unwrap();
        assert_eq!(r[0].outer.len(), 4);
    }

    #[test]
    fn rejects_bad_loops() {
        let line = Profile {
            points: vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]],
        };
        assert_eq!(regions(&[line], EPS), Err(ProfileError::ZeroArea(0)));
        let bow_tie = Profile {
            points: vec![[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 1.0]],
        };
        assert_eq!(
            regions(&[bow_tie], EPS),
            Err(ProfileError::SelfIntersecting(0))
        );
        let two = Profile {
            points: vec![[0.0, 0.0], [1.0, 0.0]],
        };
        assert_eq!(regions(&[two], EPS), Err(ProfileError::TooFewPoints(0)));
        assert_eq!(
            regions(&[square(0.0, 0.0, 2.0), square(1.0, 1.0, 2.0)], EPS),
            Err(ProfileError::LoopsIntersect(0, 1))
        );
    }

    #[test]
    fn large_loops_are_fast() {
        let n = crate::MAX_SKETCH_POINTS / 2;
        let circle = |r: f64| Profile {
            points: (0..n)
                .map(|i| {
                    let a = i as f64 / n as f64 * std::f64::consts::TAU;
                    [r * a.cos(), r * a.sin()]
                })
                .collect(),
        };
        let start = std::time::Instant::now();
        let r = regions(&[circle(10.0), circle(5.0)], EPS).unwrap();
        assert_eq!(r[0].holes.len(), 1);
        // Pairwise checks took seconds here; the sweep takes milliseconds.
        assert!(start.elapsed().as_secs_f64() < 1.0, "{:?}", start.elapsed());
    }

    #[test]
    fn nested_loops_become_holes_and_islands() {
        // Frame with an island inside the hole, plus a separate square.
        let r = regions(
            &[
                square(0.0, 0.0, 10.0),
                square(2.0, 2.0, 6.0),
                square(4.0, 4.0, 2.0),
                square(20.0, 0.0, 1.0),
            ],
            EPS,
        )
        .unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].holes.len(), 1);
        assert!(signed_area(&r[0].outer) > 0.0);
        assert!(signed_area(&r[0].holes[0]) < 0.0);
        assert!(r[1].holes.is_empty() && r[2].holes.is_empty());
    }
}
