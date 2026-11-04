//! Special functions and probability distributions: log-gamma (Lanczos), the regularized
//! incomplete gamma and beta functions (series + continued fractions), the error function, and
//! the CDFs / quantiles of the normal, Student t, F, chi-square and studentized range
//! distributions, plus the Kolmogorov distribution used by the Kolmogorov–Smirnov test.
//!
//! Standard numerical methods (Lanczos approximation, Lentz continued fractions, Acklam's normal
//! quantile with a Halley refinement, bracketing + bisection for the other quantiles).

use std::f64::consts::PI;

const LANCZOS: [f64; 9] = [
    0.999_999_999_999_809_9,
    676.520_368_121_885_1,
    -1_259.139_216_722_402_8,
    771.323_428_777_653_1,
    -176.615_029_162_140_6,
    12.507_343_278_686_905,
    -0.138_571_095_265_720_12,
    9.984_369_578_019_572e-6,
    1.505_632_735_149_311_6e-7,
];

/// ln Γ(x) for x > 0 (reflection for x < 0.5).
pub fn ln_gamma(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    if x < 0.5 {
        // Γ(x)Γ(1−x) = π / sin(πx)
        let s = (PI * x).sin().abs();
        if s == 0.0 {
            return f64::INFINITY;
        }
        return (PI / s).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = LANCZOS[0];
    let t = x + 7.5;
    for (i, c) in LANCZOS.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// Γ(x).
pub fn gamma(x: f64) -> f64 {
    if x < 0.5 {
        let s = (PI * x).sin();
        if s == 0.0 {
            return f64::NAN;
        }
        return PI / (s * gamma(1.0 - x));
    }
    ln_gamma(x).exp()
}

/// ln B(a, b).
pub fn ln_beta(a: f64, b: f64) -> f64 {
    ln_gamma(a) + ln_gamma(b) - ln_gamma(a + b)
}

/// Regularized lower incomplete gamma P(a, x).
pub fn gamma_p(a: f64, x: f64) -> f64 {
    if !(a > 0.0) || x.is_nan() {
        return f64::NAN;
    }
    if x <= 0.0 {
        return 0.0;
    }
    if x.is_infinite() {
        return 1.0;
    }
    if x < a + 1.0 {
        // Series.
        let mut sum = 1.0 / a;
        let mut del = sum;
        let mut ap = a;
        for _ in 0..10_000 {
            ap += 1.0;
            del *= x / ap;
            sum += del;
            if del.abs() < sum.abs() * 1e-15 {
                break;
            }
        }
        (sum.ln() - x + a * x.ln() - ln_gamma(a)).exp().clamp(0.0, 1.0)
    } else {
        1.0 - gamma_q_cf(a, x)
    }
}

/// Regularized upper incomplete gamma Q(a, x) = 1 − P(a, x).
pub fn gamma_q(a: f64, x: f64) -> f64 {
    if !(a > 0.0) || x.is_nan() {
        return f64::NAN;
    }
    if x <= 0.0 {
        return 1.0;
    }
    if x.is_infinite() {
        return 0.0;
    }
    if x < a + 1.0 { 1.0 - gamma_p(a, x) } else { gamma_q_cf(a, x) }
}

fn gamma_q_cf(a: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / TINY;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..10_000 {
        let an = -(i as f64) * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < TINY {
            d = TINY;
        }
        c = b + an / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-15 {
            break;
        }
    }
    ((-x + a * x.ln() - ln_gamma(a)).exp() * h).clamp(0.0, 1.0)
}

/// Regularized incomplete beta I_x(a, b).
pub fn beta_inc(a: f64, b: f64, x: f64) -> f64 {
    if !(a > 0.0 && b > 0.0) || x.is_nan() {
        return f64::NAN;
    }
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let bt = (a * x.ln() + b * (1.0 - x).ln() - ln_beta(a, b)).exp();
    if x < (a + 1.0) / (a + b + 2.0) { (bt * beta_cf(a, b, x) / a).clamp(0.0, 1.0) } else { (1.0 - bt * beta_cf(b, a, 1.0 - x) / b).clamp(0.0, 1.0) }
}

fn beta_cf(a: f64, b: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..10_000 {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-15 {
            break;
        }
    }
    h
}

/// The error function.
pub fn erf(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x >= 0.0 { gamma_p(0.5, x * x) } else { -gamma_p(0.5, x * x) }
}

/// The complementary error function (accurate in the tails).
pub fn erfc(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x >= 0.0 { gamma_q(0.5, x * x) } else { 1.0 + gamma_p(0.5, x * x) }
}

// ---------------------------------------------------------------------------------------------
// Normal
// ---------------------------------------------------------------------------------------------

pub fn norm_pdf(x: f64) -> f64 {
    (-0.5 * x * x).exp() / (2.0 * PI).sqrt()
}

pub fn norm_cdf(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    0.5 * erfc(-x / std::f64::consts::SQRT_2)
}

/// Standard normal quantile (Acklam's rational approximation + one Halley step).
pub fn norm_ppf(p: f64) -> f64 {
    if p.is_nan() || !(0.0..=1.0).contains(&p) {
        return f64::NAN;
    }
    if p == 0.0 {
        return f64::NEG_INFINITY;
    }
    if p == 1.0 {
        return f64::INFINITY;
    }
    const A: [f64; 6] = [-3.969_683_028_665_376e1, 2.209_460_984_245_205e2, -2.759_285_104_469_687e2, 1.383_577_518_672_69e2, -3.066_479_806_614_716e1, 2.506_628_277_459_239];
    const B: [f64; 5] = [-5.447_609_879_822_406e1, 1.615_858_368_580_409e2, -1.556_989_798_598_866e2, 6.680_131_188_771_972e1, -1.328_068_155_288_572e1];
    const C: [f64; 6] = [-7.784_894_002_430_293e-3, -3.223_964_580_411_365e-1, -2.400_758_277_161_838, -2.549_732_539_343_734, 4.374_664_141_464_968, 2.938_163_982_698_783];
    const D: [f64; 4] = [7.784_695_709_041_462e-3, 3.224_671_290_700_398e-1, 2.445_134_137_142_996, 3.754_408_661_907_416];
    let plow = 0.02425;
    let x = if p < plow {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5]) / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - plow {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5]) / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    // Halley refinement.
    let e = norm_cdf(x) - p;
    let u = e * (2.0 * PI).sqrt() * (x * x / 2.0).exp();
    x - u / (1.0 + x * u / 2.0)
}

