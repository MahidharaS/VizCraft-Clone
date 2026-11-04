//! The model registry record ([`RegisteredModel`]), training-data fingerprints, data-drift
//! monitoring (PSI and Kolmogorov–Smirnov per feature, prediction drift), batch scoring and
//! model export (JSON artifact and a standalone Python scoring script).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as J;

use super::learn::{FeatureEnc, Learned, Predictor};
use crate::build::{ColType, TableData};
use crate::scalar::Scalar;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage {
    #[default]
    None,
    Staging,
    Production,
    Archived,
}

impl Stage {
    pub const ALL: [Stage; 4] = [Stage::None, Stage::Staging, Stage::Production, Stage::Archived];
    pub fn label(self) -> &'static str {
        match self {
            Stage::None => "None",
            Stage::Staging => "Staging",
            Stage::Production => "Production",
            Stage::Archived => "Archived",
        }
    }
}

/// A feature of the scoring schema.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FeatureSchema {
    pub name: String,
    /// "number" or "category".
    pub dtype: String,
    pub levels: Vec<String>,
}

/// Summary of one training column (for drift checks).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ColumnProfile {
    pub name: String,
    pub numeric: bool,
    /// Decile cut points (numeric).
    pub edges: Vec<f64>,
    /// Fraction of rows per bin (edges.len() + 1 bins) or per category.
    pub fracs: Vec<f64>,
    pub categories: Vec<String>,
    pub mean: f64,
    pub std: f64,
    pub missing: f64,
    /// 101 quantiles (0 %, 1 %, … 100 %) for KS comparisons.
    pub quantiles: Vec<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Fingerprint {
    pub rows: usize,
    /// FNV-1a hash of the training data.
    pub hash: String,
    pub columns: Vec<ColumnProfile>,
    /// Prediction distribution on the training data (class fractions, or a profile of values).
    pub predictions: ColumnProfile,
}

/// A registered model version.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RegisteredModel {
    pub name: String,
    pub version: u32,
    pub algorithm: String,
    pub params: BTreeMap<String, J>,
    pub features: Vec<FeatureSchema>,
    pub target: String,
    /// "classification" or "regression".
    pub task: String,
    pub classes: Vec<String>,
    pub fingerprint: Fingerprint,
    pub metrics: BTreeMap<String, f64>,
    /// Creation time (UTC, "YYYY-MM-DD HH:MM:SS").
    pub created: String,
    pub stage: Stage,
    pub description: String,
    /// Where it was trained ("workflow › widget").
    pub source: String,
    pub model: Learned,
}

impl Default for Learned {
    fn default() -> Self {
        Learned { spec: Default::default(), target: String::new(), classes: vec![], encoder: Default::default(), predictor: Predictor::Linear { intercept: 0.0, coef: vec![] }, train_rows: 0 }
    }
}

