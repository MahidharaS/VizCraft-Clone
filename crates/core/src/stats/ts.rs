//! Time series: ACF / PACF, classical and iterative (STL-like) seasonal decomposition,
//! ARIMA(p,d,q) estimated by conditional sum of squares (Yule–Walker start + Nelder–Mead), with
//! forecasts and prediction intervals from ψ-weights, automatic order selection by AIC, and
//! exponential smoothing (simple, Holt linear trend, Holt–Winters additive / multiplicative).

use super::optim::{levinson, nelder_mead};
use super::special::norm_ppf;
use super::{fmt, Report};

// =============================================================================================
// Autocorrelation
// =============================================================================================

/// Sample autocorrelations r₀..r_nlags (denominator n, overall mean).
pub fn acf(x: &[f64], nlags: usize) -> Vec<f64> {
    let n = x.len();
    if n < 2 {
        return vec![1.0];
    }
    let m = super::mean(x);
    let c0: f64 = x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / n as f64;
    (0..=nlags.min(n - 1))
        .map(|k| {
            if c0 <= 0.0 {
                return if k == 0 { 1.0 } else { 0.0 };
            }
            let ck: f64 = (0..n - k).map(|t| (x[t] - m) * (x[t + k] - m)).sum::<f64>() / n as f64;
            ck / c0
        })
        .collect()
}

/// Partial autocorrelations (Yule–Walker / Durbin–Levinson) for lags 1..=nlags.
pub fn pacf(x: &[f64], nlags: usize) -> Vec<f64> {
    let r = acf(x, nlags);
    let p = nlags.min(r.len().saturating_sub(1));
    levinson(&r, p).1
}

/// The ±1.96/√n significance band.
pub fn acf_band(n: usize) -> f64 {
    1.959_963_984_540_054 / (n.max(1) as f64).sqrt()
}

/// Ljung–Box Q statistic and p-value for `lags` autocorrelations (minus `fitted` parameters).
pub fn ljung_box(x: &[f64], lags: usize, fitted: usize) -> (f64, f64) {
    let n = x.len() as f64;
    let r = acf(x, lags);
    let q: f64 = n * (n + 2.0) * r.iter().enumerate().skip(1).map(|(k, rk)| rk * rk / (n - k as f64)).sum::<f64>();
    let df = (lags as f64 - fitted as f64).max(1.0);
    (q, super::special::chi2_sf(q, df))
}

// =============================================================================================
// Decomposition
// =============================================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decomposition {
    pub observed: Vec<f64>,
    pub trend: Vec<f64>,
    pub seasonal: Vec<f64>,
    pub resid: Vec<f64>,
    pub period: usize,
    pub multiplicative: bool,
    /// Strength of trend and seasonality (Wang, Smith & Hyndman), 0..1.
    pub trend_strength: f64,
    pub seasonal_strength: f64,
}

/// Centered moving average of window `p` (2×p MA when p is even); NaN at the ends.
pub fn centered_ma(x: &[f64], p: usize) -> Vec<f64> {
    let n = x.len();
    let mut out = vec![f64::NAN; n];
    if p < 2 || n < p + 1 {
        return out;
    }
    if p % 2 == 1 {
        let h = p / 2;
        for t in h..n - h {
            out[t] = x[t - h..=t + h].iter().sum::<f64>() / p as f64;
        }
    } else {
        let h = p / 2;
        for t in h..n - h {
            let mut s = 0.5 * x[t - h] + 0.5 * x[t + h];
            s += x[t - h + 1..t + h].iter().sum::<f64>();
            out[t] = s / p as f64;
        }
    }
    out
}

