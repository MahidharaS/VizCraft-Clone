//! Hypothesis tests: t-tests, ANOVA + Tukey HSD, chi-square independence, correlation tests,
//! normality tests and rank tests.

use std::collections::BTreeMap;

use super::special::*;
use super::{fmt, fmt_p, mean, ranks, variance, Report};

/// A test result: statistic, degrees of freedom, p-value (two-sided unless stated) and extras.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TestResult {
    pub name: String,
    pub statistic_name: String,
    pub statistic: f64,
    pub df: Option<f64>,
    pub p_value: f64,
    /// Extra values (mean difference, CI, effect size…).
    pub extra: Vec<(String, f64)>,
}

impl TestResult {
    pub fn get(&self, k: &str) -> f64 {
        self.extra.iter().find(|(n, _)| n == k).map(|(_, v)| *v).unwrap_or(f64::NAN)
    }
    pub fn report(&self, h0: &str) -> Report {
        let mut r = Report::new(self.name.clone());
        let s = r.section("Result");
        s.kv(self.statistic_name.clone(), fmt(self.statistic));
        if let Some(df) = self.df {
            s.kv("Degrees of freedom", fmt(df));
        }
        s.kv("p-value", fmt_p(self.p_value));
        for (k, v) in &self.extra {
            s.kv(k.clone(), fmt(*v));
        }
        s.note = conclusion(self.p_value, h0);
        r
    }
}

/// "Reject H0…" sentence at 5 %.
pub fn conclusion(p: f64, h0: &str) -> String {
    if p.is_nan() {
        "Not enough data for a conclusion.".into()
    } else if p < 0.05 {
        format!("p = {} < 0.05: reject the null hypothesis ({h0}) at the 5% level.", fmt_p(p))
    } else {
        format!("p = {} ≥ 0.05: no evidence against the null hypothesis ({h0}) at the 5% level.", fmt_p(p))
    }
}

fn fin(x: &[f64]) -> Vec<f64> {
    super::finite(x)
}

// =============================================================================================
// t-tests
// =============================================================================================

pub fn one_sample_t(x: &[f64], mu: f64) -> TestResult {
    let x = fin(x);
    let n = x.len() as f64;
    let m = mean(&x);
    let sd = variance(&x).sqrt();
    let se = sd / n.sqrt();
    let t = (m - mu) / se;
    let df = n - 1.0;
    let tc = t_ppf(0.975, df);
    TestResult {
        name: "One-sample t-test".into(),
        statistic_name: "t".into(),
        statistic: t,
        df: Some(df),
        p_value: t_two_sided(t, df),
        extra: vec![("Mean".into(), m), ("Hypothesised mean".into(), mu), ("Mean difference".into(), m - mu), ("95% CI low".into(), m - mu - tc * se), ("95% CI high".into(), m - mu + tc * se), ("Cohen's d".into(), (m - mu) / sd)],
    }
}

/// Two independent samples: Student (pooled variance) or Welch.
pub fn two_sample_t(a: &[f64], b: &[f64], equal_var: bool) -> TestResult {
    let (a, b) = (fin(a), fin(b));
    let (n1, n2) = (a.len() as f64, b.len() as f64);
    let (m1, m2) = (mean(&a), mean(&b));
    let (v1, v2) = (variance(&a), variance(&b));
    let (t, df, se) = if equal_var {
        let sp = ((n1 - 1.0) * v1 + (n2 - 1.0) * v2) / (n1 + n2 - 2.0);
        let se = (sp * (1.0 / n1 + 1.0 / n2)).sqrt();
        ((m1 - m2) / se, n1 + n2 - 2.0, se)
    } else {
        let se2 = v1 / n1 + v2 / n2;
        let df = se2 * se2 / ((v1 / n1).powi(2) / (n1 - 1.0) + (v2 / n2).powi(2) / (n2 - 1.0));
        ((m1 - m2) / se2.sqrt(), df, se2.sqrt())
    };
    let tc = t_ppf(0.975, df);
    let sp = (((n1 - 1.0) * v1 + (n2 - 1.0) * v2) / (n1 + n2 - 2.0)).sqrt();
    TestResult {
        name: if equal_var { "Two-sample t-test (equal variances)".into() } else { "Welch's t-test (unequal variances)".into() },
        statistic_name: "t".into(),
        statistic: t,
        df: Some(df),
        p_value: t_two_sided(t, df),
        extra: vec![("Mean (group 1)".into(), m1), ("Mean (group 2)".into(), m2), ("Mean difference".into(), m1 - m2), ("95% CI low".into(), m1 - m2 - tc * se), ("95% CI high".into(), m1 - m2 + tc * se), ("Cohen's d".into(), (m1 - m2) / sp)],
    }
}

