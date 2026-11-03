//! Analytics pane: reference lines, trend lines, forecasting, clustering and anomalies,
//! computed from a visual's data.
//!
//! Category charts work on the *category series*: the value summed per distinct axis value,
//! in the order categories first appear in the rows (the same order the renderers lay them
//! out). Indices in [`Computed::Series`], [`Computed::Forecast`], [`Computed::ErrorBars`] and
//! [`Computed::Anomalies`] (for category charts) are category indices; for scatter charts
//! `Anomalies::rows` and `Clusters::assignment` are data rows.

use crate::color::Color;
use crate::doc::AnalyticsLine;
use crate::vquery::VisualData;

/// A computed analytics result, in data space.
#[derive(Clone, Debug, PartialEq)]
pub enum Computed {
    /// A horizontal (value-axis) reference line.
    ValueLine { value: f64, label: String, color: Color, width: f32, dash: String },
    /// A band between two values.
    Band { from: f64, to: f64, label: String, color: Color },
    /// A series of points over category indices (trend lines; `x` = category index).
    Series { points: Vec<(f64, f64)>, label: String, color: Color, width: f32, dash: String },
    /// Forecast continuation from category `start_index` on: values and confidence bounds.
    Forecast { start_index: usize, values: Vec<f64>, lower: Vec<f64>, upper: Vec<f64>, color: Color },
    /// Cluster index per row.
    Clusters { assignment: Vec<usize>, k: usize },
    /// Rows (category indices for category charts) flagged as anomalies.
    Anomalies { rows: Vec<usize>, color: Color },
    /// Error bars per category index: (low, high).
    ErrorBars { low: Vec<f64>, high: Vec<f64>, color: Color },
}

/// The value column an analytics line applies to.
fn value_col(l: &AnalyticsLine, data: &VisualData) -> Option<usize> {
    if !l.series.is_empty()
        && let Some(i) = data.cols.iter().position(|c| c.is_value && c.label == l.series)
    {
        return Some(i);
    }
    data.first("values").filter(|i| data.cols.get(*i).is_some_and(|c| c.is_value)).or_else(|| data.first("y")).or_else(|| data.values().first().copied())
}

/// Category key columns: the axis columns (or the first category column).
pub fn category_cols(data: &VisualData) -> Vec<usize> {
    let axis = data.role("axis");
    if !axis.is_empty() {
        return axis;
    }
    for r in ["rows", "details", "location", "legend"] {
        let v = data.role(r);
        if !v.is_empty() {
            return v;
        }
    }
    vec![]
}

/// The category series: the value summed per category, in first-appearance order.
pub fn category_values(data: &VisualData, vi: usize) -> Vec<f64> {
    let cc = category_cols(data);
    if cc.is_empty() {
        return data.rows.iter().map(|r| r.get(vi).and_then(|v| v.as_f64()).unwrap_or(f64::NAN)).collect();
    }
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut out: Vec<f64> = Vec::new();
    for r in &data.rows {
        let k = cc.iter().map(|c| r.get(*c).map(crate::scalar::Scalar::key).unwrap_or_default()).collect::<Vec<_>>().join("\u{1}");
        let v = r.get(vi).and_then(|v| v.as_f64());
        let i = *index.entry(k).or_insert_with(|| {
            out.push(f64::NAN);
            out.len() - 1
        });
        if let (Some(slot), Some(v)) = (out.get_mut(i), v) {
            *slot = if slot.is_nan() { v } else { *slot + v };
        }
    }
    out
}

