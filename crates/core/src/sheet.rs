//! Data sheets: an Excel-like grid with a small formula engine.

use crate::build::TableData;
use crate::doc::DataSheet;
use crate::scalar::Scalar;

/// Evaluates a sheet to table data (row 0 = headers).
pub fn evaluate(s: &DataSheet) -> TableData {
    let headers: Vec<String> = s.cells.first().cloned().unwrap_or_default();
    let width = headers.len();
    let rows: Vec<Vec<Scalar>> = s
        .cells
        .iter()
        .skip(1)
        .filter(|r| r.iter().any(|c| !c.trim().is_empty()))
        .map(|r| (0..width).map(|c| parse_input(r.get(c).map(String::as_str).unwrap_or(""))).collect())
        .collect();
    let mut t = TableData { name: s.name.clone(), columns: headers, types: vec![], rows };
    t.types = s.col_types.iter().map(|ty| crate::build::ColType::from_label(ty)).collect();
    t.infer_types();
    t
}

/// A raw cell input as a value: numbers, booleans, dates, text.
pub fn parse_input(s: &str) -> Scalar {
    let t = s.trim();
    if t.is_empty() {
        return Scalar::Null;
    }
    if let Ok(n) = t.replace(',', "").parse::<f64>() {
        return Scalar::Num(n);
    }
    match t.to_ascii_lowercase().as_str() {
        "true" => return Scalar::Bool(true),
        "false" => return Scalar::Bool(false),
        _ => {}
    }
    Scalar::Text(t.to_string())
}