pub fn paired_t(a: &[f64], b: &[f64]) -> TestResult {
    let d: Vec<f64> = a.iter().zip(b).filter(|(x, y)| x.is_finite() && y.is_finite()).map(|(x, y)| x - y).collect();
    let mut r = one_sample_t(&d, 0.0);
    r.name = "Paired t-test".into();
    r
}

// =============================================================================================
// ANOVA
// =============================================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnovaRow {
    pub source: String,
    pub df: f64,
    pub ss: f64,
    pub ms: f64,
    pub f: f64,
    pub p: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Anova {
    pub rows: Vec<AnovaRow>,
    pub eta_squared: f64,
}

pub fn one_way_anova(groups: &[Vec<f64>]) -> Anova {
    let groups: Vec<Vec<f64>> = groups.iter().map(|g| fin(g)).filter(|g| !g.is_empty()).collect();
    let all: Vec<f64> = groups.iter().flatten().copied().collect();
    let n = all.len() as f64;
    let k = groups.len() as f64;
    let gm = mean(&all);
    let ssb: f64 = groups.iter().map(|g| g.len() as f64 * (mean(g) - gm).powi(2)).sum();
    let ssw: f64 = groups.iter().map(|g| {
        let m = mean(g);
        g.iter().map(|v| (v - m).powi(2)).sum::<f64>()
    }).sum();
    let (dfb, dfw) = (k - 1.0, n - k);
    let (msb, msw) = (ssb / dfb, ssw / dfw);
    let f = msb / msw;
    Anova {
        rows: vec![
            AnovaRow { source: "Between groups".into(), df: dfb, ss: ssb, ms: msb, f, p: f_sf(f, dfb, dfw) },
            AnovaRow { source: "Within groups".into(), df: dfw, ss: ssw, ms: msw, f: f64::NAN, p: f64::NAN },
            AnovaRow { source: "Total".into(), df: n - 1.0, ss: ssb + ssw, ms: f64::NAN, f: f64::NAN, p: f64::NAN },
        ],
        eta_squared: ssb / (ssb + ssw),
    }
}

/// Residual sum of squares of y on dummy-coded factors (with intercept).
fn sse_dummies(y: &[f64], cols: &[Vec<f64>]) -> (f64, usize) {
    let n = y.len();
    let mut x: Vec<Vec<f64>> = (0..n).map(|i| std::iter::once(1.0).chain(cols.iter().map(|c| c[i])).collect()).collect();
    if x.is_empty() {
        x = vec![];
    }
    let (xtx, xty) = super::optim::normal_equations(&x, y, None);
    let Some(inv) = super::optim::inverse(&xtx) else { return (f64::NAN, cols.len() + 1) };
    let b = super::optim::mat_vec(&inv, &xty);
    let sse = x.iter().zip(y).map(|(r, yi)| (yi - super::optim::dot(r, &b)).powi(2)).sum();
    (sse, cols.len() + 1)
}

fn dummies(levels: &[String], lab: &[String]) -> Vec<Vec<f64>> {
    levels.iter().skip(1).map(|l| lab.iter().map(|v| if v == l { 1.0 } else { 0.0 }).collect()).collect()
}

