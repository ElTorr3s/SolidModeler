//! Sparse symmetric positive definite solves for the solver's normal
//! equations, using a skyline (variable band) Cholesky factorization after
//! reverse Cuthill–McKee reordering.
//!
//! Sketch equations each touch a handful of variables, so `J W⁻¹ Jᵀ` is
//! very sparse, and chains and loops of curves reorder to a narrow band.
//! Factoring then costs about `n·b²` for bandwidth `b` instead of `n³`.

/// A symmetric matrix stored as its lower skyline in reordered positions.
/// The sparsity pattern is fixed at construction; values can be refilled.
#[derive(Debug, Clone)]
pub(crate) struct Skyline {
    /// Position → original index.
    perm: Vec<usize>,
    /// Original index → position.
    pos: Vec<usize>,
    /// First stored column of each row (in positions). Row `i` stores
    /// columns `first[i]..=i`.
    first: Vec<usize>,
    /// Offset of each row's first stored value in `vals`.
    start: Vec<usize>,
    vals: Vec<f64>,
}

impl Skyline {
    /// A zero matrix with room for every entry in `adjacency`, which lists
    /// for each index the other indices it couples to (symmetric).
    pub(crate) fn new(adjacency: &[Vec<usize>]) -> Self {
        let n = adjacency.len();
        let perm = reverse_cuthill_mckee(adjacency);
        let mut pos = vec![0; n];
        for (p, &i) in perm.iter().enumerate() {
            pos[i] = p;
        }
        let first: Vec<usize> = (0..n)
            .map(|p| {
                adjacency[perm[p]]
                    .iter()
                    .map(|&j| pos[j])
                    .filter(|&q| q < p)
                    .min()
                    .unwrap_or(p)
            })
            .collect();
        let mut start = Vec::with_capacity(n);
        let mut len = 0;
        for (p, &f) in first.iter().enumerate() {
            start.push(len);
            len += p - f + 1;
        }
        Skyline {
            perm,
            pos,
            first,
            start,
            vals: vec![0.0; len],
        }
    }

    /// Stored entries, roughly the memory and the factorization's work.
    #[cfg(test)]
    pub(crate) fn profile(&self) -> usize {
        self.vals.len()
    }

    /// Factorization cost estimate: the sum of squared row lengths.
    pub(crate) fn work(&self) -> f64 {
        (0..self.first.len())
            .map(|p| {
                let l = (p - self.first[p] + 1) as f64;
                l * l
            })
            .sum()
    }

    pub(crate) fn clear(&mut self) {
        self.vals.iter_mut().for_each(|v| *v = 0.0);
    }

    fn index(&self, r: usize, c: usize) -> usize {
        debug_assert!(c <= r && c >= self.first[r]);
        self.start[r] + c - self.first[r]
    }

    /// Adds `v` to entry `(i, j)` (and so `(j, i)`), in original indices.
    /// The entry must be in the pattern: `i == j` or `j` adjacent to `i`.
    pub(crate) fn add(&mut self, i: usize, j: usize, v: f64) {
        let (a, b) = (self.pos[i], self.pos[j]);
        let k = if a >= b {
            self.index(a, b)
        } else {
            self.index(b, a)
        };
        self.vals[k] += v;
    }

    pub(crate) fn add_to_diagonal(&mut self, v: f64) {
        for p in 0..self.first.len() {
            let k = self.index(p, p);
            self.vals[k] += v;
        }
    }

    pub(crate) fn diagonal_sum(&self) -> f64 {
        (0..self.first.len())
            .map(|p| self.vals[self.index(p, p)])
            .sum()
    }

    /// Replaces the matrix by its Cholesky factor `L` in place. `false` if
    /// the matrix isn't numerically positive definite.
    pub(crate) fn factor(&mut self) -> bool {
        let n = self.first.len();
        for i in 0..n {
            for j in self.first[i]..=i {
                let lo = self.first[i].max(self.first[j]);
                let (ri, rj) = (self.index(i, lo), self.index(j, lo));
                let dot: f64 = (0..j - lo)
                    .map(|k| self.vals[ri + k] * self.vals[rj + k])
                    .sum();
                let s = self.vals[self.index(i, j)] - dot;
                if i == j {
                    if !(s > 0.0 && s.is_finite()) {
                        return false;
                    }
                    let k = self.index(i, i);
                    self.vals[k] = s.sqrt();
                } else {
                    let k = self.index(i, j);
                    self.vals[k] = s / self.vals[self.index(j, j)];
                }
            }
        }
        true
    }

