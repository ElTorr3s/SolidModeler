//! Numerical core: splits the equations into independent clusters, solves
//! each with a damped minimum-norm Newton method, and inspects the Jacobian
//! for degrees of freedom and redundant equations.

use crate::equations::Equation;
use crate::sparse::Skyline;

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
    /// units (or radians for angle equations).
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

/// Most factorization work (see [`Skyline::work`]) one cluster may need per
/// step, about a tenth of a second. Sketches past it are refused rather than
/// freezing the program: ordinary profiles are far below it.
pub(crate) const MAX_FACTOR_WORK: f64 = 2e8;

/// Most work the analysis may do on one cluster.
pub(crate) const MAX_ANALYSIS_WORK: f64 = 2e9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ClusterOutcome {
    pub converged: bool,
    pub iterations: usize,
    pub max_residual: f64,
    /// The cluster was over the work budget and left unsolved.
    pub too_large: bool,
}

/// Sparse Jacobian rows of a cluster as `(column, ∂)` pairs sorted by
/// column. Every unknown an equation reads gets an entry, even when its
/// derivative is zero, so the pattern only depends on the equations.
fn sparse_rows(
    x: &[f64],
    eqs: &[Equation],
    cluster: &Cluster,
    col: &[usize],
) -> Vec<Vec<(usize, f64)>> {
    cluster
        .eqs
        .iter()
        .map(|&e| {
            let mut row: Vec<(usize, f64)> = Vec::new();
            eqs[e].gradient(x, |v| col[v] != usize::MAX, |v, d| row.push((col[v], d)));
            row.sort_by_key(|&(c, _)| c);
            row.dedup_by(|later, kept| {
                let same = later.0 == kept.0;
                if same {
                    kept.1 += later.1;
                }
                same
            });
            row
        })
        .collect()
}

/// For each column, the `(row, ∂)` entries of `rows` (indexed by position in
/// `rows`) that touch it.
fn columns_of(rows: &[&Vec<(usize, f64)>], n: usize) -> Vec<Vec<(usize, f64)>> {
    let mut cols = vec![Vec::new(); n];
    for (r, row) in rows.iter().enumerate() {
        for &(c, d) in row.iter() {
            cols[c].push((r, d));
        }
    }
    cols
}

/// Which rows share a column, the sparsity pattern of `J Jᵀ`.
fn row_adjacency(cols: &[Vec<(usize, f64)>], m: usize) -> Vec<Vec<usize>> {
    let mut adj = vec![Vec::new(); m];
    for entries in cols {
        for (a, &(r, _)) in entries.iter().enumerate() {
            for &(q, _) in &entries[..a] {
                if q != r {
                    adj[r].push(q);
                    adj[q].push(r);
                }
            }
        }
    }
    for list in &mut adj {
        list.sort_unstable();
        list.dedup();
    }
    adj
}

/// Fills `sky` with `J W⁻¹ Jᵀ` from the column view of `J`.
fn fill_gram(sky: &mut Skyline, cols: &[Vec<(usize, f64)>], inv_w: &[f64]) {
    sky.clear();
    for (c, entries) in cols.iter().enumerate() {
        for (a, &(r, dr)) in entries.iter().enumerate() {
            for &(q, dq) in &entries[..=a] {
                sky.add(r, q, dr * dq * inv_w[c]);
            }
        }
    }
}

