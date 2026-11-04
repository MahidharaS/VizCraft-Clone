//! Distribution fitting by maximum likelihood (normal, log-normal, exponential, gamma, Weibull,
//! uniform, Poisson) with log-likelihood, AIC / BIC and the Kolmogorov–Smirnov goodness of fit.

use super::special::*;
use super::{fmt, fmt_p, RTable, Report};

#[derive(Clone, Debug, PartialEq)]
pub enum Dist {
    Normal { mu: f64, sigma: f64 },
    LogNormal { mu: f64, sigma: f64 },
    Exponential { rate: f64 },
    Gamma { shape: f64, scale: f64 },
    Weibull { shape: f64, scale: f64 },
    Uniform { a: f64, b: f64 },
    Poisson { lambda: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    pub dist: Dist,
    pub loglik: f64,
    pub aic: f64,
    pub bic: f64,
    pub ks: f64,
    pub ks_p: f64,
}

impl Dist {
    pub fn name(&self) -> &'static str {
        match self {
            Dist::Normal { .. } => "Normal",
            Dist::LogNormal { .. } => "Log-normal",
            Dist::Exponential { .. } => "Exponential",
            Dist::Gamma { .. } => "Gamma",
            Dist::Weibull { .. } => "Weibull",
            Dist::Uniform { .. } => "Uniform",
            Dist::Poisson { .. } => "Poisson",
        }
    }
    pub fn params(&self) -> String {
        match self {
            Dist::Normal { mu, sigma } => format!("μ = {}, σ = {}", fmt(*mu), fmt(*sigma)),
            Dist::LogNormal { mu, sigma } => format!("μ = {}, σ = {} (log scale)", fmt(*mu), fmt(*sigma)),
            Dist::Exponential { rate } => format!("λ = {}", fmt(*rate)),
            Dist::Gamma { shape, scale } => format!("k = {}, θ = {}", fmt(*shape), fmt(*scale)),
            Dist::Weibull { shape, scale } => format!("k = {}, λ = {}", fmt(*shape), fmt(*scale)),
            Dist::Uniform { a, b } => format!("a = {}, b = {}", fmt(*a), fmt(*b)),
            Dist::Poisson { lambda } => format!("λ = {}", fmt(*lambda)),
        }
    }
    fn k(&self) -> f64 {
        match self {
            Dist::Exponential { .. } | Dist::Poisson { .. } => 1.0,
            _ => 2.0,
        }
    }
    pub fn pdf(&self, x: f64) -> f64 {
        match *self {
            Dist::Normal { mu, sigma } => norm_pdf((x - mu) / sigma) / sigma,
            Dist::LogNormal { mu, sigma } => {
                if x <= 0.0 {
                    0.0
                } else {
                    norm_pdf((x.ln() - mu) / sigma) / (sigma * x)
                }
            }
            Dist::Exponential { rate } => {
                if x < 0.0 {
                    0.0
                } else {
                    rate * (-rate * x).exp()
                }
            }
            Dist::Gamma { shape, scale } => {
                if x <= 0.0 {
                    0.0
                } else {
                    ((shape - 1.0) * x.ln() - x / scale - ln_gamma(shape) - shape * scale.ln()).exp()
                }
            }
            Dist::Weibull { shape, scale } => {
                if x < 0.0 {
                    0.0
                } else {
                    shape / scale * (x / scale).powf(shape - 1.0) * (-(x / scale).powf(shape)).exp()
                }
            }
            Dist::Uniform { a, b } => {
                if x >= a && x <= b && b > a {
                    1.0 / (b - a)
                } else {
                    0.0
                }
            }
            Dist::Poisson { lambda } => {
                let k = x.round();
                if k < 0.0 || (x - k).abs() > 1e-9 {
                    0.0
                } else {
                    (k * lambda.ln() - lambda - ln_gamma(k + 1.0)).exp()
                }
            }
        }
    }
    pub fn cdf(&self, x: f64) -> f64 {
        match *self {
            Dist::Normal { mu, sigma } => norm_cdf((x - mu) / sigma),
            Dist::LogNormal { mu, sigma } => {
                if x <= 0.0 {
                    0.0
                } else {
                    norm_cdf((x.ln() - mu) / sigma)
                }
            }
            Dist::Exponential { rate } => {
                if x <= 0.0 {
                    0.0
                } else {
                    1.0 - (-rate * x).exp()
                }
            }
            Dist::Gamma { shape, scale } => gamma_p(shape, (x / scale).max(0.0)),
            Dist::Weibull { shape, scale } => {
                if x <= 0.0 {
                    0.0
                } else {
                    1.0 - (-(x / scale).powf(shape)).exp()
                }
            }
            Dist::Uniform { a, b } => ((x - a) / (b - a)).clamp(0.0, 1.0),
            Dist::Poisson { lambda } => {
                if x < 0.0 {
                    0.0
                } else {
                    gamma_q(x.floor() + 1.0, lambda)
                }
            }
        }
    }
    fn logpdf(&self, x: f64) -> f64 {
        let p = self.pdf(x);
        if p > 0.0 { p.ln() } else { -1e6 }
    }
}

