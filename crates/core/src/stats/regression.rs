//! Regression with inference: OLS (coefficients, standard errors, t, p, confidence intervals,
//! R², adjusted R², F, AIC/BIC, Durbin–Watson, Jarque–Bera on residuals, VIF, condition number),
//! logistic regression (IRLS / Newton–Raphson) with odds ratios and pseudo R², and Poisson GLM
//! (log link, IRLS) with deviance.

use std::collections::BTreeMap;

use super::optim::{dot, inverse, mat_vec, normal_equations, Matrix};
use super::special::*;
use super::{fmt, fmt_p, stars, RTable, Report};
use crate::build::TableData;
use crate::scalar::Scalar;

/// A design matrix built from a table: numeric features as-is, categorical features dummy-coded
/// against their first level (`Channel[Online]`), rows with missing values dropped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Design {
    pub names: Vec<String>,
    pub x: Matrix,
    pub y: Vec<f64>,
    /// Original row index of every kept row.
    pub rows: Vec<usize>,
    /// Target classes when the target is categorical (binary: [reference, event]).
    pub classes: Vec<String>,
}

fn is_numeric_col(t: &TableData, c: usize) -> bool {
    let ty = t.types.get(c).copied().unwrap_or_default();
    if ty.is_numeric() {
        return true;
    }
    let mut num = 0;
    let mut other = 0;
    for r in t.rows.iter().take(500) {
        match r.get(c) {
            Some(Scalar::Num(_)) => num += 1,
            Some(Scalar::Null) | None => {}
            _ => other += 1,
        }
    }
    num > 0 && other == 0 && !matches!(ty, crate::build::ColType::Text | crate::build::ColType::Bool)
}

/// Builds the design. `binary_target`: map the target to 0/1 (the event is the last class, or
/// `event` when given).
pub fn design(t: &TableData, target: &str, features: &[String], intercept: bool, binary_target: bool, event: Option<&str>) -> Result<Design, String> {
    let ti = t.column_index(target).ok_or_else(|| format!("There's no target column `{target}`."))?;
    if features.is_empty() {
        return Err("Choose at least one predictor.".into());
    }
    let mut cols: Vec<(usize, Option<Vec<String>>)> = vec![];
    for f in features {
        let i = t.column_index(f).ok_or_else(|| format!("There's no column `{f}`."))?;
        if i == ti {
            continue;
        }
        if is_numeric_col(t, i) {
            cols.push((i, None));
        } else {
            let mut lv: Vec<String> = t.rows.iter().filter_map(|r| r.get(i)).filter(|v| !v.is_null()).map(Scalar::text).collect();
            lv.sort();
            lv.dedup();
            if lv.len() > 60 {
                return Err(format!("`{f}` has {} categories: too many to dummy-code.", lv.len()));
            }
            cols.push((i, Some(lv)));
        }
    }
    let mut names = vec![];
    if intercept {
        names.push("const".to_string());
    }
    for (i, lv) in &cols {
        let n = t.columns.get(*i).cloned().unwrap_or_default();
        match lv {
            None => names.push(n),
            Some(lv) => {
                for l in lv.iter().skip(1) {
                    names.push(format!("{n}[{l}]"));
                }
            }
        }
    }
    let mut classes = vec![];
    if binary_target {
        let mut lv: Vec<String> = t.rows.iter().filter_map(|r| r.get(ti)).filter(|v| !v.is_null()).map(Scalar::text).collect();
        lv.sort_by(|a, b| match (a.parse::<f64>(), b.parse::<f64>()) {
            (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
            _ => a.cmp(b),
        });
        lv.dedup();
        if lv.len() != 2 && event.is_none() {
            return Err(format!("Logistic regression needs a binary target; `{target}` has {} values. Pick the event class.", lv.len()));
        }
        let ev = event.map(str::to_string).or_else(|| lv.last().cloned()).unwrap_or_default();
        let reference = if lv.len() == 2 { lv.iter().find(|l| **l != ev).cloned().unwrap_or_default() } else { format!("not {ev}") };
        classes = vec![reference, ev];
    }
    let mut x = vec![];
    let mut y = vec![];
    let mut rows = vec![];
    'rows: for (ri, r) in t.rows.iter().enumerate() {
        let yv = match r.get(ti) {
            None | Some(Scalar::Null) => continue,
            Some(v) => {
                if binary_target {
                    if v.text() == classes[1] { 1.0 } else { 0.0 }
                } else {
                    match v.as_f64() {
                        Some(f) if f.is_finite() => f,
                        _ => continue,
                    }
                }
            }
        };
        let mut row = Vec::with_capacity(names.len());
        if intercept {
            row.push(1.0);
        }
        for (i, lv) in &cols {
            let v = r.get(*i).unwrap_or(&Scalar::Null);
            match lv {
                None => match v.as_f64() {
                    Some(f) if f.is_finite() => row.push(f),
                    _ => continue 'rows,
                },
                Some(lv) => {
                    if v.is_null() {
                        continue 'rows;
                    }
                    let s = v.text();
                    for l in lv.iter().skip(1) {
                        row.push(if *l == s { 1.0 } else { 0.0 });
                    }
                }
            }
        }
        x.push(row);
        y.push(yv);
        rows.push(ri);
    }
    if x.len() <= names.len() {
        return Err(format!("Not enough complete rows ({}) for {} coefficients.", x.len(), names.len()));
    }
    Ok(Design { names, x, y, rows, classes })
}