fn seasonal_means(detr: &[f64], period: usize, mult: bool) -> Vec<f64> {
    let mut sums = vec![0.0; period];
    let mut cnt = vec![0usize; period];
    for (t, v) in detr.iter().enumerate() {
        if v.is_finite() {
            sums[t % period] += v;
            cnt[t % period] += 1;
        }
    }
    let mut s: Vec<f64> = sums.iter().zip(&cnt).map(|(a, c)| if *c > 0 { a / *c as f64 } else if mult { 1.0 } else { 0.0 }).collect();
    // Normalise: sum 0 (additive) / mean 1 (multiplicative).
    let m = super::mean(&s);
    for v in s.iter_mut() {
        if mult {
            *v /= if m != 0.0 { m } else { 1.0 };
        } else {
            *v -= m;
        }
    }
    s
}

/// Classical decomposition (moving-average trend, seasonal means), additive or multiplicative.
pub fn decompose(x: &[f64], period: usize, multiplicative: bool) -> Result<Decomposition, String> {
    if period < 2 {
        return Err("The seasonal period must be at least 2.".into());
    }
    if x.len() < 2 * period {
        return Err(format!("Decomposition needs at least two full seasons ({} points); the series has {}.", 2 * period, x.len()));
    }
    if multiplicative && x.iter().any(|v| *v <= 0.0) {
        return Err("Multiplicative decomposition needs positive values.".into());
    }
    let trend = centered_ma(x, period);
    let detr: Vec<f64> = x.iter().zip(&trend).map(|(a, b)| if multiplicative { a / b } else { a - b }).collect();
    let s = seasonal_means(&detr, period, multiplicative);
    let seasonal: Vec<f64> = (0..x.len()).map(|t| s[t % period]).collect();
    let resid: Vec<f64> = (0..x.len()).map(|t| if multiplicative { x[t] / (trend[t] * seasonal[t]) } else { x[t] - trend[t] - seasonal[t] }).collect();
    Ok(finish(x, trend, seasonal, resid, period, multiplicative))
}

fn finish(x: &[f64], trend: Vec<f64>, seasonal: Vec<f64>, resid: Vec<f64>, period: usize, mult: bool) -> Decomposition {
    let r: Vec<f64> = resid.iter().copied().filter(|v| v.is_finite()).collect();
    let tr: Vec<f64> = resid.iter().zip(&trend).filter(|(a, b)| a.is_finite() && b.is_finite()).map(|(a, b)| if mult { a * b } else { a + b }).collect();
    let sr: Vec<f64> = resid.iter().zip(&seasonal).filter(|(a, _)| a.is_finite()).map(|(a, b)| if mult { a * b } else { a + b }).collect();
    let vr = super::variance(&r);
    let strength = |v: f64| if v > 0.0 { (1.0 - vr / v).max(0.0) } else { 0.0 };
    Decomposition { observed: x.to_vec(), trend_strength: strength(super::variance(&tr)), seasonal_strength: strength(super::variance(&sr)), trend, seasonal, resid, period, multiplicative: mult }
}

/// Iterative robust decomposition in the spirit of STL: alternates a seasonal estimate from
/// smoothed cycle-subseries and a trend from a moving average of the deseasonalised series,
/// extending the trend to the ends by linear extrapolation.
pub fn stl(x: &[f64], period: usize) -> Result<Decomposition, String> {
    if period < 2 || x.len() < 2 * period {
        return Err(format!("STL needs at least two full seasons ({} points).", 2 * period));
    }
    let n = x.len();
    let mut trend = vec![0.0; n];
    let mut seasonal = vec![0.0; n];
    for _ in 0..3 {
        // Seasonal: cycle-subseries smoothed with a 3-point moving average.
        let detr: Vec<f64> = (0..n).map(|t| x[t] - trend[t]).collect();
        let mut seas = vec![0.0; n];
        for c in 0..period {
            let idx: Vec<usize> = (c..n).step_by(period).collect();
            let vals: Vec<f64> = idx.iter().map(|i| detr[*i]).collect();
            for (j, i) in idx.iter().enumerate() {
                let lo = j.saturating_sub(1);
                let hi = (j + 1).min(vals.len() - 1);
                seas[*i] = vals[lo..=hi].iter().sum::<f64>() / (hi - lo + 1) as f64;
            }
        }
        // Remove the low-frequency part of the seasonal.
        let low = centered_ma(&seas, period);
        let lm = super::mean(&super::finite(&low));
        for t in 0..n {
            seasonal[t] = seas[t] - if low[t].is_finite() { low[t] } else { lm };
        }
        let deseas: Vec<f64> = (0..n).map(|t| x[t] - seasonal[t]).collect();
        let mut tr = centered_ma(&deseas, period);
        extend_ends(&mut tr);
        trend = tr;
    }
    let resid: Vec<f64> = (0..n).map(|t| x[t] - trend[t] - seasonal[t]).collect();
    Ok(finish(x, trend, seasonal, resid, period, false))
}

