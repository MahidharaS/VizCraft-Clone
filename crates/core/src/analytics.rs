//! Analytics pane: reference lines, trend lines, forecasting, clustering and anomalies,
//! computed from a visual's data.

use crate::color::Color;
use crate::doc::AnalyticsLine;
use crate::vquery::VisualData;

/// A computed analytics result, in data space.
#[derive(Clone, Debug, PartialEq)]
pub enum Computed {
    /// A horizontal (value-axis) reference line.
    ValueLine { value: f64, label: String, color: Color, width: f32, dash: String },
    /// A band between two values.
    Band { from: f64, to: f64, label: String, color: Color },
    /// A series of points over category indices (trend lines; `x` = category index).
    Series { points: Vec<(f64, f64)>, label: String, color: Color, width: f32, dash: String },
    /// Forecast continuation after the last category: values and confidence bounds.
    Forecast { start_index: usize, values: Vec<f64>, lower: Vec<f64>, upper: Vec<f64>, color: Color },
    /// Cluster index per row.
    Clusters { assignment: Vec<usize>, k: usize },
    /// Rows flagged as anomalies.
    Anomalies { rows: Vec<usize>, color: Color },
}

/// Computes the analytics lines of a visual.
pub fn compute(lines: &[AnalyticsLine], data: &VisualData) -> Vec<Computed> {
    let mut out = Vec::new();
    let Some(vi) = data.values().first().copied() else { return out };
    let vals: Vec<f64> = data.rows.iter().filter_map(|r| r.get(vi)?.as_f64()).collect();
    if vals.is_empty() {
        return out;
    }
    for l in lines {
        let label = if l.name.is_empty() { l.kind.clone() } else { l.name.clone() };
        let v = match l.kind.as_str() {
            "Constant" => l.value.as_num(),
            "Min" => vals.iter().cloned().reduce(f64::min),
            "Max" => vals.iter().cloned().reduce(f64::max),
            "Average" => Some(vals.iter().sum::<f64>() / vals.len() as f64),
            "Median" => Some(percentile(&vals, 50.0)),
            "Percentile" => Some(percentile(&vals, l.percentile)),
            _ => None,
        };
        if let Some(v) = v {
            out.push(Computed::ValueLine { value: v, label, color: l.color, width: l.width, dash: l.dash.clone() });
        }
    }
    out
}

pub fn percentile(vals: &[f64], p: f64) -> f64 {
    let mut v: Vec<f64> = vals.iter().cloned().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = (p / 100.0).clamp(0.0, 1.0) * (v.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let a = v.get(lo).copied().unwrap_or(0.0);
    let b = v.get(hi).copied().unwrap_or(a);
    a + (b - a) * (rank - lo as f64)
}
