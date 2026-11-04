//! Model evaluation: resampling (k-fold, stratified k-fold, leave-one-out, repeated random
//! sampling, test on training data, test on a separate set), scores per learner, confusion
//! matrices, ROC curves with AUC, cumulative gain / lift curves and calibration curves.

use std::collections::BTreeMap;

use super::learn::{self, Learned, LearnerSpec};
use crate::build::TableData;
use crate::ml::{Matrix, Rng};

/// Predictions of several learners on the same test rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EvalResults {
    pub learners: Vec<String>,
    pub target: String,
    /// Class labels (empty = regression).
    pub classes: Vec<String>,
    /// Actual class index / value per test row.
    pub actual: Vec<f64>,
    /// `predicted[learner][row]`.
    pub predicted: Vec<Vec<f64>>,
    /// `probs[learner][row][class]` (classification).
    pub probs: Vec<Vec<Vec<f64>>>,
    /// Fold of each test row.
    pub folds: Vec<usize>,
    /// Row index (in the input data) of each test row.
    pub rows: Vec<usize>,
    pub train_seconds: Vec<f64>,
    pub test_seconds: Vec<f64>,
    pub method: String,
    pub errors: Vec<(String, String)>,
}

impl EvalResults {
    pub fn is_classification(&self) -> bool {
        !self.classes.is_empty()
    }
}

/// How to split the data.
#[derive(Clone, Debug, PartialEq)]
pub enum Sampling {
    KFold { k: usize, stratified: bool },
    LeaveOneOut,
    Random { repeats: usize, train_fraction: f64, stratified: bool },
    TrainOnTest,
}

impl Sampling {
    pub fn label(&self) -> String {
        match self {
            Sampling::KFold { k, stratified } => format!("{}{k}-fold cross-validation", if *stratified { "Stratified " } else { "" }),
            Sampling::LeaveOneOut => "Leave-one-out".into(),
            Sampling::Random { repeats, train_fraction, .. } => format!("Random sampling ({repeats} × {:.0}% train)", train_fraction * 100.0),
            Sampling::TrainOnTest => "Test on train data".into(),
        }
    }
}

/// Train/test index pairs for a sampling scheme. `y` are class indices for stratification.
pub fn splits(n: usize, y: Option<&[f64]>, s: &Sampling, seed: u64) -> Vec<(Vec<usize>, Vec<usize>)> {
    let mut rng = Rng::new(seed);
    match s {
        Sampling::TrainOnTest => vec![((0..n).collect(), (0..n).collect())],
        Sampling::LeaveOneOut => (0..n).map(|i| ((0..n).filter(|j| *j != i).collect(), vec![i])).collect(),
        Sampling::KFold { k, stratified } => {
            let k = (*k).clamp(2, n.max(2));
            let mut fold_of = vec![0usize; n];
            match (stratified, y) {
                (true, Some(y)) => {
                    let mut by: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
                    for i in 0..n {
                        by.entry(y.get(i).copied().unwrap_or(0.0) as i64).or_default().push(i);
                    }
                    let mut off = 0;
                    for (_, mut idx) in by {
                        rng.shuffle(&mut idx);
                        for (j, i) in idx.iter().enumerate() {
                            fold_of[*i] = (j + off) % k;
                        }
                        off += idx.len();
                    }
                }
                _ => {
                    let mut idx: Vec<usize> = (0..n).collect();
                    rng.shuffle(&mut idx);
                    for (j, i) in idx.iter().enumerate() {
                        fold_of[*i] = j % k;
                    }
                }
            }
            (0..k).map(|f| ((0..n).filter(|i| fold_of[*i] != f).collect(), (0..n).filter(|i| fold_of[*i] == f).collect())).filter(|(a, b): &(Vec<usize>, Vec<usize>)| !a.is_empty() && !b.is_empty()).collect()
        }
        Sampling::Random { repeats, train_fraction, stratified } => (0..(*repeats).clamp(1, 100))
            .map(|_| {
                let mut train = vec![];
                let mut test = vec![];
                let groups: Vec<Vec<usize>> = match (stratified, y) {
                    (true, Some(y)) => {
                        let mut by: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
                        for i in 0..n {
                            by.entry(y.get(i).copied().unwrap_or(0.0) as i64).or_default().push(i);
                        }
                        by.into_values().collect()
                    }
                    _ => vec![(0..n).collect()],
                };
                for mut g in groups {
                    rng.shuffle(&mut g);
                    let cut = ((g.len() as f64) * train_fraction.clamp(0.05, 0.95)).round() as usize;
                    train.extend_from_slice(&g[..cut.min(g.len())]);
                    test.extend_from_slice(&g[cut.min(g.len())..]);
                }
                (train, test)
            })
            .filter(|(a, b)| !a.is_empty() && !b.is_empty())
            .collect(),
    }
}