fn extend_ends(v: &mut [f64]) {
    let first = v.iter().position(|x| x.is_finite());
    let last = v.iter().rposition(|x| x.is_finite());
    let (Some(f), Some(l)) = (first, last) else { return };
    let span = (l - f).min(6).max(1);
    let slope_l = if l > f { (v[f + span.min(l - f)] - v[f]) / span.min(l - f) as f64 } else { 0.0 };
    let slope_r = if l > f { (v[l] - v[l - span.min(l - f)]) / span.min(l - f) as f64 } else { 0.0 };
    for t in 0..f {
        v[t] = v[f] - slope_l * (f - t) as f64;
    }
    for t in l + 1..v.len() {
        v[t] = v[l] + slope_r * (t - l) as f64;
    }
}

// =============================================================================================
// ARIMA
// =============================================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arima {
    pub p: usize,
    pub d: usize,
    pub q: usize,
    pub constant: f64,
    pub ar: Vec<f64>,
    pub ma: Vec<f64>,
    pub sigma2: f64,
    pub loglik: f64,
    pub aic: f64,
    pub bic: f64,
    /// In-sample one-step residuals on the differenced series.
    pub residuals: Vec<f64>,
    /// The original series.
    pub series: Vec<f64>,
    /// In-sample fitted values on the original scale (NaN where not available).
    pub fitted: Vec<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Forecast {
    pub mean: Vec<f64>,
    pub lower: Vec<f64>,
    pub upper: Vec<f64>,
    pub level: f64,
}

pub fn difference(x: &[f64], d: usize) -> Vec<f64> {
    let mut v = x.to_vec();
    for _ in 0..d {
        v = v.windows(2).map(|w| w[1] - w[0]).collect();
    }
    v
}

/// One-step residuals of an ARMA(p,q) with constant `c` (CSS: pre-sample errors = 0).
fn arma_residuals(w: &[f64], c: f64, ar: &[f64], ma: &[f64]) -> Vec<f64> {
    let n = w.len();
    let p = ar.len();
    let mut e = vec![0.0; n];
    for t in p..n {
        let mut pred = c;
        for (i, a) in ar.iter().enumerate() {
            pred += a * w[t - i - 1];
        }
        for (j, m) in ma.iter().enumerate() {
            if t > j {
                pred += m * e[t - j - 1];
            }
        }
        e[t] = w[t] - pred;
    }
    e
}

fn stationary_penalty(ar: &[f64]) -> f64 {
    // Penalise non-stationary / non-invertible regions softly via the coefficient sum.
    let s: f64 = ar.iter().map(|a| a.abs()).sum();
    if s >= 0.999 { (s - 0.999) * 1e6 } else { 0.0 }
}