pub fn fnv(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

pub fn data_hash(t: &TableData) -> String {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for c in &t.columns {
        h = fnv(c.as_bytes(), h);
    }
    for r in &t.rows {
        for v in r {
            h = fnv(v.text().as_bytes(), h);
            h = fnv(&[0x1f], h);
        }
    }
    format!("{h:016x}")
}

fn numeric_profile(name: &str, vals: &[f64], total: usize) -> ColumnProfile {
    let s = crate::stats::sorted(vals);
    let edges: Vec<f64> = (1..10).map(|i| crate::stats::quantile(&s, i as f64 / 10.0)).collect();
    let mut edges_d = edges.clone();
    edges_d.dedup();
    let mut counts = vec![0.0; edges_d.len() + 1];
    for v in &s {
        counts[edges_d.partition_point(|e| e < v)] += 1.0;
    }
    let n = s.len().max(1) as f64;
    let d = crate::stats::describe(&s);
    ColumnProfile {
        name: name.into(),
        numeric: true,
        fracs: counts.iter().map(|c| c / n).collect(),
        edges: edges_d,
        categories: vec![],
        mean: d.mean,
        std: d.std,
        missing: 1.0 - s.len() as f64 / total.max(1) as f64,
        quantiles: (0..=100).map(|i| crate::stats::quantile(&s, i as f64 / 100.0)).collect(),
    }
}

fn categorical_profile(name: &str, vals: &[Option<String>]) -> ColumnProfile {
    let mut counts: BTreeMap<String, f64> = BTreeMap::new();
    let mut present: f64 = 0.0;
    for v in vals.iter().flatten() {
        *counts.entry(v.clone()).or_default() += 1.0;
        present += 1.0;
    }
    ColumnProfile {
        name: name.into(),
        numeric: false,
        fracs: counts.values().map(|c| c / present.max(1.0)).collect(),
        categories: counts.keys().cloned().collect(),
        missing: 1.0 - present / vals.len().max(1) as f64,
        ..Default::default()
    }
}

fn profile(t: &TableData, f: &FeatureEnc) -> ColumnProfile {
    let c = t.column_index(f.name());
    match f {
        FeatureEnc::Numeric { name, .. } => {
            let vals: Vec<f64> = t.rows.iter().filter_map(|r| c.and_then(|c| r.get(c)).and_then(Scalar::as_f64)).filter(|v| v.is_finite()).collect();
            numeric_profile(name, &vals, t.rows.len())
        }
        FeatureEnc::Categorical { name, .. } => {
            let vals: Vec<Option<String>> = t.rows.iter().map(|r| c.and_then(|c| r.get(c)).filter(|v| !v.is_null()).map(Scalar::text)).collect();
            categorical_profile(name, &vals)
        }
    }
}

fn prediction_profile(m: &Learned, t: &TableData) -> ColumnProfile {
    let p = m.predict_table(t);
    if m.is_classifier() {
        let labels: Vec<Option<String>> = p.iter().map(|x| m.classes.get(x.0 as usize).cloned()).collect();
        categorical_profile("prediction", &labels)
    } else {
        numeric_profile("prediction", &p.iter().map(|x| x.0).collect::<Vec<_>>(), t.rows.len())
    }
}

pub fn fingerprint(m: &Learned, t: &TableData) -> Fingerprint {
    Fingerprint { rows: t.rows.len(), hash: data_hash(t), columns: m.encoder.features.iter().map(|f| profile(t, f)).collect(), predictions: prediction_profile(m, t) }
}

pub fn schema(m: &Learned) -> Vec<FeatureSchema> {
    m.encoder
        .features
        .iter()
        .map(|f| match f {
            FeatureEnc::Numeric { name, .. } => FeatureSchema { name: name.clone(), dtype: "number".into(), levels: vec![] },
            FeatureEnc::Categorical { name, levels } => FeatureSchema { name: name.clone(), dtype: "category".into(), levels: levels.clone() },
        })
        .collect()
}

pub fn now_utc() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let date = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).and_then(|d| d.checked_add_days(chrono::Days::new(days.max(0) as u64)));
    match date {
        Some(d) => format!("{} {:02}:{:02}:{:02}", d.format("%Y-%m-%d"), rem / 3600, rem % 3600 / 60, rem % 60),
        None => String::new(),
    }
}

/// Builds a registry record (version 0: the registry assigns the next version).
pub fn register(name: &str, m: &Learned, train: &TableData, metrics: BTreeMap<String, f64>, source: &str, description: &str) -> RegisteredModel {
    RegisteredModel {
        name: name.to_string(),
        version: 0,
        algorithm: m.name(),
        params: m.spec.params.clone(),
        features: schema(m),
        target: m.target.clone(),
        task: if m.is_classifier() { "classification".into() } else { "regression".into() },
        classes: m.classes.clone(),
        fingerprint: fingerprint(m, train),
        metrics,
        created: now_utc(),
        stage: Stage::None,
        description: description.to_string(),
        source: source.to_string(),
        model: m.clone(),
    }
}

/// Next version number for `name` among `existing`.
pub fn next_version(existing: &[RegisteredModel], name: &str) -> u32 {
    existing.iter().filter(|m| m.name == name).map(|m| m.version).max().unwrap_or(0) + 1
}

/// The version to serve: Production, else Staging, else the latest.
pub fn serving_version<'a>(models: &'a [RegisteredModel], name: &str) -> Option<&'a RegisteredModel> {
    let of: Vec<&RegisteredModel> = models.iter().filter(|m| m.name == name && m.stage != Stage::Archived).collect();
    of.iter().filter(|m| m.stage == Stage::Production).max_by_key(|m| m.version).or_else(|| of.iter().filter(|m| m.stage == Stage::Staging).max_by_key(|m| m.version)).or_else(|| of.iter().max_by_key(|m| m.version)).copied()
}

