//! The online learners of the personal brain. All train incrementally from single events, on
//! device, and serialize to JSON.
//!
//! - [`ChartRecommender`]: contextual multinomial logistic regression (SGD) over field-type
//!   features, initialised with visualization best practice, blended with Dirichlet counts per
//!   exact context ("what this user picked for a date + a measure").
//! - [`Preferences`]: decayed value counts per formatting key (labels on/off, legend position,
//!   theme, title style…).
//! - [`Sequences`]: an n-gram (trigram → bigram → unigram backoff) model of Power Query step
//!   types per source kind.
//! - [`Bandit`]: Thompson sampling over suggestion families, tuned by accept / dismiss.
//! - [`MeasurePatterns`]: which DAX patterns the user writes (ratios, YoY, YTD, rankings…).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::features::{FieldSig, N_FEATURES, context_key, features};

/// The chart types the recommender ranks (serde names of `VisualKind`).
pub const CHART_CLASSES: &[&str] = &[
    "clusteredColumn",
    "clusteredBar",
    "stackedColumn",
    "line",
    "area",
    "pie",
    "donut",
    "treemap",
    "scatter",
    "card",
    "multiRowCard",
    "table",
    "matrix",
    "map",
    "filledMap",
    "waterfall",
    "funnel",
    "heatMap",
    "kpi",
    "gauge",
    "histogram",
    "lineClusteredColumn",
];