/// Cross-validates learners on a table. When `test` is given, trains on `t` and tests on it.
pub fn evaluate(specs: &[LearnerSpec], t: &TableData, features: &[String], target: &str, sampling: &Sampling, seed: u64, test: Option<&TableData>) -> Result<EvalResults, String> {
    if specs.is_empty() {
        return Err("Connect at least one model.".into());
    }
    // Decide the task from the first learner (all must agree).
    let classify = specs.iter().any(|s| matches!(s.kind.as_str(), "logistic" | "naive_bayes"));
    let classify = if classify { Some(true) } else if specs.iter().any(|s| s.kind == "linear") { Some(false) } else { None };
    let td = learn::target_data(t, target, classify)?;
    let data = learn::take_rows(t, &td.rows);
    let k = td.classes.len();
    let mut res = EvalResults { learners: specs.iter().map(|s| if s.name.is_empty() { learn::learner_label(&s.kind).to_string() } else { s.name.clone() }).collect(), target: target.to_string(), classes: td.classes.clone(), method: sampling.label(), ..Default::default() };
    if let Some(test_t) = test {
        let tt = learn::target_data(test_t, target, Some(k > 0))?;
        let test_data = learn::take_rows(test_t, &tt.rows);
        // Map test classes to training classes.
        let actual: Vec<f64> = if k > 0 { tt.y.iter().map(|c| tt.classes.get(*c as usize).and_then(|l| td.classes.iter().position(|x| x == l)).map(|v| v as f64).unwrap_or(f64::NAN)).collect() } else { tt.y.clone() };
        res.method = "Test on separate data".into();
        res.actual = actual;
        res.rows = tt.rows.clone();
        res.folds = vec![0; tt.rows.len()];
        for s in specs {
            let t0 = std::time::Instant::now();
            match learn::fit(s, &data, features, target) {
                Ok(m) => {
                    res.train_seconds.push(t0.elapsed().as_secs_f64());
                    let t1 = std::time::Instant::now();
                    let p = m.predict_table(&test_data);
                    res.test_seconds.push(t1.elapsed().as_secs_f64());
                    res.predicted.push(p.iter().map(|x| x.0).collect());
                    res.probs.push(p.into_iter().map(|x| x.1).collect());
                }
                Err(e) => {
                    res.errors.push((s.name.clone(), e));
                    res.train_seconds.push(0.0);
                    res.test_seconds.push(0.0);
                    res.predicted.push(vec![f64::NAN; tt.rows.len()]);
                    res.probs.push(vec![vec![]; tt.rows.len()]);
                }
            }
        }
        return Ok(res);
    }
    let feats: Vec<String> = features.iter().filter(|f| !f.eq_ignore_ascii_case(target)).cloned().collect();
    // Encode once on all data (unsupervised preprocessing: imputation, one-hot, scaling).
    let enc = learn::Encoder::fit(&data, &feats)?;
    let x: Matrix = enc.encode(&data);
    let n = x.len();
    if n < 4 {
        return Err("Too few rows to evaluate.".into());
    }
    let sp = splits(n, if k > 0 { Some(&td.y) } else { None }, sampling, seed);
    for (f, (_, test)) in sp.iter().enumerate() {
        for i in test {
            res.actual.push(td.y[*i]);
            res.rows.push(td.rows[*i]);
            res.folds.push(f);
        }
    }
    for s in specs {
        let mut pred = vec![];
        let mut probs = vec![];
        let (mut tt, mut te) = (0.0, 0.0);
        let mut err = None;
        for (train, test) in &sp {
            let bx: Matrix = train.iter().map(|i| x[*i].clone()).collect();
            let by: Vec<f64> = train.iter().map(|i| td.y[*i]).collect();
            let t0 = std::time::Instant::now();
            match learn::fit_matrix(s, &bx, &by, k) {
                Ok(p) => {
                    tt += t0.elapsed().as_secs_f64();
                    let m = Learned { spec: s.clone(), target: target.into(), classes: td.classes.clone(), encoder: enc.clone(), predictor: p, train_rows: bx.len() };
                    let t1 = std::time::Instant::now();
                    for i in test {
                        let (v, pr) = m.predict_x(&x[*i]);
                        pred.push(v);
                        probs.push(pr);
                    }
                    te += t1.elapsed().as_secs_f64();
                }
                Err(e) => {
                    err = Some(e);
                    for _ in test {
                        pred.push(f64::NAN);
                        probs.push(vec![]);
                    }
                }
            }
        }
        if let Some(e) = err {
            res.errors.push((if s.name.is_empty() { learn::learner_label(&s.kind).to_string() } else { s.name.clone() }, e));
        }
        res.predicted.push(pred);
        res.probs.push(probs);
        res.train_seconds.push(tt);
        res.test_seconds.push(te);
    }
    Ok(res)
}