// =============================================================================================
// Scoring
// =============================================================================================

/// Appends prediction columns (and class probabilities) to a table.
pub fn score_table(m: &Learned, t: &TableData, column: &str, probabilities: bool) -> TableData {
    let mut out = t.clone();
    let p = m.predict_table(t);
    let col = if out.column_index(column).is_some() { format!("{column} (2)") } else { column.to_string() };
    out.columns.push(col.clone());
    out.types.push(if m.is_classifier() { ColType::Text } else { ColType::Number });
    for (r, (v, _)) in out.rows.iter_mut().zip(&p) {
        r.push(if m.is_classifier() { m.classes.get(*v as usize).map(|c| Scalar::Text(c.clone())).unwrap_or(Scalar::Null) } else if v.is_finite() { Scalar::Num(*v) } else { Scalar::Null });
    }
    if probabilities && m.is_classifier() {
        for (k, c) in m.classes.iter().enumerate() {
            out.columns.push(format!("{col} P({c})"));
            out.types.push(ColType::Number);
            for (r, (_, pr)) in out.rows.iter_mut().zip(&p) {
                r.push(pr.get(k).map(|v| Scalar::Num(*v)).unwrap_or(Scalar::Null));
            }
        }
    }
    out
}

// =============================================================================================
// Drift
// =============================================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DriftRow {
    pub feature: String,
    pub psi: f64,
    pub ks: f64,
    pub ks_p: f64,
    pub train_mean: f64,
    pub new_mean: f64,
    pub missing_train: f64,
    pub missing_new: f64,
    /// "Stable", "Moderate drift", "Significant drift".
    pub status: String,
    /// Expected vs actual bin fractions (for charts).
    pub expected: Vec<f64>,
    pub actual: Vec<f64>,
    pub bins: Vec<String>,
}

pub fn psi(expected: &[f64], actual: &[f64]) -> f64 {
    expected.iter().zip(actual).map(|(e, a)| {
        let (e, a) = (e.max(1e-4), a.max(1e-4));
        (a - e) * (a / e).ln()
    }).sum()
}

fn status(psi: f64) -> String {
    if psi < 0.1 {
        "Stable".into()
    } else if psi < 0.25 {
        "Moderate drift".into()
    } else {
        "Significant drift".into()
    }
}

fn compare(p: &ColumnProfile, cur: &ColumnProfile) -> DriftRow {
    if p.numeric {
        let mut counts = vec![0.0; p.edges.len() + 1];
        // Recover current values from its quantiles is lossy; use its fractions on the training edges.
        let vals = &cur.quantiles;
        for v in vals {
            counts[p.edges.partition_point(|e| e < v)] += 1.0;
        }
        let n = vals.len().max(1) as f64;
        let actual: Vec<f64> = counts.iter().map(|c| c / n).collect();
        let ks = crate::stats::hypothesis::ks_two_sample(&p.quantiles, &cur.quantiles);
        let mut bins = vec![];
        let mut lo = "−∞".to_string();
        for e in &p.edges {
            bins.push(format!("{lo} … {}", crate::stats::fmt(*e)));
            lo = crate::stats::fmt(*e);
        }
        bins.push(format!("{lo} … ∞"));
        let psi_v = psi(&p.fracs, &actual);
        DriftRow { feature: p.name.clone(), psi: psi_v, ks: ks.statistic, ks_p: f64::NAN, train_mean: p.mean, new_mean: cur.mean, missing_train: p.missing, missing_new: cur.missing, status: status(psi_v), expected: p.fracs.clone(), actual, bins }
    } else {
        let mut cats = p.categories.clone();
        for c in &cur.categories {
            if !cats.contains(c) {
                cats.push(c.clone());
            }
        }
        let get = |pr: &ColumnProfile, c: &str| pr.categories.iter().position(|x| x == c).and_then(|i| pr.fracs.get(i)).copied().unwrap_or(0.0);
        let expected: Vec<f64> = cats.iter().map(|c| get(p, c)).collect();
        let actual: Vec<f64> = cats.iter().map(|c| get(cur, c)).collect();
        let psi_v = psi(&expected, &actual);
        // Total variation distance as the "KS" analogue for categories.
        let tv = expected.iter().zip(&actual).map(|(a, b)| (a - b).abs()).sum::<f64>() / 2.0;
        DriftRow { feature: p.name.clone(), psi: psi_v, ks: tv, ks_p: f64::NAN, train_mean: f64::NAN, new_mean: f64::NAN, missing_train: p.missing, missing_new: cur.missing, status: status(psi_v), expected, actual, bins: cats }
    }
}