fn idx(name: &str) -> usize {
    CHART_CLASSES.iter().position(|c| *c == name).unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChartRecommender {
    /// weights[class][feature]
    pub weights: Vec<Vec<f64>>,
    /// context key → class → count
    pub counts: BTreeMap<String, BTreeMap<String, f64>>,
    pub updates: u64,
    pub learning_rate: f64,
    /// Prior strength of the logistic model against the counts (pseudo-observations).
    pub alpha: f64,
}

impl Default for ChartRecommender {
    fn default() -> Self {
        let mut w = vec![vec![0.0; N_FEATURES]; CHART_CLASSES.len()];
        // Best-practice prior: feature index from FEATURE_NAMES.
        let mut set = |class: &str, f: usize, v: f64| {
            if let Some(row) = w.get_mut(idx(class))
                && let Some(x) = row.get_mut(f)
            {
                *x += v;
            }
        };
        // 0 bias, 1 date, 2 m=1, 3 m=2, 4 m>=3, 5 cats=0, 6 cats=1, 7 cats>=2, 8 geo, 9 id, 10 low, 11 mid, 12 high, 13 only values, 14 bool
        set("clusteredColumn", 0, 0.6);
        set("clusteredColumn", 6, 1.2);
        set("clusteredColumn", 10, 0.6);
        set("clusteredBar", 6, 1.1);
        set("clusteredBar", 11, 1.0);
        set("clusteredBar", 12, 0.6);
        set("stackedColumn", 7, 1.2);
        set("stackedColumn", 1, 0.3);
        set("line", 1, 2.4);
        set("line", 7, -0.4);
        set("area", 1, 1.3);
        set("pie", 10, 0.9);
        set("pie", 6, 0.4);
        set("pie", 3, -1.0);
        set("donut", 10, 0.8);
        set("donut", 6, 0.3);
        set("treemap", 12, 0.7);
        set("treemap", 6, 0.3);
        set("scatter", 3, 1.6);
        set("scatter", 9, 0.4);
        set("card", 13, 2.2);
        set("card", 2, 0.5);
        set("multiRowCard", 13, 1.0);
        set("multiRowCard", 4, 1.2);
        set("table", 9, 1.4);
        set("table", 4, 0.8);
        set("table", 12, 0.6);
        set("matrix", 7, 1.0);
        set("matrix", 4, 0.6);
        set("map", 8, 2.0);
        set("filledMap", 8, 1.2);
        set("waterfall", 1, 0.2);
        set("funnel", 10, 0.3);
        set("heatMap", 7, 0.7);
        set("kpi", 13, 0.6);
        set("kpi", 1, 0.3);
        set("gauge", 13, 0.5);
        set("histogram", 5, 0.2);
        set("lineClusteredColumn", 1, 0.6);
        set("lineClusteredColumn", 3, 1.0);
        for row in w.iter_mut() {
            if let Some(b) = row.get_mut(0) {
                *b -= 0.5;
            }
        }
        ChartRecommender { weights: w, counts: BTreeMap::new(), updates: 0, learning_rate: 0.15, alpha: 4.0 }
    }
}

pub fn softmax(z: &[f64]) -> Vec<f64> {
    let m = z.iter().cloned().fold(f64::MIN, f64::max);
    let e: Vec<f64> = z.iter().map(|x| (x - m).exp()).collect();
    let s: f64 = e.iter().sum::<f64>().max(1e-12);
    e.iter().map(|x| x / s).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChartScore {
    pub kind: String,
    pub prob: f64,
    /// Share of this choice among the user's own choices in this context, and how many.
    pub personal_share: f64,
    pub personal_n: f64,
}

impl ChartRecommender {
    fn logits(&self, x: &[f64; N_FEATURES]) -> Vec<f64> {
        self.weights.iter().map(|w| w.iter().zip(x.iter()).map(|(a, b)| a * b).sum()).collect()
    }

    pub fn lr_probs(&self, fields: &[FieldSig]) -> Vec<f64> {
        softmax(&self.logits(&features(fields)))
    }

    /// Ranked chart types for these fields.
    pub fn rank(&self, fields: &[FieldSig]) -> Vec<ChartScore> {
        let p = self.lr_probs(fields);
        let key = context_key(fields);
        let counts = self.counts.get(&key);
        let n: f64 = counts.map(|c| c.values().sum()).unwrap_or(0.0);
        let mut out: Vec<ChartScore> = CHART_CLASSES
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let c = counts.and_then(|m| m.get(*k)).copied().unwrap_or(0.0);
                let lr = p.get(i).copied().unwrap_or(0.0);
                let prob = (c + self.alpha * lr) / (n + self.alpha);
                ChartScore { kind: k.to_string(), prob, personal_share: if n > 0.0 { c / n } else { 0.0 }, personal_n: n }
            })
            .collect();
        out.sort_by(|a, b| b.prob.partial_cmp(&a.prob).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    /// One SGD step of cross-entropy towards `chosen`, and a count.
    pub fn learn(&mut self, fields: &[FieldSig], chosen: &str, weight: f64) {
        let Some(ci) = CHART_CLASSES.iter().position(|c| *c == chosen) else { return };
        let x = features(fields);
        let p = softmax(&self.logits(&x));
        let lr = self.learning_rate * weight / (1.0 + self.updates as f64 / 500.0);
        for (k, row) in self.weights.iter_mut().enumerate() {
            let g = p.get(k).copied().unwrap_or(0.0) - if k == ci { 1.0 } else { 0.0 };
            for (w, xi) in row.iter_mut().zip(x.iter()) {
                *w -= lr * g * xi;
            }
        }
        *self.counts.entry(context_key(fields)).or_default().entry(chosen.to_string()).or_default() += weight;
        self.updates += 1;
    }
}

/// Decayed counts of values per formatting key.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// "scope|key" → value → weight. Scope is a visual kind or "report".
    pub values: BTreeMap<String, BTreeMap<String, f64>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pref {
    pub scope: String,
    pub key: String,
    pub value: String,
    pub share: f64,
    pub n: f64,
}

impl Preferences {
    pub const DECAY: f64 = 0.97;

    pub fn learn(&mut self, scope: &str, key: &str, value: &str) {
        let m = self.values.entry(format!("{scope}|{key}")).or_default();
        for v in m.values_mut() {
            *v *= Self::DECAY;
        }
        *m.entry(value.to_string()).or_default() += 1.0;
        // Also learn across all visuals.
        if scope != "*" && scope != "report" {
            let m = self.values.entry(format!("*|{key}")).or_default();
            for v in m.values_mut() {
                *v *= Self::DECAY;
            }
            *m.entry(value.to_string()).or_default() += 1.0;
        }
    }

    /// The dominant value of each key for `scope` (falling back to all visuals), when it is a
    /// clear habit (share ≥ `min_share`, weight ≥ `min_n`).
    pub fn habits(&self, scope: &str, min_share: f64, min_n: f64) -> Vec<Pref> {
        let mut out: BTreeMap<String, Pref> = BTreeMap::new();
        for s in ["*", scope] {
            for (k, vals) in self.values.iter().filter(|(k, _)| k.starts_with(&format!("{s}|"))) {
                let n: f64 = vals.values().sum();
                if let Some((v, w)) = vals.iter().max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal)) {
                    let share = w / n.max(1e-9);
                    if share >= min_share && n >= min_n {
                        let key = k.split_once('|').map(|x| x.1).unwrap_or("").to_string();
                        out.insert(key.clone(), Pref { scope: s.to_string(), key, value: v.clone(), share, n });
                    }
                }
            }
        }
        out.into_values().collect()
    }
}