// =============================================================================================
// OLS
// =============================================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OlsFit {
    pub names: Vec<String>,
    pub coef: Vec<f64>,
    pub se: Vec<f64>,
    pub t: Vec<f64>,
    pub p: Vec<f64>,
    pub ci_low: Vec<f64>,
    pub ci_high: Vec<f64>,
    pub n: usize,
    pub df_model: f64,
    pub df_resid: f64,
    pub r2: f64,
    pub adj_r2: f64,
    pub f: f64,
    pub f_p: f64,
    pub loglik: f64,
    pub aic: f64,
    pub bic: f64,
    pub sigma: f64,
    pub durbin_watson: f64,
    pub jb: f64,
    pub jb_p: f64,
    pub skew: f64,
    pub kurtosis: f64,
    pub cond_no: f64,
    pub vif: Vec<f64>,
    pub fitted: Vec<f64>,
    pub residuals: Vec<f64>,
}

/// OLS on a design matrix (include a column of ones for the intercept, named "const").
pub fn ols(names: &[String], x: &Matrix, y: &[f64]) -> Result<OlsFit, String> {
    let n = x.len();
    let k = x.first().map(|r| r.len()).unwrap_or(0);
    if n <= k || k == 0 {
        return Err(format!("OLS needs more rows ({n}) than coefficients ({k})."));
    }
    let (xtx, xty) = normal_equations(x, y, None);
    let inv = inverse(&xtx).ok_or("The predictors are perfectly collinear: remove one of them.")?;
    let coef = mat_vec(&inv, &xty);
    let fitted: Vec<f64> = x.iter().map(|r| dot(r, &coef)).collect();
    let residuals: Vec<f64> = y.iter().zip(&fitted).map(|(a, b)| a - b).collect();
    let sse: f64 = residuals.iter().map(|e| e * e).sum();
    let has_const = names.first().is_some_and(|n| n == "const");
    let ym = super::mean(y);
    let sst: f64 = if has_const { y.iter().map(|v| (v - ym).powi(2)).sum() } else { y.iter().map(|v| v * v).sum() };
    let df_resid = (n - k) as f64;
    let df_model = (k - usize::from(has_const)) as f64;
    let sigma2 = sse / df_resid;
    let se: Vec<f64> = (0..k).map(|i| (sigma2 * inv[i][i]).max(0.0).sqrt()).collect();
    let t: Vec<f64> = coef.iter().zip(&se).map(|(b, s)| b / s).collect();
    let p: Vec<f64> = t.iter().map(|t| t_two_sided(*t, df_resid)).collect();
    let tc = t_ppf(0.975, df_resid);
    let r2 = 1.0 - sse / sst;
    let adj_r2 = 1.0 - (1.0 - r2) * (n as f64 - f64::from(u8::from(has_const))) / df_resid;
    let f = ((sst - sse) / df_model.max(1.0)) / sigma2;
    let nf = n as f64;
    let loglik = -nf / 2.0 * ((2.0 * std::f64::consts::PI).ln() + (sse / nf).ln() + 1.0);
    let dw = residuals.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f64>() / sse.max(1e-300);
    let jb = super::hypothesis::jarque_bera(&residuals);
    // Condition number of the (unscaled) design: sqrt(λmax/λmin) of XᵀX.
    let (vals, _) = crate::ml::jacobi_eigen(&xtx);
    let lmax = vals.iter().copied().fold(f64::MIN, f64::max);
    let lmin = vals.iter().copied().fold(f64::MAX, f64::min);
    let cond_no = if lmin > 0.0 { (lmax / lmin).sqrt() } else { f64::INFINITY };
    let vif = vif(names, x);
    Ok(OlsFit {
        names: names.to_vec(),
        ci_low: coef.iter().zip(&se).map(|(b, s)| b - tc * s).collect(),
        ci_high: coef.iter().zip(&se).map(|(b, s)| b + tc * s).collect(),
        coef,
        se,
        t,
        p,
        n,
        df_model,
        df_resid,
        r2,
        adj_r2,
        f,
        f_p: f_sf(f, df_model.max(1.0), df_resid),
        loglik,
        aic: -2.0 * loglik + 2.0 * k as f64,
        bic: -2.0 * loglik + (k as f64) * nf.ln(),
        sigma: sigma2.sqrt(),
        durbin_watson: dw,
        jb: jb.statistic,
        jb_p: jb.p_value,
        skew: jb.get("Skewness"),
        kurtosis: jb.get("Excess kurtosis") + 3.0,
        cond_no,
        vif,
        fitted,
        residuals,
    })
}