/// Two-way ANOVA with interaction (sequential / Type I sums of squares, as R's `aov`).
pub fn two_way_anova(y: &[f64], a: &[String], b: &[String], name_a: &str, name_b: &str) -> Anova {
    let keep: Vec<usize> = (0..y.len().min(a.len()).min(b.len())).filter(|i| y[*i].is_finite()).collect();
    let y: Vec<f64> = keep.iter().map(|i| y[*i]).collect();
    let a: Vec<String> = keep.iter().map(|i| a[*i].clone()).collect();
    let b: Vec<String> = keep.iter().map(|i| b[*i].clone()).collect();
    let lev = |v: &[String]| {
        let mut l: Vec<String> = v.to_vec();
        l.sort();
        l.dedup();
        l
    };
    let (la, lb) = (lev(&a), lev(&b));
    let da = dummies(&la, &a);
    let db = dummies(&lb, &b);
    let mut dab = Vec::new();
    for ca in &da {
        for cb in &db {
            dab.push(ca.iter().zip(cb).map(|(x, z)| x * z).collect::<Vec<f64>>());
        }
    }
    let n = y.len() as f64;
    let m = mean(&y);
    let sst: f64 = y.iter().map(|v| (v - m).powi(2)).sum();
    let (s_a, _) = sse_dummies(&y, &da);
    let ab: Vec<Vec<f64>> = da.iter().chain(db.iter()).cloned().collect();
    let (s_ab, _) = sse_dummies(&y, &ab);
    let full: Vec<Vec<f64>> = ab.iter().chain(dab.iter()).cloned().collect();
    let (s_full, p_full) = sse_dummies(&y, &full);
    let df_a = (la.len() as f64 - 1.0).max(0.0);
    let df_b = (lb.len() as f64 - 1.0).max(0.0);
    let df_ab = df_a * df_b;
    let df_e = n - p_full as f64;
    let mse = s_full / df_e;
    let row = |src: String, df: f64, ss: f64| {
        let ms = ss / df;
        let f = ms / mse;
        AnovaRow { source: src, df, ss, ms, f, p: f_sf(f, df, df_e) }
    };
    Anova {
        rows: vec![
            row(name_a.to_string(), df_a, sst - s_a),
            row(name_b.to_string(), df_b, s_a - s_ab),
            row(format!("{name_a} × {name_b}"), df_ab, s_ab - s_full),
            AnovaRow { source: "Residual".into(), df: df_e, ss: s_full, ms: mse, f: f64::NAN, p: f64::NAN },
        ],
        eta_squared: (sst - s_full) / sst,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TukeyPair {
    pub a: String,
    pub b: String,
    pub diff: f64,
    pub lower: f64,
    pub upper: f64,
    pub p_adj: f64,
}

/// Tukey's honestly significant difference (Tukey–Kramer for unequal sizes), 95 %.
pub fn tukey_hsd(names: &[String], groups: &[Vec<f64>]) -> Vec<TukeyPair> {
    let groups: Vec<Vec<f64>> = groups.iter().map(|g| fin(g)).collect();
    let k = groups.len();
    let n: usize = groups.iter().map(|g| g.len()).sum();
    if k < 2 || n <= k {
        return vec![];
    }
    let dfw = (n - k) as f64;
    let ssw: f64 = groups.iter().map(|g| {
        let m = mean(g);
        g.iter().map(|v| (v - m).powi(2)).sum::<f64>()
    }).sum();
    let msw = ssw / dfw;
    let qc = qtukey(0.95, k as f64, dfw);
    let mut out = vec![];
    for i in 0..k {
        for j in (i + 1)..k {
            let (gi, gj) = (&groups[i], &groups[j]);
            if gi.is_empty() || gj.is_empty() {
                continue;
            }
            let diff = mean(gj) - mean(gi);
            let se = (msw / 2.0 * (1.0 / gi.len() as f64 + 1.0 / gj.len() as f64)).sqrt();
            let q = diff.abs() / se;
            out.push(TukeyPair {
                a: names.get(i).cloned().unwrap_or_default(),
                b: names.get(j).cloned().unwrap_or_default(),
                diff,
                lower: diff - qc * se,
                upper: diff + qc * se,
                p_adj: (1.0 - ptukey(q, k as f64, dfw)).clamp(0.0, 1.0),
            });
        }
    }
    out
}

pub fn anova_report(title: &str, a: &Anova) -> Report {
    let mut r = Report::new(title);
    let s = r.section("ANOVA table");
    s.table = Some(super::RTable {
        columns: ["Source", "df", "Sum Sq", "Mean Sq", "F", "p-value"].iter().map(|s| s.to_string()).collect(),
        rows: a.rows.iter().map(|x| vec![x.source.clone(), fmt(x.df), fmt(x.ss), fmt(x.ms), fmt(x.f), fmt_p(x.p)]).collect(),
        p_col: Some(5),
    });
    s.kv("η² (effect size)", fmt(a.eta_squared));
    if let Some(first) = a.rows.first() {
        s.note = conclusion(first.p, &format!("all group means are equal for {}", first.source));
    }
    r
}

// =============================================================================================
// Chi-square test of independence
// =============================================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contingency {
    pub rows: Vec<String>,
    pub cols: Vec<String>,
    pub observed: Vec<Vec<f64>>,
    pub expected: Vec<Vec<f64>>,
    pub chi2: f64,
    pub df: f64,
    pub p: f64,
    pub cramers_v: f64,
}

pub fn crosstab(a: &[Option<String>], b: &[Option<String>]) -> (Vec<String>, Vec<String>, Vec<Vec<f64>>) {
    let mut ra: BTreeMap<String, usize> = BTreeMap::new();
    let mut cb: BTreeMap<String, usize> = BTreeMap::new();
    for (x, y) in a.iter().zip(b) {
        if let (Some(x), Some(y)) = (x, y) {
            ra.entry(x.clone()).or_default();
            cb.entry(y.clone()).or_default();
        }
    }
    let rows: Vec<String> = ra.keys().cloned().collect();
    let cols: Vec<String> = cb.keys().cloned().collect();
    for (i, k) in rows.iter().enumerate() {
        ra.insert(k.clone(), i);
    }
    for (i, k) in cols.iter().enumerate() {
        cb.insert(k.clone(), i);
    }
    let mut obs = vec![vec![0.0; cols.len()]; rows.len()];
    for (x, y) in a.iter().zip(b) {
        if let (Some(x), Some(y)) = (x, y)
            && let (Some(i), Some(j)) = (ra.get(x), cb.get(y))
        {
            obs[*i][*j] += 1.0;
        }
    }
    (rows, cols, obs)
}

pub fn chi2_independence(rows: Vec<String>, cols: Vec<String>, observed: Vec<Vec<f64>>) -> Contingency {
    let rs: Vec<f64> = observed.iter().map(|r| r.iter().sum()).collect();
    let nc = cols.len();
    let cs: Vec<f64> = (0..nc).map(|j| observed.iter().map(|r| r.get(j).copied().unwrap_or(0.0)).sum()).collect();
    let n: f64 = rs.iter().sum();
    let expected: Vec<Vec<f64>> = rs.iter().map(|r| cs.iter().map(|c| r * c / n).collect()).collect();
    let mut chi2 = 0.0;
    for (o, e) in observed.iter().zip(&expected) {
        for (a, b) in o.iter().zip(e) {
            if *b > 0.0 {
                chi2 += (a - b).powi(2) / b;
            }
        }
    }
    let df = ((rows.len() as f64 - 1.0) * (nc as f64 - 1.0)).max(1.0);
    let mind = (rows.len().min(nc) as f64 - 1.0).max(1.0);
    Contingency { p: chi2_sf(chi2, df), cramers_v: (chi2 / (n * mind)).sqrt(), rows, cols, observed, expected, chi2, df }
}

// =============================================================================================
// Correlation tests
// =============================================================================================

fn pairs(x: &[f64], y: &[f64]) -> (Vec<f64>, Vec<f64>) {
    x.iter().zip(y).filter(|(a, b)| a.is_finite() && b.is_finite()).map(|(a, b)| (*a, *b)).unzip()
}

fn r_of(x: &[f64], y: &[f64]) -> f64 {
    let (mx, my) = (mean(x), mean(y));
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (a, b) in x.iter().zip(y) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx).powi(2);
        syy += (b - my).powi(2);
    }
    if sxx <= 0.0 || syy <= 0.0 { f64::NAN } else { sxy / (sxx * syy).sqrt() }
}