/// N-gram model of transform steps per source kind.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sequences {
    /// "source|prev2|prev1" → next → count (prev2 / prev1 may be "^" for the start).
    pub trigrams: BTreeMap<String, BTreeMap<String, f64>>,
    pub bigrams: BTreeMap<String, BTreeMap<String, f64>>,
    pub unigrams: BTreeMap<String, BTreeMap<String, f64>>,
    /// Source kinds used (csv, excel, sql, web…).
    pub sources: BTreeMap<String, f64>,
}

impl Sequences {
    pub fn learn(&mut self, source: &str, history: &[String], step: &str) {
        let p1 = history.last().map(String::as_str).unwrap_or("^");
        let p2 = if history.len() >= 2 { history.get(history.len() - 2).map(String::as_str).unwrap_or("^") } else { "^" };
        *self.trigrams.entry(format!("{source}|{p2}|{p1}")).or_default().entry(step.into()).or_default() += 1.0;
        *self.bigrams.entry(format!("{source}|{p1}")).or_default().entry(step.into()).or_default() += 1.0;
        *self.unigrams.entry(source.to_string()).or_default().entry(step.into()).or_default() += 1.0;
    }

    /// Next-step probabilities (interpolated trigram/bigram/unigram).
    pub fn predict(&self, source: &str, history: &[String]) -> Vec<(String, f64)> {
        let p1 = history.last().map(String::as_str).unwrap_or("^");
        let p2 = if history.len() >= 2 { history.get(history.len() - 2).map(String::as_str).unwrap_or("^") } else { "^" };
        let mut scores: BTreeMap<String, f64> = BTreeMap::new();
        let mut add = |m: Option<&BTreeMap<String, f64>>, w: f64| {
            if let Some(m) = m {
                let n: f64 = m.values().sum();
                if n > 0.0 {
                    for (k, c) in m {
                        *scores.entry(k.clone()).or_default() += w * c / n;
                    }
                }
            }
        };
        add(self.trigrams.get(&format!("{source}|{p2}|{p1}")), 0.6);
        add(self.bigrams.get(&format!("{source}|{p1}")), 0.3);
        add(self.unigrams.get(source), 0.1);
        let total: f64 = scores.values().sum();
        let mut v: Vec<(String, f64)> = scores.into_iter().map(|(k, s)| (k, if total > 0.0 { s / total } else { 0.0 })).collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        v
    }
}

/// Thompson sampling with Beta(1 + accepts, 1 + rejects) per suggestion family.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bandit {
    pub arms: BTreeMap<String, (f64, f64)>,
}