/// Variance inflation factors (NaN for the intercept).
pub fn vif(names: &[String], x: &Matrix) -> Vec<f64> {
    let k = names.len();
    let has_const = names.first().is_some_and(|n| n == "const");
    (0..k)
        .map(|j| {
            if has_const && j == 0 {
                return f64::NAN;
            }
            let yj: Vec<f64> = x.iter().map(|r| r.get(j).copied().unwrap_or(0.0)).collect();
            let xo: Matrix = x.iter().map(|r| r.iter().enumerate().filter(|(i, _)| *i != j).map(|(_, v)| *v).collect()).collect();
            if xo.first().is_none_or(|r| r.is_empty()) {
                return 1.0;
            }
            let (a, b) = normal_equations(&xo, &yj, None);
            let Some(inv) = inverse(&a) else { return f64::INFINITY };
            let c = mat_vec(&inv, &b);
            let m = super::mean(&yj);
            let sse: f64 = xo.iter().zip(&yj).map(|(r, v)| (v - dot(r, &c)).powi(2)).sum();
            let sst: f64 = yj.iter().map(|v| (v - m).powi(2)).sum();
            if sst <= 0.0 {
                return f64::INFINITY;
            }
            let r2 = 1.0 - sse / sst;
            if r2 >= 1.0 { f64::INFINITY } else { 1.0 / (1.0 - r2) }
        })
        .collect()
}

