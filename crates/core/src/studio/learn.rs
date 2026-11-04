//! Learners for the Data Science Studio, built on the [`crate::ml`] tree builder and written
//! from scratch with standard methods: linear regression (OLS / ridge / lasso / elastic net by
//! coordinate descent), multinomial logistic regression (Adam), CART trees, random forests,
//! gradient boosting (squared loss; log-loss one-vs-rest with Newton leaf values), AdaBoost
//! (SAMME by weighted resampling; AdaBoost.R2 for regression), linear SVM (Pegasos hinge /
//! ε-insensitive SGD, one-vs-rest), multilayer perceptron (1–2 hidden layers, ReLU / tanh /
//! logistic, Adam, L2), Gaussian naive Bayes and k-nearest neighbours.
//!
//! Every trained model is a serializable [`Learned`]: an [`Encoder`] (mean imputation, one-hot
//! categories, standardisation) plus a [`Predictor`]. That is what the model registry stores and
//! the scoring server loads.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as J;

use crate::build::TableData;
use crate::ml::{self, Matrix, Rng};
use crate::scalar::Scalar;

// =============================================================================================
// Encoding
// =============================================================================================

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FeatureEnc {
    Numeric { name: String, fill: f64 },
    Categorical { name: String, levels: Vec<String> },
}

impl FeatureEnc {
    pub fn name(&self) -> &str {
        match self {
            FeatureEnc::Numeric { name, .. } | FeatureEnc::Categorical { name, .. } => name,
        }
    }
}

/// Maps raw rows to standardized numeric vectors.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Encoder {
    pub features: Vec<FeatureEnc>,
    /// Encoded column names (`Channel=Online`).
    pub names: Vec<String>,
    pub mean: Vec<f64>,
    pub std: Vec<f64>,
}

/// Is a column categorical (text / bool, or numbers with few integer values when `strict`)?
pub fn categorical_column(t: &TableData, c: usize) -> bool {
    use crate::build::ColType;
    let ty = t.types.get(c).copied().unwrap_or_default();
    if matches!(ty, ColType::Text | ColType::Bool) {
        return true;
    }
    if ty.is_numeric() || ty.is_temporal() {
        return false;
    }
    t.rows.iter().take(500).any(|r| matches!(r.get(c), Some(Scalar::Text(_)) | Some(Scalar::Bool(_))))
}

impl Encoder {
    pub fn fit(t: &TableData, features: &[String]) -> Result<Encoder, String> {
        let mut enc = Encoder::default();
        for f in features {
            let c = t.column_index(f).ok_or_else(|| format!("There's no column `{f}`."))?;
            if categorical_column(t, c) {
                let mut counts: BTreeMap<String, usize> = BTreeMap::new();
                for r in &t.rows {
                    if let Some(v) = r.get(c).filter(|v| !v.is_null()) {
                        *counts.entry(v.text()).or_default() += 1;
                    }
                }
                let mut lv: Vec<(String, usize)> = counts.into_iter().collect();
                lv.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                lv.truncate(50);
                let mut levels: Vec<String> = lv.into_iter().map(|x| x.0).collect();
                levels.sort();
                for l in &levels {
                    enc.names.push(format!("{f}={l}"));
                }
                enc.features.push(FeatureEnc::Categorical { name: f.clone(), levels });
            } else {
                let vals: Vec<f64> = t.rows.iter().filter_map(|r| r.get(c).and_then(Scalar::as_f64)).filter(|v| v.is_finite()).collect();
                let fill = if vals.is_empty() { 0.0 } else { vals.iter().sum::<f64>() / vals.len() as f64 };
                enc.names.push(f.clone());
                enc.features.push(FeatureEnc::Numeric { name: f.clone(), fill });
            }
        }
        if enc.names.is_empty() {
            return Err("No usable feature columns.".into());
        }
        let raw = enc.raw_table(t);
        let sc = ml::Scaler::fit(&raw);
        enc.mean = sc.mean;
        enc.std = sc.std;
        Ok(enc)
    }

    /// Unscaled encoded row from a value getter.
    pub fn raw_with(&self, get: &dyn Fn(&str) -> Scalar) -> Vec<f64> {
        let mut out = Vec::with_capacity(self.names.len());
        for f in &self.features {
            match f {
                FeatureEnc::Numeric { name, fill } => out.push(get(name).as_f64().filter(|v| v.is_finite()).unwrap_or(*fill)),
                FeatureEnc::Categorical { name, levels } => {
                    let v = get(name);
                    let s = if v.is_null() { None } else { Some(v.text()) };
                    for l in levels {
                        out.push(if s.as_deref() == Some(l.as_str()) { 1.0 } else { 0.0 });
                    }
                }
            }
        }
        out
    }

    pub fn scale(&self, mut r: Vec<f64>) -> Vec<f64> {
        for (j, v) in r.iter_mut().enumerate() {
            *v = (*v - self.mean.get(j).copied().unwrap_or(0.0)) / self.std.get(j).copied().filter(|s| *s > 0.0).unwrap_or(1.0);
        }
        r
    }

    fn raw_table(&self, t: &TableData) -> Matrix {
        let idx: BTreeMap<&str, Option<usize>> = self.features.iter().map(|f| (f.name(), t.column_index(f.name()))).collect();
        t.rows
            .iter()
            .map(|r| {
                let get = |n: &str| idx.get(n).copied().flatten().and_then(|c| r.get(c)).cloned().unwrap_or(Scalar::Null);
                self.raw_with(&get)
            })
            .collect()
    }

    /// Standardized matrix of a table (missing columns → fill values).
    pub fn encode(&self, t: &TableData) -> Matrix {
        self.raw_table(t).into_iter().map(|r| self.scale(r)).collect()
    }

    /// Standardized row from a JSON object (`{"Quantity": 3, "Channel": "Online"}`).
    pub fn encode_json(&self, obj: &serde_json::Map<String, J>) -> Vec<f64> {
        let get = |n: &str| match obj.get(n).or_else(|| obj.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v)) {
            Some(J::Number(x)) => x.as_f64().map(Scalar::Num).unwrap_or(Scalar::Null),
            Some(J::String(s)) => s.trim().parse::<f64>().ok().filter(|_| self.is_numeric(n)).map(Scalar::Num).unwrap_or_else(|| Scalar::Text(s.clone())),
            Some(J::Bool(b)) => Scalar::Bool(*b),
            _ => Scalar::Null,
        };
        self.scale(self.raw_with(&get))
    }

    fn is_numeric(&self, n: &str) -> bool {
        self.features.iter().any(|f| matches!(f, FeatureEnc::Numeric { name, .. } if name.eq_ignore_ascii_case(n)))
    }

    pub fn dim(&self) -> usize {
        self.names.len()
    }
}

// =============================================================================================
// Learner specs
// =============================================================================================