// =============================================================================================
// Scores
// =============================================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scores {
    pub learner: String,
    /// Ordered (metric name, value).
    pub values: Vec<(String, f64)>,
}

impl Scores {
    pub fn get(&self, k: &str) -> f64 {
        self.values.iter().find(|(n, _)| n == k).map(|(_, v)| *v).unwrap_or(f64::NAN)
    }
}

pub fn confusion(r: &EvalResults, learner: usize) -> Vec<Vec<usize>> {
    let k = r.classes.len();
    let mut m = vec![vec![0usize; k]; k];
    if let Some(p) = r.predicted.get(learner) {
        for (a, b) in r.actual.iter().zip(p) {
            if a.is_finite() && b.is_finite() {
                let (a, b) = (*a as usize, *b as usize);
                if a < k && b < k {
                    m[a][b] += 1;
                }
            }
        }
    }
    m
}

/// Scores per learner (classification: AUC, CA, F1, Precision, Recall, LogLoss, MCC;
/// regression: MSE, RMSE, MAE, R², MAPE).
pub fn scores(r: &EvalResults) -> Vec<Scores> {
    (0..r.learners.len())
        .map(|l| {
            let name = r.learners[l].clone();
            let pred = r.predicted.get(l).cloned().unwrap_or_default();
            if r.is_classification() {
                let k = r.classes.len();
                let cm = confusion(r, l);
                let n: usize = cm.iter().flatten().sum();
                let correct: usize = (0..k).map(|i| cm[i][i]).sum();
                let support: Vec<f64> = (0..k).map(|i| cm[i].iter().sum::<usize>() as f64).collect();
                let (mut p_w, mut r_w, mut f_w) = (0.0, 0.0, 0.0);
                for c in 0..k {
                    let tp = cm[c][c] as f64;
                    let pp: f64 = (0..k).map(|i| cm[i][c] as f64).sum();
                    let prec = if pp > 0.0 { tp / pp } else { 0.0 };
                    let rec = if support[c] > 0.0 { tp / support[c] } else { 0.0 };
                    let f1 = if prec + rec > 0.0 { 2.0 * prec * rec / (prec + rec) } else { 0.0 };
                    let w = support[c] / n.max(1) as f64;
                    p_w += w * prec;
                    r_w += w * rec;
                    f_w += w * f1;
                }
                let probs = r.probs.get(l);
                // Weighted one-vs-rest AUC.
                let mut auc = 0.0;
                let mut wsum = 0.0;
                for c in 0..k {
                    if k == 2 && c == 0 {
                        continue;
                    }
                    let a = roc(r, l, c).auc;
                    if a.is_finite() {
                        let w = if k == 2 { 1.0 } else { support[c] };
                        auc += w * a;
                        wsum += w;
                    }
                }
                let ll = probs
                    .map(|p| {
                        let v: Vec<f64> = r.actual.iter().zip(p).filter_map(|(a, pr)| pr.get(*a as usize).map(|x| -x.clamp(1e-15, 1.0).ln())).collect();
                        v.iter().sum::<f64>() / v.len().max(1) as f64
                    })
                    .unwrap_or(f64::NAN);
                // Matthews correlation (multiclass generalisation).
                let t: Vec<f64> = support.clone();
                let pcol: Vec<f64> = (0..k).map(|c| (0..k).map(|i| cm[i][c] as f64).sum()).collect();
                let s = n as f64;
                let c_ = correct as f64;
                let num = c_ * s - t.iter().zip(&pcol).map(|(a, b)| a * b).sum::<f64>();
                let den = ((s * s - pcol.iter().map(|v| v * v).sum::<f64>()) * (s * s - t.iter().map(|v| v * v).sum::<f64>())).sqrt();
                Scores {
                    learner: name,
                    values: vec![
                        ("AUC".into(), if wsum > 0.0 { auc / wsum } else { f64::NAN }),
                        ("CA".into(), correct as f64 / n.max(1) as f64),
                        ("F1".into(), f_w),
                        ("Precision".into(), p_w),
                        ("Recall".into(), r_w),
                        ("LogLoss".into(), ll),
                        ("MCC".into(), if den > 0.0 { num / den } else { 0.0 }),
                    ],
                }
            } else {
                let pairs: Vec<(f64, f64)> = r.actual.iter().zip(&pred).filter(|(a, b)| a.is_finite() && b.is_finite()).map(|(a, b)| (*a, *b)).collect();
                let n = pairs.len().max(1) as f64;
                let mse = pairs.iter().map(|(a, b)| (a - b).powi(2)).sum::<f64>() / n;
                let mae = pairs.iter().map(|(a, b)| (a - b).abs()).sum::<f64>() / n;
                let mean = pairs.iter().map(|p| p.0).sum::<f64>() / n;
                let sst = pairs.iter().map(|(a, _)| (a - mean).powi(2)).sum::<f64>() / n;
                let mape_v: Vec<f64> = pairs.iter().filter(|(a, _)| *a != 0.0).map(|(a, b)| ((a - b) / a).abs()).collect();
                Scores {
                    learner: name,
                    values: vec![
                        ("MSE".into(), mse),
                        ("RMSE".into(), mse.sqrt()),
                        ("MAE".into(), mae),
                        ("R²".into(), if sst > 0.0 { 1.0 - mse / sst } else { f64::NAN }),
                        ("MAPE".into(), if mape_v.is_empty() { f64::NAN } else { mape_v.iter().sum::<f64>() / mape_v.len() as f64 }),
                    ],
                }
            }
        })
        .collect()
}