/// Compares new data against the training fingerprint: one row per feature plus "prediction".
pub fn drift(m: &RegisteredModel, t: &TableData) -> Vec<DriftRow> {
    let mut out = vec![];
    for (f, p) in m.model.encoder.features.iter().zip(&m.fingerprint.columns) {
        let cur = profile(t, f);
        let mut row = compare(p, &cur);
        if p.numeric {
            // Exact KS p-value against the stored quantiles, using the full new column.
            if let Some(c) = t.column_index(&p.name) {
                let vals: Vec<f64> = t.rows.iter().filter_map(|r| r.get(c).and_then(Scalar::as_f64)).filter(|v| v.is_finite()).collect();
                let ks = crate::stats::hypothesis::ks_two_sample(&p.quantiles, &vals);
                let n_train = m.fingerprint.rows.max(1) as f64;
                let n_new = vals.len().max(1) as f64;
                row.ks = ks.statistic;
                row.ks_p = crate::stats::special::ks_pvalue(ks.statistic, n_train * n_new / (n_train + n_new));
                let mut counts = vec![0.0; p.edges.len() + 1];
                for v in &vals {
                    counts[p.edges.partition_point(|e| e < v)] += 1.0;
                }
                row.actual = counts.iter().map(|c| c / n_new).collect();
                row.psi = psi(&p.fracs, &row.actual);
                row.status = status(row.psi);
            }
        }
        out.push(row);
    }
    let cur = prediction_profile(&m.model, t);
    let mut row = compare(&m.fingerprint.predictions, &cur);
    row.feature = "▶ prediction".into();
    out.push(row);
    out
}

// =============================================================================================
// Export
// =============================================================================================

/// JSON artifact (the registry format).
pub fn to_json(m: &RegisteredModel) -> String {
    serde_json::to_string_pretty(m).unwrap_or_else(|_| "{}".into())
}