/// A small deterministic PRNG (xorshift64*), seeded from the clock in the app.
#[derive(Clone, Debug)]
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0.max(1);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn uniform(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
    pub fn normal(&mut self) -> f64 {
        let (u1, u2) = (self.uniform(), self.uniform());
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
    /// Gamma(shape, 1) by Marsaglia–Tsang.
    pub fn gamma(&mut self, shape: f64) -> f64 {
        if shape < 1.0 {
            let u = self.uniform();
            return self.gamma(shape + 1.0) * u.powf(1.0 / shape);
        }
        let d = shape - 1.0 / 3.0;
        let c = 1.0 / (9.0 * d).sqrt();
        loop {
            let x = self.normal();
            let v = (1.0 + c * x).powi(3);
            if v <= 0.0 {
                continue;
            }
            let u = self.uniform();
            if u.ln() < 0.5 * x * x + d - d * v + d * v.ln() {
                return d * v;
            }
        }
    }
    pub fn beta(&mut self, a: f64, b: f64) -> f64 {
        let x = self.gamma(a.max(1e-3));
        let y = self.gamma(b.max(1e-3));
        x / (x + y).max(1e-12)
    }
}

impl Bandit {
    pub fn learn(&mut self, arm: &str, accepted: bool) {
        let e = self.arms.entry(arm.to_string()).or_insert((0.0, 0.0));
        if accepted {
            e.0 += 1.0;
        } else {
            e.1 += 1.0;
        }
    }
    pub fn mean(&self, arm: &str) -> f64 {
        let (a, r) = self.arms.get(arm).copied().unwrap_or((0.0, 0.0));
        (1.0 + a) / (2.0 + a + r)
    }
    pub fn sample(&self, arm: &str, rng: &mut Rng) -> f64 {
        let (a, r) = self.arms.get(arm).copied().unwrap_or((0.0, 0.0));
        rng.beta(1.0 + a, 1.0 + r)
    }
}

/// DAX pattern classes and how often the user writes them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MeasurePatterns {
    pub counts: BTreeMap<String, f64>,
    /// Format strings used for measures of each pattern.
    pub formats: BTreeMap<String, BTreeMap<String, f64>>,
}

pub fn dax_pattern(dax: &str) -> &'static str {
    let d = dax.to_uppercase();
    if ["SAMEPERIODLASTYEAR", "DATEADD", "PARALLELPERIOD", "PREVIOUSYEAR"].iter().any(|f| d.contains(f)) {
        "year-over-year"
    } else if ["TOTALYTD", "DATESYTD", "TOTALQTD", "TOTALMTD", "DATESMTD"].iter().any(|f| d.contains(f)) {
        "period-to-date"
    } else if d.contains("RANKX") || d.contains("TOPN") {
        "ranking"
    } else if d.contains("DIVIDE") || d.contains(" / ") || d.contains(")/") {
        "ratio"
    } else if d.contains("CALCULATE") && (d.contains("ALL(") || d.contains("ALLSELECTED") || d.contains("REMOVEFILTERS")) {
        "share-of-total"
    } else if d.contains("CALCULATE") {
        "filtered"
    } else if ["SUMX", "AVERAGEX", "MAXX", "MINX", "COUNTX"].iter().any(|f| d.contains(f)) {
        "iterator"
    } else if d.contains("DISTINCTCOUNT") || d.contains("COUNTROWS") || d.contains("COUNT(") {
        "count"
    } else if d.contains("AVERAGE") {
        "average"
    } else if d.contains("SUM(") {
        "sum"
    } else {
        "other"
    }
}

impl MeasurePatterns {
    pub fn learn(&mut self, dax: &str, format: &str) {
        let p = dax_pattern(dax);
        *self.counts.entry(p.to_string()).or_default() += 1.0;
        if !format.is_empty() {
            *self.formats.entry(p.to_string()).or_default().entry(format.to_string()).or_default() += 1.0;
        }
    }