/// Solves one cluster in place with Levenberg–Marquardt steps taken in
/// weighted minimum-norm form:
///
/// `dx = −W⁻¹Jᵀ (J W⁻¹ Jᵀ + λI)⁻¹ f`
///
/// Among all steps that fix the residuals, this one moves the variables the
/// least (weighted by `weight`, higher meaning stiffer), so geometry the
/// constraints don't pin stays where the user drew it, as in Inventor.
/// `J W⁻¹ Jᵀ` is factored sparsely, so the cost grows with the cluster's
/// bandwidth rather than the cube of its size.
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
    let outcome = |f: &[f64], iterations: usize, too_large: bool| {
        let max_residual = max_abs(f);
        ClusterOutcome {
            converged: !too_large && max_residual <= settings.tolerance,
            iterations,
            max_residual,
            too_large,
        }
    };
    if cluster.vars.is_empty() {
        return outcome(&f, 0, false);
    }

    set_columns(col, cluster);
    let m = cluster.eqs.len();
    let n = cluster.vars.len();
    let inv_w: Vec<f64> = cluster.vars.iter().map(|&v| 1.0 / weight[v]).collect();
    let mut rows = sparse_rows(x, eqs, cluster, col);
    let mut cols = columns_of(&rows.iter().collect::<Vec<_>>(), n);
    let mut gram = Skyline::new(&row_adjacency(&cols, m));
    if gram.work() > MAX_FACTOR_WORK {
        clear_columns(col, cluster);
        return outcome(&f, 0, true);
    }

    let mut mu = 1e-9;
    let mut iterations = 0;
    let saved_len = cluster.vars.len();
    let mut saved = vec![0.0; saved_len];

    // Once within tolerance, one more step "polishes" the result to near
    // machine precision, so errors don't add up along long chains of
    // geometry. It is kept only if it helps.
    let mut polished = false;
    while iterations < settings.max_iterations {
        if max_abs(&f) <= settings.tolerance {
            if polished || iterations == 0 {
                break;
            }
            polished = true;
        }
        iterations += 1;
        if iterations > 1 {
            rows = sparse_rows(x, eqs, cluster, col);
            cols = columns_of(&rows.iter().collect::<Vec<_>>(), n);
        }
        fill_gram(&mut gram, &cols, &inv_w);
        let scale = (gram.diagonal_sum() / m as f64).max(1e-12);

        let f_sq = sq(&f);
        let mut improved = false;
        while mu < 1e8 {
            let mut damped = gram.clone();
            damped.add_to_diagonal(mu * scale);
            if !damped.factor() {
                mu *= 10.0;
                continue;
            }
            let y = damped.solve(&f);
            for (c, &v) in cluster.vars.iter().enumerate() {
                saved[c] = x[v];
                let step: f64 = cols[c].iter().map(|&(r, d)| d * y[r]).sum();
                x[v] -= inv_w[c] * step;
            }
            let f_trial = residuals(x);
            if sq(&f_trial) < f_sq {
                f = f_trial;
                mu = (mu * 0.1).max(1e-12);
                improved = true;
                break;
            }
            for (c, &v) in cluster.vars.iter().enumerate() {
                x[v] = saved[c];
            }
            mu *= 10.0;
        }
        if !improved {
            break;
        }
    }

    clear_columns(col, cluster);
    outcome(&f, iterations, false)
}

/// How a row relates to a set of other rows of its cluster.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Dependency {
    /// It constrains something the others don't.
    Independent,
    /// It is a combination of the listed equations (indices into the
    /// sketch's equation list).
    Redundant(Vec<usize>),
    /// It has no usable slope here, as with a distance between two points
    /// in the same place.
    Degenerate,
    /// The cluster is over the work budget.
    TooLarge,
}

/// Decides whether row `target` (a position in `cluster.eqs`) is a linear
/// combination of the rows at positions `base`, by solving the regularised
/// least-squares problem `min |Jᵦᵀc − r|² + ε|c|²` with the sparse
/// factorization. Cheap enough to run on every constraint the user adds.
pub(crate) fn dependency(
    x: &[f64],
    eqs: &[Equation],
    cluster: &Cluster,
    col: &mut [usize],
    base: &[usize],
    target: usize,
) -> Dependency {
    set_columns(col, cluster);
    let rows = sparse_rows(x, eqs, cluster, col);
    clear_columns(col, cluster);
    let n = cluster.vars.len();

    let r = &rows[target];
    let norm_r = r.iter().map(|&(_, d)| d * d).sum::<f64>().sqrt();
    if !norm_r.is_finite() || norm_r < 1e-12 {
        return Dependency::Degenerate;
    }
    if base.is_empty() {
        return Dependency::Independent;
    }
    let base_rows: Vec<&Vec<(usize, f64)>> = base.iter().map(|&b| &rows[b]).collect();
    if base_rows
        .iter()
        .any(|row| row.iter().any(|&(_, d)| !d.is_finite()))
    {
        // Degenerate earlier rows say nothing; judge against the rest.
        let finite: Vec<usize> = base
            .iter()
            .copied()
            .filter(|&b| rows[b].iter().all(|&(_, d)| d.is_finite()))
            .collect();
        return dependency(x, eqs, cluster, col, &finite, target);
    }
    let cols = columns_of(&base_rows, n);
    let mut gram = Skyline::new(&row_adjacency(&cols, base.len()));
    if gram.work() > MAX_FACTOR_WORK {
        return Dependency::TooLarge;
    }
    fill_gram(&mut gram, &cols, &vec![1.0; n]);
    let mut dense_r = vec![0.0; n];
    for &(c, d) in r {
        dense_r[c] = d;
    }
    let b: Vec<f64> = base_rows
        .iter()
        .map(|row| row.iter().map(|&(c, d)| d * dense_r[c]).sum())
        .collect();

    let mut eps = 1e-12 * (gram.diagonal_sum() / base.len() as f64).max(1e-12);
    let c = loop {
        let mut damped = gram.clone();
        damped.add_to_diagonal(eps);
        if damped.factor() {
            break damped.solve(&b);
        }
        eps *= 100.0;
        if eps > 1.0 {
            return Dependency::Independent;
        }
    };
    let mut resid = dense_r;
    for (row, &ci) in base_rows.iter().zip(&c) {
        for &(col_index, d) in row.iter() {
            resid[col_index] -= ci * d;
        }
    }
    let rest = resid.iter().map(|v| v * v).sum::<f64>().sqrt();
    if rest > DEPENDENCY_TOLERANCE * norm_r {
        return Dependency::Independent;
    }
    let largest = c.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    Dependency::Redundant(
        base.iter()
            .zip(&c)
            .filter(|(_, ci)| ci.abs() > 1e-6 * largest)
            .map(|(&b, _)| cluster.eqs[b])
            .collect(),
    )
}

