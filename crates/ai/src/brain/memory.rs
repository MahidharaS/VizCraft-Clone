//! Memory of the user's past questions and how they were resolved, searched with BM25 (and,
//! optionally, cosine similarity over embeddings a provider computed).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::decide::tokens;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryItem {
    pub question: String,
    /// What answered it: a chart spec, a DAX query, the copilot's answer…
    pub resolution: String,
    pub accepted: bool,
    pub ts: f64,
    /// Optional embedding of the question.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub embedding: Vec<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Memory {
    pub items: Vec<MemoryItem>,
    /// term → number of items containing it.
    pub df: BTreeMap<String, usize>,
    pub total_len: usize,
}

const K1: f64 = 1.2;
const B: f64 = 0.75;
const MAX_ITEMS: usize = 5000;

impl Memory {
    pub fn add(&mut self, item: MemoryItem) {
        let toks = tokens(&item.question);
        let uniq: std::collections::BTreeSet<&String> = toks.iter().collect();
        for t in uniq {
            *self.df.entry(t.clone()).or_default() += 1;
        }
        self.total_len += toks.len();
        self.items.push(item);
        if self.items.len() > MAX_ITEMS {
            let old = self.items.remove(0);
            let toks = tokens(&old.question);
            let uniq: std::collections::BTreeSet<&String> = toks.iter().collect();
            for t in uniq {
                if let Some(c) = self.df.get_mut(t) {
                    *c = c.saturating_sub(1);
                }
            }
            self.total_len = self.total_len.saturating_sub(toks.len());
        }
    }

    pub fn bm25(&self, query: &str, item: &MemoryItem) -> f64 {
        let n = self.items.len().max(1) as f64;
        let avg = (self.total_len as f64 / n).max(1.0);
        let doc = tokens(&item.question);
        let dl = doc.len() as f64;
        let mut score = 0.0;
        let q: std::collections::BTreeSet<String> = tokens(query).into_iter().collect();
        for t in &q {
            let tf = doc.iter().filter(|d| *d == t).count() as f64;
            if tf == 0.0 {
                continue;
            }
            let df = self.df.get(t).copied().unwrap_or(0) as f64;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            score += idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * dl / avg));
        }
        score
    }

    /// The `k` best-matching past questions (accepted resolutions weigh more), with scores.
    pub fn recall(&self, query: &str, k: usize, query_embedding: Option<&[f32]>) -> Vec<(MemoryItem, f64)> {
        let mut v: Vec<(MemoryItem, f64)> = self
            .items
            .iter()
            .map(|it| {
                let mut s = self.bm25(query, it);
                if let Some(qe) = query_embedding
                    && !it.embedding.is_empty()
                {
                    s += 3.0 * cosine(qe, &it.embedding).max(0.0);
                }
                if it.accepted {
                    s *= 1.25;
                }
                (it.clone(), s)
            })
            .filter(|(_, s)| *s > 0.0)
            .collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        v.truncate(k);
        v
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let (mut d, mut na, mut nb) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b.iter()) {
        d += f64::from(*x) * f64::from(*y);
        na += f64::from(*x) * f64::from(*x);
        nb += f64::from(*y) * f64::from(*y);
    }
    if na == 0.0 || nb == 0.0 { 0.0 } else { d / (na.sqrt() * nb.sqrt()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bm25_recall() {
        let mut m = Memory::default();
        for (q, r) in [("total sales by region", "column chart Region × Total Sales"), ("profit margin per product category", "measure Margin = DIVIDE"), ("how many customers per month", "line chart")] {
            m.add(MemoryItem { question: q.into(), resolution: r.into(), accepted: true, ..Default::default() });
        }
        let r = m.recall("sales by region last year", 2, None);
        assert_eq!(r[0].0.question, "total sales by region");
        assert!(m.recall("zebra", 3, None).is_empty());
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-9);
    }
}