/// Fits ARIMA(p, d, q) by conditional sum of squares. `include_constant` adds a drift (d = 1) or
/// mean (d = 0) term.
pub fn arima(x: &[f64], p: usize, d: usize, q: usize, include_constant: bool) -> Result<Arima, String> {
    let x: Vec<f64> = x.iter().copied().filter(|v| v.is_finite()).collect();
    if p > 8 || q > 8 || d > 2 {
        return Err("Use p, q ≤ 8 and d ≤ 2.".into());
    }
    let w = difference(&x, d);
    let n = w.len();
    if n < p + q + 3 + p.max(q) {
        return Err(format!("The series is too short ({} points) for ARIMA({p},{d},{q}).", x.len()));
    }
    // Start: Yule–Walker on the demeaned differenced series.
    let m = if include_constant { super::mean(&w) } else { 0.0 };
    let wc: Vec<f64> = w.iter().map(|v| v - m).collect();
    let r = acf(&wc, p.max(1));
    let (ar0, _) = levinson(&r, p);
    let c0 = m * (1.0 - ar0.iter().sum::<f64>());
    let k = p + q + usize::from(include_constant);
    let unpack = |th: &[f64]| -> (f64, Vec<f64>, Vec<f64>) {
        let c = if include_constant { th[0] } else { 0.0 };
        let o = usize::from(include_constant);
        (c, th[o..o + p].to_vec(), th[o + p..o + p + q].to_vec())
    };
    let mut start = vec![];
    if include_constant {
        start.push(c0);
    }
    start.extend(ar0.iter().map(|a| a.clamp(-0.95, 0.95)));
    start.extend(std::iter::repeat_n(0.0, q));
    let obj = |th: &[f64]| {
        let (c, ar, ma) = unpack(th);
        let e = arma_residuals(&w, c, &ar, &ma);
        e.iter().skip(p).map(|v| v * v).sum::<f64>() + stationary_penalty(&ar) + stationary_penalty(&ma)
    };
    let (best, _) = if k == 0 { (vec![], obj(&[])) } else { nelder_mead(&obj, &start, 0.1, 400 * k.max(1)) };
    let (c, ar, ma) = unpack(&best);
    let e = arma_residuals(&w, c, &ar, &ma);
    let used = (n - p) as f64;
    let sse: f64 = e.iter().skip(p).map(|v| v * v).sum();
    let sigma2 = sse / used;
    let loglik = -used / 2.0 * ((2.0 * std::f64::consts::PI * sigma2).ln() + 1.0);
    let kk = (k + 1) as f64;
    // Fitted on the original scale: x_t − e_t (for d = 0), or x_{t−1} + ŵ_t (d = 1) …
    let mut fitted = vec![f64::NAN; x.len()];
    for t in p..n {
        let what = w[t] - e[t];
        let idx = t + d;
        fitted[idx] = match d {
            0 => what,
            1 => x[idx - 1] + what,
            _ => 2.0 * x[idx - 1] - x[idx - 2] + what,
        };
    }
    Ok(Arima { p, d, q, constant: c, ar, ma, sigma2, loglik, aic: -2.0 * loglik + 2.0 * kk, bic: -2.0 * loglik + kk * used.ln(), residuals: e, series: x, fitted })
}

/// Picks (p, q) ≤ (3, 2) with the lowest AIC for a given d (d chosen by a simple variance rule
/// when `d` is None).
pub fn auto_arima(x: &[f64], d: Option<usize>, include_constant: bool) -> Result<Arima, String> {
    let d = d.unwrap_or_else(|| {
        let v0 = super::variance(x);
        let v1 = super::variance(&difference(x, 1));
        let v2 = super::variance(&difference(x, 2));
        if v1 < 0.8 * v0 { if v2 < 0.8 * v1 { 2 } else { 1 } } else { 0 }
    });
    let mut best: Option<Arima> = None;
    for p in 0..=3 {
        for q in 0..=2 {
            if let Ok(m) = arima(x, p, d, q, include_constant)
                && best.as_ref().is_none_or(|b| m.aic < b.aic)
            {
                best = Some(m);
            }
        }
    }
    best.ok_or_else(|| "No ARIMA model could be fitted to this series.".into())
}