// =============================================================================================
// Curves
// =============================================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve {
    /// (x, y, threshold) points.
    pub points: Vec<(f64, f64, f64)>,
    pub auc: f64,
}

fn scored(r: &EvalResults, learner: usize, class: usize) -> Vec<(f64, bool)> {
    let Some(p) = r.probs.get(learner) else { return vec![] };
    r.actual.iter().zip(p).filter_map(|(a, pr)| pr.get(class).map(|s| (*s, *a as usize == class && a.is_finite()))).collect()
}

/// ROC curve (FPR, TPR, threshold) for `class` vs rest; AUC by the trapezoid rule.
pub fn roc(r: &EvalResults, learner: usize, class: usize) -> Curve {
    roc_from(&scored(r, learner, class))
}

pub fn roc_from(scores: &[(f64, bool)]) -> Curve {
    let mut s: Vec<(f64, bool)> = scores.iter().copied().filter(|(v, _)| v.is_finite()).collect();
    s.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let pos = s.iter().filter(|x| x.1).count() as f64;
    let neg = s.len() as f64 - pos;
    if pos == 0.0 || neg == 0.0 {
        return Curve { points: vec![(0.0, 0.0, 1.0), (1.0, 1.0, 0.0)], auc: f64::NAN };
    }
    let mut pts = vec![(0.0, 0.0, f64::INFINITY)];
    let (mut tp, mut fp) = (0.0, 0.0);
    let mut i = 0;
    while i < s.len() {
        let th = s[i].0;
        while i < s.len() && s[i].0 == th {
            if s[i].1 {
                tp += 1.0;
            } else {
                fp += 1.0;
            }
            i += 1;
        }
        pts.push((fp / neg, tp / pos, th));
    }
    let auc = pts.windows(2).map(|w| (w[1].0 - w[0].0) * (w[1].1 + w[0].1) / 2.0).sum();
    Curve { points: pts, auc }
}

