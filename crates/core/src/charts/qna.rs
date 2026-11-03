//! Q&A: turns a natural-language question ("total sales by region", "top 5 products by
//! profit", "average price per category in 2024") into visual wells.

use std::collections::BTreeMap;

use crate::build::Built;
use crate::doc::*;

/// Wells for the question, and a restatement of how it was understood.
pub fn interpret(p: &Project, built: &Built, question: &str) -> (BTreeMap<String, Vec<FieldUse>>, String) {
    let q = question.to_lowercase();
    let mut wells: BTreeMap<String, Vec<FieldUse>> = BTreeMap::new();
    let mut understood = Vec::new();
    // Measures named in the question.
    for m in &p.model.measures {
        if q.contains(&m.name.to_lowercase()) {
            wells.entry("values".into()).or_default().push(FieldUse::new(FieldRef::measure(&m.table, &m.name)));
            understood.push(m.name.clone());
        }
    }
    let agg = if q.contains("average") || q.contains("avg") || q.contains("mean") {
        Agg::Average
    } else if q.contains("count") || q.contains("how many") || q.contains("number of") {
        Agg::Count
    } else if q.contains("max") || q.contains("highest") {
        Agg::Max
    } else if q.contains("min") || q.contains("lowest") {
        Agg::Min
    } else {
        Agg::Sum
    };
    let by_pos = q.find(" by ").or_else(|| q.find(" per ")).or_else(|| q.find(" for each "));
    for (tname, t) in &built.tables {
        for (ci, c) in t.columns.iter().enumerate() {
            let cl = c.to_lowercase();
            if cl.len() < 2 || !q.contains(&cl) {
                continue;
            }
            let pos = q.find(&cl).unwrap_or(0);
            let numeric = t.types.get(ci).is_some_and(|ty| ty.is_numeric());
            let after_by = by_pos.is_some_and(|b| pos > b);
            if numeric && !after_by {
                if !wells.get("values").is_some_and(|v| !v.is_empty()) {
                    wells.entry("values".into()).or_default().push(FieldUse::agg(FieldRef::col(tname, c), agg));
                    understood.push(format!("{} of {c}", agg.label()));
                }
            } else if after_by || !numeric {
                wells.entry("axis".into()).or_default().push(FieldUse::new(FieldRef::col(tname, c)));
                understood.push(format!("by {c}"));
            }
        }
    }
    (wells, understood.join(" "))
}
