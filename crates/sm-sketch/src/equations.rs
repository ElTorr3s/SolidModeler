//! Scalar equations the solver drives to zero, with exact derivatives.
//!
//! Each equation reads a handful of solver variables. Residuals are written
//! once, generically over [`Real`], and differentiated by evaluating them
//! with forward-mode dual numbers, so there are no hand-derived Jacobians to
//! get wrong.

use std::ops::{Add, Div, Mul, Neg, Sub};

pub(crate) trait Real:
    Copy
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Neg<Output = Self>
{
    fn c(v: f64) -> Self;
    fn sqrt(self) -> Self;
    fn atan2(self, x: Self) -> Self;
}

impl Real for f64 {
    fn c(v: f64) -> Self {
        v
    }
    fn sqrt(self) -> Self {
        f64::sqrt(self)
    }
    fn atan2(self, x: Self) -> Self {
        f64::atan2(self, x)
    }
}

/// A value and its derivative with respect to one seeded variable.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Dual {
    v: f64,
    d: f64,
}

impl Add for Dual {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Dual {
            v: self.v + o.v,
            d: self.d + o.d,
        }
    }
}
impl Sub for Dual {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Dual {
            v: self.v - o.v,
            d: self.d - o.d,
        }
    }
}
impl Mul for Dual {
    type Output = Self;
    fn mul(self, o: Self) -> Self {
        Dual {
            v: self.v * o.v,
            d: self.d * o.v + self.v * o.d,
        }
    }
}
impl Div for Dual {
    type Output = Self;
    fn div(self, o: Self) -> Self {
        Dual {
            v: self.v / o.v,
            d: (self.d * o.v - self.v * o.d) / (o.v * o.v),
        }
    }
}
impl Neg for Dual {
    type Output = Self;
    fn neg(self) -> Self {
        Dual {
            v: -self.v,
            d: -self.d,
        }
    }
}
impl Real for Dual {
    fn c(v: f64) -> Self {
        Dual { v, d: 0.0 }
    }
    fn sqrt(self) -> Self {
        let s = self.v.sqrt();
        // A zero-length vector has no direction; report no slope rather than
        // infinity so the solver stays finite.
        let d = if s > 0.0 { self.d / (2.0 * s) } else { 0.0 };
        Dual { v: s, d }
    }
    fn atan2(self, x: Self) -> Self {
        let r2 = self.v * self.v + x.v * x.v;
        let d = if r2 > 0.0 {
            (x.v * self.d - self.v * x.d) / r2
        } else {
            0.0
        };
        Dual {
            v: self.v.atan2(x.v),
            d,
        }
    }
}

/// The shape of one equation. The comment on each variant lists the
/// variables it reads, in order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EqKind {
    /// `[v..]`: `Σ coeffs[i]·v[i] − constant`.
    Linear { coeffs: Vec<f64>, constant: f64 },
    /// `[px, py, qx, qy]`: `|q − p| − d`.
    Distance(f64),
    /// `[cx, cy, px, py, r]`: `|p − c| − r`.
    OnCircle,
    /// `[c1x, c1y, c2x, c2y, r1, r2]`: `|c2 − c1| − (s1·r1 + s2·r2)`.
    /// External tangency is `(1, 1)`, internal is `(1, −1)` or `(−1, 1)`.
    CenterDistance { s1: f64, s2: f64 },
    /// `[ax, ay, bx, by, px, py]`: signed distance from `p` to line `ab`,
    /// minus `d`. Positive is to the left of `a → b`.
    LineDistance(f64),
    /// `[ax, ay, bx, by, cx, cy, r]`: signed distance from `c` to line `ab`,
    /// minus `sign·r`.
    LineRadius { sign: f64 },
    /// `[ax, ay, bx, by, px, py, qx, qy]`: signed distance from the midpoint
    /// of `pq` to line `ab`.
    LineMidpoint,
    /// `[ax, ay, bx, by, cx, cy, dx, dy]`: `φ − θ` wrapped to `(−π, π]`,
    /// where `φ` is the signed angle from `ab` to `cd` and `(cos, sin)` is
    /// `θ`. `undirected` treats the lines as unoriented, so `φ − θ` is taken
    /// modulo `π` (wrapped to `(−π/2, π/2]`): parallel is satisfied by lines
    /// drawn either way round.
    ///
    /// Using the angle itself rather than its sine keeps a useful slope
    /// everywhere, so a dimension can swing 90° or more in one solve.
    Angle {
        cos: f64,
        sin: f64,
        undirected: bool,
    },
    /// `[ax, ay, bx, by, cx, cy, dx, dy]`: `|b − a| − |d − c|`.
    EqualLength,
}

fn len<T: Real>(x: T, y: T) -> T {
    (x * x + y * y).sqrt()
}