pub fn pearson(x: &[f64], y: &[f64]) -> TestResult {
    let (x, y) = pairs(x, y);
    let n = x.len() as f64;
    let r = r_of(&x, &y);
    let df = n - 2.0;
    let t = r * (df / (1.0 - r * r).max(1e-300)).sqrt();
    let z = 0.5 * ((1.0 + r) / (1.0 - r)).ln();
    let se = 1.0 / (n - 3.0).sqrt();
    TestResult {
        name: "Pearson correlation".into(),
        statistic_name: "r".into(),
        statistic: r,
        df: Some(df),
        p_value: t_two_sided(t, df),
        extra: vec![("t".into(), t), ("95% CI low".into(), (z - 1.96 * se).tanh()), ("95% CI high".into(), (z + 1.96 * se).tanh()), ("n".into(), n)],
    }
}

pub fn spearman(x: &[f64], y: &[f64]) -> TestResult {
    let (x, y) = pairs(x, y);
    let n = x.len() as f64;
    let rho = r_of(&ranks(&x), &ranks(&y));
    let df = n - 2.0;
    let t = rho * (df / (1.0 - rho * rho).max(1e-300)).sqrt();
    TestResult { name: "Spearman rank correlation".into(), statistic_name: "ρ".into(), statistic: rho, df: Some(df), p_value: t_two_sided(t, df), extra: vec![("n".into(), n)] }
}