// ---------------------------------------------------------------------------------------------
// t, F, chi-square
// ---------------------------------------------------------------------------------------------

/// Student t CDF.
pub fn t_cdf(t: f64, df: f64) -> f64 {
    if t.is_nan() || !(df > 0.0) {
        return f64::NAN;
    }
    if t.is_infinite() {
        return if t > 0.0 { 1.0 } else { 0.0 };
    }
    let x = df / (df + t * t);
    let tail = 0.5 * beta_inc(df / 2.0, 0.5, x);
    if t > 0.0 { 1.0 - tail } else { tail }
}

/// Two-sided p-value of a t statistic.
pub fn t_two_sided(t: f64, df: f64) -> f64 {
    if t.is_nan() || !(df > 0.0) {
        return f64::NAN;
    }
    beta_inc(df / 2.0, 0.5, df / (df + t * t)).clamp(0.0, 1.0)
}

pub fn t_pdf(t: f64, df: f64) -> f64 {
    (ln_gamma((df + 1.0) / 2.0) - ln_gamma(df / 2.0) - 0.5 * (df * PI).ln() - (df + 1.0) / 2.0 * (1.0 + t * t / df).ln()).exp()
}

pub fn t_ppf(p: f64, df: f64) -> f64 {
    if !(0.0..=1.0).contains(&p) || !(df > 0.0) {
        return f64::NAN;
    }
    let z = norm_ppf(p);
    invert_cdf(|x| t_cdf(x, df), p, z - 50.0, z + 50.0, z)
}

pub fn chi2_cdf(x: f64, k: f64) -> f64 {
    if !(k > 0.0) {
        return f64::NAN;
    }
    if x <= 0.0 { 0.0 } else { gamma_p(k / 2.0, x / 2.0) }
}

/// Upper tail of chi-square (p-value).
pub fn chi2_sf(x: f64, k: f64) -> f64 {
    if !(k > 0.0) {
        return f64::NAN;
    }
    if x <= 0.0 { 1.0 } else { gamma_q(k / 2.0, x / 2.0) }
}

pub fn chi2_ppf(p: f64, k: f64) -> f64 {
    if !(0.0..=1.0).contains(&p) || !(k > 0.0) {
        return f64::NAN;
    }
    let hi = (k + 20.0 * (2.0 * k).sqrt() + 100.0).max(10.0);
    invert_cdf(|x| chi2_cdf(x, k), p, 0.0, hi, k)
}