/// Computes the analytics lines of a visual.
pub fn compute(lines: &[AnalyticsLine], data: &VisualData) -> Vec<Computed> {
    let mut out = Vec::new();
    for l in lines {
        let Some(vi) = value_col(l, data) else { continue };
        let series = category_values(data, vi);
        let vals: Vec<f64> = series.iter().copied().filter(|v| v.is_finite()).collect();
        let label = if l.name.is_empty() { l.kind.clone() } else { l.name.clone() };
        let simple = match l.kind.as_str() {
            "Constant" | "Constant line" => l.value.as_num(),
            "Min" => vals.iter().cloned().reduce(f64::min),
            "Max" => vals.iter().cloned().reduce(f64::max),
            "Average" => (!vals.is_empty()).then(|| mean(&vals)),
            "Median" => (!vals.is_empty()).then(|| percentile(&vals, 50.0)),
            "Percentile" => (!vals.is_empty()).then(|| percentile(&vals, l.percentile)),
            _ => None,
        };
        if let Some(v) = simple {
            if v.is_finite() {
                out.push(Computed::ValueLine { value: v, label, color: l.color, width: l.width, dash: l.dash.clone() });
            }
            continue;
        }
        match l.kind.as_str() {
            "Reference band" | "Band" => {
                let a = l.value.as_num().unwrap_or(0.0);
                let b = l.band_to.as_ref().and_then(|p| p.as_num()).unwrap_or(a);
                out.push(Computed::Band { from: a.min(b), to: a.max(b), label, color: l.color.with_alpha(40) });
            }
            "Trend" | "Trend line" => {
                if let Some((a, b)) = linear_fit(&series) {
                    let n = series.len().saturating_sub(1) as f64;
                    out.push(Computed::Series { points: vec![(0.0, a), (n, a + b * n)], label, color: l.color, width: l.width, dash: l.dash.clone() });
                }
            }
            "Forecast" => {
                if let Some(f) = forecast(&series, l) {
                    out.push(f);
                }
            }
            "Error bars" => {
                if vals.len() >= 2 {
                    let sd = stddev(&vals);
                    out.push(Computed::ErrorBars { low: series.iter().map(|v| v - sd).collect(), high: series.iter().map(|v| v + sd).collect(), color: l.color });
                }
            }
            "Cluster" | "Clusters" => {
                let (Some(x), Some(y)) = (data.first("x"), data.first("y")) else { continue };
                let pts: Vec<(f64, f64)> = data.rows.iter().map(|r| (r.get(x).and_then(|v| v.as_f64()).unwrap_or(f64::NAN), r.get(y).and_then(|v| v.as_f64()).unwrap_or(f64::NAN))).collect();
                let k = l.clusters.clamp(1, 12);
                out.push(Computed::Clusters { assignment: kmeans(&pts, k), k });
            }
            "Anomalies" | "Find anomalies" => {
                let rows = if data.first("x").is_some() && data.first("y").is_some() && data.first("axis").is_none() {
                    let y = data.first("y").unwrap_or(vi);
                    let v: Vec<f64> = data.rows.iter().map(|r| r.get(y).and_then(|v| v.as_f64()).unwrap_or(f64::NAN)).collect();
                    zscore_outliers(&v, 2.5)
                } else {
                    anomalies(&series, l)
                };
                out.push(Computed::Anomalies { rows, color: Color::rgb(0xDC, 0x26, 0x26) });
            }
            _ => {}
        }
    }
    out
}

pub fn mean(v: &[f64]) -> f64 {
    if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
}

pub fn stddev(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = mean(v);
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt()
}

