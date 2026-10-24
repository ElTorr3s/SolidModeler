//! Numerical core: splits the equations into independent clusters, solves
//! each with a damped minimum-norm Newton method, and inspects the Jacobian
//! for degrees of freedom and redundant equations.

use crate::equations::Equation;

/// Equations that share variables, and the unknowns they read. Clusters are
/// solved and analysed on their own, which keeps the dense linear algebra
/// small even in large sketches.
#[derive(Debug, Default)]
pub(crate) struct Cluster {
    pub eqs: Vec<usize>,
    pub vars: Vec<usize>,
}

/// Groups equations that share an unknown. An equation that reads no
/// unknowns gets a cluster of its own with no variables.
pub(crate) fn clusters(eqs: &[Equation], unknown: &[bool]) -> Vec<Cluster> {
    let mut parent: Vec<usize> = (0..unknown.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for eq in eqs {
        let mut vars = eq.vars.iter().copied().filter(|&v| unknown[v]);
        if let Some(first) = vars.next() {
            let root = find(&mut parent, first);
            for v in vars {
                let r = find(&mut parent, v);
                parent[r] = root;
            }
        }
    }

    let mut cluster_of_root = vec![usize::MAX; unknown.len()];
    let mut out: Vec<Cluster> = Vec::new();
    for (e, eq) in eqs.iter().enumerate() {
        match eq.vars.iter().copied().find(|&v| unknown[v]) {
            Some(v) => {
                let root = find(&mut parent, v);
                if cluster_of_root[root] == usize::MAX {
                    cluster_of_root[root] = out.len();
                    out.push(Cluster::default());
                }
                out[cluster_of_root[root]].eqs.push(e);
            }
            None => out.push(Cluster {
                eqs: vec![e],
                vars: Vec::new(),
            }),
        }
    }
    for v in (0..unknown.len()).filter(|&v| unknown[v]) {
        let root = find(&mut parent, v);
        if cluster_of_root[root] != usize::MAX {
            out[cluster_of_root[root]].vars.push(v);
        }
    }
    out
}

/// Dense row-major Jacobian of a cluster: one row per equation, one column
/// per cluster variable.
fn jacobian(x: &[f64], eqs: &[Equation], cluster: &Cluster, col: &[usize]) -> Vec<f64> {
    let n = cluster.vars.len();
    let mut j = vec![0.0; cluster.eqs.len() * n];
    for (r, &e) in cluster.eqs.iter().enumerate() {
        eqs[e].gradient(x, |v| col[v] != usize::MAX, |v, d| j[r * n + col[v]] += d);
    }
    j
}

/// Maps each sketch variable to its column in `cluster`, `usize::MAX` if it
/// isn't one. Reuses `col`, which must be all `usize::MAX` on entry and is
/// left that way by [`clear_columns`].
fn set_columns(col: &mut [usize], cluster: &Cluster) {
    for (c, &v) in cluster.vars.iter().enumerate() {
        col[v] = c;
    }
}

fn clear_columns(col: &mut [usize], cluster: &Cluster) {
    for &v in &cluster.vars {
        col[v] = usize::MAX;
    }
}

pub(crate) struct SolveSettings {
    /// A cluster is solved when every residual is at most this, in model
    /// units (or radians-ish for angle equations).
    pub tolerance: f64,
    pub max_iterations: usize,
}

impl Default for SolveSettings {
    fn default() -> Self {
        SolveSettings {
            tolerance: 1e-10,
            max_iterations: 100,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ClusterOutcome {
    pub converged: bool,
    pub iterations: usize,
    pub max_residual: f64,
}

/// Solves one cluster in place with Levenberg–Marquardt steps taken in
/// weighted minimum-norm form:
///
/// `dx = −W⁻¹Jᵀ (J W⁻¹ Jᵀ + λI)⁻¹ f`
///
/// Among all steps that fix the residuals, this one moves the variables the
/// least (weighted by `weight`, higher meaning stiffer), so geometry the
/// constraints don't pin stays where the user drew it, as in Inventor.
pub(crate) fn solve_cluster(
    x: &mut [f64],
    eqs: &[Equation],
    cluster: &Cluster,
    weight: &[f64],
    col: &mut [usize],
    settings: &SolveSettings,
) -> ClusterOutcome {
    let residuals =
        |x: &[f64]| -> Vec<f64> { cluster.eqs.iter().map(|&e| eqs[e].residual(x)).collect() };
    // NaN (from degenerate geometry such as a zero-length line) must never
    // read as solved, and `f64::max` would silently drop it.
    let max_abs = |f: &[f64]| {
        f.iter().fold(0.0f64, |m, v| {
            if v.is_finite() {
                m.max(v.abs())
            } else {
                f64::INFINITY
            }
        })
    };
    let sq = |f: &[f64]| f.iter().map(|v| v * v).sum::<f64>();

    let mut f = residuals(x);
    if cluster.vars.is_empty() {
        let max_residual = max_abs(&f);
        return ClusterOutcome {
            converged: max_residual <= settings.tolerance,
            iterations: 0,
            max_residual,
        };
    }

    set_columns(col, cluster);
    let m = cluster.eqs.len();
    let n = cluster.vars.len();
    let inv_w: Vec<f64> = cluster.vars.iter().map(|&v| 1.0 / weight[v]).collect();
    let mut mu = 1e-9;
    let mut iterations = 0;
    let mut trial = x.to_vec();

    while max_abs(&f) > settings.tolerance && iterations < settings.max_iterations {
        iterations += 1;
        let j = jacobian(x, eqs, cluster, col);
        // A = J W⁻¹ Jᵀ, symmetric positive semi-definite.
        let mut a = vec![0.0; m * m];
        for r in 0..m {
            for s in 0..=r {
                let dot: f64 = (0..n).map(|c| j[r * n + c] * inv_w[c] * j[s * n + c]).sum();
                a[r * m + s] = dot;
                a[s * m + r] = dot;
            }
        }
        let scale = ((0..m).map(|r| a[r * m + r]).sum::<f64>() / m as f64).max(1e-12);

        let f_sq = sq(&f);
        let mut improved = false;
        while mu < 1e8 {
            let mut damped = a.clone();
            for r in 0..m {
                damped[r * m + r] += mu * scale;
            }
            let Some(y) = cholesky_solve(&mut damped, m, &f) else {
                mu *= 10.0;
                continue;
            };
            trial.copy_from_slice(x);
            for (c, &v) in cluster.vars.iter().enumerate() {
                let step: f64 = (0..m).map(|r| j[r * n + c] * y[r]).sum();
                trial[v] -= inv_w[c] * step;
            }
            let f_trial = residuals(&trial);
            if sq(&f_trial) < f_sq {
                x.copy_from_slice(&trial);
                f = f_trial;
                mu = (mu * 0.1).max(1e-12);
                improved = true;
                break;
            }
            mu *= 10.0;
        }
        if !improved {
            break;
        }
    }

    clear_columns(col, cluster);
    let max_residual = max_abs(&f);
    ClusterOutcome {
        converged: max_residual <= settings.tolerance,
        iterations,
        max_residual,
    }
}

/// Solves `A y = b` for symmetric positive definite `A` (row-major, `n×n`),
/// factoring `A` in place. `None` if `A` isn't numerically positive definite.
fn cholesky_solve(a: &mut [f64], n: usize, b: &[f64]) -> Option<Vec<f64>> {
    for i in 0..n {
        for k in 0..=i {
            let mut s = a[i * n + k];
            for p in 0..k {
                s -= a[i * n + p] * a[k * n + p];
            }
            if i == k {
                if s <= 0.0 || !s.is_finite() {
                    return None;
                }
                a[i * n + i] = s.sqrt();
            } else {
                a[i * n + k] = s / a[k * n + k];
            }
        }
    }
    let mut y = b.to_vec();
    for i in 0..n {
        for p in 0..i {
            y[i] -= a[i * n + p] * y[p];
        }
        y[i] /= a[i * n + i];
    }
    for i in (0..n).rev() {
        for p in i + 1..n {
            y[i] -= a[p * n + i] * y[p];
        }
        y[i] /= a[i * n + i];
    }
    Some(y)
}

/// Structural facts about one cluster at the current variable values.
#[derive(Debug, Default)]
pub(crate) struct ClusterAnalysis {
    /// Independent equations, which is also the number of variables they fix.
    pub rank: usize,
    /// Cluster variables that can still move without breaking any equation.
    pub free_vars: Vec<usize>,
    /// Equations (indices into the sketch's equation list) that add nothing
    /// to the ones before them, each with the earlier equations it depends on.
    pub redundant: Vec<(usize, Vec<usize>)>,
}

/// Relative size below which a Jacobian row counts as dependent on earlier
/// rows. Equations are scaled so their gradients are of order one.
const RANK_TOLERANCE: f64 = 1e-8;

/// Finds the rank, free variables and redundant equations of a cluster by
/// Gram–Schmidt over its Jacobian rows in `order` (indices into
/// `cluster.eqs`). Earlier rows win, so the equation reported as redundant is
/// the later one, matching Inventor blaming the newest constraint.
pub(crate) fn analyse_cluster(
    x: &[f64],
    eqs: &[Equation],
    cluster: &Cluster,
    order: &[usize],
    col: &mut [usize],
) -> ClusterAnalysis {
    let n = cluster.vars.len();
    if n == 0 {
        // No unknowns: every equation here is either trivially redundant or
        // impossible; both read as redundant.
        return ClusterAnalysis {
            redundant: cluster.eqs.iter().map(|&e| (e, Vec::new())).collect(),
            ..Default::default()
        };
    }
    set_columns(col, cluster);
    let j = jacobian(x, eqs, cluster, col);
    clear_columns(col, cluster);

    // Orthonormal basis of the row space so far. `combo[k]` writes basis
    // vector `k` in terms of the accepted rows, so a dependent row can be
    // traced back to the equations it repeats.
    let mut basis: Vec<Vec<f64>> = Vec::new();
    let mut combo: Vec<Vec<f64>> = Vec::new();
    let mut accepted: Vec<usize> = Vec::new();
    let mut out = ClusterAnalysis::default();

    for &r in order {
        let row = &j[r * n..(r + 1) * n];
        let norm = row.iter().map(|v| v * v).sum::<f64>().sqrt();
        let mut v = row.to_vec();
        let mut coeff = vec![0.0; basis.len()];
        // Two passes of modified Gram–Schmidt keep the basis orthogonal.
        for _ in 0..2 {
            for (k, q) in basis.iter().enumerate() {
                let p: f64 = v.iter().zip(q).map(|(a, b)| a * b).sum();
                coeff[k] += p;
                for (vi, qi) in v.iter_mut().zip(q) {
                    *vi -= p * qi;
                }
            }
        }
        let rest = v.iter().map(|v| v * v).sum::<f64>().sqrt();
        // A non-finite row comes from degenerate geometry; it fixes nothing.
        if !norm.is_finite() || norm < 1e-12 || rest <= RANK_TOLERANCE * norm {
            // row ≈ Σ coeff[k]·basis[k] = Σ_k coeff[k] Σ_a combo[k][a]·accepted[a]
            let mut weights = vec![0.0; accepted.len()];
            for (k, c) in coeff.iter().enumerate() {
                for (a, w) in combo[k].iter().enumerate() {
                    weights[a] += c * w;
                }
            }
            let depends = accepted
                .iter()
                .zip(&weights)
                .filter(|(_, w)| w.abs() > 1e-6)
                .map(|(&a, _)| cluster.eqs[a])
                .collect();
            out.redundant.push((cluster.eqs[r], depends));
            continue;
        }
        // New basis vector q = (row − Σ coeff[k]·basis[k]) / rest.
        let mut new_combo = vec![0.0; accepted.len() + 1];
        for (k, c) in coeff.iter().enumerate() {
            for (a, w) in combo[k].iter().enumerate() {
                new_combo[a] -= c * w / rest;
            }
        }
        new_combo[accepted.len()] = 1.0 / rest;
        for vi in v.iter_mut() {
            *vi /= rest;
        }
        basis.push(v);
        combo.push(new_combo);
        accepted.push(r);
    }

    out.rank = basis.len();
    // A variable is pinned when its unit vector lies in the row space.
    out.free_vars = cluster
        .vars
        .iter()
        .enumerate()
        .filter(|&(c, _)| 1.0 - basis.iter().map(|q| q[c] * q[c]).sum::<f64>() > 1e-9)
        .map(|(_, &v)| v)
        .collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::equations::{EqKind, Owner};

    fn linear(vars: Vec<usize>, coeffs: Vec<f64>, constant: f64) -> Equation {
        Equation {
            kind: EqKind::Linear { coeffs, constant },
            vars,
            owner: Owner::Constraint(0),
        }
    }

    #[test]
    fn clusters_split_on_shared_unknowns() {
        let eqs = vec![
            linear(vec![0, 1], vec![1.0, -1.0], 0.0),
            linear(vec![2], vec![1.0], 3.0),
            linear(vec![1, 3], vec![1.0, 1.0], 0.0),
            linear(vec![4], vec![1.0], 0.0),
        ];
        let unknown = [true, true, true, true, false];
        let cs = clusters(&eqs, &unknown);
        assert_eq!(cs.len(), 3);
        assert_eq!(cs[0].eqs, vec![0, 2]);
        assert_eq!(cs[0].vars, vec![0, 1, 3]);
        assert_eq!(cs[1].vars, vec![2]);
        assert!(cs[2].vars.is_empty());
    }

    #[test]
    fn cholesky_solves_spd_system() {
        let mut a = vec![4.0, 2.0, 2.0, 3.0];
        let y = cholesky_solve(&mut a, 2, &[2.0, 1.0]).unwrap();
        assert!((4.0 * y[0] + 2.0 * y[1] - 2.0).abs() < 1e-12);
        assert!((2.0 * y[0] + 3.0 * y[1] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn analysis_traces_dependent_rows() {
        // x0 + x1 = 0, x1 = 1, x0 = −1 (dependent on both).
        let eqs = vec![
            linear(vec![0, 1], vec![1.0, 1.0], 0.0),
            linear(vec![1], vec![1.0], 1.0),
            linear(vec![0], vec![1.0], -1.0),
        ];
        let cs = clusters(&eqs, &[true, true]);
        let mut col = vec![usize::MAX; 2];
        let a = analyse_cluster(&[0.0, 0.0], &eqs, &cs[0], &[0, 1, 2], &mut col);
        assert_eq!(a.rank, 2);
        assert!(a.free_vars.is_empty());
        assert_eq!(a.redundant, vec![(2, vec![0, 1])]);
    }
}