    /// Solves `A x = b` with a matrix [`Skyline::factor`] has factored.
    pub(crate) fn solve(&self, b: &[f64]) -> Vec<f64> {
        let n = self.first.len();
        let mut y: Vec<f64> = self.perm.iter().map(|&i| b[i]).collect();
        for i in 0..n {
            let f = self.first[i];
            let row = self.index(i, f);
            let dot: f64 = (f..i).map(|k| self.vals[row + k - f] * y[k]).sum();
            y[i] = (y[i] - dot) / self.vals[self.index(i, i)];
        }
        for i in (0..n).rev() {
            y[i] /= self.vals[self.index(i, i)];
            let f = self.first[i];
            let row = self.index(i, f);
            for k in f..i {
                y[k] -= self.vals[row + k - f] * y[i];
            }
        }
        let mut x = vec![0.0; n];
        for (p, &i) in self.perm.iter().enumerate() {
            x[i] = y[p];
        }
        x
    }
}

/// An ordering that keeps coupled indices close together, so the skyline
/// stays narrow. Each connected component starts from a lowest-degree node.
fn reverse_cuthill_mckee(adjacency: &[Vec<usize>]) -> Vec<usize> {
    let n = adjacency.len();
    let degree = |i: usize| adjacency[i].len();
    let mut by_degree: Vec<usize> = (0..n).collect();
    by_degree.sort_by_key(|&i| (degree(i), i));
    let mut seen = vec![false; n];
    let mut order = Vec::with_capacity(n);
    let mut neighbours = Vec::new();
    for &root in &by_degree {
        if seen[root] {
            continue;
        }
        seen[root] = true;
        let mut head = order.len();
        order.push(root);
        while head < order.len() {
            let i = order[head];
            head += 1;
            neighbours.clear();
            neighbours.extend(adjacency[i].iter().copied().filter(|&j| !seen[j]));
            neighbours.sort_by_key(|&j| (degree(j), j));
            neighbours.dedup();
            for &j in &neighbours {
                seen[j] = true;
                order.push(j);
            }
        }
    }
    order.reverse();
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a random-ish sparse SPD matrix, then checks the solve against
    /// a direct multiply.
    #[test]
    fn solves_sparse_spd_systems() {
        let n = 30;
        // A ring plus a few chords, scrambled so the ordering has work to do.
        let mut edges: Vec<(usize, usize)> = (0..n).map(|i| (i, (i + 1) % n)).collect();
        edges.extend([(0, 15), (3, 22), (7, 8), (10, 29)]);
        let scramble = |i: usize| (i * 7) % n;
        let mut adjacency = vec![Vec::new(); n];
        let mut dense = vec![0.0; n * n];
        for &(a, b) in &edges {
            let (a, b) = (scramble(a), scramble(b));
            if a == b {
                continue;
            }
            adjacency[a].push(b);
            adjacency[b].push(a);
            let w = 0.1 + ((a * 31 + b * 17) % 10) as f64 * 0.1;
            dense[a * n + b] -= w;
            dense[b * n + a] -= w;
            dense[a * n + a] += w;
            dense[b * n + b] += w;
        }
        for i in 0..n {
            dense[i * n + i] += 1.0;
        }
        let mut sky = Skyline::new(&adjacency);
        for i in 0..n {
            for j in 0..=i {
                if dense[i * n + j] != 0.0 {
                    sky.add(i, j, dense[i * n + j]);
                }
            }
        }
        assert!(sky.profile() < n * (n + 1) / 2, "ordering keeps it sparse");
        let b: Vec<f64> = (0..n).map(|i| (i as f64).sin()).collect();
        assert!(sky.factor());
        let x = sky.solve(&b);
        for i in 0..n {
            let ax: f64 = (0..n).map(|j| dense[i * n + j] * x[j]).sum();
            assert!((ax - b[i]).abs() < 1e-10, "row {i}: {ax} vs {}", b[i]);
        }
    }

    #[test]
    fn rejects_indefinite_matrices() {
        let adjacency = vec![vec![1], vec![0]];
        let mut sky = Skyline::new(&adjacency);
        sky.add(0, 0, 1.0);
        sky.add(1, 1, 1.0);
        sky.add(1, 0, 2.0);
        assert!(!sky.factor());
    }

    #[test]
    fn chain_has_narrow_profile() {
        let n = 1000;
        let adjacency: Vec<Vec<usize>> = (0..n)
            .map(|i| {
                let mut v = Vec::new();
                if i > 0 {
                    v.push(i - 1);
                }
                if i + 1 < n {
                    v.push(i + 1);
                }
                v
            })
            .collect();
        let sky = Skyline::new(&adjacency);
        assert!(sky.profile() <= 2 * n);
    }
}