/// Relative leftover below which [`dependency`] calls a row redundant.
const DEPENDENCY_TOLERANCE: f64 = 1e-6;

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
    /// The cluster was over the analysis budget and wasn't analysed.
    pub too_large: bool,
}

/// Clusters up to this much dense work (`m·n·min(m, n)`) are analysed with
/// [`analyse_dense`], larger ones with [`analyse_sparse`].
const DENSE_ANALYSIS_WORK: f64 = 1e7;

/// Finds the rank, free variables and redundant equations of a cluster, in
/// `order` (indices into `cluster.eqs`), with whichever method suits its
/// size.
pub(crate) fn analyse_cluster(
    x: &[f64],
    eqs: &[Equation],
    cluster: &Cluster,
    order: &[usize],
    col: &mut [usize],
) -> ClusterAnalysis {
    let (m, n) = (cluster.eqs.len(), cluster.vars.len());
    if (m * n) as f64 * m.min(n) as f64 <= DENSE_ANALYSIS_WORK {
        analyse_dense(x, eqs, cluster, order, col)
    } else {
        analyse_sparse(x, eqs, cluster, order, col)
    }
}

/// Relative leftover below which the analysis calls a row dependent
/// on the rows before it.
const RANK_TOLERANCE: f64 = 1e-6;

/// Rows whose Cholesky pivot is below this fraction of their squared norm
/// get an exact residual check before being called independent or not.
const PIVOT_SCREEN: f64 = 1e-4;

/// A variable is free when at least this fraction of its unit vector lies
/// outside the row space.
const FREE_TOLERANCE: f64 = 1e-6;

