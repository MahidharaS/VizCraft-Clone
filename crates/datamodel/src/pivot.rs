//! A data-model PivotTable: row and column fields from any related tables, measures as values,
//! laid out as a grid with grand totals.

use std::collections::HashMap;

use crate::dax::{Expr, parse};
use crate::model::Model;
use crate::value::{Key, Value};
use crate::{Error, Result};

/// The most cells a report may produce.
pub const MAX_CELLS: usize = 2_000_000;

#[derive(Clone, Debug, Default)]
pub struct PivotSpec {
    /// Row fields, `Table[Column]`.
    pub rows: Vec<String>,
    /// Column fields, `Table[Column]`.
    pub columns: Vec<String>,
    /// Measure names (`Total Sales` or `[Total Sales]`) or DAX expressions.
    pub values: Vec<String>,
    /// DAX filter arguments for the whole report.
    pub filters: Vec<String>,
}

/// The laid-out report: `grid[row][col]`, with the number of header rows.
#[derive(Clone, Debug, PartialEq)]
pub struct PivotGrid {
    pub grid: Vec<Vec<Value>>,
    pub header_rows: usize,
    /// For each grid column, the index (into the spec's values) of the value it shows, if any.
    pub value_columns: Vec<Option<usize>>,
    /// The measure format (number format code) of each value, if it has one.
    pub value_formats: Vec<Option<String>>,
}

fn field(m: &Model, f: &str) -> Result<(String, String)> {
    match parse(f)? {
        Expr::Column { table, column } => {
            m.column_ref(&table, &column)?;
            Ok((table, column))
        }
        _ => Err(Error::Model(format!("`{f}` is not a column such as Products[Category]"))),
    }
}

/// `(label, DAX)` of one value field: a measure name, `Label := expression` (an implicit
/// measure such as `Sum of Amount := SUM(Sales[Amount])`), or a bare expression.
fn value_field(m: &Model, v: &str) -> (String, String) {
    if let Some((label, expr)) = v.split_once(":=") {
        return (label.trim().to_string(), expr.trim().to_string());
    }
    let bare = v.trim().trim_start_matches('[').trim_end_matches(']');
    match m.measure(bare) {
        Some(ms) => (ms.name.clone(), format!("[{}]", ms.name.replace(']', "]]"))),
        None => (v.trim().to_string(), v.to_string()),
    }
}

fn value_format(m: &Model, v: &str) -> Option<String> {
    let bare = v.trim().trim_start_matches('[').trim_end_matches(']');
    m.measure(bare).and_then(|ms| ms.format.clone()).filter(|f| !f.trim().is_empty())
}

fn label(tuple: &[Value]) -> Value {
    match tuple {
        [one] if !one.is_blank() => one.clone(),
        [_] => Value::text("(blank)"),
        many => Value::text(many.iter().map(|v| if v.is_blank() { "(blank)".into() } else { v.to_text() }).collect::<Vec<_>>().join(" - ")),
    }
}

fn keys(t: &[Value]) -> Vec<Key> {
    t.iter().map(Value::key).collect()
}