/// Kendall's τ-b with the tie-corrected normal approximation.
pub fn kendall(x: &[f64], y: &[f64]) -> TestResult {
    let (x, y) = pairs(x, y);
    let n = x.len();
    let (mut s, mut tx, mut ty) = (0.0f64, 0.0f64, 0.0f64);
    let (mut conc, mut disc) = (0.0f64, 0.0f64);
    for i in 0..n {
        for j in (i + 1)..n {
            let a = (x[i] - x[j]).signum() * if x[i] == x[j] { 0.0 } else { 1.0 };
            let b = (y[i] - y[j]).signum() * if y[i] == y[j] { 0.0 } else { 1.0 };
            if a == 0.0 && b != 0.0 {
                tx += 1.0;
            } else if b == 0.0 && a != 0.0 {
                ty += 1.0;
            } else if a != 0.0 && b != 0.0 {
                if a * b > 0.0 {
                    conc += 1.0;
                } else {
                    disc += 1.0;
                }
            }
            s += a * b;
        }
    }
    let tau = (conc - disc) / ((conc + disc + tx) * (conc + disc + ty)).sqrt();
    // Tie groups.
    let ties = |v: &[f64]| {
        let mut sv = v.to_vec();
        sv.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut out = vec![];
        let mut i = 0;
        while i < sv.len() {
            let mut j = i;
            while j + 1 < sv.len() && sv[j + 1] == sv[i] {
                j += 1;
            }
            if j > i {
                out.push((j - i + 1) as f64);
            }
            i = j + 1;
        }
        out
    };
    let (t, u) = (ties(&x), ties(&y));
    let nf = n as f64;
    let v0 = nf * (nf - 1.0) * (2.0 * nf + 5.0);
    let vt: f64 = t.iter().map(|t| t * (t - 1.0) * (2.0 * t + 5.0)).sum();
    let vu: f64 = u.iter().map(|u| u * (u - 1.0) * (2.0 * u + 5.0)).sum();
    let v1 = t.iter().map(|t| t * (t - 1.0)).sum::<f64>() * u.iter().map(|u| u * (u - 1.0)).sum::<f64>() / (2.0 * nf * (nf - 1.0));
    let v2 = t.iter().map(|t| t * (t - 1.0) * (t - 2.0)).sum::<f64>() * u.iter().map(|u| u * (u - 1.0) * (u - 2.0)).sum::<f64>() / (9.0 * nf * (nf - 1.0) * (nf - 2.0)).max(1e-300);
    let var = (v0 - vt - vu) / 18.0 + v1 + v2;
    let z = s / var.sqrt();
    TestResult { name: "Kendall rank correlation (τ-b)".into(), statistic_name: "τ".into(), statistic: tau, df: None, p_value: 2.0 * (1.0 - norm_cdf(z.abs())), extra: vec![("z".into(), z), ("n".into(), nf)] }
}

// =============================================================================================
// Normality
// =============================================================================================

/// Shapiro–Wilk W test (Royston's 1995 approximation, 3 ≤ n ≤ 5000).
pub fn shapiro_wilk(x: &[f64]) -> TestResult {
    let mut v = super::sorted(x);
    if v.len() > 5000 {
        // Deterministic thinning to 5000 values.
        let step = v.len() as f64 / 5000.0;
        v = (0..5000).filter_map(|i| v.get((i as f64 * step) as usize).copied()).collect();
    }
    let n = v.len();
    let res = |w: f64, p: f64| TestResult { name: "Shapiro–Wilk normality test".into(), statistic_name: "W".into(), statistic: w, df: None, p_value: p, extra: vec![("n".into(), n as f64)] };
    if n < 3 {
        return res(f64::NAN, f64::NAN);
    }
    let nf = n as f64;
    let m: Vec<f64> = (1..=n).map(|i| norm_ppf((i as f64 - 0.375) / (nf + 0.25))).collect();
    let mm: f64 = m.iter().map(|x| x * x).sum();
    let mut a = vec![0.0; n];
    if n == 3 {
        a[0] = -std::f64::consts::FRAC_1_SQRT_2;
        a[2] = std::f64::consts::FRAC_1_SQRT_2;
    } else {
        let u = 1.0 / nf.sqrt();
        let c = |i: usize| m[i] / mm.sqrt();
        let an = c(n - 1) + 0.221157 * u - 0.147981 * u.powi(2) - 2.071190 * u.powi(3) + 4.434685 * u.powi(4) - 2.706056 * u.powi(5);
        if n > 5 {
            let an1 = c(n - 2) + 0.042981 * u - 0.293762 * u.powi(2) - 1.752461 * u.powi(3) + 5.682633 * u.powi(4) - 3.582633 * u.powi(5);
            let phi = (mm - 2.0 * m[n - 1].powi(2) - 2.0 * m[n - 2].powi(2)) / (1.0 - 2.0 * an * an - 2.0 * an1 * an1);
            for i in 2..n - 2 {
                a[i] = m[i] / phi.sqrt();
            }
            a[n - 1] = an;
            a[n - 2] = an1;
            a[0] = -an;
            a[1] = -an1;
        } else {
            let phi = (mm - 2.0 * m[n - 1].powi(2)) / (1.0 - 2.0 * an * an);
            for i in 1..n - 1 {
                a[i] = m[i] / phi.sqrt();
            }
            a[n - 1] = an;
            a[0] = -an;
        }
    }
    let mean_v = mean(&v);
    let ss: f64 = v.iter().map(|x| (x - mean_v).powi(2)).sum();
    if ss <= 0.0 {
        return res(1.0, 1.0);
    }
    let w = (a.iter().zip(&v).map(|(a, x)| a * x).sum::<f64>()).powi(2) / ss;
    let w = w.min(1.0);
    let p = if n == 3 {
        (6.0 / std::f64::consts::PI * (w.sqrt().asin() - 0.75f64.sqrt().asin())).clamp(0.0, 1.0)
    } else if n <= 11 {
        let g = -2.273 + 0.459 * nf;
        let mu = 0.5440 - 0.39978 * nf + 0.025054 * nf * nf - 0.0006714 * nf.powi(3);
        let sigma = (1.3822 - 0.77857 * nf + 0.062767 * nf * nf - 0.0020322 * nf.powi(3)).exp();
        let inner = g - (1.0 - w).ln();
        if inner <= 0.0 {
            0.0
        } else {
            let z = (-inner.ln() - mu) / sigma;
            1.0 - norm_cdf(z)
        }
    } else {
        let l = nf.ln();
        let mu = 0.0038915 * l.powi(3) - 0.083751 * l * l - 0.31082 * l - 1.5861;
        let sigma = (0.0030302 * l * l - 0.082676 * l - 0.4803).exp();
        let z = ((1.0 - w).ln() - mu) / sigma;
        1.0 - norm_cdf(z)
    };
    res(w, p.clamp(0.0, 1.0))
}