pub fn percentile(vals: &[f64], p: f64) -> f64 {
    let mut v: Vec<f64> = vals.iter().cloned().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = (p / 100.0).clamp(0.0, 1.0) * (v.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let a = v.get(lo).copied().unwrap_or(0.0);
    let b = v.get(hi).copied().unwrap_or(a);
    a + (b - a) * (rank - lo as f64)
}

/// Least squares `y = a + b·x` over the finite points of a series (x = index).
pub fn linear_fit(series: &[f64]) -> Option<(f64, f64)> {
    let pts: Vec<(f64, f64)> = series.iter().enumerate().filter(|(_, v)| v.is_finite()).map(|(i, v)| (i as f64, *v)).collect();
    if pts.len() < 2 {
        return None;
    }
    let n = pts.len() as f64;
    let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let sxy: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    if sxx == 0.0 {
        return None;
    }
    let b = sxy / sxx;
    Some((my - b * mx, b))
}

/// Inverse of the standard normal CDF (Acklam's approximation).
#[allow(clippy::excessive_precision)]
pub fn probit(p: f64) -> f64 {
    let p = p.clamp(1e-9, 1.0 - 1e-9);
    let a = [-3.969683028665376e+01, 2.209460984245205e+02, -2.759285104469687e+02, 1.383577518672690e+02, -3.066479806614716e+01, 2.506628277459239e+00];
    let b = [-5.447609879822406e+01, 1.615858368580409e+02, -1.556989798598866e+02, 6.680131188771972e+01, -1.328068155288572e+01];
    let c = [-7.784894002430293e-03, -3.223964580411365e-01, -2.400758277161838e+00, -2.549732539343734e+00, 4.374664141464968e+00, 2.938163982698783e+00];
    let d = [7.784695709041462e-03, 3.224671290700398e-01, 2.445134137142996e+00, 3.754408661907416e+00];
    let pl = 0.02425;
    if p < pl {
        let q = (-2.0 * p.ln()).sqrt();
        (((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5]) / ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0)
    } else if p <= 1.0 - pl {
        let q = p - 0.5;
        let r = q * q;
        (((((a[0] * r + a[1]) * r + a[2]) * r + a[3]) * r + a[4]) * r + a[5]) * q / (((((b[0] * r + b[1]) * r + b[2]) * r + b[3]) * r + b[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5]) / ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0)
    }
}

/// Seasonality by autocorrelation: the lag (2..n/2) with the highest autocorrelation peak
/// above 0.3, or 0.
pub fn detect_season(y: &[f64]) -> usize {
    let n = y.len();
    if n < 8 {
        return 0;
    }
    // Detrend first so a strong trend doesn't dominate.
    let (a, b) = linear_fit(y).unwrap_or((mean(y), 0.0));
    let r: Vec<f64> = y.iter().enumerate().map(|(i, v)| v - (a + b * i as f64)).collect();
    let m = mean(&r);
    let var: f64 = r.iter().map(|x| (x - m).powi(2)).sum();
    if var <= 0.0 {
        return 0;
    }
    let acf = |lag: usize| -> f64 { (lag..n).map(|i| (r.get(i).copied().unwrap_or(0.0) - m) * (r.get(i - lag).copied().unwrap_or(0.0) - m)).sum::<f64>() / var };
    let mut best = (0usize, 0.3f64);
    for lag in 2..=(n / 2) {
        let v = acf(lag);
        let prev = acf(lag - 1);
        let next = if lag < n / 2 { acf(lag + 1) } else { f64::MIN };
        if v > best.1 && v >= prev && v >= next {
            best = (lag, v);
        }
    }
    best.0
}

/// Holt-Winters additive smoothing fit: returns (one-step fitted values, final level, trend,
/// seasonal components, sse).
fn hw_fit(y: &[f64], m: usize, alpha: f64, beta: f64, gamma: f64) -> (Vec<f64>, f64, f64, Vec<f64>, f64) {
    let n = y.len();
    let mut fitted = vec![f64::NAN; n];
    if m >= 2 && n >= 2 * m {
        let s1 = mean(y.get(..m).unwrap_or(&[]));
        let s2 = mean(y.get(m..2 * m).unwrap_or(&[]));
        let mut level = s1;
        let mut trend = (s2 - s1) / m as f64;
        let mut season: Vec<f64> = (0..m).map(|i| y.get(i).copied().unwrap_or(0.0) - s1).collect();
        let mut sse = 0.0;
        for t in 0..n {
            let si = t % m;
            let s = season.get(si).copied().unwrap_or(0.0);
            let f = level + trend + s;
            let yt = y.get(t).copied().unwrap_or(f);
            if t >= m {
                if let Some(x) = fitted.get_mut(t) {
                    *x = f;
                }
                sse += (yt - f).powi(2);
            }
            let prev_level = level;
            level = alpha * (yt - s) + (1.0 - alpha) * (level + trend);
            trend = beta * (level - prev_level) + (1.0 - beta) * trend;
            if let Some(x) = season.get_mut(si) {
                *x = gamma * (yt - level) + (1.0 - gamma) * s;
            }
        }
        // Rotate so season[0] is the component of time n.
        let rot: Vec<f64> = (0..m).map(|i| season.get((n + i) % m).copied().unwrap_or(0.0)).collect();
        (fitted, level, trend, rot, sse)
    } else {
        // Holt linear.
        let mut level = y.first().copied().unwrap_or(0.0);
        let mut trend = if n >= 2 { y.get(1).copied().unwrap_or(level) - level } else { 0.0 };
        let mut sse = 0.0;
        for t in 1..n {
            let f = level + trend;
            let yt = y.get(t).copied().unwrap_or(f);
            if let Some(x) = fitted.get_mut(t) {
                *x = f;
            }
            sse += (yt - f).powi(2);
            let prev = level;
            level = alpha * yt + (1.0 - alpha) * (level + trend);
            trend = beta * (level - prev) + (1.0 - beta) * trend;
        }
        (fitted, level, trend, vec![], sse)
    }
}

/// Best Holt-Winters fit by grid search over the smoothing parameters.
#[allow(clippy::type_complexity)]
fn hw_best(y: &[f64], m: usize) -> (Vec<f64>, f64, f64, Vec<f64>, f64) {
    let grid = [0.1, 0.3, 0.5, 0.7, 0.9];
    let gammas: &[f64] = if m >= 2 && y.len() >= 2 * m { &[0.05, 0.2, 0.4, 0.6] } else { &[0.0] };
    let mut best: Option<(Vec<f64>, f64, f64, Vec<f64>, f64)> = None;
    for a in grid {
        for b in [0.01, 0.1, 0.3, 0.5] {
            for g in gammas {
                let r = hw_fit(y, m, a, b, *g);
                if best.as_ref().is_none_or(|bst| r.4 < bst.4) {
                    best = Some(r);
                }
            }
        }
    }
    best.unwrap_or_else(|| hw_fit(y, m, 0.5, 0.1, 0.1))
}

/// Forecast after the series (Holt-Winters additive; Holt linear without seasonality).
pub fn forecast(series: &[f64], l: &AnalyticsLine) -> Option<Computed> {
    let n_all = series.len();
    let ignore = l.ignore_last.min(n_all.saturating_sub(3));
    let mut y: Vec<f64> = series.get(..n_all - ignore).unwrap_or(&[]).to_vec();
    // Fill gaps with the previous value.
    let mut last = y.iter().copied().find(|v| v.is_finite())?;
    for v in y.iter_mut() {
        if v.is_finite() {
            last = *v;
        } else {
            *v = last;
        }
    }
    let n = y.len();
    if n < 3 {
        return None;
    }
    let mut m = if l.seasonality > 0 { l.seasonality } else { detect_season(&y) };
    if m < 2 || n < 2 * m {
        m = 0;
    }
    let (fitted, level, trend, season, sse) = hw_best(&y, m);
    let used = fitted.iter().filter(|v| v.is_finite()).count().max(1);
    let params = if m > 0 { 3 } else { 2 };
    let sigma = (sse / (used.saturating_sub(params).max(1)) as f64).sqrt();
    let conf = if l.confidence > 1.0 { l.confidence / 100.0 } else if l.confidence > 0.0 { l.confidence } else { 0.95 };
    let z = probit(0.5 + conf / 2.0);
    let h_len = l.forecast_length.max(1) + ignore;
    let mut values = Vec::with_capacity(h_len);
    let mut lower = Vec::with_capacity(h_len);
    let mut upper = Vec::with_capacity(h_len);
    for h in 1..=h_len {
        let s = if m > 0 { season.get((h - 1) % m).copied().unwrap_or(0.0) } else { 0.0 };
        let f = level + trend * h as f64 + s;
        let w = z * sigma * (1.0 + 0.25 * (h as f64 - 1.0)).sqrt();
        values.push(f);
        lower.push(f - w);
        upper.push(f + w);
    }
    Some(Computed::Forecast { start_index: n, values, lower, upper, color: l.color })
}

/// Category indices whose value is unusual: residual from a Holt-Winters fit beyond 2.5σ, or
/// a global |z| > 2.5.
pub fn anomalies(series: &[f64], l: &AnalyticsLine) -> Vec<usize> {
    let vals: Vec<f64> = series.iter().copied().filter(|v| v.is_finite()).collect();
    let mut out = zscore_outliers(series, 2.5);
    if vals.len() >= 8 && vals.len() == series.len() {
        let m = if l.seasonality > 0 { l.seasonality } else { detect_season(series) };
        let m = if m >= 2 && series.len() >= 2 * m { m } else { 0 };
        let (fitted, ..) = hw_best(series, m);
        let res: Vec<f64> = series.iter().zip(fitted.iter()).filter(|(_, f)| f.is_finite()).map(|(y, f)| y - f).collect();
        let sd = stddev(&res);
        if sd > 0.0 {
            for (i, (y, f)) in series.iter().zip(fitted.iter()).enumerate() {
                if f.is_finite() && ((y - f) / sd).abs() > 2.5 && !out.contains(&i) {
                    out.push(i);
                }
            }
        }
    }
    out.sort_unstable();
    out
}

pub fn zscore_outliers(v: &[f64], z: f64) -> Vec<usize> {
    let vals: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    let (m, sd) = (mean(&vals), stddev(&vals));
    if sd <= 0.0 {
        return vec![];
    }
    v.iter().enumerate().filter(|(_, x)| x.is_finite() && ((**x - m) / sd).abs() > z).map(|(i, _)| i).collect()
}

/// Deterministic k-means on standardized points (farthest-point initialisation). NaN points
/// get cluster 0.
pub fn kmeans(pts: &[(f64, f64)], k: usize) -> Vec<usize> {
    let valid: Vec<usize> = (0..pts.len()).filter(|i| pts.get(*i).is_some_and(|p| p.0.is_finite() && p.1.is_finite())).collect();
    let mut assign = vec![0usize; pts.len()];
    if valid.is_empty() || k <= 1 {
        return assign;
    }
    let xs: Vec<f64> = valid.iter().filter_map(|i| pts.get(*i).map(|p| p.0)).collect();
    let ys: Vec<f64> = valid.iter().filter_map(|i| pts.get(*i).map(|p| p.1)).collect();
    let (mx, sx) = (mean(&xs), stddev(&xs).max(1e-12));
    let (my, sy) = (mean(&ys), stddev(&ys).max(1e-12));
    let z: Vec<(f64, f64)> = valid.iter().filter_map(|i| pts.get(*i)).map(|p| ((p.0 - mx) / sx, (p.1 - my) / sy)).collect();
    let k = k.min(z.len());
    let d2 = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2);
    // Init: the point farthest from the mean, then farthest from chosen centres.
    let mut centres: Vec<(f64, f64)> = Vec::new();
    let first = z.iter().enumerate().max_by(|a, b| d2(*a.1, (0.0, 0.0)).partial_cmp(&d2(*b.1, (0.0, 0.0))).unwrap_or(std::cmp::Ordering::Equal)).map(|(i, _)| i).unwrap_or(0);
    centres.push(z.get(first).copied().unwrap_or_default());
    while centres.len() < k {
        let far = z
            .iter()
            .map(|p| centres.iter().map(|c| d2(*p, *c)).fold(f64::INFINITY, f64::min))
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap_or(0);
        centres.push(z.get(far).copied().unwrap_or_default());
    }
    let mut lab = vec![0usize; z.len()];
    for _ in 0..50 {
        let mut changed = false;
        for (i, p) in z.iter().enumerate() {
            let best = centres.iter().enumerate().map(|(ci, c)| (ci, d2(*p, *c))).min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)).map(|(ci, _)| ci).unwrap_or(0);
            if let Some(l) = lab.get_mut(i)
                && *l != best
            {
                *l = best;
                changed = true;
            }
        }
        for (ci, c) in centres.iter_mut().enumerate() {
            let members: Vec<&(f64, f64)> = z.iter().zip(lab.iter()).filter(|(_, l)| **l == ci).map(|(p, _)| p).collect();
            if !members.is_empty() {
                *c = (members.iter().map(|p| p.0).sum::<f64>() / members.len() as f64, members.iter().map(|p| p.1).sum::<f64>() / members.len() as f64);
            }
        }
        if !changed {
            break;
        }
    }
    // Number clusters by their centre's x so colours are stable.
    let mut order: Vec<usize> = (0..centres.len()).collect();
    order.sort_by(|a, b| centres.get(*a).map(|c| c.0).partial_cmp(&centres.get(*b).map(|c| c.0)).unwrap_or(std::cmp::Ordering::Equal));
    let rank: Vec<usize> = (0..centres.len()).map(|c| order.iter().position(|x| *x == c).unwrap_or(0)).collect();
    for (vi, i) in valid.iter().enumerate() {
        if let Some(a) = assign.get_mut(*i) {
            *a = lab.get(vi).and_then(|l| rank.get(*l)).copied().unwrap_or(0);
        }
    }
    assign
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forecast_seasonal() {
        let y: Vec<f64> = (0..36).map(|i| 100.0 + i as f64 * 2.0 + 20.0 * ((i % 12) as f64 / 12.0 * std::f64::consts::TAU).sin()).collect();
        assert_eq!(detect_season(&y), 12);
        let l = AnalyticsLine { kind: "Forecast".into(), forecast_length: 12, ..AnalyticsLine::default() };
        let Some(Computed::Forecast { values, lower, upper, start_index, .. }) = forecast(&y, &l) else { panic!("no forecast") };
        assert_eq!(start_index, 36);
        assert_eq!(values.len(), 12);
        // Next point ≈ 100 + 72 + 0.
        assert!((values[0] - 172.0).abs() < 8.0, "{}", values[0]);
        assert!(lower[0] <= values[0] && upper[0] >= values[0]);
    }

    #[test]
    fn kmeans_separates() {
        let mut pts = vec![];
        for i in 0..10 {
            pts.push((i as f64 * 0.1, 0.0));
            pts.push((100.0 + i as f64 * 0.1, 50.0));
        }
        let a = kmeans(&pts, 2);
        assert_ne!(a[0], a[1]);
        assert_eq!(a[0], a[2]);
    }

    #[test]
    fn probit_values() {
        assert!((probit(0.975) - 1.96).abs() < 0.01);
    }
}