impl Arima {
    /// ψ-weights of the full model including the differencing (for prediction intervals).
    fn psi(&self, h: usize) -> Vec<f64> {
        // AR polynomial with differencing: φ*(B) = φ(B)(1−B)^d.
        let mut phi: Vec<f64> = vec![1.0];
        phi.extend(self.ar.iter().map(|a| -a));
        for _ in 0..self.d {
            let mut np = vec![0.0; phi.len() + 1];
            for (i, c) in phi.iter().enumerate() {
                np[i] += c;
                np[i + 1] -= c;
            }
            phi = np;
        }
        let mut psi = vec![0.0; h];
        if h == 0 {
            return psi;
        }
        psi[0] = 1.0;
        for j in 1..h {
            let mut v = self.ma.get(j - 1).copied().unwrap_or(0.0);
            for i in 1..phi.len() {
                if j >= i {
                    v -= phi[i] * psi[j - i];
                }
            }
            psi[j] = v;
        }
        psi
    }

    pub fn forecast(&self, h: usize, level: f64) -> Forecast {
        let x = &self.series;
        let mut w = difference(x, self.d);
        let mut e = self.residuals.clone();
        let mut out_w = vec![];
        for _ in 0..h {
            let t = w.len();
            let mut pred = self.constant;
            for (i, a) in self.ar.iter().enumerate() {
                pred += a * w.get(t.wrapping_sub(i + 1)).copied().unwrap_or(0.0);
            }
            for (j, m) in self.ma.iter().enumerate() {
                pred += m * e.get(t.wrapping_sub(j + 1)).copied().unwrap_or(0.0);
            }
            w.push(pred);
            e.push(0.0);
            out_w.push(pred);
        }
        // Undifference.
        let mut mean = out_w;
        let mut tails: Vec<Vec<f64>> = vec![];
        let mut cur = x.clone();
        for _ in 0..self.d {
            tails.push(cur.clone());
            cur = difference(&cur, 1);
        }
        for level_series in tails.iter().rev() {
            let mut last = level_series.last().copied().unwrap_or(0.0);
            mean = mean
                .iter()
                .map(|v| {
                    last += v;
                    last
                })
                .collect();
        }
        let psi = self.psi(h);
        let z = norm_ppf(0.5 + level / 2.0);
        let mut acc = 0.0;
        let mut lower = vec![];
        let mut upper = vec![];
        for (i, m) in mean.iter().enumerate() {
            acc += psi.get(i).copied().unwrap_or(0.0).powi(2);
            let se = (self.sigma2 * acc).sqrt();
            lower.push(m - z * se);
            upper.push(m + z * se);
        }
        Forecast { mean, lower, upper, level }
    }

    pub fn report(&self, name: &str) -> Report {
        let mut r = Report::new(format!("ARIMA({},{},{}) Results", self.p, self.d, self.q));
        let s = r.section("Model");
        s.kv("Series", name)
            .kv("Observations", self.series.len().to_string())
            .kv("Method", "Conditional sum of squares")
            .kv("σ² (innovations)", fmt(self.sigma2))
            .kv("Log-likelihood", fmt(self.loglik))
            .kv("AIC", fmt(self.aic))
            .kv("BIC", fmt(self.bic));
        let s = r.section("Coefficients");
        let mut rows = vec![];
        if self.constant != 0.0 {
            rows.push(vec![if self.d == 0 { "const (mean term)".into() } else { "drift".to_string() }, fmt(self.constant)]);
        }
        for (i, a) in self.ar.iter().enumerate() {
            rows.push(vec![format!("ar.L{}", i + 1), fmt(*a)]);
        }
        for (i, m) in self.ma.iter().enumerate() {
            rows.push(vec![format!("ma.L{}", i + 1), fmt(*m)]);
        }
        s.table = Some(super::RTable { columns: vec!["".into(), "coef".into()], rows, p_col: None });
        let (q, pv) = ljung_box(&self.residuals.iter().skip(self.p).copied().collect::<Vec<_>>(), 10.min(self.residuals.len().saturating_sub(2)).max(1), self.p + self.q);
        let s = r.section("Residual diagnostics");
        s.kv("Ljung–Box Q (lag 10)", fmt(q)).kv("Prob (Q)", super::fmt_p(pv));
        s.note = if pv < 0.05 { "Residuals still show autocorrelation (Ljung–Box p < 0.05): try a higher order.".into() } else { "Residuals look like white noise.".into() };
        r
    }
}