/// A learner and its hyper-parameters (unfitted).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LearnerSpec {
    /// "linear", "logistic", "tree", "forest", "gbm", "adaboost", "svm", "mlp", "naive_bayes", "knn".
    pub kind: String,
    /// Display name ("Random Forest").
    pub name: String,
    pub params: BTreeMap<String, J>,
}

impl LearnerSpec {
    pub fn f(&self, k: &str, d: f64) -> f64 {
        self.params.get(k).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).filter(|v| v.is_finite()).unwrap_or(d)
    }
    pub fn u(&self, k: &str, d: usize) -> usize {
        let v = self.f(k, d as f64);
        if v < 0.0 { d } else { v as usize }
    }
    pub fn s(&self, k: &str, d: &str) -> String {
        self.params.get(k).and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| d.to_string())
    }
}

pub fn learner_label(kind: &str) -> &'static str {
    match kind {
        "linear" => "Linear Regression",
        "logistic" => "Logistic Regression",
        "tree" => "Tree",
        "forest" => "Random Forest",
        "gbm" => "Gradient Boosting",
        "adaboost" => "AdaBoost",
        "svm" => "SVM",
        "mlp" => "Neural Network",
        "naive_bayes" => "Naive Bayes",
        "knn" => "kNN",
        _ => "Model",
    }
}

// =============================================================================================
// Predictors
// =============================================================================================

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "n", rename_all = "camelCase")]
pub enum SNode {
    Leaf { v: Vec<f64> },
    Split { f: usize, t: f64, l: usize, r: usize },
}

pub fn tree_predict<'a>(nodes: &'a [SNode], x: &[f64]) -> &'a [f64] {
    let mut i = 0;
    for _ in 0..100_000 {
        match nodes.get(i) {
            Some(SNode::Leaf { v }) => return v,
            Some(SNode::Split { f, t, l, r }) => i = if x.get(*f).copied().unwrap_or(0.0) <= *t { *l } else { *r },
            None => break,
        }
    }
    &[]
}

fn leaf_index(nodes: &[SNode], x: &[f64]) -> usize {
    let mut i = 0;
    for _ in 0..100_000 {
        match nodes.get(i) {
            Some(SNode::Split { f, t, l, r }) => i = if x.get(*f).copied().unwrap_or(0.0) <= *t { *l } else { *r },
            _ => return i,
        }
    }
    i
}