pub fn jarque_bera(x: &[f64]) -> TestResult {
    let d = super::describe(x);
    let n = d.n as f64;
    let jb = n / 6.0 * (d.skewness.powi(2) + d.kurtosis.powi(2) / 4.0);
    TestResult { name: "Jarque–Bera normality test".into(), statistic_name: "JB".into(), statistic: jb, df: Some(2.0), p_value: chi2_sf(jb, 2.0), extra: vec![("Skewness".into(), d.skewness), ("Excess kurtosis".into(), d.kurtosis), ("n".into(), n)] }
}

// =============================================================================================
// Rank tests
// =============================================================================================

/// Mann–Whitney U (normal approximation with tie and continuity corrections).
pub fn mann_whitney(a: &[f64], b: &[f64]) -> TestResult {
    let (a, b) = (fin(a), fin(b));
    let (n1, n2) = (a.len() as f64, b.len() as f64);
    let all: Vec<f64> = a.iter().chain(b.iter()).copied().collect();
    let r = ranks(&all);
    let r1: f64 = r.iter().take(a.len()).sum();
    let u1 = r1 - n1 * (n1 + 1.0) / 2.0;
    let u2 = n1 * n2 - u1;
    let n = n1 + n2;
    let mut tie = 0.0;
    let mut sv = all.clone();
    sv.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    let mut i = 0;
    while i < sv.len() {
        let mut j = i;
        while j + 1 < sv.len() && sv[j + 1] == sv[i] {
            j += 1;
        }
        let t = (j - i + 1) as f64;
        tie += t * t * t - t;
        i = j + 1;
    }
    let mu = n1 * n2 / 2.0;
    let sigma = (n1 * n2 / 12.0 * ((n + 1.0) - tie / (n * (n - 1.0)))).sqrt();
    let u = u1.min(u2);
    let z = ((u - mu).abs() - 0.5).max(0.0) / sigma;
    TestResult {
        name: "Mann–Whitney U test".into(),
        statistic_name: "U".into(),
        statistic: u1,
        df: None,
        p_value: (2.0 * (1.0 - norm_cdf(z))).clamp(0.0, 1.0),
        extra: vec![("z".into(), z), ("Effect size r".into(), z / n.sqrt()), ("Median (group 1)".into(), super::quantile(&super::sorted(&a), 0.5)), ("Median (group 2)".into(), super::quantile(&super::sorted(&b), 0.5))],
    }
}

