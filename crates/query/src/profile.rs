//! Column profiling: quality (valid, error, empty), distribution (distinct, unique, top values)
//! and statistics, as the editor's View options show them.

use std::collections::HashMap;

use crate::funcs::table::key_of;
use crate::value::{Table, Value, compare};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColumnProfile {
    pub count: usize,
    pub errors: usize,
    pub empty: usize,
    pub distinct: usize,
    pub unique: usize,
    pub min: Option<Value>,
    pub max: Option<Value>,
    pub average: Option<f64>,
    pub std_dev: Option<f64>,
    pub sum: Option<f64>,
    pub zeros: usize,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    /// Most frequent values with their counts, most frequent first.
    pub top: Vec<(String, usize)>,
}

impl ColumnProfile {
    pub fn valid(&self) -> usize {
        self.count - self.errors - self.empty
    }
}

/// Profiles column `c` over the first `limit` rows.
pub fn profile(t: &Table, c: usize, limit: usize) -> ColumnProfile {
    let mut p = ColumnProfile::default();
    let mut counts: HashMap<String, (String, usize)> = HashMap::new();
    let mut nums = Vec::new();
    for r in t.rows.iter().take(limit) {
        let v = r.get(c).cloned().unwrap_or_default();
        p.count += 1;
        if v.is_error() {
            p.errors += 1;
            continue;
        }
        if v.is_empty_cell() {
            p.empty += 1;
        }
        let e = counts.entry(key_of(&[&v])).or_insert_with(|| (v.display(), 0));
        e.1 += 1;
        if v.is_null() {
            continue;
        }
        if let Value::Number(n) = v {
            nums.push(n);
            if n == 0.0 {
                p.zeros += 1;
            }
        }
        if let Value::Text(s) = &v {
            let l = s.chars().count();
            p.min_length = Some(p.min_length.map_or(l, |m| m.min(l)));
            p.max_length = Some(p.max_length.map_or(l, |m| m.max(l)));
        }
        if p.min.as_ref().is_none_or(|m| compare(&v, m).is_lt()) {
            p.min = Some(v.clone());
        }
        if p.max.as_ref().is_none_or(|m| compare(&v, m).is_gt()) {
            p.max = Some(v.clone());
        }
    }
    p.distinct = counts.len();
    p.unique = counts.values().filter(|(_, n)| *n == 1).count();
    if !nums.is_empty() {
        let n = nums.len() as f64;
        let sum: f64 = nums.iter().sum();
        let mean = sum / n;
        p.sum = Some(sum);
        p.average = Some(mean);
        if nums.len() > 1 {
            p.std_dev = Some((nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt());
        }
    }
    let mut top: Vec<(String, usize)> = counts.into_values().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    top.truncate(30);
    p.top = top;
    p
}

/// `Table.Profile`.
pub fn profile_table(t: &Table) -> Value {
    let rows = (0..t.columns.len())
        .map(|c| {
            let p = profile(t, c, usize::MAX);
            vec![
                Value::text(t.columns.get(c).cloned().unwrap_or_default()),
                p.min.clone().unwrap_or_default(),
                p.max.clone().unwrap_or_default(),
                p.average.map(Value::Number).unwrap_or_default(),
                p.std_dev.map(Value::Number).unwrap_or_default(),
                Value::Number(p.count as f64),
                Value::Number(p.empty as f64),
                Value::Number(p.distinct as f64),
            ]
        })
        .collect();
    let cols = ["Column", "Min", "Max", "Average", "StandardDeviation", "Count", "NullCount", "DistinctCount"];
    Value::table(Table::new(cols.iter().map(|s| s.to_string()).collect(), rows))
}