    /// A DAX template of `pattern` built on `base` (a measure name), with the usual format.
    pub fn template(&self, pattern: &str, base: &str, date_col: Option<&str>) -> Option<(String, String, String)> {
        let fmt = |dflt: &str| self.formats.get(pattern).and_then(|m| m.iter().max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))).map(|(f, _)| f.clone()).unwrap_or_else(|| dflt.to_string());
        let m = format!("[{base}]");
        Some(match pattern {
            "year-over-year" => {
                let d = date_col?;
                (format!("{base} YoY %"), format!("VAR prev = CALCULATE({m}, SAMEPERIODLASTYEAR({d})) RETURN DIVIDE({m} - prev, prev)"), fmt("0.0%"))
            }
            "period-to-date" => {
                let d = date_col?;
                (format!("{base} YTD"), format!("TOTALYTD({m}, {d})"), fmt("#,0"))
            }
            "share-of-total" | "ratio" => (format!("{base} % of total"), format!("DIVIDE({m}, CALCULATE({m}, ALLSELECTED()))"), fmt("0.0%")),
            "ranking" => return None,
            _ => return None,
        })
    }

    pub fn top(&self) -> Vec<(String, f64)> {
        let mut v: Vec<(String, f64)> = self.counts.iter().map(|(k, c)| (k.clone(), *c)).collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date_measure() -> Vec<FieldSig> {
        vec![FieldSig::new("date", "Date"), FieldSig::new("measure", "Sales")]
    }

    #[test]
    fn prior_is_best_practice_and_learning_moves_it() {
        let mut r = ChartRecommender::default();
        assert_eq!(r.rank(&date_measure())[0].kind, "line");
        assert_eq!(r.rank(&[FieldSig::new("measure", "Sales")])[0].kind, "card");
        let cat = vec![FieldSig { ty: "category".into(), name: "Product".into(), card: "mid".into() }, FieldSig::new("measure", "Sales")];
        assert!(["clusteredBar", "clusteredColumn"].contains(&r.rank(&cat)[0].kind.as_str()));
        assert_eq!(r.rank(&[FieldSig::new("geo", "Country"), FieldSig::new("measure", "S")])[0].kind, "map");
        // This user always makes area charts for dates.
        for _ in 0..12 {
            r.learn(&date_measure(), "area", 1.0);
        }
        let top = &r.rank(&date_measure())[0];
        assert_eq!(top.kind, "area");
        assert!(top.personal_share > 0.99 && top.personal_n >= 12.0);
        // The logistic part generalises to a similar context (date + 2 measures).
        let ctx2 = vec![FieldSig::new("date", "Date"), FieldSig::new("measure", "A"), FieldSig::new("measure", "B")];
        let p = r.lr_probs(&ctx2);
        assert!(p[idx("area")] > ChartRecommender::default().lr_probs(&ctx2)[idx("area")]);
    }

    #[test]
    fn preferences_sequences_bandit_patterns() {
        let mut p = Preferences::default();
        for _ in 0..5 {
            p.learn("line", "labels.show", "true");
        }
        p.learn("line", "labels.show", "false");
        let h = p.habits("line", 0.6, 3.0);
        assert!(h.iter().any(|x| x.key == "labels.show" && x.value == "true"));
        assert!(p.habits("pie", 0.6, 3.0).iter().any(|x| x.key == "labels.show"), "global habits apply to other visuals");

        let mut s = Sequences::default();
        for _ in 0..4 {
            s.learn("csv", &[], "PromoteHeaders");
            s.learn("csv", &["PromoteHeaders".into()], "ChangeType");
            s.learn("csv", &["PromoteHeaders".into(), "ChangeType".into()], "RemoveColumns");
        }
        assert_eq!(s.predict("csv", &[]).first().map(|x| x.0.as_str()), Some("PromoteHeaders"));
        assert_eq!(s.predict("csv", &["PromoteHeaders".into(), "ChangeType".into()])[0].0, "RemoveColumns");

        let mut b = Bandit::default();
        for _ in 0..20 {
            b.learn("format", false);
            b.learn("chart", true);
        }
        let mut rng = Rng(42);
        let wins = (0..200).filter(|_| b.sample("chart", &mut rng) > b.sample("format", &mut rng)).count();
        assert!(wins > 180, "{wins}");
        assert!(b.mean("chart") > 0.9);

        let mut m = MeasurePatterns::default();
        m.learn("DIVIDE([Profit], [Sales])", "0.0%");
        m.learn("CALCULATE([Sales], SAMEPERIODLASTYEAR('Date'[Date]))", "");
        assert_eq!(dax_pattern("TOTALYTD([S], 'D'[D])"), "period-to-date");
        assert_eq!(m.template("ratio", "Sales", None).map(|t| t.2), Some("0.0%".into()));
        assert!(m.template("year-over-year", "Sales", Some("Sales[Date]")).is_some());
    }
}
