//! Statistical modelling (statsmodels / SciPy style), implemented from scratch:
//!
//! - [`special`]: gamma / beta functions and the normal, t, F, chi-square and studentized range
//!   distributions.
//! - [`hypothesis`]: t-tests, ANOVA (one- and two-way) with Tukey HSD, chi-square independence,
//!   correlation tests (Pearson / Spearman / Kendall), normality (Shapiro–Wilk, Jarque–Bera),
//!   Mann–Whitney U and Kruskal–Wallis.
//! - [`regression`]: OLS with full inference, logistic regression (IRLS) with odds ratios,
//!   Poisson GLM, VIF and residual diagnostics.
//! - [`ts`]: decomposition, ACF / PACF, ARIMA(p,d,q) by conditional sum of squares, exponential
//!   smoothing (simple, Holt, Holt–Winters).
//! - [`dist`]: distribution fitting by maximum likelihood with goodness-of-fit.
//!
//! Results are plain structs; [`Report`] is a renderer-independent "summary" (sections of
//! key/value pairs and tables) the Studio shows.

pub mod dist;
pub mod hypothesis;
pub mod optim;
pub mod regression;
pub mod special;
pub mod ts;

use crate::build::TableData;
use crate::scalar::Scalar;

// =============================================================================================
// Reports
// =============================================================================================

/// A readable statistical report: a title and sections.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub title: String,
    pub sections: Vec<Section>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    pub heading: String,
    /// Key / value pairs shown in two columns.
    pub kv: Vec<(String, String)>,
    pub table: Option<RTable>,
    /// A conclusion sentence ("Reject H0 at 5%…").
    pub note: String,
}

/// A result table; `p_col` marks the column holding p-values (highlighted when < 0.05).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RTable {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub p_col: Option<usize>,
}

impl Report {
    pub fn new(title: impl Into<String>) -> Report {
        Report { title: title.into(), sections: vec![] }
    }
    pub fn section(&mut self, heading: impl Into<String>) -> &mut Section {
        self.sections.push(Section { heading: heading.into(), ..Default::default() });
        let n = self.sections.len() - 1;
        &mut self.sections[n]
    }
    /// Plain-text rendering (for export / copying).
    pub fn to_text(&self) -> String {
        let mut s = format!("{}\n{}\n", self.title, "=".repeat(self.title.chars().count().max(8)));
        for sec in &self.sections {
            if !sec.heading.is_empty() {
                s.push_str(&format!("\n{}\n{}\n", sec.heading, "-".repeat(sec.heading.chars().count())));
            }
            for (k, v) in &sec.kv {
                s.push_str(&format!("{k:<28}{v}\n"));
            }
            if let Some(t) = &sec.table {
                let mut w: Vec<usize> = t.columns.iter().map(|c| c.chars().count()).collect();
                for r in &t.rows {
                    for (i, c) in r.iter().enumerate() {
                        if let Some(x) = w.get_mut(i) {
                            *x = (*x).max(c.chars().count());
                        }
                    }
                }
                let line = |cells: &[String]| cells.iter().enumerate().map(|(i, c)| format!("{:>width$}", c, width = w.get(i).copied().unwrap_or(8))).collect::<Vec<_>>().join("  ");
                s.push_str(&line(&t.columns));
                s.push('\n');
                for r in &t.rows {
                    s.push_str(&line(r));
                    s.push('\n');
                }
            }
            if !sec.note.is_empty() {
                s.push_str(&format!("{}\n", sec.note));
            }
        }
        s
    }
}

impl Section {
    pub fn kv(&mut self, k: impl Into<String>, v: impl Into<String>) -> &mut Section {
        self.kv.push((k.into(), v.into()));
        self
    }
}

/// Formats a number compactly for reports.
pub fn fmt(x: f64) -> String {
    if x.is_nan() {
        return "—".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "∞".into() } else { "−∞".into() };
    }
    let a = x.abs();
    if a != 0.0 && (a >= 1e7 || a < 1e-4) { format!("{x:.3e}") } else if a >= 1000.0 { format!("{x:.1}") } else { format!("{x:.4}") }
}

/// Formats a p-value ("< 0.001" style below 1e-3 shown with exponent).
pub fn fmt_p(p: f64) -> String {
    if p.is_nan() {
        "—".into()
    } else if p < 1e-4 {
        format!("{p:.2e}")
    } else {
        format!("{p:.4}")
    }
}

/// Significance stars (*** < 0.001, ** < 0.01, * < 0.05, . < 0.1).
pub fn stars(p: f64) -> &'static str {
    if p < 0.001 {
        "***"
    } else if p < 0.01 {
        "**"
    } else if p < 0.05 {
        "*"
    } else if p < 0.1 {
        "."
    } else {
        ""
    }
}

// =============================================================================================
// Descriptive statistics
// =============================================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Describe {
    pub n: usize,
    pub missing: usize,
    pub mean: f64,
    pub std: f64,
    pub var: f64,
    pub se: f64,
    pub min: f64,
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    pub max: f64,
    pub sum: f64,
    pub skewness: f64,
    /// Excess kurtosis.
    pub kurtosis: f64,
    pub ci_low: f64,
    pub ci_high: f64,
}