pub fn f_cdf(f: f64, d1: f64, d2: f64) -> f64 {
    if !(d1 > 0.0 && d2 > 0.0) || f.is_nan() {
        return f64::NAN;
    }
    if f <= 0.0 { 0.0 } else { beta_inc(d1 / 2.0, d2 / 2.0, d1 * f / (d1 * f + d2)) }
}

/// Upper tail of F (p-value).
pub fn f_sf(f: f64, d1: f64, d2: f64) -> f64 {
    if !(d1 > 0.0 && d2 > 0.0) || f.is_nan() {
        return f64::NAN;
    }
    if f <= 0.0 { 1.0 } else { beta_inc(d2 / 2.0, d1 / 2.0, d2 / (d2 + d1 * f)) }
}

pub fn f_ppf(p: f64, d1: f64, d2: f64) -> f64 {
    if !(0.0..=1.0).contains(&p) {
        return f64::NAN;
    }
    invert_cdf(|x| f_cdf(x, d1, d2), p, 0.0, 1e6, 1.0)
}

/// Numerically inverts a monotone CDF on [lo, hi] (expanding the bracket if needed).
fn invert_cdf(cdf: impl Fn(f64) -> f64, p: f64, mut lo: f64, mut hi: f64, _guess: f64) -> f64 {
    if p <= 0.0 {
        return lo;
    }
    if p >= 1.0 {
        return hi;
    }
    for _ in 0..60 {
        if cdf(hi) >= p {
            break;
        }
        hi = hi * 2.0 + 1.0;
    }
    for _ in 0..60 {
        if cdf(lo) <= p {
            break;
        }
        lo = lo * 2.0 - 1.0;
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if cdf(mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
        if (hi - lo).abs() < 1e-12 * (1.0 + mid.abs()) {
            break;
        }
    }
    0.5 * (lo + hi)
}

// ---------------------------------------------------------------------------------------------
// Studentized range (Tukey HSD)
// ---------------------------------------------------------------------------------------------

/// P(range of k standard normals ≤ w) (infinite degrees of freedom).
fn range_cdf_inf(w: f64, k: f64) -> f64 {
    if w <= 0.0 {
        return 0.0;
    }
    // k ∫ φ(z) [Φ(z) − Φ(z − w)]^(k−1) dz, Simpson on [−8, 8 + w].
    let (a, b) = (-8.0, 8.0);
    let n = 240;
    let h = (b - a) / n as f64;
    let f = |z: f64| {
        let d = (norm_cdf(z + w) - norm_cdf(z)).max(0.0);
        norm_pdf(z) * d.powf(k - 1.0)
    };
    let mut s = f(a) + f(b);
    for i in 1..n {
        let z = a + h * i as f64;
        s += if i % 2 == 1 { 4.0 } else { 2.0 } * f(z);
    }
    (k * s * h / 3.0).clamp(0.0, 1.0)
}

/// CDF of the studentized range distribution Q(k groups, df).
pub fn ptukey(q: f64, k: f64, df: f64) -> f64 {
    if q.is_nan() || !(k >= 2.0) || !(df > 0.0) {
        return f64::NAN;
    }
    if q <= 0.0 {
        return 0.0;
    }
    if df > 5000.0 {
        return range_cdf_inf(q, k);
    }
    // ∫ f_s(s) W(q s) ds where s = sqrt(χ²_df / df).
    let lo = (chi2_ppf(1e-9, df) / df).sqrt().max(1e-6);
    let hi = (chi2_ppf(1.0 - 1e-9, df) / df).sqrt();
    let n = 160;
    let h = (hi - lo) / n as f64;
    let ln_c = (df / 2.0) * df.ln() - ln_gamma(df / 2.0) - (df / 2.0 - 1.0) * 2f64.ln();
    let dens = |s: f64| (ln_c + (df - 1.0) * s.ln() - df * s * s / 2.0).exp();
    let f = |s: f64| dens(s) * range_cdf_inf(q * s, k);
    let mut sum = f(lo) + f(hi);
    for i in 1..n {
        let s = lo + h * i as f64;
        sum += if i % 2 == 1 { 4.0 } else { 2.0 } * f(s);
    }
    (sum * h / 3.0).clamp(0.0, 1.0)
}

/// Quantile of the studentized range distribution.
pub fn qtukey(p: f64, k: f64, df: f64) -> f64 {
    if !(0.0..1.0).contains(&p) {
        return f64::NAN;
    }
    let (mut lo, mut hi) = (0.0, 100.0);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if ptukey(mid, k, df) < p {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo < 1e-6 {
            break;
        }
    }
    0.5 * (lo + hi)
}

// ---------------------------------------------------------------------------------------------
// Kolmogorov distribution
// ---------------------------------------------------------------------------------------------

/// P(K > λ) for the Kolmogorov distribution (asymptotic KS p-value).
pub fn kolmogorov_sf(lambda: f64) -> f64 {
    if lambda <= 0.0 {
        return 1.0;
    }
    if lambda < 0.2 {
        return 1.0;
    }
    let mut s = 0.0;
    for j in 1..200 {
        let j = j as f64;
        let term = 2.0 * (if (j as i64) % 2 == 1 { 1.0 } else { -1.0 }) * (-2.0 * j * j * lambda * lambda).exp();
        s += term;
        if term.abs() < 1e-14 {
            break;
        }
    }
    s.clamp(0.0, 1.0)
}

/// Two-sample / one-sample KS p-value with the effective sample size `ne`.
pub fn ks_pvalue(d: f64, ne: f64) -> f64 {
    let sq = ne.sqrt();
    kolmogorov_sf((sq + 0.12 + 0.11 / sq) * d)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn gamma_values() {
        assert!(close(ln_gamma(0.5), 0.572_364_942_924_700_1, 1e-12));
        assert!(close(gamma(5.0), 24.0, 1e-9));
        assert!(close(gamma(0.5), PI.sqrt(), 1e-12));
        assert!(close(ln_gamma(100.0), 359.134_205_369_575_4, 1e-9));
        assert!(close(gamma(-0.5), -2.0 * PI.sqrt(), 1e-9));
    }

    #[test]
    fn incomplete() {
        assert!(close(gamma_p(1.0, 1.0), 1.0 - (-1f64).exp(), 1e-12));
        assert!(close(beta_inc(2.0, 3.0, 0.4), 0.5248, 1e-10));
        assert!(close(erf(1.0), 0.842_700_792_949_714_9, 1e-12));
        assert!(close(erfc(3.0), 2.209_049_699_858_544e-5, 1e-15));
    }

    #[test]
    fn normal() {
        assert!(close(norm_cdf(1.96), 0.975_002_104_851_780, 1e-12));
        assert!(close(norm_cdf(-3.0), 0.001_349_898_031_630_094_6, 1e-14));
        assert!(close(norm_ppf(0.975), 1.959_963_984_540_054, 1e-10));
        assert!(close(norm_ppf(1e-6), -4.753_424_308_822_899, 1e-8));
    }

    #[test]
    fn t_f_chi() {
        assert!(close(t_cdf(2.228_138_851_986_274, 10.0), 0.975, 1e-9));
        assert!(close(t_two_sided(2.0, 10.0), 0.073_388_034_770_740_4, 1e-9));
        assert!(close(t_ppf(0.975, 3.0), 3.182_446_305_284_263, 1e-7));
        assert!(close(t_cdf(1.0, 1.0), 0.75, 1e-12)); // Cauchy
        assert!(close(chi2_cdf(3.841_458_820_694_124, 1.0), 0.95, 1e-9));
        assert!(close(chi2_ppf(0.95, 10.0), 18.307_038_053_275_14, 1e-6));
        assert!(close(f_cdf(4.102_821_015_130_399, 2.0, 10.0), 0.95, 1e-9));
        assert!(close(f_ppf(0.95, 3.0, 20.0), 3.098_391_212_407_06, 1e-6));
        assert!(close(f_sf(4.102_821_015_130_399, 2.0, 10.0), 0.05, 1e-9));
    }

    #[test]
    fn tukey() {
        // Standard tables: q(0.95; 3, 10) = 3.877, q(0.95; 4, 20) = 3.958, q(0.95; 2, inf)=2.772.
        assert!(close(qtukey(0.95, 3.0, 10.0), 3.877, 2e-3));
        assert!(close(qtukey(0.95, 4.0, 20.0), 3.958, 2e-3));
        assert!(close(qtukey(0.95, 2.0, 1e6), 2.772, 2e-3));
    }

    #[test]
    fn kolmogorov() {
        // Critical value 1.358 at alpha = 0.05.
        assert!(close(kolmogorov_sf(1.3581), 0.05, 1e-3));
    }
}
