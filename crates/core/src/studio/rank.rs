//! Feature ranking (information gain, gain ratio, Gini decrease, χ², ANOVA F, univariate
//! regression F) and permutation feature importance.

use std::collections::BTreeMap;

use super::learn::{self, Learned};
use crate::build::TableData;
use crate::ml::Rng;
use crate::scalar::Scalar;
use crate::stats::special::{chi2_sf, f_sf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RankRow {
    pub feature: String,
    pub categorical: bool,
    /// (score name, value).
    pub scores: Vec<(String, f64)>,
}

/// Discretises a column into labels: categories as-is, numbers into `bins` equal-frequency bins.
fn discretize(t: &TableData, c: usize, bins: usize) -> (Vec<Option<usize>>, bool) {
    if learn::categorical_column(t, c) {
        let mut map: BTreeMap<String, usize> = BTreeMap::new();
        let v = t
            .rows
            .iter()
            .map(|r| {
                r.get(c).filter(|v| !v.is_null()).map(|v| {
                    let n = map.len();
                    *map.entry(v.text()).or_insert(n)
                })
            })
            .collect();
        return (v, true);
    }
    let vals: Vec<f64> = t.rows.iter().map(|r| r.get(c).and_then(Scalar::as_f64).unwrap_or(f64::NAN)).collect();
    let s = crate::stats::sorted(&vals);
    let cuts: Vec<f64> = (1..bins).map(|i| crate::stats::quantile(&s, i as f64 / bins as f64)).collect();
    (vals.iter().map(|v| if v.is_finite() { Some(cuts.partition_point(|c| c < v)) } else { None }).collect(), false)
}

fn entropy(counts: &[f64]) -> f64 {
    let n: f64 = counts.iter().sum();
    if n <= 0.0 {
        return 0.0;
    }
    -counts.iter().filter(|c| **c > 0.0).map(|c| (c / n) * (c / n).log2()).sum::<f64>()
}

fn gini(counts: &[f64]) -> f64 {
    let n: f64 = counts.iter().sum();
    if n <= 0.0 {
        return 0.0;
    }
    1.0 - counts.iter().map(|c| (c / n).powi(2)).sum::<f64>()
}

/// Scores every feature against the target.
pub fn rank(t: &TableData, features: &[String], target: &str) -> Result<Vec<RankRow>, String> {
    let ti = t.column_index(target).ok_or_else(|| format!("There's no target column `{target}`."))?;
    let classify = crate::ml::is_categorical(t, ti);
    let td = learn::target_data(t, target, Some(classify))?;
    let data = learn::take_rows(t, &td.rows);
    let mut out = vec![];
    for f in features {
        if f.eq_ignore_ascii_case(target) {
            continue;
        }
        let Some(c) = data.column_index(f) else { continue };
        let (disc, categorical) = discretize(&data, c, 5);
        let mut scores = vec![];
        if classify {
            let k = td.classes.len();
            let nb = disc.iter().flatten().copied().max().map(|m| m + 1).unwrap_or(0);
            let mut table = vec![vec![0.0; k]; nb];
            let mut total = vec![0.0; k];
            for (d, y) in disc.iter().zip(&td.y) {
                if let Some(b) = d {
                    table[*b][*y as usize] += 1.0;
                    total[*y as usize] += 1.0;
                }
            }
            let n: f64 = total.iter().sum();
            let h = entropy(&total);
            let cond: f64 = table.iter().map(|r| r.iter().sum::<f64>() / n * entropy(r)).sum();
            let split_info = entropy(&table.iter().map(|r| r.iter().sum::<f64>()).collect::<Vec<_>>());
            let gi = gini(&total) - table.iter().map(|r| r.iter().sum::<f64>() / n * gini(r)).sum::<f64>();
            let ig = h - cond;
            // χ² of the contingency table.
            let mut chi = 0.0;
            for r in &table {
                let rs: f64 = r.iter().sum();
                for (j, o) in r.iter().enumerate() {
                    let e = rs * total[j] / n;
                    if e > 0.0 {
                        chi += (o - e).powi(2) / e;
                    }
                }
            }
            let df = ((nb as f64 - 1.0) * (k as f64 - 1.0)).max(1.0);
            scores.push(("Info. gain".to_string(), ig));
            scores.push(("Gain ratio".into(), if split_info > 0.0 { ig / split_info } else { 0.0 }));
            scores.push(("Gini".into(), gi));
            scores.push(("χ²".into(), chi));
            scores.push(("χ² p".into(), chi2_sf(chi, df)));
            // ANOVA F for numeric features.
            if !categorical {
                let vals: Vec<f64> = data.rows.iter().map(|r| r.get(c).and_then(Scalar::as_f64).unwrap_or(f64::NAN)).collect();
                let mut groups = vec![vec![]; k];
                for (v, y) in vals.iter().zip(&td.y) {
                    if v.is_finite() {
                        groups[*y as usize].push(*v);
                    }
                }
                let a = crate::stats::hypothesis::one_way_anova(&groups);
                scores.push(("ANOVA F".into(), a.rows.first().map(|r| r.f).unwrap_or(f64::NAN)));
            } else {
                scores.push(("ANOVA F".into(), f64::NAN));
            }
        } else {
            // Univariate regression F (numeric) or one-way ANOVA F (categorical feature).
            let (f_val, p) = if categorical {
                let mut groups: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
                for (d, y) in disc.iter().zip(&td.y) {
                    if let Some(b) = d {
                        groups.entry(*b).or_default().push(*y);
                    }
                }
                let a = crate::stats::hypothesis::one_way_anova(&groups.into_values().collect::<Vec<_>>());
                a.rows.first().map(|r| (r.f, r.p)).unwrap_or((f64::NAN, f64::NAN))
            } else {
                let vals: Vec<f64> = data.rows.iter().map(|r| r.get(c).and_then(Scalar::as_f64).unwrap_or(f64::NAN)).collect();
                let pr = crate::stats::hypothesis::pearson(&vals, &td.y);
                let r2 = pr.statistic * pr.statistic;
                let df = pr.df.unwrap_or(1.0);
                let f = r2 / (1.0 - r2).max(1e-300) * df;
                (f, f_sf(f, 1.0, df))
            };
            // Variance reduction from a 5-bin split (regression "info gain").
            let mut groups: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
            for (d, y) in disc.iter().zip(&td.y) {
                if let Some(b) = d {
                    groups.entry(*b).or_default().push(*y);
                }
            }
            let all: Vec<f64> = groups.values().flatten().copied().collect();
            let tv = crate::stats::variance(&all);
            let wv: f64 = groups.values().map(|g| g.len() as f64 / all.len().max(1) as f64 * if g.len() > 1 { crate::stats::variance(g) } else { 0.0 }).sum();
            scores.push(("Univar. reg. F".to_string(), f_val));
            scores.push(("p-value".into(), p));
            scores.push(("RReliefF-like var. reduction".into(), if tv > 0.0 { 1.0 - wv / tv } else { 0.0 }));
        }
        out.push(RankRow { feature: f.clone(), categorical, scores });
    }
    let key = if classify { "Info. gain" } else { "Univar. reg. F" };
    out.sort_by(|a, b| {
        let g = |r: &RankRow| r.scores.iter().find(|s| s.0 == key).map(|s| s.1).filter(|v| v.is_finite()).unwrap_or(f64::NEG_INFINITY);
        g(b).partial_cmp(&g(a)).unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(out)
}

/// Permutation feature importance: drop in score (accuracy or R²) when a feature is shuffled,
/// averaged over `repeats`. Returns (feature, mean drop, std of drop), largest first.
pub fn permutation_importance(m: &Learned, t: &TableData, repeats: usize, seed: u64) -> Result<Vec<(String, f64, f64)>, String> {
    let td = learn::target_data(t, &m.target, Some(m.is_classifier()))?;
    let data = learn::take_rows(t, &td.rows);
    let y: Vec<f64> = if m.is_classifier() { td.y.iter().map(|c| td.classes.get(*c as usize).and_then(|l| m.classes.iter().position(|x| x == l)).map(|v| v as f64).unwrap_or(f64::NAN)).collect() } else { td.y.clone() };
    let score = |tab: &TableData| -> f64 {
        let p = m.predict_table(tab);
        if m.is_classifier() {
            p.iter().zip(&y).filter(|(p, a)| p.0 == **a).count() as f64 / y.len().max(1) as f64
        } else {
            crate::ml::regression_metrics(&y, &p.iter().map(|x| x.0).collect::<Vec<_>>()).r2
        }
    };
    let base = score(&data);
    let mut rng = Rng::new(seed);
    let mut out = vec![];
    for f in &m.encoder.features {
        let Some(c) = data.column_index(f.name()) else { continue };
        let mut drops = vec![];
        for _ in 0..repeats.clamp(1, 20) {
            let mut col: Vec<Scalar> = data.rows.iter().map(|r| r.get(c).cloned().unwrap_or(Scalar::Null)).collect();
            rng.shuffle(&mut col);
            let mut shuffled = data.clone();
            for (r, v) in shuffled.rows.iter_mut().zip(col) {
                if let Some(cell) = r.get_mut(c) {
                    *cell = v;
                }
            }
            drops.push(base - score(&shuffled));
        }
        let mean = drops.iter().sum::<f64>() / drops.len() as f64;
        let sd = (drops.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / drops.len() as f64).sqrt();
        out.push((f.name().to_string(), mean, sd));
    }
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(out)
}

/// Per-row explanation for linear-style models: contribution of each encoded feature
/// (coefficient × standardized value) to the score of `class` (or the value).
pub fn explain_row(m: &Learned, x: &[f64], class: usize) -> Vec<(String, f64)> {
    match m.coefficients() {
        Some(coefs) => {
            let mut v: Vec<(String, f64)> = coefs.iter().skip(1).zip(x).map(|((n, c), xv)| (n.clone(), c.get(class.min(c.len().saturating_sub(1))).copied().unwrap_or(0.0) * xv)).collect();
            v.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap_or(std::cmp::Ordering::Equal));
            v
        }
        None => {
            // Occlusion: replace each feature by its mean (0 after scaling) and measure the change.
            let base = m.predict_x(x);
            let pick = |p: &(f64, Vec<f64>)| if m.is_classifier() { p.1.get(class).copied().unwrap_or(0.0) } else { p.0 };
            let b = pick(&base);
            let mut v: Vec<(String, f64)> = m
                .encoder
                .names
                .iter()
                .enumerate()
                .map(|(j, n)| {
                    let mut z = x.to_vec();
                    if let Some(e) = z.get_mut(j) {
                        *e = 0.0;
                    }
                    (n.clone(), b - pick(&m.predict_x(&z)))
                })
                .collect();
            v.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap_or(std::cmp::Ordering::Equal));
            v
        }
    }
}

/// Convenience for the Test & Score table.
pub fn score_names(classification: bool) -> Vec<&'static str> {
    if classification { vec!["AUC", "CA", "F1", "Precision", "Recall", "LogLoss", "MCC"] } else { vec!["MSE", "RMSE", "MAE", "R²", "MAPE"] }
}


#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::studio::learn::tests_support::sample;

    #[test]
    fn ranking_and_importance() {
        let t = sample(300);
        let r = rank(&t, &["a".into(), "b".into(), "c".into()], "label").unwrap();
        // The informative numeric features outrank the random category.
        assert_eq!(r.last().unwrap().feature, "c");
        let m = learn::fit(&learn::LearnerSpec { kind: "forest".into(), ..Default::default() }, &t, &["a".into(), "b".into(), "c".into()], "y").unwrap();
        let imp = permutation_importance(&m, &t, 3, 1).unwrap();
        assert_eq!(imp[0].0, "a");
        let rr = rank(&t, &["a".into(), "b".into(), "c".into()], "y").unwrap();
        assert_eq!(rr[0].feature, "a");
    }
}