/// Linear-interpolated quantile of sorted data (type 7, as R / NumPy default).
pub fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let h = (sorted.len() - 1) as f64 * q.clamp(0.0, 1.0);
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    let a = sorted.get(lo).copied().unwrap_or(f64::NAN);
    let b = sorted.get(hi).copied().unwrap_or(a);
    a + (b - a) * (h - lo as f64)
}

pub fn mean(x: &[f64]) -> f64 {
    if x.is_empty() { f64::NAN } else { x.iter().sum::<f64>() / x.len() as f64 }
}

/// Sample variance (n − 1).
pub fn variance(x: &[f64]) -> f64 {
    if x.len() < 2 {
        return f64::NAN;
    }
    let m = mean(x);
    x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (x.len() - 1) as f64
}

pub fn finite(x: &[f64]) -> Vec<f64> {
    x.iter().copied().filter(|v| v.is_finite()).collect()
}

pub fn sorted(x: &[f64]) -> Vec<f64> {
    let mut s = finite(x);
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    s
}

pub fn describe(raw: &[f64]) -> Describe {
    let x = finite(raw);
    let n = x.len();
    let missing = raw.len() - n;
    if n == 0 {
        return Describe { missing, mean: f64::NAN, std: f64::NAN, var: f64::NAN, se: f64::NAN, min: f64::NAN, q1: f64::NAN, median: f64::NAN, q3: f64::NAN, max: f64::NAN, skewness: f64::NAN, kurtosis: f64::NAN, ci_low: f64::NAN, ci_high: f64::NAN, ..Default::default() };
    }
    let s = sorted(&x);
    let m = mean(&x);
    let var = variance(&x);
    let sd = var.sqrt();
    let nf = n as f64;
    let m2 = x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / nf;
    let m3 = x.iter().map(|v| (v - m).powi(3)).sum::<f64>() / nf;
    let m4 = x.iter().map(|v| (v - m).powi(4)).sum::<f64>() / nf;
    let skew = if m2 > 0.0 { m3 / m2.powf(1.5) } else { 0.0 };
    let kurt = if m2 > 0.0 { m4 / (m2 * m2) - 3.0 } else { 0.0 };
    let se = sd / nf.sqrt();
    let tc = if n > 1 { special::t_ppf(0.975, nf - 1.0) } else { f64::NAN };
    Describe {
        n,
        missing,
        mean: m,
        std: sd,
        var,
        se,
        min: s.first().copied().unwrap_or(f64::NAN),
        q1: quantile(&s, 0.25),
        median: quantile(&s, 0.5),
        q3: quantile(&s, 0.75),
        max: s.last().copied().unwrap_or(f64::NAN),
        sum: x.iter().sum(),
        skewness: skew,
        kurtosis: kurt,
        ci_low: m - tc * se,
        ci_high: m + tc * se,
    }
}

/// Ranks with ties averaged (1-based).
pub fn ranks(x: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..x.len()).collect();
    idx.sort_by(|a, b| x[*a].partial_cmp(&x[*b]).unwrap_or(std::cmp::Ordering::Equal));
    let mut r = vec![0.0; x.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && x[idx[j + 1]] == x[idx[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        for k in i..=j {
            r[idx[k]] = avg;
        }
        i = j + 1;
    }
    r
}

// =============================================================================================
// Table helpers
// =============================================================================================

/// A numeric column (non-numbers → NaN).
pub fn column(t: &TableData, name: &str) -> Result<Vec<f64>, String> {
    let i = t.column_index(name).ok_or_else(|| format!("There's no column `{name}`."))?;
    Ok(t.rows.iter().map(|r| r.get(i).and_then(Scalar::as_f64).unwrap_or(f64::NAN)).collect())
}

/// A column as text labels (nulls → None).
pub fn labels(t: &TableData, name: &str) -> Result<Vec<Option<String>>, String> {
    let i = t.column_index(name).ok_or_else(|| format!("There's no column `{name}`."))?;
    Ok(t.rows.iter().map(|r| r.get(i).filter(|v| !v.is_null()).map(Scalar::text)).collect())
}

/// Groups a numeric column by a label column: (group names sorted, values per group).
pub fn groups(t: &TableData, value: &str, by: &str) -> Result<(Vec<String>, Vec<Vec<f64>>), String> {
    let v = column(t, value)?;
    let g = labels(t, by)?;
    let mut map: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
    for (x, l) in v.iter().zip(g) {
        if let Some(l) = l
            && x.is_finite()
        {
            map.entry(l).or_default().push(*x);
        }
    }
    Ok(map.into_iter().unzip())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn describe_basic() {
        let d = describe(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0, f64::NAN]);
        assert_eq!(d.n, 8);
        assert_eq!(d.missing, 1);
        assert!((d.mean - 5.0).abs() < 1e-12);
        assert!((d.var - 32.0 / 7.0).abs() < 1e-12);
        assert!((d.median - 4.5).abs() < 1e-12);
        assert!((d.q1 - 4.0).abs() < 1e-12);
    }

    #[test]
    fn rank_ties() {
        assert_eq!(ranks(&[10.0, 20.0, 10.0, 30.0]), vec![1.5, 3.0, 1.5, 4.0]);
    }
}