/// Kruskal–Wallis H (tie corrected, chi-square approximation).
pub fn kruskal_wallis(groups: &[Vec<f64>]) -> TestResult {
    let groups: Vec<Vec<f64>> = groups.iter().map(|g| fin(g)).filter(|g| !g.is_empty()).collect();
    let all: Vec<f64> = groups.iter().flatten().copied().collect();
    let n = all.len() as f64;
    let r = ranks(&all);
    let mut h = 0.0;
    let mut off = 0;
    for g in &groups {
        let rs: f64 = r.iter().skip(off).take(g.len()).sum();
        h += rs * rs / g.len() as f64;
        off += g.len();
    }
    h = 12.0 / (n * (n + 1.0)) * h - 3.0 * (n + 1.0);
    let mut sv = all;
    sv.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    let mut tie = 0.0;
    let mut i = 0;
    while i < sv.len() {
        let mut j = i;
        while j + 1 < sv.len() && sv[j + 1] == sv[i] {
            j += 1;
        }
        let t = (j - i + 1) as f64;
        tie += t * t * t - t;
        i = j + 1;
    }
    let c = 1.0 - tie / (n * n * n - n);
    if c > 0.0 {
        h /= c;
    }
    let df = groups.len() as f64 - 1.0;
    TestResult { name: "Kruskal–Wallis H test".into(), statistic_name: "H".into(), statistic: h, df: Some(df), p_value: chi2_sf(h, df), extra: vec![("Groups".into(), groups.len() as f64), ("n".into(), n)] }
}