pub fn ols_report(fit: &OlsFit, target: &str) -> Report {
    let mut r = Report::new("OLS Regression Results");
    let s = r.section("Model");
    s.kv("Dependent variable", target)
        .kv("Method", "Least squares")
        .kv("Observations", fit.n.to_string())
        .kv("Df residuals", fmt(fit.df_resid))
        .kv("Df model", fmt(fit.df_model))
        .kv("R-squared", format!("{:.4}", fit.r2))
        .kv("Adj. R-squared", format!("{:.4}", fit.adj_r2))
        .kv("F-statistic", fmt(fit.f))
        .kv("Prob (F-statistic)", fmt_p(fit.f_p))
        .kv("Log-likelihood", fmt(fit.loglik))
        .kv("AIC", fmt(fit.aic))
        .kv("BIC", fmt(fit.bic))
        .kv("Residual std. error", fmt(fit.sigma));
    let s = r.section("Coefficients");
    s.table = Some(RTable {
        columns: ["", "coef", "std err", "t", "P>|t|", "[0.025", "0.975]", "VIF", ""].iter().map(|s| s.to_string()).collect(),
        rows: (0..fit.names.len())
            .map(|i| {
                vec![
                    fit.names[i].clone(),
                    fmt(fit.coef[i]),
                    fmt(fit.se[i]),
                    format!("{:.3}", fit.t[i]),
                    fmt_p(fit.p[i]),
                    fmt(fit.ci_low[i]),
                    fmt(fit.ci_high[i]),
                    fit.vif.get(i).map(|v| if v.is_nan() { String::new() } else { format!("{v:.2}") }).unwrap_or_default(),
                    stars(fit.p[i]).to_string(),
                ]
            })
            .collect(),
        p_col: Some(4),
    });
    s.note = "Signif. codes: *** 0.001  ** 0.01  * 0.05  . 0.1".into();
    let s = r.section("Residual diagnostics");
    s.kv("Durbin–Watson", format!("{:.3}", fit.durbin_watson))
        .kv("Jarque–Bera (JB)", fmt(fit.jb))
        .kv("Prob (JB)", fmt_p(fit.jb_p))
        .kv("Skew", fmt(fit.skew))
        .kv("Kurtosis", fmt(fit.kurtosis))
        .kv("Cond. No.", fmt(fit.cond_no));
    let mut notes = vec![];
    if fit.vif.iter().any(|v| *v > 10.0) {
        notes.push("Some VIF > 10: strong multicollinearity.".to_string());
    }
    if fit.jb_p < 0.05 {
        notes.push("Residuals are not normally distributed (JB p < 0.05).".into());
    }
    if !(1.5..=2.5).contains(&fit.durbin_watson) {
        notes.push("Durbin–Watson far from 2: residuals may be autocorrelated.".into());
    }
    s.note = notes.join(" ");
    r
}

// =============================================================================================
// Logistic regression (binomial GLM, logit link)
// =============================================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlmFit {
    pub family: String,
    pub names: Vec<String>,
    pub coef: Vec<f64>,
    pub se: Vec<f64>,
    pub z: Vec<f64>,
    pub p: Vec<f64>,
    pub ci_low: Vec<f64>,
    pub ci_high: Vec<f64>,
    pub n: usize,
    pub loglik: f64,
    pub loglik_null: f64,
    pub pseudo_r2: f64,
    pub llr_p: f64,
    pub deviance: f64,
    pub pearson_chi2: f64,
    pub aic: f64,
    pub bic: f64,
    pub iterations: usize,
    pub converged: bool,
    pub fitted: Vec<f64>,
    pub classes: Vec<String>,
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z.clamp(-500.0, 500.0)).exp())
}

/// IRLS for a GLM with canonical link: `mu(eta)`, `var(mu)`, log-likelihood term per row.
fn irls(x: &Matrix, y: &[f64], mu_of: &dyn Fn(f64) -> f64, var_of: &dyn Fn(f64) -> f64, ll: &dyn Fn(f64, f64) -> f64) -> Result<(Vec<f64>, Matrix, usize, bool), String> {
    let k = x.first().map(|r| r.len()).unwrap_or(0);
    let mut b = vec![0.0; k];
    // Start from the mean on the link scale for the intercept.
    let mut prev_ll = f64::NEG_INFINITY;
    let mut converged = false;
    let mut it = 0;
    let mut cov = vec![vec![0.0; k]; k];
    for i in 0..100 {
        it = i + 1;
        let eta: Vec<f64> = x.iter().map(|r| dot(r, &b)).collect();
        let mu: Vec<f64> = eta.iter().map(|e| mu_of(*e)).collect();
        let w: Vec<f64> = mu.iter().map(|m| var_of(*m).max(1e-10)).collect();
        // Newton step: (XᵀWX)⁻¹ Xᵀ(y − μ)
        let (h, _) = normal_equations(x, y, Some(&w));
        let g: Vec<f64> = (0..k).map(|j| x.iter().zip(y).zip(&mu).map(|((r, yi), m)| r.get(j).copied().unwrap_or(0.0) * (yi - m)).sum()).collect();
        let Some(inv) = inverse(&h) else { return Err("The model can't be estimated (singular information matrix).".into()) };
        let step = mat_vec(&inv, &g);
        // Step halving if the likelihood drops.
        let cur_ll: f64 = y.iter().zip(&mu).map(|(yi, m)| ll(*yi, *m)).sum();
        let mut scale = 1.0;
        let mut nb: Vec<f64> = b.iter().zip(&step).map(|(a, s)| a + s).collect();
        for _ in 0..20 {
            let new_ll: f64 = x.iter().zip(y).map(|(r, yi)| ll(*yi, mu_of(dot(r, &nb)))).sum();
            if new_ll.is_finite() && new_ll >= cur_ll - 1e-9 {
                break;
            }
            scale /= 2.0;
            nb = b.iter().zip(&step).map(|(a, s)| a + scale * s).collect();
        }
        b = nb;
        cov = inv;
        if (cur_ll - prev_ll).abs() < 1e-10 * (1.0 + cur_ll.abs()) && i > 0 {
            converged = true;
            break;
        }
        prev_ll = cur_ll;
    }
    // Covariance at the final estimate.
    let mu: Vec<f64> = x.iter().map(|r| mu_of(dot(r, &b))).collect();
    let w: Vec<f64> = mu.iter().map(|m| var_of(*m).max(1e-10)).collect();
    let (h, _) = normal_equations(x, y, Some(&w));
    if let Some(inv) = inverse(&h) {
        cov = inv;
    }
    Ok((b, cov, it, converged))
}