fn convert_tree(t: &ml::Tree) -> Vec<SNode> {
    t.nodes
        .iter()
        .map(|n| match n {
            ml::TreeNode::Leaf { value, .. } => SNode::Leaf { v: value.clone() },
            ml::TreeNode::Split { feature, threshold, left, right, .. } => SNode::Split { f: *feature, t: *threshold, l: *left, r: *right },
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layer {
    /// `w[out][in]`.
    pub w: Matrix,
    pub b: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Predictor {
    /// y = intercept + coef · x (standardized x).
    Linear { intercept: f64, coef: Vec<f64> },
    /// Softmax regression: `w[class] = [bias, weights…]`.
    Logistic { w: Matrix },
    Tree { nodes: Vec<SNode> },
    Forest { trees: Vec<Vec<SNode>> },
    /// `stages[m][k]` trees; regression has one output, classification one per class (OvR).
    Gbm { init: Vec<f64>, rate: f64, stages: Vec<Vec<Vec<SNode>>> },
    /// Weighted trees (SAMME for classes; weighted median for regression).
    AdaBoost { trees: Vec<Vec<SNode>>, alphas: Vec<f64> },
    /// Linear SVM: `w[k] = [bias, weights…]` (one-vs-rest, or one row for regression).
    Svm { w: Matrix, scale: f64 },
    Mlp { layers: Vec<Layer>, activation: String, y_mean: f64, y_std: f64 },
    NaiveBayes { prior: Vec<f64>, mean: Matrix, var: Matrix },
    Knn { x: Matrix, y: Vec<f64>, k: usize, distance_weighted: bool },
}

/// A trained model: encoder + predictor + target description.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Learned {
    pub spec: LearnerSpec,
    pub target: String,
    /// Class labels (classification); empty for regression.
    pub classes: Vec<String>,
    pub encoder: Encoder,
    pub predictor: Predictor,
    pub train_rows: usize,
}

fn softmax(z: &[f64]) -> Vec<f64> {
    let m = z.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let e: Vec<f64> = z.iter().map(|v| (v - m).exp()).collect();
    let s: f64 = e.iter().sum::<f64>().max(1e-300);
    e.iter().map(|v| v / s).collect()
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z.clamp(-40.0, 40.0)).exp())
}

fn act(name: &str, v: f64) -> f64 {
    match name {
        "tanh" => v.tanh(),
        "logistic" => sigmoid(v),
        "identity" => v,
        _ => v.max(0.0),
    }
}

fn act_grad(name: &str, out: f64) -> f64 {
    match name {
        "tanh" => 1.0 - out * out,
        "logistic" => out * (1.0 - out),
        "identity" => 1.0,
        _ => {
            if out > 0.0 {
                1.0
            } else {
                0.0
            }
        }
    }
}

fn affine(w: &[f64], x: &[f64]) -> f64 {
    w.first().copied().unwrap_or(0.0) + w.iter().skip(1).zip(x).map(|(a, b)| a * b).sum::<f64>()
}

fn mlp_forward(layers: &[Layer], activation: &str, x: &[f64]) -> Vec<Vec<f64>> {
    let mut outs = vec![x.to_vec()];
    for (li, l) in layers.iter().enumerate() {
        let last = li + 1 == layers.len();
        let input = outs.last().cloned().unwrap_or_default();
        let o: Vec<f64> = l.w.iter().zip(&l.b).map(|(w, b)| b + w.iter().zip(&input).map(|(a, c)| a * c).sum::<f64>()).map(|v| if last { v } else { act(activation, v) }).collect();
        outs.push(o);
    }
    outs
}

impl Learned {
    pub fn is_classifier(&self) -> bool {
        !self.classes.is_empty()
    }

    pub fn name(&self) -> String {
        if self.spec.name.is_empty() { learner_label(&self.spec.kind).to_string() } else { self.spec.name.clone() }
    }

    /// Predicts a standardized row: (value or class index, class probabilities).
    pub fn predict_x(&self, x: &[f64]) -> (f64, Vec<f64>) {
        let k = self.classes.len();
        let probs: Vec<f64> = match &self.predictor {
            Predictor::Linear { intercept, coef } => return (intercept + coef.iter().zip(x).map(|(a, b)| a * b).sum::<f64>(), vec![]),
            Predictor::Logistic { w } => softmax(&w.iter().map(|wk| affine(wk, x)).collect::<Vec<_>>()),
            Predictor::Tree { nodes } => {
                let v = tree_predict(nodes, x);
                if k == 0 {
                    return (v.first().copied().unwrap_or(0.0), vec![]);
                }
                v.to_vec()
            }
            Predictor::Forest { trees } => {
                if k == 0 {
                    let s: f64 = trees.iter().map(|t| tree_predict(t, x).first().copied().unwrap_or(0.0)).sum();
                    return (s / trees.len().max(1) as f64, vec![]);
                }
                let mut acc = vec![0.0; k];
                for t in trees {
                    for (a, b) in acc.iter_mut().zip(tree_predict(t, x)) {
                        *a += b;
                    }
                }
                acc.iter().map(|v| v / trees.len().max(1) as f64).collect()
            }
            Predictor::Gbm { init, rate, stages } => {
                let mut f = init.clone();
                for st in stages {
                    for (fk, t) in f.iter_mut().zip(st) {
                        *fk += rate * tree_predict(t, x).first().copied().unwrap_or(0.0);
                    }
                }
                if k == 0 {
                    return (f.first().copied().unwrap_or(0.0), vec![]);
                }
                if k == 2 && f.len() == 1 {
                    let p = sigmoid(f[0]);
                    vec![1.0 - p, p]
                } else {
                    let p: Vec<f64> = f.iter().map(|v| sigmoid(*v)).collect();
                    let s = p.iter().sum::<f64>().max(1e-12);
                    p.iter().map(|v| v / s).collect()
                }
            }
            Predictor::AdaBoost { trees, alphas } => {
                if k == 0 {
                    // Weighted median of tree predictions.
                    let mut preds: Vec<(f64, f64)> = trees.iter().zip(alphas).map(|(t, a)| (tree_predict(t, x).first().copied().unwrap_or(0.0), *a)).collect();
                    preds.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                    let total: f64 = preds.iter().map(|p| p.1).sum();
                    let mut acc = 0.0;
                    for (v, a) in &preds {
                        acc += a;
                        if acc >= total / 2.0 {
                            return (*v, vec![]);
                        }
                    }
                    return (preds.last().map(|p| p.0).unwrap_or(0.0), vec![]);
                }
                let mut votes = vec![0.0; k];
                for (t, a) in trees.iter().zip(alphas) {
                    let v = tree_predict(t, x);
                    let best = argmax(v);
                    if let Some(s) = votes.get_mut(best) {
                        *s += a;
                    }
                }
                // SAMME scores → probabilities.
                let kk = (k as f64 - 1.0).max(1.0);
                softmax(&votes.iter().map(|v| v / kk).collect::<Vec<_>>())
            }
            Predictor::Svm { w, scale } => {
                if k == 0 {
                    return (affine(w.first().map(|v| v.as_slice()).unwrap_or(&[]), x), vec![]);
                }
                let d: Vec<f64> = w.iter().map(|wk| affine(wk, x)).collect();
                if k == 2 && d.len() == 1 {
                    let p = sigmoid(scale * d[0]);
                    vec![1.0 - p, p]
                } else {
                    softmax(&d.iter().map(|v| v * scale).collect::<Vec<_>>())
                }
            }
            Predictor::Mlp { layers, activation, y_mean, y_std } => {
                let outs = mlp_forward(layers, activation, x);
                let o = outs.last().cloned().unwrap_or_default();
                if k == 0 {
                    return (o.first().copied().unwrap_or(0.0) * y_std + y_mean, vec![]);
                }
                softmax(&o)
            }
            Predictor::NaiveBayes { prior, mean, var } => {
                let logs: Vec<f64> = (0..k)
                    .map(|c| {
                        let mut lp = prior.get(c).copied().unwrap_or(1e-9).max(1e-12).ln();
                        for (j, v) in x.iter().enumerate() {
                            let m = mean.get(c).and_then(|r| r.get(j)).copied().unwrap_or(0.0);
                            let s2 = var.get(c).and_then(|r| r.get(j)).copied().unwrap_or(1.0).max(1e-9);
                            lp += -0.5 * ((v - m).powi(2) / s2 + (2.0 * std::f64::consts::PI * s2).ln());
                        }
                        lp
                    })
                    .collect();
                softmax(&logs)
            }
            Predictor::Knn { x: xs, y, k: kk, distance_weighted } => {
                let mut d: Vec<(f64, usize)> = xs.iter().enumerate().map(|(i, p)| (p.iter().zip(x).map(|(a, b)| (a - b) * (a - b)).sum::<f64>(), i)).collect();
                let kk = (*kk).clamp(1, d.len().max(1));
                if kk < d.len() {
                    d.select_nth_unstable_by(kk - 1, |a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                }
                let near = d.get(..kk).unwrap_or(&[]);
                let wt = |dd: f64| if *distance_weighted { 1.0 / (dd.sqrt() + 1e-9) } else { 1.0 };
                if k == 0 {
                    let (s, ws) = near.iter().fold((0.0, 0.0), |acc, (dd, i)| (acc.0 + wt(*dd) * y.get(*i).copied().unwrap_or(0.0), acc.1 + wt(*dd)));
                    return (s / ws.max(1e-300), vec![]);
                }
                let mut acc = vec![0.0; k];
                for (dd, i) in near {
                    if let Some(s) = y.get(*i).and_then(|c| acc.get_mut(*c as usize)) {
                        *s += wt(*dd);
                    }
                }
                let s: f64 = acc.iter().sum::<f64>().max(1e-300);
                acc.iter().map(|v| v / s).collect()
            }
        };
        (argmax(&probs) as f64, probs)
    }

    /// Predicts every row of a table.
    pub fn predict_table(&self, t: &TableData) -> Vec<(f64, Vec<f64>)> {
        self.encoder.encode(t).iter().map(|r| self.predict_x(r)).collect()
    }

    /// Predicts a JSON object row (for serving).
    pub fn predict_json(&self, obj: &serde_json::Map<String, J>) -> J {
        let (v, p) = self.predict_x(&self.encoder.encode_json(obj));
        if self.is_classifier() {
            let label = self.classes.get(v as usize).cloned().unwrap_or_default();
            let probs: serde_json::Map<String, J> = self.classes.iter().zip(&p).map(|(c, x)| (c.clone(), serde_json::json!(x))).collect();
            serde_json::json!({ "prediction": label, "probabilities": probs })
        } else {
            serde_json::json!({ "prediction": v })
        }
    }

    /// Linear coefficients by encoded feature (standardized scale), when the model has them.
    pub fn coefficients(&self) -> Option<Vec<(String, Vec<f64>)>> {
        let names = &self.encoder.names;
        match &self.predictor {
            Predictor::Linear { intercept, coef } => Some(std::iter::once(("intercept".to_string(), vec![*intercept])).chain(names.iter().zip(coef).map(|(n, c)| (n.clone(), vec![*c]))).collect()),
            Predictor::Logistic { w } | Predictor::Svm { w, .. } => {
                let mut out = vec![("intercept".to_string(), w.iter().map(|r| r.first().copied().unwrap_or(0.0)).collect())];
                for (j, n) in names.iter().enumerate() {
                    out.push((n.clone(), w.iter().map(|r| r.get(j + 1).copied().unwrap_or(0.0)).collect()));
                }
                Some(out)
            }
            _ => None,
        }
    }

    /// One-line description of the fitted model.
    pub fn summary(&self) -> String {
        let task = if self.is_classifier() { format!("{} classes", self.classes.len()) } else { "regression".into() };
        let detail = match &self.predictor {
            Predictor::Linear { coef, .. } => format!("{} coefficients", coef.len() + 1),
            Predictor::Logistic { w } => format!("{} weight vectors", w.len()),
            Predictor::Tree { nodes } => format!("{} nodes", nodes.len()),
            Predictor::Forest { trees } => format!("{} trees", trees.len()),
            Predictor::Gbm { stages, .. } => format!("{} boosting stages", stages.len()),
            Predictor::AdaBoost { trees, .. } => format!("{} weak learners", trees.len()),
            Predictor::Svm { w, .. } => format!("{} hyperplanes", w.len()),
            Predictor::Mlp { layers, .. } => format!("layers {}", layers.iter().map(|l| l.w.len().to_string()).collect::<Vec<_>>().join("→")),
            Predictor::NaiveBayes { .. } => "Gaussian".into(),
            Predictor::Knn { x, k, .. } => format!("k = {k}, {} reference rows", x.len()),
        };
        format!("{} → {} ({task}; {detail}; {} features; trained on {} rows)", self.name(), self.target, self.encoder.dim(), self.train_rows)
    }
}

pub fn argmax(v: &[f64]) -> usize {
    v.iter().enumerate().fold((0usize, f64::NEG_INFINITY), |acc, (i, p)| if *p > acc.1 { (i, *p) } else { acc }).0
}

// =============================================================================================
// Training
// =============================================================================================

/// Target values: class indices (as f64) with their labels, or numbers.
pub struct TargetData {
    pub y: Vec<f64>,
    pub classes: Vec<String>,
    pub rows: Vec<usize>,
}

/// Sorts class labels numerically when possible.
pub fn sort_labels(v: &mut Vec<String>) {
    v.sort_by(|a, b| match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        _ => a.cmp(b),
    });
    v.dedup();
}

/// Reads the target: categorical → classes; numeric → values. Rows without a target are skipped.
pub fn target_data(t: &TableData, target: &str, classify: Option<bool>) -> Result<TargetData, String> {
    let ti = t.column_index(target).ok_or_else(|| format!("There's no target column `{target}`."))?;
    let classify = classify.unwrap_or_else(|| ml::is_categorical(t, ti));
    let rows: Vec<usize> = (0..t.rows.len()).filter(|r| !t.cell(*r, ti).is_null() && (classify || t.cell(*r, ti).as_f64().is_some_and(f64::is_finite))).collect();
    if classify {
        let labels: Vec<String> = rows.iter().map(|r| t.cell(*r, ti).text()).collect();
        let mut classes = labels.clone();
        sort_labels(&mut classes);
        if classes.len() < 2 {
            return Err(format!("`{target}` has a single class: nothing to learn."));
        }
        if classes.len() > 50 {
            return Err(format!("`{target}` has {} classes: too many for classification (use a numeric target for regression).", classes.len()));
        }
        let y = labels.iter().map(|l| classes.iter().position(|c| c == l).unwrap_or(0) as f64).collect();
        Ok(TargetData { y, classes, rows })
    } else {
        Ok(TargetData { y: rows.iter().map(|r| t.cell(*r, ti).as_f64().unwrap_or(0.0)).collect(), classes: vec![], rows })
    }
}

/// Subset of a table by row indices.
pub fn take_rows(t: &TableData, rows: &[usize]) -> TableData {
    TableData { name: t.name.clone(), columns: t.columns.clone(), types: t.types.clone(), rows: rows.iter().filter_map(|r| t.rows.get(*r).cloned()).collect() }
}

/// Fits a learner on `features` → `target` of a table.
pub fn fit(spec: &LearnerSpec, t: &TableData, features: &[String], target: &str) -> Result<Learned, String> {
    let feats: Vec<String> = features.iter().filter(|f| !f.eq_ignore_ascii_case(target)).cloned().collect();
    let classify = match spec.kind.as_str() {
        "linear" => Some(false),
        "logistic" | "naive_bayes" => Some(true),
        _ => None,
    };
    let td = target_data(t, target, classify)?;
    if td.rows.len() < 3 {
        return Err("Not enough rows with a target value to learn from.".into());
    }
    let sub = take_rows(t, &td.rows);
    let encoder = Encoder::fit(&sub, &feats)?;
    let x = encoder.encode(&sub);
    let predictor = fit_matrix(spec, &x, &td.y, td.classes.len())?;
    Ok(Learned { spec: spec.clone(), target: target.to_string(), classes: td.classes, encoder, predictor, train_rows: x.len() })
}

fn tree_params(spec: &LearnerSpec, depth: usize) -> ml::TreeParams {
    ml::TreeParams { max_depth: spec.u("max_depth", depth).clamp(1, 30), min_samples_split: spec.u("min_samples_split", 4).max(2), min_samples_leaf: spec.u("min_samples_leaf", 2).max(1), max_features: None }
}

fn ml_target(y: &[f64], k: usize) -> ml::Target {
    if k > 0 { ml::Target::Class(y.iter().map(|v| *v as usize).collect(), k) } else { ml::Target::Value(y.to_vec()) }
}

/// Fits a predictor on a standardized matrix. `k` = number of classes (0 = regression).
pub fn fit_matrix(spec: &LearnerSpec, x: &Matrix, y: &[f64], k: usize) -> Result<Predictor, String> {
    let n = x.len();
    let d = x.first().map(|r| r.len()).unwrap_or(0);
    if n == 0 || d == 0 {
        return Err("No training data.".into());
    }
    let seed = spec.u("seed", 42) as u64;
    let mut rng = Rng::new(seed);
    Ok(match spec.kind.as_str() {
        "linear" => {
            if k > 0 {
                return Err("Linear regression predicts numbers: choose a numeric target.".into());
            }
            fit_linear(x, y, &spec.s("penalty", "none"), spec.f("alpha", 1.0), spec.f("l1_ratio", 0.5))?
        }
        "logistic" => {
            if k == 0 {
                return Err("Logistic regression needs a categorical target.".into());
            }
            Predictor::Logistic { w: fit_softmax(x, y, k, 1.0 / spec.f("C", 1.0).max(1e-6), spec.u("iterations", 300)) }
        }
        "tree" => Predictor::Tree { nodes: convert_tree(&ml::fit_tree(x, &ml_target(y, k), &tree_params(spec, 8), &mut rng)) },
        "forest" => {
            let mf = match spec.s("max_features", "sqrt").as_str() {
                "all" => d,
                "log2" => (d as f64).log2().ceil() as usize,
                "third" => (d / 3).max(1),
                _ => (d as f64).sqrt().ceil() as usize,
            };
            let tp = ml::TreeParams { max_features: Some(mf.max(1)), ..tree_params(spec, 10) };
            let trees = (0..spec.u("trees", 50).clamp(1, 500))
                .map(|_| {
                    let idx: Vec<usize> = (0..n).map(|_| rng.below(n)).collect();
                    let bx: Matrix = idx.iter().map(|i| x[*i].clone()).collect();
                    let by: Vec<f64> = idx.iter().map(|i| y[*i]).collect();
                    convert_tree(&ml::fit_tree(&bx, &ml_target(&by, k), &tp, &mut rng))
                })
                .collect();
            Predictor::Forest { trees }
        }
        "gbm" => fit_gbm(spec, x, y, k, &mut rng),
        "adaboost" => fit_adaboost(spec, x, y, k, &mut rng),
        "svm" => fit_svm(spec, x, y, k, &mut rng),
        "mlp" => fit_mlp(spec, x, y, k, &mut rng),
        "naive_bayes" => {
            if k == 0 {
                return Err("Naive Bayes needs a categorical target.".into());
            }
            let mut prior: Vec<f64> = vec![0.0; k];
            let mut mean = vec![vec![0.0; d]; k];
            let mut var = vec![vec![0.0; d]; k];
            for (r, c) in x.iter().zip(y) {
                let c = (*c as usize).min(k - 1);
                prior[c] += 1.0;
                for (a, b) in mean[c].iter_mut().zip(r) {
                    *a += b;
                }
            }
            for c in 0..k {
                let cnt = prior[c].max(1.0);
                mean[c].iter_mut().for_each(|a| *a /= cnt);
            }
            for (r, c) in x.iter().zip(y) {
                let c = (*c as usize).min(k - 1);
                for ((a, b), m) in var[c].iter_mut().zip(r).zip(&mean[c]) {
                    *a += (b - m).powi(2);
                }
            }
            for c in 0..k {
                let cnt = prior[c].max(1.0);
                var[c].iter_mut().for_each(|a| *a = *a / cnt + 1e-3);
            }
            prior.iter_mut().for_each(|p| *p /= n as f64);
            Predictor::NaiveBayes { prior, mean, var }
        }
        "knn" => {
            let cap = 20_000;
            let (xs, ys) = if n > cap {
                let mut idx: Vec<usize> = (0..n).collect();
                rng.shuffle(&mut idx);
                idx.truncate(cap);
                (idx.iter().map(|i| x[*i].clone()).collect(), idx.iter().map(|i| y[*i]).collect())
            } else {
                (x.clone(), y.to_vec())
            };
            Predictor::Knn { x: xs, y: ys, k: spec.u("k", 5).max(1), distance_weighted: spec.s("weights", "uniform") == "distance" }
        }
        other => return Err(format!("Unknown learner `{other}`.")),
    })
}

/// OLS / ridge (closed form) or lasso / elastic net (coordinate descent), standardized x.
pub fn fit_linear(x: &Matrix, y: &[f64], penalty: &str, alpha: f64, l1_ratio: f64) -> Result<Predictor, String> {
    let n = x.len() as f64;
    let d = x.first().map(|r| r.len()).unwrap_or(0);
    let ym = y.iter().sum::<f64>() / n;
    let yc: Vec<f64> = y.iter().map(|v| v - ym).collect();
    let xm: Vec<f64> = (0..d).map(|j| x.iter().map(|r| r[j]).sum::<f64>() / n).collect();
    let coef = match penalty {
        "lasso" | "elastic" => {
            let l1 = if penalty == "lasso" { 1.0 } else { l1_ratio.clamp(0.0, 1.0) };
            let (a1, a2) = (alpha * l1, alpha * (1.0 - l1));
            let xc: Matrix = x.iter().map(|r| r.iter().zip(&xm).map(|(a, m)| a - m).collect()).collect();
            let z: Vec<f64> = (0..d).map(|j| xc.iter().map(|r| r[j] * r[j]).sum::<f64>() / n).collect();
            let mut w = vec![0.0; d];
            let mut resid = yc.clone();
            for _ in 0..1000 {
                let mut maxd = 0.0f64;
                for j in 0..d {
                    if z[j] <= 0.0 {
                        continue;
                    }
                    let rho: f64 = xc.iter().zip(&resid).map(|(r, e)| r[j] * (e + r[j] * w[j])).sum::<f64>() / n;
                    let nw = soft(rho, a1) / (z[j] + a2);
                    let delta = nw - w[j];
                    if delta != 0.0 {
                        for (r, e) in xc.iter().zip(resid.iter_mut()) {
                            *e -= r[j] * delta;
                        }
                        w[j] = nw;
                    }
                    maxd = maxd.max(delta.abs());
                }
                if maxd < 1e-7 {
                    break;
                }
            }
            w
        }
        _ => {
            let lambda = if penalty == "ridge" { alpha.max(0.0) * n } else { 1e-8 };
            let xc: Matrix = x.iter().map(|r| r.iter().zip(&xm).map(|(a, m)| a - m).collect()).collect();
            let (mut xtx, xty) = crate::stats::optim::normal_equations(&xc, &yc, None);
            for (i, r) in xtx.iter_mut().enumerate() {
                r[i] += lambda;
            }
            let inv = crate::stats::optim::inverse(&xtx).ok_or("The features are perfectly collinear: remove some or use ridge regression.")?;
            crate::stats::optim::mat_vec(&inv, &xty)
        }
    };
    let intercept = ym - coef.iter().zip(&xm).map(|(a, b)| a * b).sum::<f64>();
    Ok(Predictor::Linear { intercept, coef })
}

fn soft(v: f64, t: f64) -> f64 {
    if v > t {
        v - t
    } else if v < -t {
        v + t
    } else {
        0.0
    }
}

/// Multinomial logistic regression by full-batch Adam with L2 penalty `lambda`.
fn fit_softmax(x: &Matrix, y: &[f64], k: usize, lambda: f64, iters: usize) -> Matrix {
    let d = x.first().map(|r| r.len()).unwrap_or(0);
    let n = x.len() as f64;
    let mut w = vec![vec![0.0; d + 1]; k];
    let (mut m, mut v) = (vec![vec![0.0; d + 1]; k], vec![vec![0.0; d + 1]; k]);
    let (b1, b2, lr, eps) = (0.9, 0.999, 0.1, 1e-8);
    for it in 1..=iters.clamp(10, 5000) {
        let mut g = vec![vec![0.0; d + 1]; k];
        for (r, yi) in x.iter().zip(y) {
            let p = softmax(&w.iter().map(|wk| affine(wk, r)).collect::<Vec<_>>());
            for c in 0..k {
                let e = p[c] - if *yi as usize == c { 1.0 } else { 0.0 };
                g[c][0] += e;
                for j in 0..d {
                    g[c][j + 1] += e * r[j];
                }
            }
        }
        for c in 0..k {
            for j in 0..=d {
                let reg = if j == 0 { 0.0 } else { lambda * w[c][j] / n };
                let gj = g[c][j] / n + reg;
                m[c][j] = b1 * m[c][j] + (1.0 - b1) * gj;
                v[c][j] = b2 * v[c][j] + (1.0 - b2) * gj * gj;
                let mh = m[c][j] / (1.0 - b1.powi(it as i32));
                let vh = v[c][j] / (1.0 - b2.powi(it as i32));
                w[c][j] -= lr * mh / (vh.sqrt() + eps);
            }
        }
    }
    w
}

fn fit_gbm(spec: &LearnerSpec, x: &Matrix, y: &[f64], k: usize, rng: &mut Rng) -> Predictor {
    let n = x.len();
    let m = spec.u("trees", 100).clamp(1, 1000);
    let rate = spec.f("learning_rate", 0.1).clamp(1e-4, 1.0);
    let subsample = spec.f("subsample", 1.0).clamp(0.1, 1.0);
    let tp = ml::TreeParams { max_depth: spec.u("max_depth", 3).clamp(1, 12), min_samples_split: 4, min_samples_leaf: spec.u("min_samples_leaf", 3).max(1), max_features: None };
    // Outputs: regression 1; binary 1 (log-odds of class 1); multiclass k (one-vs-rest).
    let outs = if k == 0 || k == 2 { 1 } else { k };
    let targets: Vec<Vec<f64>> = (0..outs)
        .map(|o| match k {
            0 => y.to_vec(),
            2 => y.iter().map(|v| if *v as usize == 1 { 1.0 } else { 0.0 }).collect(),
            _ => y.iter().map(|v| if *v as usize == o { 1.0 } else { 0.0 }).collect(),
        })
        .collect();
    let init: Vec<f64> = targets
        .iter()
        .map(|t| {
            let mean = t.iter().sum::<f64>() / n.max(1) as f64;
            if k == 0 { mean } else { (mean.clamp(1e-6, 1.0 - 1e-6) / (1.0 - mean.clamp(1e-6, 1.0 - 1e-6))).ln() }
        })
        .collect();
    let mut f: Vec<Vec<f64>> = init.iter().map(|v| vec![*v; n]).collect();
    let mut stages = vec![];
    for _ in 0..m {
        let rows: Vec<usize> = if subsample < 1.0 { (0..n).filter(|_| rng.f64() < subsample).collect() } else { (0..n).collect() };
        let rows = if rows.len() < 4 { (0..n).collect() } else { rows };
        let mut stage = vec![];
        for o in 0..outs {
            let resid: Vec<f64> = rows.iter().map(|i| if k == 0 { targets[o][*i] - f[o][*i] } else { targets[o][*i] - sigmoid(f[o][*i]) }).collect();
            let bx: Matrix = rows.iter().map(|i| x[*i].clone()).collect();
            let tree = ml::fit_tree(&bx, &ml::Target::Value(resid.clone()), &tp, rng);
            let mut nodes = convert_tree(&tree);
            if k > 0 {
                // Newton leaf values: Σr / Σp(1−p).
                let mut num: BTreeMap<usize, (f64, f64)> = BTreeMap::new();
                for (j, i) in rows.iter().enumerate() {
                    let leaf = leaf_index(&nodes, &x[*i]);
                    let p = sigmoid(f[o][*i]);
                    let e = num.entry(leaf).or_default();
                    e.0 += resid[j];
                    e.1 += p * (1.0 - p);
                }
                for (leaf, (s, h)) in num {
                    if let Some(SNode::Leaf { v }) = nodes.get_mut(leaf) {
                        *v = vec![(s / h.max(1e-9)).clamp(-8.0, 8.0)];
                    }
                }
            }
            for i in 0..n {
                f[o][i] += rate * tree_predict(&nodes, &x[i]).first().copied().unwrap_or(0.0);
            }
            stage.push(nodes);
        }
        stages.push(stage);
    }
    Predictor::Gbm { init, rate, stages }
}

fn weighted_sample(w: &[f64], n: usize, rng: &mut Rng) -> Vec<usize> {
    let total: f64 = w.iter().sum();
    let mut cum = Vec::with_capacity(w.len());
    let mut acc = 0.0;
    for v in w {
        acc += v / total;
        cum.push(acc);
    }
    (0..n)
        .map(|_| {
            let u = rng.f64();
            cum.partition_point(|c| *c < u).min(w.len() - 1)
        })
        .collect()
}

fn fit_adaboost(spec: &LearnerSpec, x: &Matrix, y: &[f64], k: usize, rng: &mut Rng) -> Predictor {
    let n = x.len();
    let m = spec.u("trees", 50).clamp(1, 500);
    let rate = spec.f("learning_rate", 1.0).clamp(1e-3, 2.0);
    let tp = ml::TreeParams { max_depth: spec.u("max_depth", if k > 0 { 1 } else { 3 }).clamp(1, 10), min_samples_split: 2, min_samples_leaf: 1, max_features: None };
    let mut w = vec![1.0 / n as f64; n];
    let mut trees = vec![];
    let mut alphas = vec![];
    for _ in 0..m {
        let idx = weighted_sample(&w, n, rng);
        let bx: Matrix = idx.iter().map(|i| x[*i].clone()).collect();
        let by: Vec<f64> = idx.iter().map(|i| y[*i]).collect();
        let nodes = convert_tree(&ml::fit_tree(&bx, &ml_target(&by, k), &tp, rng));
        if k > 0 {
            let miss: Vec<bool> = (0..n).map(|i| argmax(tree_predict(&nodes, &x[i])) != y[i] as usize).collect();
            let err: f64 = miss.iter().zip(&w).filter(|(m, _)| **m).map(|(_, w)| w).sum::<f64>() / w.iter().sum::<f64>();
            if err >= 1.0 - 1.0 / k as f64 {
                if trees.is_empty() {
                    trees.push(nodes);
                    alphas.push(1.0);
                }
                break;
            }
            let err = err.max(1e-10);
            let alpha = rate * (((1.0 - err) / err).ln() + (k as f64 - 1.0).ln());
            for (wi, m) in w.iter_mut().zip(&miss) {
                if *m {
                    *wi *= alpha.exp();
                }
            }
            let s: f64 = w.iter().sum();
            w.iter_mut().for_each(|v| *v /= s);
            trees.push(nodes);
            alphas.push(alpha);
            if err <= 1e-10 {
                break;
            }
        } else {
            // AdaBoost.R2 with linear loss.
            let errs: Vec<f64> = (0..n).map(|i| (tree_predict(&nodes, &x[i]).first().copied().unwrap_or(0.0) - y[i]).abs()).collect();
            let maxe = errs.iter().copied().fold(0.0, f64::max);
            if maxe <= 0.0 {
                trees.push(nodes);
                alphas.push(1.0);
                break;
            }
            let l: Vec<f64> = errs.iter().map(|e| e / maxe).collect();
            let lbar: f64 = l.iter().zip(&w).map(|(a, b)| a * b).sum();
            if lbar >= 0.5 {
                if trees.is_empty() {
                    trees.push(nodes);
                    alphas.push(1.0);
                }
                break;
            }
            let beta = lbar / (1.0 - lbar);
            for (wi, li) in w.iter_mut().zip(&l) {
                *wi *= beta.powf(rate * (1.0 - li));
            }
            let s: f64 = w.iter().sum();
            w.iter_mut().for_each(|v| *v /= s);
            trees.push(nodes);
            alphas.push(rate * (1.0 / beta.max(1e-12)).ln());
        }
    }
    Predictor::AdaBoost { trees, alphas }
}

fn fit_svm(spec: &LearnerSpec, x: &Matrix, y: &[f64], k: usize, rng: &mut Rng) -> Predictor {
    let n = x.len();
    let d = x.first().map(|r| r.len()).unwrap_or(0);
    let c = spec.f("C", 1.0).max(1e-6);
    let lambda = 1.0 / (c * n as f64);
    let epochs = spec.u("iterations", 40).clamp(1, 1000);
    let eps = spec.f("epsilon", 0.1);
    let outs = if k == 0 || k == 2 { 1 } else { k };
    let mut ws = vec![];
    let ym = if k == 0 { y.iter().sum::<f64>() / n as f64 } else { 0.0 };
    let ys = if k == 0 { (y.iter().map(|v| (v - ym).powi(2)).sum::<f64>() / n as f64).sqrt().max(1e-12) } else { 1.0 };
    for o in 0..outs {
        let target = |i: usize| -> f64 {
            match k {
                0 => (y[i] - ym) / ys,
                2 => {
                    if y[i] as usize == 1 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                _ => {
                    if y[i] as usize == o {
                        1.0
                    } else {
                        -1.0
                    }
                }
            }
        };
        let mut w = vec![0.0; d + 1];
        let mut avg = vec![0.0; d + 1];
        let mut t = 0usize;
        let mut order: Vec<usize> = (0..n).collect();
        for _ in 0..epochs {
            rng.shuffle(&mut order);
            for &i in &order {
                t += 1;
                let eta = 1.0 / (lambda * (t as f64 + 10.0));
                let s = affine(&w, &x[i]);
                let yi = target(i);
                let g = if k == 0 {
                    let r = s - yi;
                    if r > eps {
                        1.0
                    } else if r < -eps {
                        -1.0
                    } else {
                        0.0
                    }
                } else if yi * s < 1.0 {
                    -yi
                } else {
                    0.0
                };
                for j in 1..=d {
                    w[j] -= eta * (lambda * w[j] + g * x[i][j - 1]);
                }
                w[0] -= eta * g * 0.1;
                for j in 0..=d {
                    avg[j] += (w[j] - avg[j]) / t as f64;
                }
            }
        }
        if k == 0 {
            // Back to the original target scale.
            avg = avg.iter().enumerate().map(|(j, v)| if j == 0 { v * ys + ym } else { v * ys }).collect();
        }
        ws.push(avg);
    }
    Predictor::Svm { w: ws, scale: 2.0 }
}

fn fit_mlp(spec: &LearnerSpec, x: &Matrix, y: &[f64], k: usize, rng: &mut Rng) -> Predictor {
    let n = x.len();
    let d = x.first().map(|r| r.len()).unwrap_or(0);
    let hidden: Vec<usize> = spec.s("hidden", "32").split([',', ' ']).filter_map(|s| s.trim().parse::<usize>().ok()).filter(|v| *v > 0).take(2).map(|v| v.min(512)).collect();
    let hidden = if hidden.is_empty() { vec![32] } else { hidden };
    let activation = spec.s("activation", "relu");
    let alpha = spec.f("alpha", 1e-4);
    let lr = spec.f("learning_rate", 0.01).clamp(1e-5, 1.0);
    let epochs = spec.u("iterations", 200).clamp(1, 5000);
    let outs = if k == 0 { 1 } else { k };
    let ym = if k == 0 { y.iter().sum::<f64>() / n as f64 } else { 0.0 };
    let ys = if k == 0 { (y.iter().map(|v| (v - ym).powi(2)).sum::<f64>() / n as f64).sqrt().max(1e-12) } else { 1.0 };
    let mut sizes = vec![d];
    sizes.extend(hidden);
    sizes.push(outs);
    let mut layers: Vec<Layer> = sizes
        .windows(2)
        .map(|s| {
            let bound = (6.0 / (s[0] + s[1]) as f64).sqrt();
            Layer { w: (0..s[1]).map(|_| (0..s[0]).map(|_| (rng.f64() * 2.0 - 1.0) * bound).collect()).collect(), b: vec![0.0; s[1]] }
        })
        .collect();
    // Adam state.
    let mut mw: Vec<Matrix> = layers.iter().map(|l| l.w.iter().map(|r| vec![0.0; r.len()]).collect()).collect();
    let mut vw = mw.clone();
    let mut mb: Vec<Vec<f64>> = layers.iter().map(|l| vec![0.0; l.b.len()]).collect();
    let mut vb = mb.clone();
    let (b1, b2, eps) = (0.9, 0.999, 1e-8);
    let batch = 32usize.min(n).max(1);
    let mut order: Vec<usize> = (0..n).collect();
    let mut step = 0i32;
    for _ in 0..epochs {
        rng.shuffle(&mut order);
        for chunk in order.chunks(batch) {
            let mut gw: Vec<Matrix> = layers.iter().map(|l| l.w.iter().map(|r| vec![0.0; r.len()]).collect()).collect();
            let mut gb: Vec<Vec<f64>> = layers.iter().map(|l| vec![0.0; l.b.len()]).collect();
            for &i in chunk {
                let a = mlp_forward(&layers, &activation, &x[i]);
                let out = a.last().cloned().unwrap_or_default();
                // Output delta.
                let mut delta: Vec<f64> = if k == 0 {
                    vec![out[0] - (y[i] - ym) / ys]
                } else {
                    let p = softmax(&out);
                    p.iter().enumerate().map(|(c, pc)| pc - if y[i] as usize == c { 1.0 } else { 0.0 }).collect()
                };
                for li in (0..layers.len()).rev() {
                    let input = &a[li];
                    for (o, dl) in delta.iter().enumerate() {
                        gb[li][o] += dl;
                        for (j, inp) in input.iter().enumerate() {
                            gw[li][o][j] += dl * inp;
                        }
                    }
                    if li > 0 {
                        let prev = &a[li];
                        delta = (0..prev.len()).map(|j| layers[li].w.iter().zip(&delta).map(|(row, dl)| row[j] * dl).sum::<f64>() * act_grad(&activation, prev[j])).collect();
                    }
                }
            }
            step += 1;
            let bs = chunk.len() as f64;
            for li in 0..layers.len() {
                for o in 0..layers[li].b.len() {
                    let g = gb[li][o] / bs;
                    mb[li][o] = b1 * mb[li][o] + (1.0 - b1) * g;
                    vb[li][o] = b2 * vb[li][o] + (1.0 - b2) * g * g;
                    layers[li].b[o] -= lr * (mb[li][o] / (1.0 - b1.powi(step))) / ((vb[li][o] / (1.0 - b2.powi(step))).sqrt() + eps);
                    for j in 0..layers[li].w[o].len() {
                        let g = gw[li][o][j] / bs + alpha * layers[li].w[o][j];
                        mw[li][o][j] = b1 * mw[li][o][j] + (1.0 - b1) * g;
                        vw[li][o][j] = b2 * vw[li][o][j] + (1.0 - b2) * g * g;
                        layers[li].w[o][j] -= lr * (mw[li][o][j] / (1.0 - b1.powi(step))) / ((vw[li][o][j] / (1.0 - b2.powi(step))).sqrt() + eps);
                    }
                }
            }
        }
    }
    Predictor::Mlp { layers, activation, y_mean: ym, y_std: ys }
}

/// Polynomial feature expansion (degree 2 or 3, with interactions) of numeric columns.
pub fn polynomial(t: &TableData, columns: &[String], degree: usize, interactions: bool) -> TableData {
    let mut out = t.clone();
    let idx: Vec<(String, usize)> = columns.iter().filter_map(|c| t.column_index(c).map(|i| (c.clone(), i))).collect();
    let val = |r: &[Scalar], i: usize| r.get(i).and_then(Scalar::as_f64);
    let mut add = |name: String, f: &dyn Fn(&[Scalar]) -> Option<f64>| {
        if out.column_index(&name).is_some() {
            return;
        }
        out.columns.push(name);
        out.types.push(crate::build::ColType::Number);
        for (row, src) in out.rows.iter_mut().zip(&t.rows) {
            row.push(f(src).map(Scalar::Num).unwrap_or(Scalar::Null));
        }
    };
    for p in 2..=degree.clamp(2, 3) {
        for (name, i) in &idx {
            let i = *i;
            add(format!("{name}^{p}"), &|r| val(r, i).map(|v| v.powi(p as i32)));
        }
    }
    if interactions {
        for a in 0..idx.len() {
            for b in a + 1..idx.len() {
                let (ia, ib) = (idx[a].1, idx[b].1);
                add(format!("{}×{}", idx[a].0, idx[b].0), &|r| Some(val(r, ia)? * val(r, ib)?));
            }
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests_support {
    use super::*;
    use crate::build::ColType;

    /// Two noisy blobs (classification) plus a numeric target y = 3a − 2b + noise.
    pub fn sample(n: usize) -> TableData {
        let mut rng = Rng::new(11);
        let mut rows = vec![];
        for i in 0..n {
            let cls = i % 2;
            let a = rng.f64() * 2.0 + cls as f64 * 1.6;
            let b = rng.f64() * 2.0 - cls as f64 * 0.8;
            let cat = if rng.f64() < 0.5 { "u" } else { "v" };
            let y = 3.0 * a - 2.0 * b + if cat == "u" { 1.0 } else { 0.0 } + (rng.f64() - 0.5) * 0.2;
            rows.push(vec![Scalar::Num(a), Scalar::Num(b), Scalar::Text(cat.into()), Scalar::Text(if cls == 1 { "yes".into() } else { "no".into() }), Scalar::Num(y)]);
        }
        TableData { name: "s".into(), columns: vec!["a".into(), "b".into(), "c".into(), "label".into(), "y".into()], types: vec![ColType::Number, ColType::Number, ColType::Text, ColType::Text, ColType::Number], rows }
    }

}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use super::tests_support::sample;

    fn spec(kind: &str) -> LearnerSpec {
        LearnerSpec { kind: kind.into(), name: String::new(), params: BTreeMap::new() }
    }

    fn accuracy(m: &Learned, t: &TableData) -> f64 {
        let p = m.predict_table(t);
        let li = t.column_index("label").unwrap();
        p.iter().zip(&t.rows).filter(|(p, r)| m.classes[p.0 as usize] == r[li].text()).count() as f64 / t.rows.len() as f64
    }

    #[test]
    fn classifiers_learn() {
        let t = sample(300);
        let feats = vec!["a".to_string(), "b".into(), "c".into()];
        for k in ["logistic", "tree", "forest", "gbm", "adaboost", "svm", "mlp", "naive_bayes", "knn"] {
            let m = fit(&spec(k), &t, &feats, "label").unwrap();
            let acc = accuracy(&m, &t);
            assert!(acc > 0.8, "{k}: {acc}");
            // JSON round trip predicts the same.
            let j = serde_json::to_string(&m).unwrap();
            let m2: Learned = serde_json::from_str(&j).unwrap();
            assert_eq!(m.predict_table(&t), m2.predict_table(&t));
        }
    }

    #[test]
    fn regressors_learn() {
        let t = sample(300);
        let feats = vec!["a".to_string(), "b".into(), "c".into()];
        let yi = t.column_index("y").unwrap();
        for k in ["linear", "tree", "forest", "gbm", "adaboost", "svm", "mlp", "knn"] {
            let m = fit(&spec(k), &t, &feats, "y").unwrap();
            let p: Vec<f64> = m.predict_table(&t).iter().map(|p| p.0).collect();
            let y: Vec<f64> = t.rows.iter().map(|r| r[yi].as_f64().unwrap()).collect();
            let r2 = ml::regression_metrics(&y, &p).r2;
            assert!(r2 > 0.8, "{k}: {r2}");
        }
    }

    #[test]
    fn linear_recovers_coefficients_and_lasso_shrinks() {
        let t = sample(400);
        let m = fit(&spec("linear"), &t, &["a".into(), "b".into(), "c".into()], "y").unwrap();
        let Predictor::Linear { coef, .. } = &m.predictor else { panic!() };
        // Raw coefficient = standardized / std.
        let raw_a = coef[0] / m.encoder.std[0];
        let raw_b = coef[1] / m.encoder.std[1];
        assert!((raw_a - 3.0).abs() < 0.05, "{raw_a}");
        assert!((raw_b + 2.0).abs() < 0.05, "{raw_b}");
        let mut s = spec("linear");
        s.params.insert("penalty".into(), J::from("lasso"));
        s.params.insert("alpha".into(), J::from(100.0));
        let l = fit(&s, &t, &["a".into(), "b".into(), "c".into()], "y").unwrap();
        let Predictor::Linear { coef, .. } = &l.predictor else { panic!() };
        assert!(coef.iter().all(|c| c.abs() < 1e-9), "{coef:?}");
    }

    #[test]
    fn polynomial_columns() {
        let t = sample(10);
        let p = polynomial(&t, &["a".into(), "b".into()], 2, true);
        assert!(p.column_index("a^2").is_some() && p.column_index("a×b").is_some());
    }
}