/// A standalone Python 3 scoring script (standard library only) that reproduces the model's
/// predictions: `python3 score.py input.csv > predictions.csv`.
pub fn python_script(m: &RegisteredModel) -> String {
    let payload = serde_json::json!({
        "name": m.name, "version": m.version, "target": m.target, "classes": m.model.classes,
        "encoder": m.model.encoder, "predictor": m.model.predictor,
    });
    let json = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".into()).replace("'''", "\\u0027\\u0027\\u0027");
    format!(
        r#"#!/usr/bin/env python3
"""Scoring script for the VizardCraft model "{name}" v{version} ({algo}).

Generated by VizardCraft Data Science Studio. Standard library only.

    python3 {file}.py input.csv > predictions.csv
    python3 -c "import {file}; print({file}.predict({{'feature': 1.0}}))"
"""
import csv, json, math, sys

MODEL = json.loads(r'''{json}''')
ENC = MODEL["encoder"]
P = MODEL["predictor"]
CLASSES = MODEL["classes"]


def _num(v):
    try:
        f = float(v)
        return f if math.isfinite(f) else None
    except (TypeError, ValueError):
        return None


def encode(row):
    """Raw feature dict -> standardized vector (mean imputation, one-hot categories)."""
    out = []
    for f in ENC["features"]:
        v = row.get(f["name"])
        if f["type"] == "numeric":
            x = _num(v)
            out.append(f["fill"] if x is None else x)
        else:
            s = None if v is None or v == "" else str(v)
            out.extend(1.0 if s == lvl else 0.0 for lvl in f["levels"])
    return [(x - m) / (s if s > 0 else 1.0) for x, m, s in zip(out, ENC["mean"], ENC["std"])]


def _affine(w, x):
    return w[0] + sum(a * b for a, b in zip(w[1:], x))


def _softmax(z):
    m = max(z)
    e = [math.exp(v - m) for v in z]
    s = sum(e) or 1e-300
    return [v / s for v in e]


def _sigmoid(z):
    z = max(-40.0, min(40.0, z))
    return 1.0 / (1.0 + math.exp(-z))


def _tree(nodes, x):
    i = 0
    while True:
        n = nodes[i]
        if n["n"] == "leaf":
            return n["v"]
        i = n["l"] if (x[n["f"]] if n["f"] < len(x) else 0.0) <= n["t"] else n["r"]


def _argmax(v):
    return max(range(len(v)), key=lambda i: (v[i], -i)) if v else 0


def _act(name, v):
    if name == "tanh":
        return math.tanh(v)
    if name == "logistic":
        return _sigmoid(v)
    if name == "identity":
        return v
    return max(v, 0.0)


def raw_predict(x):
    """Standardized vector -> (value or class index, probabilities)."""
    t = P["type"]
    k = len(CLASSES)
    if t == "linear":
        return P["intercept"] + sum(a * b for a, b in zip(P["coef"], x)), []
    if t == "logistic":
        probs = _softmax([_affine(w, x) for w in P["w"]])
    elif t == "tree":
        v = _tree(P["nodes"], x)
        if k == 0:
            return v[0], []
        probs = list(v)
    elif t == "forest":
        outs = [_tree(tr, x) for tr in P["trees"]]
        if k == 0:
            return sum(o[0] for o in outs) / len(outs), []
        probs = [sum(o[c] for o in outs) / len(outs) for c in range(k)]
    elif t == "gbm":
        f = list(P["init"])
        for stage in P["stages"]:
            for j, tr in enumerate(stage):
                f[j] += P["rate"] * _tree(tr, x)[0]
        if k == 0:
            return f[0], []
        if k == 2 and len(f) == 1:
            p = _sigmoid(f[0])
            probs = [1 - p, p]
        else:
            p = [_sigmoid(v) for v in f]
            s = sum(p) or 1e-12
            probs = [v / s for v in p]
    elif t == "adaBoost":
        if k == 0:
            preds = sorted((_tree(tr, x)[0], a) for tr, a in zip(P["trees"], P["alphas"]))
            total, acc = sum(a for _, a in preds), 0.0
            for v, a in preds:
                acc += a
                if acc >= total / 2:
                    return v, []
            return preds[-1][0], []
        votes = [0.0] * k
        for tr, a in zip(P["trees"], P["alphas"]):
            votes[_argmax(_tree(tr, x))] += a
        probs = _softmax([v / max(k - 1, 1) for v in votes])
    elif t == "svm":
        d = [_affine(w, x) for w in P["w"]]
        if k == 0:
            return d[0], []
        if k == 2 and len(d) == 1:
            p = _sigmoid(P["scale"] * d[0])
            probs = [1 - p, p]
        else:
            probs = _softmax([v * P["scale"] for v in d])
    elif t == "mlp":
        a = x
        layers = P["layers"]
        for li, layer in enumerate(layers):
            z = [b + sum(wi * ai for wi, ai in zip(row, a)) for row, b in zip(layer["w"], layer["b"])]
            a = z if li == len(layers) - 1 else [_act(P["activation"], v) for v in z]
        if k == 0:
            return a[0] * P["y_std"] + P["y_mean"], []
        probs = _softmax(a)
    elif t == "naiveBayes":
        logs = []
        for c in range(k):
            lp = math.log(max(P["prior"][c], 1e-12))
            for j, v in enumerate(x):
                m, s2 = P["mean"][c][j], max(P["var"][c][j], 1e-9)
                lp += -0.5 * ((v - m) ** 2 / s2 + math.log(2 * math.pi * s2))
            logs.append(lp)
        probs = _softmax(logs)
    elif t == "knn":
        d = sorted((sum((a - b) ** 2 for a, b in zip(r, x)), i) for i, r in enumerate(P["x"]))[: max(1, P["k"])]
        wt = (lambda dd: 1.0 / (math.sqrt(dd) + 1e-9)) if P["distance_weighted"] else (lambda dd: 1.0)
        if k == 0:
            ws = sum(wt(dd) for dd, _ in d)
            return sum(wt(dd) * P["y"][i] for dd, i in d) / ws, []
        acc = [0.0] * k
        for dd, i in d:
            acc[int(P["y"][i])] += wt(dd)
        s = sum(acc) or 1e-300
        probs = [v / s for v in acc]
    else:
        raise ValueError("unsupported model type " + t)
    return _argmax(probs), probs


def predict(row):
    """Feature dict -> prediction (class label or number) and class probabilities."""
    v, probs = raw_predict(encode(row))
    if CLASSES:
        return CLASSES[v], dict(zip(CLASSES, probs))
    return v, {{}}


def main():
    src = open(sys.argv[1], newline="", encoding="utf-8") if len(sys.argv) > 1 else sys.stdin
    rows = csv.DictReader(src)
    out = csv.writer(sys.stdout)
    header = ["prediction"] + ["P(%s)" % c for c in CLASSES]
    out.writerow(header)
    for row in rows:
        label, probs = predict(row)
        out.writerow([label] + [repr(probs[c]) for c in CLASSES])


if __name__ == "__main__":
    main()
"#,
        name = m.name,
        version = m.version,
        algo = m.algorithm,
        file = python_ident(&m.name),
        json = json
    )
}

pub fn python_ident(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) { format!("model_{s}") } else { format!("score_{s}") }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::studio::learn::{self, tests_support::sample, LearnerSpec};

    #[test]
    fn registry_drift_and_python_export() {
        let t = sample(200);
        let feats = vec!["a".to_string(), "b".into(), "c".into()];
        let mut shifted = t.clone();
        for r in shifted.rows.iter_mut() {
            if let Scalar::Num(v) = &mut r[0] {
                *v += 3.0;
            }
        }
        for kind in ["logistic", "tree", "forest", "gbm", "adaboost", "svm", "mlp", "naive_bayes", "knn"] {
            let m = learn::fit(&LearnerSpec { kind: kind.into(), ..Default::default() }, &t, &feats, "label").unwrap();
            let mut rm = register("churn", &m, &t, BTreeMap::new(), "test", "");
            rm.version = next_version(&[], "churn");
            assert_eq!(rm.version, 1);
            let d = drift(&rm, &t);
            assert!(d.iter().all(|r| r.psi < 0.1), "{kind}: {:?}", d.iter().map(|r| r.psi).collect::<Vec<_>>());
            let d2 = drift(&rm, &shifted);
            assert!(d2[0].psi > 0.25 && d2[0].ks_p < 0.001, "{:?}", d2[0]);
            // Round trip.
            let back: RegisteredModel = serde_json::from_str(&to_json(&rm)).unwrap();
            assert_eq!(back.model.predict_table(&t), m.predict_table(&t));
            // Python reproduces the predictions (when python3 is available).
            check_python(&rm, &t);
        }
        let m = learn::fit(&LearnerSpec { kind: "gbm".into(), ..Default::default() }, &t, &feats, "y").unwrap();
        let rm = register("y", &m, &t, BTreeMap::new(), "test", "");
        check_python(&rm, &t);
    }

    fn check_python(rm: &RegisteredModel, t: &TableData) {
        let Ok(dir) = std::env::var("CARGO_TARGET_TMPDIR").or_else(|_| std::env::temp_dir().into_os_string().into_string()) else { return };
        let script = std::path::Path::new(&dir).join(format!("vc_score_{}_{}.py", rm.model.spec.kind, std::process::id()));
        let csv_path = script.with_extension("csv");
        std::fs::write(&script, python_script(rm)).unwrap();
        std::fs::write(&csv_path, crate::flow::write_csv(t, ',')).unwrap();
        let Ok(out) = std::process::Command::new("python3").arg(&script).arg(&csv_path).output() else { return };
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&out.stdout);
        let preds = rm.model.predict_table(t);
        for (line, (v, _)) in text.lines().skip(1).zip(&preds) {
            let first = line.split(',').next().unwrap();
            if rm.model.is_classifier() {
                assert_eq!(first, rm.model.classes[*v as usize], "{}", rm.model.spec.kind);
            } else {
                assert!((first.parse::<f64>().unwrap() - v).abs() < 1e-6 * v.abs().max(1.0));
            }
        }
        let _ = std::fs::remove_file(script);
        let _ = std::fs::remove_file(csv_path);
    }
}