fn glm_finish(family: &str, names: &[String], x: &Matrix, y: &[f64], b: Vec<f64>, cov: Matrix, it: usize, converged: bool, mu_of: &dyn Fn(f64) -> f64, ll: &dyn Fn(f64, f64) -> f64, null_mu: f64, dev: &dyn Fn(f64, f64) -> f64, var_of: &dyn Fn(f64) -> f64) -> GlmFit {
    let k = b.len();
    let n = x.len();
    let se: Vec<f64> = (0..k).map(|i| cov.get(i).and_then(|r| r.get(i)).copied().unwrap_or(f64::NAN).max(0.0).sqrt()).collect();
    let z: Vec<f64> = b.iter().zip(&se).map(|(a, s)| a / s).collect();
    let p: Vec<f64> = z.iter().map(|z| 2.0 * (1.0 - norm_cdf(z.abs()))).collect();
    let fitted: Vec<f64> = x.iter().map(|r| mu_of(dot(r, &b))).collect();
    let loglik: f64 = y.iter().zip(&fitted).map(|(yi, m)| ll(*yi, *m)).sum();
    let loglik_null: f64 = y.iter().map(|yi| ll(*yi, null_mu)).sum();
    let deviance: f64 = y.iter().zip(&fitted).map(|(yi, m)| dev(*yi, *m)).sum();
    let pearson: f64 = y.iter().zip(&fitted).map(|(yi, m)| (yi - m).powi(2) / var_of(*m).max(1e-12)).sum();
    let llr = 2.0 * (loglik - loglik_null);
    GlmFit {
        family: family.into(),
        names: names.to_vec(),
        ci_low: b.iter().zip(&se).map(|(a, s)| a - 1.959_963_984_540_054 * s).collect(),
        ci_high: b.iter().zip(&se).map(|(a, s)| a + 1.959_963_984_540_054 * s).collect(),
        coef: b,
        se,
        z,
        p,
        n,
        loglik,
        loglik_null,
        pseudo_r2: 1.0 - loglik / loglik_null,
        llr_p: chi2_sf(llr, (k as f64 - 1.0).max(1.0)),
        deviance,
        pearson_chi2: pearson,
        aic: -2.0 * loglik + 2.0 * k as f64,
        bic: -2.0 * loglik + k as f64 * (n as f64).ln(),
        iterations: it,
        converged,
        fitted,
        classes: vec![],
    }
}

pub fn logit(names: &[String], x: &Matrix, y: &[f64]) -> Result<GlmFit, String> {
    if x.len() <= names.len() {
        return Err("Not enough rows for logistic regression.".into());
    }
    let ll = |yi: f64, m: f64| {
        let m = m.clamp(1e-15, 1.0 - 1e-15);
        yi * m.ln() + (1.0 - yi) * (1.0 - m).ln()
    };
    let var = |m: f64| m * (1.0 - m);
    let (b, cov, it, conv) = irls(x, y, &sigmoid, &var, &ll)?;
    let ybar = super::mean(y);
    let dev = |yi: f64, m: f64| -2.0 * ll(yi, m);
    Ok(glm_finish("Binomial (logit)", names, x, y, b, cov, it, conv, &sigmoid, &ll, ybar, &dev, &var))
}