/// Fits one family; None when the data doesn't fit the support.
pub fn fit(x: &[f64], family: &str) -> Option<Fit> {
    let x = super::sorted(x);
    let n = x.len();
    if n < 3 {
        return None;
    }
    let nf = n as f64;
    let m = super::mean(&x);
    let positive = x.first().is_some_and(|v| *v > 0.0);
    let nonneg = x.first().is_some_and(|v| *v >= 0.0);
    let d = match family {
        "normal" => {
            let s = (x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / nf).sqrt();
            Dist::Normal { mu: m, sigma: s.max(1e-12) }
        }
        "lognormal" if positive => {
            let l: Vec<f64> = x.iter().map(|v| v.ln()).collect();
            let lm = super::mean(&l);
            let s = (l.iter().map(|v| (v - lm).powi(2)).sum::<f64>() / nf).sqrt();
            Dist::LogNormal { mu: lm, sigma: s.max(1e-12) }
        }
        "exponential" if nonneg && m > 0.0 => Dist::Exponential { rate: 1.0 / m },
        "gamma" if positive => {
            // Minka's approximation + Newton refinement for the shape.
            let lm = super::mean(&x.iter().map(|v| v.ln()).collect::<Vec<_>>());
            let s = m.ln() - lm;
            let mut k = (3.0 - s + ((s - 3.0).powi(2) + 24.0 * s).sqrt()) / (12.0 * s);
            for _ in 0..50 {
                let num = k.ln() - digamma(k) - s;
                let den = 1.0 / k - trigamma(k);
                let nk = k - num / den;
                if !nk.is_finite() || nk <= 0.0 {
                    break;
                }
                if (nk - k).abs() < 1e-12 {
                    k = nk;
                    break;
                }
                k = nk;
            }
            Dist::Gamma { shape: k, scale: m / k }
        }
        "weibull" if positive => {
            let l: Vec<f64> = x.iter().map(|v| v.ln()).collect();
            let lm = super::mean(&l);
            let mut k = 1.2;
            for _ in 0..100 {
                let a: f64 = x.iter().map(|v| v.powf(k)).sum();
                let b: f64 = x.iter().zip(&l).map(|(v, lv)| v.powf(k) * lv).sum();
                let c: f64 = x.iter().zip(&l).map(|(v, lv)| v.powf(k) * lv * lv).sum();
                let f = b / a - 1.0 / k - lm;
                let fp = (c * a - b * b) / (a * a) + 1.0 / (k * k);
                let nk = k - f / fp;
                if !nk.is_finite() || nk <= 0.0 {
                    break;
                }
                if (nk - k).abs() < 1e-10 {
                    k = nk;
                    break;
                }
                k = nk;
            }
            let lam = (x.iter().map(|v| v.powf(k)).sum::<f64>() / nf).powf(1.0 / k);
            Dist::Weibull { shape: k, scale: lam }
        }
        "uniform" => Dist::Uniform { a: *x.first()?, b: *x.last()? },
        "poisson" if nonneg && x.iter().all(|v| v.fract() == 0.0) => Dist::Poisson { lambda: m.max(1e-12) },
        _ => return None,
    };
    let loglik: f64 = x.iter().map(|v| d.logpdf(*v)).sum();
    let mut ks = 0.0f64;
    for (i, v) in x.iter().enumerate() {
        let c = d.cdf(*v);
        ks = ks.max((c - i as f64 / nf).abs()).max(((i + 1) as f64 / nf - c).abs());
    }
    Some(Fit { aic: -2.0 * loglik + 2.0 * d.k(), bic: -2.0 * loglik + d.k() * nf.ln(), ks_p: ks_pvalue(ks, nf), ks, loglik, dist: d })
}