impl EqKind {
    pub(crate) fn eval<T: Real>(&self, v: &[T]) -> T {
        match self {
            EqKind::Linear { coeffs, constant } => coeffs
                .iter()
                .zip(v)
                .fold(T::c(-constant), |acc, (&k, &x)| acc + T::c(k) * x),
            EqKind::Distance(d) => len(v[2] - v[0], v[3] - v[1]) - T::c(*d),
            EqKind::OnCircle => len(v[2] - v[0], v[3] - v[1]) - v[4],
            EqKind::CenterDistance { s1, s2 } => {
                len(v[2] - v[0], v[3] - v[1]) - (T::c(*s1) * v[4] + T::c(*s2) * v[5])
            }
            EqKind::LineDistance(d) => line_distance(v, v[4], v[5]) - T::c(*d),
            EqKind::LineRadius { sign } => line_distance(v, v[4], v[5]) - T::c(*sign) * v[6],
            EqKind::LineMidpoint => {
                let half = T::c(0.5);
                line_distance(v, (v[4] + v[6]) * half, (v[5] + v[7]) * half)
            }
            EqKind::Angle {
                cos,
                sin,
                undirected,
            } => {
                let (ux, uy) = (v[2] - v[0], v[3] - v[1]);
                let (wx, wy) = (v[6] - v[4], v[7] - v[5]);
                let cross = ux * wy - uy * wx;
                let dot = ux * wx + uy * wy;
                // Rotate by −θ: (s, c) ∝ (sin(φ − θ), cos(φ − θ)).
                let s = cross * T::c(*cos) - dot * T::c(*sin);
                let c = dot * T::c(*cos) + cross * T::c(*sin);
                if *undirected {
                    (T::c(2.0) * s * c).atan2(c * c - s * s) * T::c(0.5)
                } else {
                    s.atan2(c)
                }
            }
            EqKind::EqualLength => len(v[2] - v[0], v[3] - v[1]) - len(v[6] - v[4], v[7] - v[5]),
        }
    }
}

/// Signed distance from `(px, py)` to the line through `v[0..2]`, `v[2..4]`.
fn line_distance<T: Real>(v: &[T], px: T, py: T) -> T {
    let (ux, uy) = (v[2] - v[0], v[3] - v[1]);
    (ux * (py - v[1]) - uy * (px - v[0])) / len(ux, uy)
}

/// What an equation enforces, for reporting problems back to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Owner {
    /// Built into a curve, such as an arc's endpoints lying on its circle.
    Curve(u32),
    Constraint(u32),
}

#[derive(Debug, Clone)]
pub(crate) struct Equation {
    pub kind: EqKind,
    /// Indices into the sketch's variable vector.
    pub vars: Vec<usize>,
    pub owner: Owner,
}

impl Equation {
    pub(crate) fn residual(&self, x: &[f64]) -> f64 {
        let local: Vec<f64> = self.vars.iter().map(|&i| x[i]).collect();
        self.kind.eval(&local)
    }

    /// Calls `f(variable, ∂residual/∂variable)` for every variable the
    /// equation reads that `free` allows. A variable read twice reports the
    /// two partial derivatives separately; callers add them up.
    pub(crate) fn gradient(
        &self,
        x: &[f64],
        free: impl Fn(usize) -> bool,
        mut f: impl FnMut(usize, f64),
    ) {
        let mut local: Vec<Dual> = self.vars.iter().map(|&i| Dual::c(x[i])).collect();
        for k in 0..local.len() {
            if !free(self.vars[k]) {
                continue;
            }
            local[k].d = 1.0;
            f(self.vars[k], self.kind.eval(&local).d);
            local[k].d = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every analytic derivative matches a central finite difference.
    #[test]
    fn gradients_match_finite_differences() {
        let x = [0.3, -1.2, 2.5, 0.7, -0.4, 1.9, 1.1, -2.2];
        let kinds = [
            (
                EqKind::Linear {
                    coeffs: vec![2.0, -1.0, -1.0],
                    constant: 0.5,
                },
                3,
            ),
            (EqKind::Distance(1.5), 4),
            (EqKind::OnCircle, 5),
            (EqKind::CenterDistance { s1: 1.0, s2: -1.0 }, 6),
            (EqKind::LineDistance(0.2), 6),
            (EqKind::LineRadius { sign: -1.0 }, 7),
            (EqKind::LineMidpoint, 8),
            (
                EqKind::Angle {
                    cos: 0.6,
                    sin: 0.8,
                    undirected: false,
                },
                8,
            ),
            (
                EqKind::Angle {
                    cos: 0.6,
                    sin: 0.8,
                    undirected: true,
                },
                8,
            ),
            (EqKind::EqualLength, 8),
        ];
        for (kind, n) in kinds {
            let eq = Equation {
                kind: kind.clone(),
                vars: (0..n).collect(),
                owner: Owner::Curve(0),
            };
            let mut grad = vec![0.0; n];
            eq.gradient(&x, |_| true, |i, d| grad[i] += d);
            for i in 0..n {
                let h = 1e-6;
                let (mut lo, mut hi) = (x, x);
                lo[i] -= h;
                hi[i] += h;
                let fd = (eq.residual(&hi) - eq.residual(&lo)) / (2.0 * h);
                assert!(
                    (fd - grad[i]).abs() < 1e-6,
                    "{kind:?} var {i}: {fd} vs {}",
                    grad[i]
                );
            }
        }
    }
}