pub fn poisson(names: &[String], x: &Matrix, y: &[f64]) -> Result<GlmFit, String> {
    if y.iter().any(|v| *v < 0.0) {
        return Err("Poisson regression needs non-negative counts.".into());
    }
    let mu_of = |e: f64| e.clamp(-700.0, 700.0).exp();
    let ll = |yi: f64, m: f64| yi * m.max(1e-300).ln() - m - ln_gamma(yi + 1.0);
    let var = |m: f64| m;
    let (b, cov, it, conv) = irls(x, y, &mu_of, &var, &ll)?;
    let ybar = super::mean(y);
    let dev = |yi: f64, m: f64| 2.0 * (if yi > 0.0 { yi * (yi / m).ln() } else { 0.0 } - (yi - m));
    Ok(glm_finish("Poisson (log)", names, x, y, b, cov, it, conv, &mu_of, &ll, ybar, &dev, &var))
}

pub fn glm_report(fit: &GlmFit, target: &str) -> Report {
    let logistic = fit.family.starts_with("Binomial");
    let mut r = Report::new(if logistic { "Logistic Regression Results" } else { "Generalized Linear Model Results" });
    let s = r.section("Model");
    s.kv("Dependent variable", target).kv("Family (link)", fit.family.clone()).kv("Observations", fit.n.to_string());
    if logistic && fit.classes.len() == 2 {
        s.kv("Event (y = 1)", fit.classes[1].clone());
    }
    s.kv("Log-likelihood", fmt(fit.loglik)).kv("LL-Null", fmt(fit.loglik_null));
    if logistic {
        s.kv("Pseudo R² (McFadden)", format!("{:.4}", fit.pseudo_r2)).kv("LLR p-value", fmt_p(fit.llr_p));
    } else {
        s.kv("Deviance", fmt(fit.deviance)).kv("Pearson χ²", fmt(fit.pearson_chi2)).kv("Pseudo R² (McFadden)", format!("{:.4}", fit.pseudo_r2));
    }
    s.kv("AIC", fmt(fit.aic)).kv("BIC", fmt(fit.bic)).kv("Iterations", format!("{}{}", fit.iterations, if fit.converged { "" } else { " (not converged)" }));
    let s = r.section("Coefficients");
    let ratio = if logistic { "Odds ratio" } else { "Rate ratio" };
    s.table = Some(RTable {
        columns: ["", "coef", "std err", "z", "P>|z|", "[0.025", "0.975]", ratio, ""].iter().map(|s| s.to_string()).collect(),
        rows: (0..fit.names.len())
            .map(|i| vec![fit.names[i].clone(), fmt(fit.coef[i]), fmt(fit.se[i]), format!("{:.3}", fit.z[i]), fmt_p(fit.p[i]), fmt(fit.ci_low[i]), fmt(fit.ci_high[i]), fmt(fit.coef[i].exp()), stars(fit.p[i]).into()])
            .collect(),
        p_col: Some(4),
    });
    s.note = if logistic { "Odds ratio = exp(coef): the multiplicative change in the odds of the event per unit increase.".into() } else { "Rate ratio = exp(coef): the multiplicative change in the expected count per unit increase.".into() };
    r
}

/// Convenience: fits OLS from a table and returns the report.
pub fn ols_table(t: &TableData, target: &str, features: &[String]) -> Result<(OlsFit, Design), String> {
    let d = design(t, target, features, true, false, None)?;
    Ok((ols(&d.names, &d.x, &d.y)?, d))
}