pub const FAMILIES: [&str; 7] = ["normal", "lognormal", "exponential", "gamma", "weibull", "uniform", "poisson"];

/// Fits every family that applies, best (lowest AIC) first.
pub fn fit_all(x: &[f64]) -> Vec<Fit> {
    let mut v: Vec<Fit> = FAMILIES.iter().filter_map(|f| fit(x, f)).collect();
    v.sort_by(|a, b| a.aic.partial_cmp(&b.aic).unwrap_or(std::cmp::Ordering::Equal));
    v
}

pub fn report(fits: &[Fit], name: &str) -> Report {
    let mut r = Report::new(format!("Distribution fitting: {name}"));
    let s = r.section("Candidates (best AIC first)");
    s.table = Some(RTable {
        columns: ["Distribution", "Parameters", "Log-lik", "AIC", "BIC", "KS D", "KS p-value"].iter().map(|s| s.to_string()).collect(),
        rows: fits.iter().map(|f| vec![f.dist.name().into(), f.dist.params(), fmt(f.loglik), fmt(f.aic), fmt(f.bic), format!("{:.4}", f.ks), fmt_p(f.ks_p)]).collect(),
        p_col: None,
    });
    if let Some(b) = fits.first() {
        s.note = format!("Best fit: {} ({}). KS p = {}: {}.", b.dist.name(), b.dist.params(), fmt_p(b.ks_p), if b.ks_p < 0.05 { "the fit is rejected at 5% (approximate test; parameters were estimated)" } else { "consistent with the data" });
    }
    r
}

pub fn digamma(mut x: f64) -> f64 {
    let mut r = 0.0;
    while x < 6.0 {
        r -= 1.0 / x;
        x += 1.0;
    }
    let f = 1.0 / (x * x);
    r + x.ln() - 0.5 / x - f * (1.0 / 12.0 - f * (1.0 / 120.0 - f * (1.0 / 252.0 - f * (1.0 / 240.0 - f / 132.0))))
}

pub fn trigamma(mut x: f64) -> f64 {
    let mut r = 0.0;
    while x < 6.0 {
        r += 1.0 / (x * x);
        x += 1.0;
    }
    let f = 1.0 / (x * x);
    r + 1.0 / x + f / 2.0 + f / x * (1.0 / 6.0 - f * (1.0 / 30.0 - f * (1.0 / 42.0 - f / 30.0)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn digamma_values() {
        assert!((digamma(1.0) + 0.577_215_664_901_532_9).abs() < 1e-10);
        assert!((trigamma(1.0) - std::f64::consts::PI.powi(2) / 6.0).abs() < 1e-9);
    }

    #[test]
    fn fits() {
        let x: Vec<f64> = (1..=200).map(|i| -(1.0 - i as f64 / 201.0f64).ln() * 2.0).collect();
        let fits = fit_all(&x);
        let best = &fits[0];
        assert!(matches!(best.dist.name(), "Exponential" | "Gamma" | "Weibull"), "{}", best.dist.name());
        let e = fit(&x, "exponential").unwrap();
        if let Dist::Exponential { rate } = e.dist {
            assert!((rate - 0.5).abs() < 0.05, "{rate}");
        }
        let g = fit(&x, "gamma").unwrap();
        if let Dist::Gamma { shape, .. } = g.dist {
            assert!((shape - 1.0).abs() < 0.15, "{shape}");
        }
        let n: Vec<f64> = (1..=100).map(|i| 5.0 + 2.0 * norm_ppf(i as f64 / 101.0)).collect();
        let f = fit(&n, "normal").unwrap();
        assert!(f.ks_p > 0.9);
    }
}