/// Finds the rank, free variables and redundant equations of a cluster.
///
/// Rows are taken in `order` (indices into `cluster.eqs`), scaled to unit
/// length, and the Gram matrix `J Jᵀ` is Cholesky-factored row by row in a
/// skyline, which is Gram–Schmidt in disguise: a row whose pivot vanishes is
/// a combination of the rows before it. Earlier rows win, so the equation
/// reported as redundant is the later one, matching Inventor blaming the
/// newest constraint. Pivots below [`PIVOT_SCREEN`] are confirmed with an
/// exact residual. Working on `J Jᵀ` squares the conditioning, so small
/// clusters use [`analyse_dense`] instead.
pub(crate) fn analyse_sparse(
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
    let raw = sparse_rows(x, eqs, cluster, col);
    clear_columns(col, cluster);

    // Unit rows in analysis order; `None` for rows with no usable slope.
    let m = order.len();
    let rows: Vec<Option<Vec<(usize, f64)>>> = order
        .iter()
        .map(|&p| {
            let r = &raw[p];
            let norm = r.iter().map(|&(_, d)| d * d).sum::<f64>().sqrt();
            (norm.is_finite() && norm >= 1e-12)
                .then(|| r.iter().map(|&(c, d)| (c, d / norm)).collect())
        })
        .collect();
    let mut col_rows: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, r) in rows.iter().enumerate() {
        for &(c, _) in r.iter().flatten() {
            col_rows[c].push(i);
        }
    }
    let first: Vec<usize> = (0..m)
        .map(|i| {
            rows[i]
                .iter()
                .flatten()
                .flat_map(|&(c, _)| col_rows[c].iter().copied())
                .filter(|&j| j <= i)
                .min()
                .unwrap_or(i)
        })
        .collect();
    let mut start = Vec::with_capacity(m);
    let mut profile = 0;
    let mut work = 0.0;
    for (i, &f) in first.iter().enumerate() {
        start.push(profile);
        profile += i - f + 1;
        work += ((i - f + 1) as f64).powi(2);
    }
    if work + (n as f64) * (profile as f64) > MAX_ANALYSIS_WORK {
        return ClusterAnalysis {
            too_large: true,
            ..Default::default()
        };
    }
    let at = |i: usize, j: usize| start[i] + j - first[i];
    let dot = |a: &[(usize, f64)], b: &[(usize, f64)]| {
        let (mut p, mut q, mut sum) = (0, 0, 0.0);
        while p < a.len() && q < b.len() {
            match a[p].0.cmp(&b[q].0) {
                std::cmp::Ordering::Less => p += 1,
                std::cmp::Ordering::Greater => q += 1,
                std::cmp::Ordering::Equal => {
                    sum += a[p].1 * b[q].1;
                    p += 1;
                    q += 1;
                }
            }
        }
        sum
    };

    let mut l = vec![0.0; profile];
    let mut accepted = vec![false; m];
    let mut out = ClusterAnalysis::default();
    let mut resid = vec![0.0; n];
    for i in 0..m {
        let Some(ri) = &rows[i] else {
            out.redundant.push((cluster.eqs[order[i]], Vec::new()));
            continue;
        };
        for j in first[i]..i {
            let v = match (&rows[j], accepted[j]) {
                (Some(rj), true) => {
                    let lo = first[i].max(first[j]);
                    let s: f64 = (lo..j).map(|k| l[at(i, k)] * l[at(j, k)]).sum();
                    (dot(ri, rj) - s) / l[at(j, j)]
                }
                _ => 0.0,
            };
            l[at(i, j)] = v;
        }
        let pivot = 1.0 - (first[i]..i).map(|k| l[at(i, k)].powi(2)).sum::<f64>();
        if pivot > PIVOT_SCREEN {
            l[at(i, i)] = pivot.sqrt();
            accepted[i] = true;
            continue;
        }
        // Possibly dependent: express the row in the accepted rows, c =
        // L⁻ᵀ ℓ, and measure what is left over exactly.
        let mut c = vec![0.0; i];
        c[first[i]..i].copy_from_slice(&l[at(i, first[i])..at(i, i)]);
        for p in (0..i).rev() {
            if !accepted[p] {
                c[p] = 0.0;
                continue;
            }
            c[p] /= l[at(p, p)];
            for k in first[p]..p {
                c[k] -= l[at(p, k)] * c[p];
            }
        }
        for &(col_index, d) in ri {
            resid[col_index] = d;
        }
        for (p, &cp) in c.iter().enumerate() {
            if cp != 0.0 {
                for &(col_index, d) in rows[p].iter().flatten() {
                    resid[col_index] -= cp * d;
                }
            }
        }
        let rest = resid.iter().map(|v| v * v).sum::<f64>().sqrt();
        resid.iter_mut().for_each(|v| *v = 0.0);
        if rest > RANK_TOLERANCE {
            l[at(i, i)] = rest.max(pivot.max(0.0).sqrt());
            accepted[i] = true;
            continue;
        }
        let largest = c.iter().fold(0.0f64, |acc, v| acc.max(v.abs()));
        let depends = (0..i)
            .filter(|&p| accepted[p] && c[p].abs() > 1e-6 * largest)
            .map(|p| cluster.eqs[order[p]])
            .collect();
        out.redundant.push((cluster.eqs[order[i]], depends));
        for k in first[i]..=i {
            l[at(i, k)] = 0.0;
        }
    }
    out.rank = accepted.iter().filter(|&&a| a).count();

    // A variable is pinned when its unit vector lies in the row space:
    // |P e_c|² = |L⁻¹ J e_c|² over the accepted rows.
    let mut z = vec![0.0; m];
    for (c, touching) in col_rows.iter().enumerate() {
        let Some(&lo) = touching.first() else {
            out.free_vars.push(cluster.vars[c]);
            continue;
        };
        z.iter_mut().for_each(|v| *v = 0.0);
        for &i in touching {
            if accepted[i] {
                z[i] = rows[i]
                    .iter()
                    .flatten()
                    .find(|&&(cc, _)| cc == c)
                    .map_or(0.0, |&(_, d)| d);
            }
        }
        let mut inside = 0.0;
        for i in lo..m {
            if !accepted[i] {
                continue;
            }
            let s: f64 = (first[i]..i).map(|k| l[at(i, k)] * z[k]).sum();
            z[i] = (z[i] - s) / l[at(i, i)];
            inside += z[i] * z[i];
        }
        if 1.0 - inside > FREE_TOLERANCE {
            out.free_vars.push(cluster.vars[c]);
        }
    }
    out
}

/// Dense Gram–Schmidt version of [`analyse_sparse`] with the same
/// tolerances: the most accurate, and fast for small clusters.
pub(crate) fn analyse_dense(
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
    let m = cluster.eqs.len();
    if (m * n) as f64 * m.min(n) as f64 > MAX_ANALYSIS_WORK {
        return ClusterAnalysis {
            too_large: true,
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
        .filter(|&(c, _)| 1.0 - basis.iter().map(|q| q[c] * q[c]).sum::<f64>() > FREE_TOLERANCE)
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
            owner: Owner::Constraint(uuid::Uuid::nil()),
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