pub fn layout(m: &Model, spec: &PivotSpec) -> Result<PivotGrid> {
    let rows: Vec<_> = spec.rows.iter().map(|f| field(m, f)).collect::<Result<_>>()?;
    let cols: Vec<_> = spec.columns.iter().map(|f| field(m, f)).collect::<Result<_>>()?;
    if spec.values.is_empty() {
        return Err(Error::Model("a PivotTable needs at least one value (measure)".into()));
    }
    let values: Vec<(String, String)> = spec.values.iter().map(|v| value_field(m, v)).collect();
    let value_formats: Vec<Option<String>> = spec.values.iter().map(|v| value_format(m, v)).collect();
    let nv = values.len();
    let q = |g: &[(String, String)]| m.summarize(g, &values, &spec.filters);
    let col_name = |(_, c): &(String, String)| Value::text(c);

    let mut grid: Vec<Vec<Value>> = Vec::new();
    if cols.is_empty() {
        let mut header: Vec<Value> = rows.iter().map(col_name).collect();
        if rows.is_empty() {
            header.push(Value::Blank);
        }
        header.extend(values.iter().map(|(l, _)| Value::text(l)));
        grid.push(header);
        for r in q(&rows)?.rows {
            grid.push(r);
        }
        let mut total = vec![Value::text("Grand Total")];
        total.resize(rows.len().max(1), Value::Blank);
        total.extend(q(&[])?.rows.into_iter().next().unwrap_or_else(|| vec![Value::Blank; nv]));
        grid.push(total);
        let lead = rows.len().max(1);
        let value_columns = (0..lead + nv).map(|c| c.checked_sub(lead)).collect();
        return Ok(PivotGrid { grid, header_rows: 1, value_columns, value_formats });
    }

    let nr = rows.len();
    let col_keys: Vec<Vec<Value>> = q(&cols)?.rows.into_iter().map(|r| r.into_iter().take(cols.len()).collect()).collect();
    let row_res = q(&rows)?.rows;
    if (row_res.len() + 3).saturating_mul((col_keys.len() + 1) * nv + nr.max(1)) > MAX_CELLS {
        return Err(Error::Model("the PivotTable would be too large".into()));
    }
    let mut all = rows.clone();
    all.extend(cols.iter().cloned());
    let mut cells: HashMap<(Vec<Key>, Vec<Key>), Vec<Value>> = HashMap::new();
    for r in q(&all)?.rows {
        let rk = keys(r.get(..nr).unwrap_or_default());
        let ck = keys(r.get(nr..nr + cols.len()).unwrap_or_default());
        cells.insert((rk, ck), r.get(nr + cols.len()..).unwrap_or_default().to_vec());
    }
    let col_totals: HashMap<Vec<Key>, Vec<Value>> = q(&cols)?
        .rows
        .into_iter()
        .map(|r| (keys(r.get(..cols.len()).unwrap_or_default()), r.get(cols.len()..).unwrap_or_default().to_vec()))
        .collect();
    let grand = q(&[])?.rows.into_iter().next().unwrap_or_else(|| vec![Value::Blank; nv]);

    // Header rows: column labels (one per value when there are several values), then value names.
    let lead = nr.max(1);
    let mut h1 = vec![Value::Blank; lead];
    let mut h2: Vec<Value> = if rows.is_empty() { vec![Value::Blank] } else { rows.iter().map(col_name).collect() };
    for ck in &col_keys {
        for (i, (l, _)) in values.iter().enumerate() {
            h1.push(if i == 0 { label(ck) } else { Value::Blank });
            h2.push(Value::text(l));
        }
    }
    for (i, (l, _)) in values.iter().enumerate() {
        h1.push(if i == 0 { Value::text("Grand Total") } else { Value::Blank });
        h2.push(Value::text(l));
    }
    grid.push(h1);
    grid.push(h2);
    let blank = vec![Value::Blank; nv];
    for r in &row_res {
        let rt = r.get(..nr).unwrap_or_default();
        let rk = keys(rt);
        let mut line: Vec<Value> = if rows.is_empty() { vec![Value::Blank] } else { rt.to_vec() };
        for ck in &col_keys {
            line.extend(cells.get(&(rk.clone(), keys(ck))).unwrap_or(&blank).iter().cloned());
        }
        line.extend(r.get(nr..).unwrap_or_default().iter().cloned());
        grid.push(line);
    }
    let mut total = vec![Value::text("Grand Total")];
    total.resize(lead, Value::Blank);
    for ck in &col_keys {
        total.extend(col_totals.get(&keys(ck)).unwrap_or(&blank).iter().cloned());
    }
    total.extend(grand);
    grid.push(total);
    let width = lead + (col_keys.len() + 1) * nv;
    let value_columns = (0..width).map(|c| c.checked_sub(lead).map(|k| k % nv)).collect();
    Ok(PivotGrid { grid, header_rows: 2, value_columns, value_formats })
}