// =============================================================================================
// Exponential smoothing
// =============================================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EtsKind {
    Simple,
    Holt,
    HoltWintersAdditive,
    HoltWintersMultiplicative,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ets {
    pub kind: EtsKind,
    pub alpha: f64,
    pub beta: f64,
    pub gamma: f64,
    pub period: usize,
    pub level: f64,
    pub trend: f64,
    pub season: Vec<f64>,
    pub sse: f64,
    pub sigma2: f64,
    pub fitted: Vec<f64>,
    pub series: Vec<f64>,
    pub aic: f64,
}

fn ets_run(x: &[f64], kind: EtsKind, a: f64, b: f64, g: f64, m: usize) -> (f64, f64, f64, Vec<f64>, Vec<f64>) {
    let n = x.len();
    let seasonal = matches!(kind, EtsKind::HoltWintersAdditive | EtsKind::HoltWintersMultiplicative);
    let mult = kind == EtsKind::HoltWintersMultiplicative;
    let trended = kind != EtsKind::Simple;
    // Initial states.
    let (mut l, mut t) = if seasonal && n >= 2 * m {
        let m1 = super::mean(&x[..m]);
        let m2 = super::mean(&x[m..2 * m]);
        (m1, (m2 - m1) / m as f64)
    } else {
        (x.first().copied().unwrap_or(0.0), if n > 1 { x[1] - x[0] } else { 0.0 })
    };
    let mut s: Vec<f64> = if seasonal && n >= m {
        let m1 = super::mean(&x[..m]);
        x[..m].iter().map(|v| if mult { v / m1.max(1e-12) } else { v - m1 }).collect()
    } else {
        vec![if mult { 1.0 } else { 0.0 }; m.max(1)]
    };
    if !trended {
        t = 0.0;
    }
    let mut fitted = Vec::with_capacity(n);
    let mut sse = 0.0;
    for (i, y) in x.iter().enumerate() {
        let si = if seasonal { s[i % m] } else if mult { 1.0 } else { 0.0 };
        let f = if mult { (l + t) * si } else { l + t + si };
        fitted.push(f);
        if i > 0 || !seasonal {
            sse += (y - f).powi(2);
        }
        let l_prev = l;
        if seasonal {
            if mult {
                l = a * (y / si.max(1e-12)) + (1.0 - a) * (l + t);
            } else {
                l = a * (y - si) + (1.0 - a) * (l + t);
            }
        } else {
            l = a * y + (1.0 - a) * (l + t);
        }
        if trended {
            t = b * (l - l_prev) + (1.0 - b) * t;
        }
        if seasonal {
            s[i % m] = if mult { g * (y / l.max(1e-12)) + (1.0 - g) * si } else { g * (y - l) + (1.0 - g) * si };
        }
    }
    // Rotate the seasonal so s[0] is the next period's index.
    let mut rot = s.clone();
    if seasonal {
        for (j, v) in rot.iter_mut().enumerate() {
            *v = s[(n + j) % m];
        }
    }
    (sse, l, t, rot, fitted)
}