/// Two-sample Kolmogorov–Smirnov statistic and p-value.
pub fn ks_two_sample(a: &[f64], b: &[f64]) -> TestResult {
    let (sa, sb) = (super::sorted(a), super::sorted(b));
    let (n1, n2) = (sa.len(), sb.len());
    let (mut i, mut j, mut d) = (0, 0, 0.0f64);
    while i < n1 && j < n2 {
        let x = sa[i].min(sb[j]);
        while i < n1 && sa[i] <= x {
            i += 1;
        }
        while j < n2 && sb[j] <= x {
            j += 1;
        }
        d = d.max((i as f64 / n1 as f64 - j as f64 / n2 as f64).abs());
    }
    let ne = (n1 * n2) as f64 / (n1 + n2).max(1) as f64;
    TestResult { name: "Two-sample Kolmogorov–Smirnov test".into(), statistic_name: "D".into(), statistic: d, df: None, p_value: if n1 == 0 || n2 == 0 { f64::NAN } else { ks_pvalue(d, ne) }, extra: vec![] }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn t_tests() {
        // Reference values computed with SciPy 1.11.
        let a = [5.1, 4.9, 5.6, 5.8, 6.0, 5.5, 5.3];
        let r = one_sample_t(&a, 5.0);
        assert!(close(r.statistic, 3.127_849_019_765_153, 1e-9), "{}", r.statistic);
        assert!(close(r.p_value, 0.020_380_298_573_351_83, 1e-9), "{}", r.p_value);
        let b = [4.2, 4.8, 5.0, 4.6, 4.4, 4.9];
        let s = two_sample_t(&a, &b, true);
        let w = two_sample_t(&a, &b, false);
        assert!(close(s.statistic, 4.107_712_644_212_204, 1e-9), "{}", s.statistic);
        assert!(close(s.p_value, 0.001_736_511_820_535_303, 1e-9), "{}", s.p_value);
        assert!(close(w.statistic, 4.185_185_185_185_184, 1e-9));
        assert!(close(w.df.unwrap(), 10.963_200_875_044_675, 1e-9));
        assert!(close(w.p_value, 0.001_534_187_189_667_563, 1e-8));
        let p = paired_t(&[1.0, 2.0, 3.0, 4.0], &[1.5, 2.1, 3.9, 4.4]);
        assert!(p.statistic < 0.0);
    }

    #[test]
    fn anova_and_tukey() {
        let g = vec![vec![4.0, 5.0, 6.0, 5.0], vec![7.0, 8.0, 9.0, 8.0], vec![5.0, 6.0, 5.5, 6.5]];
        let a = one_way_anova(&g);
        // SSB = 4*(5-6.25)^2 + 4*(8-6.25)^2 + 4*(5.75-6.25)^2 = 6.25+12.25+1 = 19.5; SSW = 2+2+1.25=5.25
        assert!(close(a.rows[0].ss, 19.5, 1e-9));
        assert!(close(a.rows[1].ss, 5.25, 1e-9));
        assert!(close(a.rows[0].f, (19.5 / 2.0) / (5.25 / 9.0), 1e-9));
        assert!(a.rows[0].p < 0.001);
        let names = vec!["a".to_string(), "b".into(), "c".into()];
        let t = tukey_hsd(&names, &g);
        assert_eq!(t.len(), 3);
        assert!(t[0].p_adj < 0.01); // a vs b differ
        // scipy.stats.tukey_hsd: a–b 0.000924, a–c 0.3863, b–c 0.00616
        assert!(close(t[0].p_adj, 0.000_923_7, 2e-4), "{}", t[0].p_adj);
        assert!(close(t[1].p_adj, 0.386_307, 2e-3), "{}", t[1].p_adj);
        assert!(close(t[2].p_adj, 0.006_160, 5e-4), "{}", t[2].p_adj);
    }

    #[test]
    fn two_way() {
        let y = [10.0, 12.0, 11.0, 13.0, 20.0, 21.0, 19.0, 22.0];
        let a: Vec<String> = ["x", "x", "y", "y", "x", "x", "y", "y"].iter().map(|s| s.to_string()).collect();
        let b: Vec<String> = ["p", "p", "p", "p", "q", "q", "q", "q"].iter().map(|s| s.to_string()).collect();
        let r = two_way_anova(&y, &a, &b, "A", "B");
        assert_eq!(r.rows.len(), 4);
        assert!(r.rows[1].p < 0.01); // B dominates
        assert!(close(r.rows[1].f, 72.0, 1e-9), "{}", r.rows[1].f);
        let total: f64 = r.rows.iter().map(|x| x.ss).sum();
        let m = mean(&y);
        assert!(close(total, y.iter().map(|v| (v - m).powi(2)).sum(), 1e-6));
    }

    #[test]
    fn chi_square() {
        let c = chi2_independence(vec!["a".into(), "b".into()], vec!["x".into(), "y".into()], vec![vec![10.0, 20.0], vec![30.0, 40.0]]);
        // scipy chi2_contingency(correction=False): chi2 = 0.7937, p = 0.3730
        assert!(close(c.chi2, 0.79365, 1e-4), "{}", c.chi2);
        assert!(close(c.p, 0.37298, 1e-4), "{}", c.p);
    }

    #[test]
    fn correlations() {
        let x = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let y = [2.0, 1.0, 4.0, 3.0, 6.0, 5.0];
        let p = pearson(&x, &y);
        assert!(close(p.statistic, 0.828_571_4, 1e-6));
        // p two-sided = 0.04156
        assert!(close(p.p_value, 0.04156, 2e-4), "{}", p.p_value);
        let s = spearman(&x, &y);
        assert!(close(s.statistic, 0.828_571_4, 1e-6));
        let k = kendall(&x, &y);
        assert!(close(k.statistic, 0.6, 1e-9), "{}", k.statistic);
    }

    #[test]
    fn normality() {
        // Normal quantiles → W close to 1; exponential-like → small p.
        let normal: Vec<f64> = (1..=50).map(|i| norm_ppf(i as f64 / 51.0)).collect();
        let r = shapiro_wilk(&normal);
        assert!(r.statistic > 0.98 && r.p_value > 0.5, "{} {}", r.statistic, r.p_value);
        let skewed: Vec<f64> = (1..=50).map(|i| -(1.0 - i as f64 / 51.0f64).ln()).collect();
        let r = shapiro_wilk(&skewed);
        assert!(r.p_value < 0.01, "{}", r.p_value);
        assert!(close(r.statistic, 0.863_319_921, 2e-3), "{}", r.statistic);
        let r = shapiro_wilk(&[2.1, 3.4, 1.9, 5.6, 4.4, 3.3, 2.8, 3.9]);
        // scipy: W = 0.963348, p = 0.84132
        assert!(close(r.statistic, 0.963_348, 1e-3), "{}", r.statistic);
        assert!(close(r.p_value, 0.8413, 0.02), "{}", r.p_value);
        let jb = jarque_bera(&skewed);
        assert!(jb.p_value < 0.05);
    }

    #[test]
    fn rank_tests() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0];
        let b = [6.0, 7.0, 8.0, 9.0, 10.0];
        let m = mann_whitney(&a, &b);
        assert!(close(m.statistic, 0.0, 1e-12));
        // normal approx with continuity: z = (12.5-0.5)/sqrt(25*11/12) = 2.5067 → p = 0.01219
        assert!(close(m.p_value, 0.01219, 2e-4), "{}", m.p_value);
        let k = kruskal_wallis(&[a.to_vec(), b.to_vec()]);
        // H = 6.818
        assert!(close(k.statistic, 6.8182, 1e-3), "{}", k.statistic);
        let ks = ks_two_sample(&a, &b);
        assert!(close(ks.statistic, 1.0, 1e-12));
    }
}