/// Cumulative gains: (fraction of population targeted, fraction of positives captured).
/// `auc` holds the lift at 10 %.
pub fn gains(r: &EvalResults, learner: usize, class: usize) -> Curve {
    let mut s = scored(r, learner, class);
    s.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let pos = s.iter().filter(|x| x.1).count() as f64;
    let n = s.len() as f64;
    if pos == 0.0 || n == 0.0 {
        return Curve::default();
    }
    let mut pts = vec![(0.0, 0.0, 1.0)];
    let mut tp = 0.0;
    for (i, (v, y)) in s.iter().enumerate() {
        if *y {
            tp += 1.0;
        }
        pts.push(((i + 1) as f64 / n, tp / pos, *v));
    }
    let at10 = pts.iter().find(|p| p.0 >= 0.1).map(|p| p.1 / p.0).unwrap_or(f64::NAN);
    Curve { points: pts, auc: at10 }
}

/// Calibration curve: (mean predicted probability, observed frequency, count) per bin.
pub fn calibration(r: &EvalResults, learner: usize, class: usize, bins: usize) -> Vec<(f64, f64, usize)> {
    let s = scored(r, learner, class);
    let bins = bins.clamp(2, 50);
    let mut acc = vec![(0.0, 0.0, 0usize); bins];
    for (p, y) in s {
        let b = ((p * bins as f64) as usize).min(bins - 1);
        acc[b].0 += p;
        acc[b].1 += if y { 1.0 } else { 0.0 };
        acc[b].2 += 1;
    }
    acc.into_iter().filter(|b| b.2 > 0).map(|(p, y, c)| (p / c as f64, y / c as f64, c)).collect()
}

/// Brier score for `class`.
pub fn brier(r: &EvalResults, learner: usize, class: usize) -> f64 {
    let s = scored(r, learner, class);
    s.iter().map(|(p, y)| (p - if *y { 1.0 } else { 0.0 }).powi(2)).sum::<f64>() / s.len().max(1) as f64
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn auc_reference() {
        // sklearn.metrics.roc_auc_score([0,0,1,1],[0.1,0.4,0.35,0.8]) = 0.75
        let c = roc_from(&[(0.1, false), (0.4, false), (0.35, true), (0.8, true)]);
        assert!((c.auc - 0.75).abs() < 1e-12);
        // Ties: roc_auc_score([0,1,0,1],[0.5,0.5,0.2,0.9]) = 0.875
        let c = roc_from(&[(0.5, false), (0.5, true), (0.2, false), (0.9, true)]);
        assert!((c.auc - 0.875).abs() < 1e-12);
    }

    #[test]
    fn stratified_folds() {
        let y: Vec<f64> = (0..100).map(|i| if i < 20 { 1.0 } else { 0.0 }).collect();
        let s = splits(100, Some(&y), &Sampling::KFold { k: 5, stratified: true }, 1);
        assert_eq!(s.len(), 5);
        for (train, test) in &s {
            assert_eq!(train.len() + test.len(), 100);
            assert_eq!(test.iter().filter(|i| y[**i] == 1.0).count(), 4);
        }
        let loo = splits(10, None, &Sampling::LeaveOneOut, 1);
        assert_eq!(loo.len(), 10);
    }

    #[test]
    fn evaluate_cv() {
        let t = super::super::learn::tests_support::sample(200);
        let specs = vec![LearnerSpec { kind: "logistic".into(), ..Default::default() }, LearnerSpec { kind: "tree".into(), ..Default::default() }];
        let r = evaluate(&specs, &t, &["a".into(), "b".into(), "c".into()], "label", &Sampling::KFold { k: 5, stratified: true }, 3, None).unwrap();
        assert_eq!(r.actual.len(), 200);
        let s = scores(&r);
        assert!(s[0].get("AUC") > 0.85, "{:?}", s[0]);
        assert!(s[1].get("CA") > 0.7);
        let g = gains(&r, 0, 1);
        assert!((g.points.last().unwrap().1 - 1.0).abs() < 1e-12);
        let cal = calibration(&r, 0, 1, 10);
        assert!(!cal.is_empty());
    }
}