pub fn ets(x: &[f64], kind: EtsKind, period: usize) -> Result<Ets, String> {
    let x: Vec<f64> = x.iter().copied().filter(|v| v.is_finite()).collect();
    let seasonal = matches!(kind, EtsKind::HoltWintersAdditive | EtsKind::HoltWintersMultiplicative);
    if x.len() < 4 || (seasonal && x.len() < 2 * period.max(2)) {
        return Err("The series is too short for this smoothing model.".into());
    }
    if kind == EtsKind::HoltWintersMultiplicative && x.iter().any(|v| *v <= 0.0) {
        return Err("Multiplicative seasonality needs positive values.".into());
    }
    let m = period.max(1);
    let sig = |v: f64| 1.0 / (1.0 + (-v).exp());
    let obj = |th: &[f64]| ets_run(&x, kind, sig(th[0]), sig(th.get(1).copied().unwrap_or(0.0)), sig(th.get(2).copied().unwrap_or(0.0)), m).0;
    let np = match kind {
        EtsKind::Simple => 1,
        EtsKind::Holt => 2,
        _ => 3,
    };
    let start = vec![0.0, -1.5, -1.5];
    let (th, _) = nelder_mead(&obj, &start[..np], 0.8, 600);
    let (a, b, g) = (sig(th[0]), if np > 1 { sig(th[1]) } else { 0.0 }, if np > 2 { sig(th[2]) } else { 0.0 });
    let (sse, level, trend, season, fitted) = ets_run(&x, kind, a, b, g, m);
    let n = x.len() as f64;
    let sigma2 = sse / n;
    let k = (np + 1 + if seasonal { m } else { 0 } + usize::from(kind != EtsKind::Simple)) as f64;
    Ok(Ets { kind, alpha: a, beta: b, gamma: g, period: m, level, trend, season, sse, sigma2, fitted, series: x, aic: n * sigma2.max(1e-300).ln() + 2.0 * k })
}

impl Ets {
    pub fn forecast(&self, h: usize, level: f64) -> Forecast {
        let z = norm_ppf(0.5 + level / 2.0);
        let mult = self.kind == EtsKind::HoltWintersMultiplicative;
        let seasonal = matches!(self.kind, EtsKind::HoltWintersAdditive | EtsKind::HoltWintersMultiplicative);
        let mut mean = vec![];
        let mut lower = vec![];
        let mut upper = vec![];
        for i in 1..=h {
            let s = if seasonal { self.season.get((i - 1) % self.period).copied().unwrap_or(0.0) } else if mult { 1.0 } else { 0.0 };
            let base = self.level + i as f64 * self.trend;
            let f = if mult { base * s } else { base + s };
            // Approximate variance growth (simple / Holt closed forms).
            let hf = (i - 1) as f64;
            let c = match self.kind {
                EtsKind::Simple => 1.0 + hf * self.alpha * self.alpha,
                _ => 1.0 + hf * (self.alpha * self.alpha + self.alpha * self.beta * i as f64 + self.beta * self.beta * i as f64 * (2.0 * i as f64 - 1.0) / 6.0),
            };
            let se = (self.sigma2 * c).sqrt();
            mean.push(f);
            lower.push(f - z * se);
            upper.push(f + z * se);
        }
        Forecast { mean, lower, upper, level }
    }

