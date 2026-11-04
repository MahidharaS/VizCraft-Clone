//! Small numerical helpers: Nelder–Mead minimisation and dense linear algebra.

/// Minimises `f` from `x0` with the Nelder–Mead simplex method. Returns (argmin, min).
pub fn nelder_mead(f: &dyn Fn(&[f64]) -> f64, x0: &[f64], step: f64, max_iter: usize) -> (Vec<f64>, f64) {
    let n = x0.len();
    if n == 0 {
        return (vec![], f(&[]));
    }
    let eval = |x: &[f64]| {
        let v = f(x);
        if v.is_finite() { v } else { f64::MAX / 4.0 }
    };
    let mut simplex: Vec<(Vec<f64>, f64)> = Vec::with_capacity(n + 1);
    simplex.push((x0.to_vec(), eval(x0)));
    for i in 0..n {
        let mut x = x0.to_vec();
        x[i] += if x[i].abs() > 1e-8 { step * x[i].abs().max(0.1) } else { step };
        let v = eval(&x);
        simplex.push((x, v));
    }
    let (alpha, gamma, rho, sigma) = (1.0, 2.0, 0.5, 0.5);
    for _ in 0..max_iter {
        simplex.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let best = simplex[0].1;
        let worst = simplex[n].1;
        if (worst - best).abs() <= 1e-12 * (best.abs() + 1e-12) {
            break;
        }
        let mut c = vec![0.0; n];
        for (x, _) in simplex.iter().take(n) {
            for (cj, xj) in c.iter_mut().zip(x) {
                *cj += xj / n as f64;
            }
        }
        let xw = simplex[n].0.clone();
        let refl: Vec<f64> = c.iter().zip(&xw).map(|(c, w)| c + alpha * (c - w)).collect();
        let fr = eval(&refl);
        if fr < simplex[0].1 {
            let exp: Vec<f64> = c.iter().zip(&xw).map(|(c, w)| c + gamma * (c - w)).collect();
            let fe = eval(&exp);
            simplex[n] = if fe < fr { (exp, fe) } else { (refl, fr) };
        } else if fr < simplex[n - 1].1 {
            simplex[n] = (refl, fr);
        } else {
            let con: Vec<f64> = c.iter().zip(&xw).map(|(c, w)| c + rho * (w - c)).collect();
            let fc = eval(&con);
            if fc < worst {
                simplex[n] = (con, fc);
            } else {
                let x0 = simplex[0].0.clone();
                for s in simplex.iter_mut().skip(1) {
                    let x: Vec<f64> = x0.iter().zip(&s.0).map(|(a, b)| a + sigma * (b - a)).collect();
                    let v = eval(&x);
                    *s = (x, v);
                }
            }
        }
    }
    simplex.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    simplex.swap_remove(0)
}

pub type Matrix = Vec<Vec<f64>>;

/// Xᵀ X and Xᵀ y.
pub fn normal_equations(x: &Matrix, y: &[f64], w: Option<&[f64]>) -> (Matrix, Vec<f64>) {
    let d = x.first().map(|r| r.len()).unwrap_or(0);
    let mut xtx = vec![vec![0.0; d]; d];
    let mut xty = vec![0.0; d];
    for (i, r) in x.iter().enumerate() {
        let wi = w.and_then(|w| w.get(i)).copied().unwrap_or(1.0);
        let yi = y.get(i).copied().unwrap_or(0.0);
        for a in 0..d.min(r.len()) {
            let ra = r[a] * wi;
            xty[a] += ra * yi;
            for b in a..d.min(r.len()) {
                xtx[a][b] += ra * r[b];
            }
        }
    }
    for a in 0..d {
        for b in 0..a {
            xtx[a][b] = xtx[b][a];
        }
    }
    (xtx, xty)
}

/// Inverse of a symmetric positive (semi-)definite matrix; tiny ridge on failure.
pub fn inverse(a: &Matrix) -> Option<Matrix> {
    if let Some(m) = crate::ml::invert(a)
        && m.iter().all(|r| r.iter().all(|v| v.is_finite()))
    {
        return Some(m);
    }
    let mut b = a.clone();
    let scale = a.iter().enumerate().map(|(i, r)| r.get(i).copied().unwrap_or(0.0).abs()).fold(0.0, f64::max).max(1e-12);
    for (i, r) in b.iter_mut().enumerate() {
        if let Some(v) = r.get_mut(i) {
            *v += 1e-9 * scale;
        }
    }
    crate::ml::invert(&b).filter(|m| m.iter().all(|r| r.iter().all(|v| v.is_finite())))
}

pub fn mat_vec(a: &Matrix, v: &[f64]) -> Vec<f64> {
    a.iter().map(|r| r.iter().zip(v).map(|(x, y)| x * y).sum()).collect()
}

pub fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Solves the symmetric Toeplitz system from autocorrelations (Levinson–Durbin). Returns the AR
/// coefficients φ₁..φ_p and the PACF values.
pub fn levinson(r: &[f64], p: usize) -> (Vec<f64>, Vec<f64>) {
    let mut phi = vec![0.0; p + 1];
    let mut pacf = Vec::with_capacity(p);
    let r0 = r.first().copied().unwrap_or(1.0);
    if r0 == 0.0 || p == 0 {
        return (vec![0.0; p], vec![0.0; p]);
    }
    let mut err = r0;
    let mut prev = vec![0.0; p + 1];
    for k in 1..=p {
        let mut acc = r.get(k).copied().unwrap_or(0.0);
        for j in 1..k {
            acc -= prev[j] * r.get(k - j).copied().unwrap_or(0.0);
        }
        let kk = if err.abs() > 1e-300 { acc / err } else { 0.0 };
        phi[k] = kk;
        for j in 1..k {
            phi[j] = prev[j] - kk * prev[k - j];
        }
        err *= 1.0 - kk * kk;
        pacf.push(kk);
        prev[..=k].copy_from_slice(&phi[..=k]);
    }
    (phi.into_iter().skip(1).collect(), pacf)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn rosenbrock() {
        let f = |x: &[f64]| (1.0 - x[0]).powi(2) + 100.0 * (x[1] - x[0] * x[0]).powi(2);
        let (x, v) = nelder_mead(&f, &[-1.2, 1.0], 0.5, 5000);
        assert!(v < 1e-8, "{v}");
        assert!((x[0] - 1.0).abs() < 1e-3);
    }
}