/// Category counts (for reports).
pub fn level_counts(v: &[Option<String>]) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for x in v.iter().flatten() {
        *m.entry(x.clone()).or_default() += 1;
    }
    m
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    fn data() -> (Vec<String>, Matrix, Vec<f64>) {
        let x1 = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let x2 = [2.0, 1.5, 3.7, 2.2, 5.1, 4.4, 6.0, 5.5, 7.9, 8.1];
        let y = vec![3.1, 4.2, 7.9, 6.8, 11.2, 11.0, 14.1, 14.9, 18.5, 19.8];
        let x = x1.iter().zip(&x2).map(|(a, b)| vec![1.0, *a, *b]).collect();
        (vec!["const".into(), "x1".into(), "x2".into()], x, y)
    }

    #[test]
    fn ols_matches_statsmodels() {
        let (n, x, y) = data();
        let f = ols(&n, &x, &y).unwrap();
        // statsmodels 0.15 reference.
        for (a, b) in f.coef.iter().zip([0.153_855_19, 1.028_506_06, 1.150_724_46]) {
            assert!(close(*a, b, 1e-7), "{a} {b}");
        }
        for (a, b) in f.se.iter().zip([0.213_917_04, 0.092_281_74, 0.119_590_28]) {
            assert!(close(*a, b, 1e-7));
        }
        for (a, b) in f.p.iter().zip([4.953_039_77e-01, 1.042_374_99e-05, 2.756_127_83e-05]) {
            assert!(close(*a, b, 1e-8) || ((a - b) / b).abs() < 1e-5, "{a} {b}");
        }
        assert!(close(f.r2, 0.997_984_673_920_803_5, 1e-10));
        assert!(close(f.adj_r2, 0.997_408_866_469_604_5, 1e-10));
        assert!(close(f.f, 1733.191_663_016_312_3, 1e-6));
        assert!(close(f.aic, 6.126_624_834_071_304, 1e-8));
        assert!(close(f.bic, 7.034_380_113_053_442, 1e-8));
        assert!(close(f.loglik, -0.063_312_417_035_652_05, 1e-9));
        assert!(close(f.durbin_watson, 1.671_806_808_814_922_5, 1e-9));
        assert!(close(f.ci_low[1], 0.810_294_41, 1e-7));
        assert!(close(f.ci_high[2], 1.433_510_54, 1e-7));
    }

    #[test]
    fn logit_matches_statsmodels() {
        let x: Matrix = (1..=10).map(|i| vec![1.0, i as f64]).collect();
        let y = [0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];
        let f = logit(&["const".into(), "x".into()], &x, &y).unwrap();
        assert!(close(f.coef[0], -2.990_331_93, 1e-5), "{}", f.coef[0]);
        assert!(close(f.coef[1], 0.543_696_71, 1e-6));
        assert!(close(f.se[0], 2.009_493_82, 1e-4));
        assert!(close(f.se[1], 0.336_164_3, 1e-5));
        assert!(close(f.p[1], 0.105_801_86, 1e-5));
        assert!(close(f.loglik, -4.941_579_983_434_301, 1e-8));
        assert!(close(f.loglik_null, -6.931_471_805_599_453, 1e-9));
        assert!(close(f.pseudo_r2, 0.287_080_706_374_316_8, 1e-7));
        assert!(close(f.aic, 13.883_159_966_868_602, 1e-7));
    }

    #[test]
    fn poisson_matches_statsmodels() {
        let x: Matrix = (1..=10).map(|i| vec![1.0, i as f64]).collect();
        let y = [0.0, 1.0, 1.0, 2.0, 2.0, 3.0, 5.0, 4.0, 7.0, 9.0];
        let f = poisson(&["const".into(), "x".into()], &x, &y).unwrap();
        assert!(close(f.coef[0], -0.787_869_29, 1e-6), "{}", f.coef[0]);
        assert!(close(f.coef[1], 0.301_964_97, 1e-7));
        assert!(close(f.se[1], 0.073_205_68, 1e-6));
        assert!(close(f.deviance, 2.063_769_754_746_915_4, 1e-6));
        assert!(close(f.aic, 32.890_576_575_381_87, 1e-6));
    }

    #[test]
    fn design_dummies() {
        use crate::build::ColType;
        let t = TableData {
            name: "t".into(),
            columns: vec!["y".into(), "g".into(), "x".into()],
            types: vec![ColType::Number, ColType::Text, ColType::Number],
            rows: (0..12).map(|i| vec![Scalar::Num(i as f64), Scalar::Text(["a", "b", "c"][i % 3].into()), if i == 5 { Scalar::Null } else { Scalar::Num((i * i) as f64) }]).collect(),
        };
        let d = design(&t, "y", &["g".into(), "x".into()], true, false, None).unwrap();
        assert_eq!(d.names, vec!["const", "g[b]", "g[c]", "x"]);
        assert_eq!(d.x.len(), 11);
    }
}