    pub fn report(&self, name: &str) -> Report {
        let label = match self.kind {
            EtsKind::Simple => "Simple exponential smoothing",
            EtsKind::Holt => "Holt's linear trend",
            EtsKind::HoltWintersAdditive => "Holt–Winters (additive seasonality)",
            EtsKind::HoltWintersMultiplicative => "Holt–Winters (multiplicative seasonality)",
        };
        let mut r = Report::new(label);
        let s = r.section("Model");
        s.kv("Series", name).kv("Observations", self.series.len().to_string()).kv("α (level)", format!("{:.4}", self.alpha));
        if self.kind != EtsKind::Simple {
            s.kv("β (trend)", format!("{:.4}", self.beta));
        }
        if matches!(self.kind, EtsKind::HoltWintersAdditive | EtsKind::HoltWintersMultiplicative) {
            s.kv("γ (season)", format!("{:.4}", self.gamma)).kv("Period", self.period.to_string());
        }
        s.kv("SSE", fmt(self.sse)).kv("RMSE", fmt(self.sigma2.sqrt())).kv("AIC", fmt(self.aic));
        r
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn acf_pacf_match_statsmodels() {
        let s = [1.0, 2.0, 1.5, 3.0, 2.5, 3.5, 3.0, 4.5, 4.0, 5.0, 4.8, 6.0];
        let a = acf(&s, 4);
        for (x, y) in a.iter().zip([1.0, 0.555_687_2, 0.544_233_81, 0.146_919_43, 0.168_246_45]) {
            assert!(close(*x, y, 1e-7), "{x} {y}");
        }
        let p = pacf(&s, 3);
        for (x, y) in p.iter().zip([0.555_687_2, 0.340_627_23, -0.395_691_95]) {
            assert!(close(*x, y, 1e-7), "{x} {y}");
        }
    }

    fn ar1(n: usize, phi: f64) -> Vec<f64> {
        let mut rng = crate::ml::Rng::new(3);
        let mut x = vec![0.0];
        for _ in 1..n {
            // Box–Muller normal noise.
            let (u1, u2) = (rng.f64().max(1e-12), rng.f64());
            let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
            let prev = *x.last().unwrap();
            x.push(10.0 * (1.0 - phi) + phi * prev + z);
        }
        x
    }

    #[test]
    fn arima_recovers_ar1() {
        let x = ar1(600, 0.7);
        let m = arima(&x, 1, 0, 0, true).unwrap();
        assert!(close(m.ar[0], 0.7, 0.07), "{:?}", m.ar);
        assert!(close(m.sigma2, 1.0, 0.15), "{}", m.sigma2);
        let f = m.forecast(50, 0.95);
        // Long-run forecast goes to the process mean (10).
        assert!(close(*f.mean.last().unwrap(), super::super::mean(&x), 1.0));
        assert!(f.upper[0] > f.mean[0] && f.upper[10] - f.lower[10] > f.upper[0] - f.lower[0]);
        let auto = auto_arima(&x, Some(0), true).unwrap();
        assert!(auto.p >= 1);
    }

    #[test]
    fn arima_trend_forecast() {
        let x: Vec<f64> = (0..60).map(|i| 5.0 + 2.0 * i as f64 + if i % 2 == 0 { 0.3 } else { -0.3 }).collect();
        let m = arima(&x, 0, 1, 1, true).unwrap();
        let f = m.forecast(3, 0.95);
        assert!(close(f.mean[0], 5.0 + 2.0 * 60.0, 1.5), "{:?}", f.mean);
    }

    #[test]
    fn decomposition_and_ets() {
        let x: Vec<f64> = (0..48).map(|i| 100.0 + i as f64 + [5.0, -3.0, 2.0, -4.0][i % 4]).collect();
        let d = decompose(&x, 4, false).unwrap();
        assert!(close(d.seasonal[0], 5.0, 1e-9), "{:?}", &d.seasonal[..4]);
        assert!(close(d.trend[10], 110.0, 1e-9));
        assert!(d.seasonal_strength > 0.9);
        let s = stl(&x, 4).unwrap();
        assert!(close(s.seasonal[20], 5.0, 0.5), "{}", s.seasonal[20]);
        let hw = ets(&x, EtsKind::HoltWintersAdditive, 4).unwrap();
        let f = hw.forecast(4, 0.95);
        for (i, v) in f.mean.iter().enumerate() {
            let truth = 100.0 + (48 + i) as f64 + [5.0, -3.0, 2.0, -4.0][(48 + i) % 4];
            assert!(close(*v, truth, 1.5), "{v} {truth}");
        }
        let lin: Vec<f64> = (0..30).map(|i| 3.0 * i as f64).collect();
        let h = ets(&lin, EtsKind::Holt, 1).unwrap();
        assert!(close(h.forecast(1, 0.95).mean[0], 90.0, 0.5));
    }
}
