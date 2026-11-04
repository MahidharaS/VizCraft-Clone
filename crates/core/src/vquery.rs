//! Visual queries: from a visual's wells, drill state and filter context to a result table
//! ([`VisualData`]) the chart renderers draw.

use std::collections::BTreeMap;

use crate::build::{Built, ColType};
use crate::doc::*;
use crate::scalar::{Scalar, compare};
use crate::state::{Context, col_ref, field_column, resolve_use, value_dax};

/// One column of a visual's result.
#[derive(Clone, Debug, PartialEq)]
pub struct DataCol {
    pub label: String,
    /// The well it came from ("axis", "legend", "values", …).
    pub role: String,
    pub field: FieldRef,
    /// Values (numbers per group) vs categories (group-by).
    pub is_value: bool,
    pub format: String,
    pub col_type: ColType,
    pub show_as: ShowAs,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VisualData {
    pub cols: Vec<DataCol>,
    pub rows: Vec<Vec<Scalar>>,
    /// Under cross-highlight: the highlighted values, aligned with `rows` (None = no highlight).
    pub highlight: Option<Vec<Vec<Scalar>>>,
    /// Grand totals of the value columns (by column index; categories are Null).
    pub totals: Vec<Scalar>,
    /// Matrix: subtotal rows per row-level prefix length.
    pub subtotals: BTreeMap<usize, Vec<Vec<Scalar>>>,
    /// The drill level shown and how many levels the axis has.
    pub drill_level: usize,
    pub drill_levels: usize,
    pub error: Option<String>,
    /// True when the row cap cut the result.
    pub truncated: bool,
}

impl VisualData {
    pub fn error(msg: impl Into<String>) -> VisualData {
        VisualData { error: Some(msg.into()), ..VisualData::default() }
    }
    /// Indices of the columns from `role`.
    pub fn role(&self, role: &str) -> Vec<usize> {
        self.cols.iter().enumerate().filter(|(_, c)| c.role == role).map(|(i, _)| i).collect()
    }
    pub fn first(&self, role: &str) -> Option<usize> {
        self.cols.iter().position(|c| c.role == role)
    }
    pub fn values(&self) -> Vec<usize> {
        self.cols.iter().enumerate().filter(|(_, c)| c.is_value).map(|(i, _)| i).collect()
    }
    pub fn categories(&self) -> Vec<usize> {
        self.cols.iter().enumerate().filter(|(_, c)| !c.is_value).map(|(i, _)| i).collect()
    }
    pub fn num(&self, row: usize, col: usize) -> Option<f64> {
        self.rows.get(row)?.get(col)?.as_f64()
    }
    pub fn cell(&self, row: usize, col: usize) -> &Scalar {
        static NULL: Scalar = Scalar::Null;
        self.rows.get(row).and_then(|r| r.get(col)).unwrap_or(&NULL)
    }
    /// Display text of a cell, formatted with its column's format.
    pub fn text(&self, row: usize, col: usize) -> String {
        let v = self.cell(row, col);
        match (v, self.cols.get(col)) {
            (Scalar::Num(n), Some(c)) => format_value(*n, &c.format, c.col_type),
            (v, _) => v.text(),
        }
    }
    /// The selection identity of a row: its category values.
    pub fn identity(&self, row: usize, roles: &[&str]) -> Vec<(FieldRef, Scalar)> {
        self.cols
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.is_value && (roles.is_empty() || roles.contains(&c.role.as_str())))
            .map(|(i, c)| (c.field.clone(), self.cell(row, i).clone()))
            .collect()
    }
    /// Distinct values of a category column, in row order.
    pub fn distinct(&self, col: usize) -> Vec<Scalar> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for r in &self.rows {
            if let Some(v) = r.get(col)
                && seen.insert(v.key())
            {
                out.push(v.clone());
            }
        }
        out
    }
}

pub fn format_value(n: f64, fmt: &str, ty: ColType) -> String {
    if fmt.is_empty() {
        let df = ty.default_format();
        if !df.is_empty() {
            return crate::numfmt::format(n, df);
        }
        if n.abs() >= 1000.0 || n.fract() != 0.0 {
            return crate::numfmt::format(n, if n.abs() >= 100.0 { "#,0" } else { "#,0.##" });
        }
        return crate::numfmt::general(n);
    }
    crate::numfmt::format(n, fmt)
}

/// The order roles are grouped by (categories) and computed (values).
const CATEGORY_ROLES: &[&str] = &["axis", "rows", "columns", "legend", "breakdown", "details", "location", "field", "small", "source", "destination", "expand", "play"];
const VALUE_ROLES: &[&str] = &["values", "secondary", "x", "y", "size", "lat", "lon", "start", "target", "min", "max", "tooltips"];

/// Roles whose fields form a drillable hierarchy.
fn drill_role(kind: VisualKind) -> &'static str {
    match kind {
        VisualKind::Matrix => "rows",
        VisualKind::Pie | VisualKind::Donut => "legend",
        _ => "axis",
    }
}

pub const MAX_ROWS: usize = 30_000;

fn measure_format(p: &Project, built: &Built, fu: &FieldUse) -> (String, ColType) {
    match &fu.field {
        FieldRef::Measure { name, .. } => {
            let f = p.model.measures.iter().find(|m| &m.name == name).map(|m| m.format.clone()).unwrap_or_default();
            (f, ColType::Number)
        }
        f => {
            let (t, c) = field_column(p, built, f).unwrap_or_default();
            let fmt = p.model.tables.iter().find(|x| x.name == t).and_then(|x| x.columns.iter().find(|cp| cp.name == c)).map(|cp| cp.format.clone()).unwrap_or_default();
            let ty = built.col_type(&t, &c);
            let ty = match fu.agg {
                Agg::Count | Agg::DistinctCount => ColType::Integer,
                Agg::None | Agg::Min | Agg::Max | Agg::First | Agg::Last => ty,
                _ => {
                    if ty.is_temporal() {
                        ty
                    } else {
                        ColType::Number
                    }
                }
            };
            let fmt = if matches!(fu.agg, Agg::Count | Agg::DistinctCount) { String::new() } else { fmt };
            (fmt, ty)
        }
    }
}

/// The fields a visual groups by and computes, after drill and marks.
pub fn plan(p: &Project, built: &Built, spec: &VisualSpec) -> (Vec<(String, FieldUse)>, Vec<(String, FieldUse)>, Vec<String>, usize, usize) {
    let mut cats = Vec::new();
    let mut vals = Vec::new();
    let mut drill_filters = Vec::new();
    let dr = drill_role(spec.kind);
    let mut levels = 0;
    let mut level = 0;
    let marks: Vec<(&str, &Vec<FieldUse>)> = vec![
        ("legend", &spec.marks.color),
        ("size", &spec.marks.size),
        ("tooltips", &spec.marks.label),
        ("details", &spec.marks.detail),
        ("tooltips", &spec.marks.tooltip),
        ("details", &spec.marks.shape),
    ];
    for role in CATEGORY_ROLES.iter().chain(VALUE_ROLES.iter()) {
        let fields: Vec<FieldUse> = spec.well(role).iter().map(|f| resolve_use(p, f)).collect();
        if *role == dr && !fields.is_empty() && spec.kind != VisualKind::Table {
            // Expand user hierarchy levels and multiple fields into one hierarchy.
            let cat_fields: Vec<FieldUse> = fields.iter().filter(|f| !f.is_value()).cloned().collect();
            levels = cat_fields.len();
            level = spec.drill.level.min(levels.saturating_sub(1));
            if spec.kind == VisualKind::Matrix {
                // Matrix shows levels up to the expanded one.
                let upto = spec.drill.expand_to.max(level).min(levels.saturating_sub(1));
                for f in cat_fields.iter().take(upto + 1) {
                    cats.push((role.to_string(), f.clone()));
                }
            } else if spec.drill.expand_to > 0 {
                for f in cat_fields.iter().take(spec.drill.expand_to.min(levels.saturating_sub(1)) + 1) {
                    cats.push((role.to_string(), f.clone()));
                }
            } else if let Some(f) = cat_fields.get(level) {
                cats.push((role.to_string(), f.clone()));
                for (i, v) in spec.drill.path.iter().enumerate().take(level) {
                    if let Some(pf) = cat_fields.get(i)
                        && let Some((t, c)) = field_column(p, built, &pf.field)
                    {
                        drill_filters.push(format!("FILTER(ALL({}), {} = {})", col_ref(&t, &c), col_ref(&t, &c), v.dax()));
                    }
                }
            }
            for f in fields.iter().filter(|f| f.is_value()) {
                vals.push((role.to_string(), f.clone()));
            }
            continue;
        }
        for f in fields {
            if f.is_value() || VALUE_ROLES.contains(role) && !matches!(spec.kind, VisualKind::Table) && f.agg != Agg::None {
                vals.push((role.to_string(), f));
            } else if VALUE_ROLES.contains(role) && !f.is_value() && !matches!(spec.kind, VisualKind::Scatter | VisualKind::Gantt) {
                // A raw column in a value well: count it.
                let mut f2 = f.clone();
                f2.agg = crate::state::default_agg(p, built, f.field.table(), &f.field.label());
                vals.push((role.to_string(), f2));
            } else if VALUE_ROLES.contains(role) && !f.is_value() {
                // Scatter "values" detail / Gantt start: a category.
                cats.push((role.to_string(), f));
            } else {
                cats.push((role.to_string(), f));
            }
        }
    }
    for (role, fs) in marks {
        for f in fs {
            let f = resolve_use(p, f);
            if f.is_value() {
                vals.push((role.to_string(), f));
            } else {
                cats.push((role.to_string(), f));
            }
        }
    }
    (cats, vals, drill_filters, level, levels)
}

/// Runs a visual's query in a filter context.
pub fn run(p: &Project, built: &Built, spec: &VisualSpec, cx: &Context) -> VisualData {
    match spec.kind {
        VisualKind::KeyInfluencers | VisualKind::DecompositionTree | VisualKind::SmartNarrative => {
            if let Some(d) = crate::charts::ai::run(p, built, spec, cx) {
                return d;
            }
        }
        VisualKind::Matrix if !spec.expanded.is_empty() => {
            // Rows expanded with +/-: fetch the levels down to the deepest expanded row.
            let deepest = spec.expanded.iter().filter_map(|k| serde_json::from_str::<Vec<Scalar>>(k).ok()).map(|p| p.len()).max().unwrap_or(0);
            if deepest > spec.drill.expand_to {
                let mut s2 = spec.clone();
                s2.drill.expand_to = deepest;
                s2.expanded.clear();
                return run(p, built, &s2, cx);
            }
        }
        _ => {}
    }
    match spec.kind {
        VisualKind::Slicer => return slicer_values(p, built, spec, cx),
        VisualKind::QandA => {
            let (wells, _) = crate::charts::qna::interpret(p, built, &spec.question);
            let mut s2 = spec.clone();
            s2.kind = VisualKind::ClusteredColumn;
            s2.wells = wells;
            return run(p, built, &s2, cx);
        }
        _ => {}
    }
    let (cats, vals, drill_filters, level, levels) = plan(p, built, spec);
    if cats.is_empty() && vals.is_empty() {
        return VisualData::error("Add data fields to build this visual.");
    }
    let mut filters = cx.filters.clone();
    filters.extend(drill_filters);
    let mut value_filters = Vec::new();
    for (fu, kind) in &cx.value_filters {
        let on_visual = cats.iter().any(|(_, c)| c.field == fu.field);
        if let FilterKind::TopN { n, top, by: Some(by) } = kind
            && !on_visual
        {
            if let Some(f) = topn_filter(p, built, fu, *n, *top, by, &filters) {
                filters.push(f);
            }
            continue;
        }
        value_filters.push((fu.clone(), kind.clone()));
    }
    let mut out = match query(p, built, &cats, &vals, &filters) {
        Ok(d) => d,
        Err(e) => return VisualData::error(e),
    };
    out.drill_level = level;
    out.drill_levels = levels;
    // Value filters (visual-level filters on measures, Top N).
    for (fu, kind) in &value_filters {
        apply_value_filter(&mut out, fu, kind);
    }
    sort(&mut out, spec, p);
    // Totals.
    if !vals.is_empty() && !cats.is_empty() {
        if let Ok(t) = query(p, built, &[], &vals, &filters) {
            let mut totals = vec![Scalar::Null; cats.len()];
            totals.extend(t.rows.first().cloned().unwrap_or_default());
            out.totals = totals;
        }
    } else if cats.is_empty() {
        out.totals = out.rows.first().cloned().unwrap_or_default();
    }
    if spec.kind == VisualKind::Matrix {
        let nrows = cats.iter().filter(|(r, _)| r == "rows").count();
        let colcats: Vec<(String, FieldUse)> = cats.iter().filter(|(r, _)| r == "columns").cloned().collect();
        for k in 0..nrows {
            let mut sub: Vec<(String, FieldUse)> = cats.iter().filter(|(r, _)| r == "rows").take(k).cloned().collect();
            sub.extend(colcats.iter().cloned());
            if let Ok(d) = query(p, built, &sub, &vals, &filters) {
                out.subtotals.insert(k, d.rows);
            }
        }
        // Row totals without the column grouping.
        for k in 0..=nrows {
            let sub: Vec<(String, FieldUse)> = cats.iter().filter(|(r, _)| r == "rows").take(k).cloned().collect();
            if let Ok(d) = query(p, built, &sub, &vals, &filters) {
                out.subtotals.insert(100 + k, d.rows);
            }
        }
    }
    show_as(&mut out);
    if let Some(hl) = &cx.highlight {
        let mut hf = filters.clone();
        hf.extend(hl.iter().cloned());
        if let Ok(h) = query(p, built, &cats, &vals, &hf) {
            let ncat = cats.len();
            let index: std::collections::HashMap<String, &Vec<Scalar>> =
                h.rows.iter().map(|r| (r.iter().take(ncat).map(Scalar::key).collect::<Vec<_>>().join("\u{1}"), r)).collect();
            out.highlight = Some(
                out.rows
                    .iter()
                    .map(|r| {
                        let k = r.iter().take(ncat).map(Scalar::key).collect::<Vec<_>>().join("\u{1}");
                        match index.get(&k) {
                            Some(hr) => (*hr).clone(),
                            None => {
                                let mut z = r.iter().take(ncat).cloned().collect::<Vec<_>>();
                                z.extend(std::iter::repeat_n(Scalar::Null, r.len().saturating_sub(ncat)));
                                z
                            }
                        }
                    })
                    .collect(),
            );
        }
    }
    out
}

/// A group-by query: one row per combination of `cats` with `vals` computed.
pub fn query(p: &Project, built: &Built, cats: &[(String, FieldUse)], vals: &[(String, FieldUse)], filters: &[String]) -> Result<VisualData, String> {
    let m = &built.model;
    let mut group_by: Vec<(String, String)> = Vec::new();
    let mut cols = Vec::new();
    let mut sort_cols: Vec<(usize, usize)> = Vec::new(); // (category col index, group_by index of sort column)
    for (role, fu) in cats {
        let Some((t, c)) = field_column(p, built, &fu.field) else { return Err(format!("`{}` can't be used as a category here.", fu.label())) };
        let ty = built.col_type(&t, &c);
        let fmt = p.model.tables.iter().find(|x| x.name == t).and_then(|x| x.columns.iter().find(|cp| cp.name == c)).map(|cp| cp.format.clone()).unwrap_or_default();
        cols.push(DataCol { label: fu.label(), role: role.clone(), field: fu.field.clone(), is_value: false, format: fmt, col_type: ty, show_as: ShowAs::Value });
        group_by.push((t.clone(), c.clone()));
        // Sort-by column (and month names sort by month number).
        let sort_by = p.model.tables.iter().find(|x| x.name == t).and_then(|x| x.columns.iter().find(|cp| cp.name == c)).and_then(|cp| cp.sort_by.clone()).or_else(|| {
            c.strip_suffix(".Month").map(|base| crate::build::date_level_column(base, "MonthNo"))
        });
        if let Some(sb) = sort_by
            && m.column_ref(&t, &sb).is_ok()
        {
            sort_cols.push((cols.len() - 1, usize::MAX));
            group_by.push((t.clone(), sb));
            if let Some(last) = sort_cols.last_mut() {
                last.1 = group_by.len() - 1;
            }
        }
    }
    let mut measures: Vec<(String, String)> = Vec::new();
    for (i, (role, fu)) in vals.iter().enumerate() {
        let Some(dax) = value_dax(p, built, fu) else { return Err(format!("`{}` has no aggregation.", fu.label())) };
        let (fmt, ty) = measure_format(p, built, fu);
        cols.push(DataCol { label: fu.label(), role: role.clone(), field: fu.field.clone(), is_value: true, format: fmt, col_type: ty, show_as: fu.show_as });
        measures.push((format!("__v{i}"), dax));
    }
    let no_values = measures.is_empty();
    if no_values && !group_by.is_empty() {
        // Keep only combinations that exist: count rows of the table that relates to the others.
        let fact = fact_table(p, &group_by);
        measures.push(("__rows".into(), format!("COUNTROWS({})", vizardcraft_datamodel::model::quote_table(&fact))));
    }
    let res = m.summarize(&group_by, &measures, filters).map_err(|e| e.to_string())?;
    // Map result columns back: group_by columns come first, then measures.
    let ng = group_by.len();
    let mut rows = Vec::with_capacity(res.rows.len());
    let mut sort_keys: Vec<Vec<Scalar>> = Vec::new();
    let mut truncated = false;
    for r in res.rows.iter() {
        if rows.len() >= MAX_ROWS {
            truncated = true;
            break;
        }
        let mut out = Vec::with_capacity(cols.len());
        let mut keys = Vec::new();
        let mut gi = 0;
        for (ci, c) in cols.iter().enumerate() {
            if c.is_value {
                continue;
            }
            out.push(r.get(gi).map(Scalar::from_dm).unwrap_or_default());
            gi += 1;
            if let Some((_, si)) = sort_cols.iter().find(|(x, _)| *x == ci) {
                keys.push(r.get(*si).map(Scalar::from_dm).unwrap_or_default());
                gi += 1;
            } else {
                keys.push(out.last().cloned().unwrap_or_default());
            }
        }
        if !no_values {
            for k in 0..vals.len() {
                out.push(r.get(ng + k).map(Scalar::from_dm).unwrap_or_default());
            }
        }
        rows.push(out);
        sort_keys.push(keys);
    }
    // Natural order: by category sort keys.
    let mut idx: Vec<usize> = (0..rows.len()).collect();
    idx.sort_by(|a, b| {
        let ka = sort_keys.get(*a);
        let kb = sort_keys.get(*b);
        match (ka, kb) {
            (Some(x), Some(y)) => {
                for (u, v) in x.iter().zip(y.iter()) {
                    let o = compare(u, v);
                    if o != std::cmp::Ordering::Equal {
                        return o;
                    }
                }
                std::cmp::Ordering::Equal
            }
            _ => std::cmp::Ordering::Equal,
        }
    });
    let rows: Vec<Vec<Scalar>> = idx.into_iter().filter_map(|i| rows.get(i).cloned()).collect();
    // Bins: group numeric categories into bins and add values.
    let mut data = VisualData { cols, rows, truncated, ..VisualData::default() };
    for (ci, (_, fu)) in cats.iter().enumerate() {
        if let Some(w) = fu.bin.filter(|w| *w > 0.0) {
            bin_column(&mut data, ci, w);
        }
    }
    Ok(data)
}

fn bin_column(d: &mut VisualData, ci: usize, width: f64) {
    let mut groups: BTreeMap<String, (Vec<Scalar>, Vec<f64>)> = BTreeMap::new();
    let mut order = Vec::new();
    for r in &d.rows {
        let mut key_row: Vec<Scalar> = Vec::new();
        for (i, c) in d.cols.iter().enumerate() {
            if c.is_value {
                continue;
            }
            let v = r.get(i).cloned().unwrap_or_default();
            key_row.push(if i == ci { v.as_f64().map(|x| Scalar::Num((x / width).floor() * width)).unwrap_or(Scalar::Null) } else { v });
        }
        let k = key_row.iter().map(Scalar::key).collect::<Vec<_>>().join("\u{1}");
        let e = groups.entry(k.clone()).or_insert_with(|| {
            order.push(k.clone());
            (key_row.clone(), vec![0.0; d.cols.iter().filter(|c| c.is_value).count()])
        });
        for (vi, (i, _)) in d.cols.iter().enumerate().filter(|(_, c)| c.is_value).enumerate() {
            if let Some(slot) = e.1.get_mut(vi) {
                *slot += r.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0);
            }
        }
    }
    let mut rows: Vec<Vec<Scalar>> = order
        .iter()
        .filter_map(|k| groups.get(k))
        .map(|(cats, vals)| {
            let mut r = cats.clone();
            r.extend(vals.iter().map(|v| Scalar::Num(*v)));
            r
        })
        .collect();
    rows.sort_by(|a, b| compare(a.get(ci).unwrap_or(&Scalar::Null), b.get(ci).unwrap_or(&Scalar::Null)));
    if let Some(c) = d.cols.get_mut(ci) {
        c.label = format!("{} (bins)", c.label);
    }
    d.rows = rows;
}

/// The table to count rows of when a visual has only categories: the one on the many side of
/// relationships to the others.
fn fact_table(p: &Project, group_by: &[(String, String)]) -> String {
    let tables: Vec<&String> = group_by.iter().map(|(t, _)| t).collect();
    let first = tables.first().map(|s| s.to_string()).unwrap_or_default();
    if tables.iter().all(|t| **t == first) {
        return first;
    }
    // A table with relationships to all involved tables (or the most).
    let candidates: Vec<String> = p.model.tables.iter().map(|t| t.name.clone()).collect();
    candidates
        .into_iter()
        .max_by_key(|c| {
            tables
                .iter()
                .filter(|t| {
                    let t: &str = t.as_str();
                    t == c.as_str() || p.model.relationships.iter().any(|r| r.active && r.from_table == *c && r.to_table == t)
                })
                .count()
        })
        .unwrap_or(first)
}

/// A Top N filter on a field the visual doesn't show becomes a filter on the top values.
fn topn_filter(p: &Project, built: &Built, fu: &FieldUse, n: usize, top: bool, by: &FieldUse, filters: &[String]) -> Option<String> {
    let (t, c) = field_column(p, built, &fu.field)?;
    let r = query(p, built, &[("axis".into(), FieldUse::new(fu.field.clone()))], &[("values".into(), by.clone())], filters).ok()?;
    let mut rows = r.rows;
    rows.sort_by(|a, b| {
        let o = compare(a.get(1).unwrap_or(&Scalar::Null), b.get(1).unwrap_or(&Scalar::Null));
        if top { o.reverse() } else { o }
    });
    let keep: Vec<String> = rows.iter().take(n).filter_map(|r| r.first().map(Scalar::dax)).collect();
    Some(format!("FILTER(ALL({}), {} IN {{{}}})", col_ref(&t, &c), col_ref(&t, &c), keep.join(", ")))
}

fn apply_value_filter(d: &mut VisualData, fu: &FieldUse, kind: &FilterKind) {
    match kind {
        FilterKind::TopN { n, top, by } => {
            // Top N of the category field by `by`.
            let Some(ci) = d.cols.iter().position(|c| !c.is_value && c.field == fu.field) else { return };
            let vi = match by {
                Some(b) => d.cols.iter().position(|c| c.is_value && c.label == b.label()).or_else(|| d.values().first().copied()),
                None => d.values().first().copied(),
            };
            let Some(vi) = vi else { return };
            // Rank distinct category values by their total.
            let mut totals: BTreeMap<String, (Scalar, f64)> = BTreeMap::new();
            for r in &d.rows {
                let k = r.get(ci).cloned().unwrap_or_default();
                let e = totals.entry(k.key()).or_insert((k, 0.0));
                e.1 += r.get(vi).and_then(|v| v.as_f64()).unwrap_or(0.0);
            }
            let mut ranked: Vec<(Scalar, f64)> = totals.into_values().collect();
            ranked.sort_by(|a, b| {
                let o = a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal);
                if *top { o.reverse() } else { o }
            });
            let keep: std::collections::HashSet<String> = ranked.iter().take(*n).map(|(k, _)| k.key()).collect();
            d.rows.retain(|r| r.get(ci).is_some_and(|v| keep.contains(&v.key())));
        }
        other => {
            let Some(vi) = d.cols.iter().position(|c| c.is_value && c.field == fu.field) else { return };
            d.rows.retain(|r| value_passes(r.get(vi).unwrap_or(&Scalar::Null), other));
        }
    }
}

pub fn value_passes(v: &Scalar, kind: &FilterKind) -> bool {
    match kind {
        FilterKind::Advanced { conditions, and } => {
            let res: Vec<bool> = conditions.iter().filter(|c| !c.op.is_empty()).map(|c| cond_holds(v, c)).collect();
            if res.is_empty() {
                true
            } else if *and {
                res.iter().all(|b| *b)
            } else {
                res.iter().any(|b| *b)
            }
        }
        FilterKind::Range { min, max } => {
            let x = v.as_f64().unwrap_or(f64::NAN);
            min.is_none_or(|m| x >= m) && max.is_none_or(|m| x <= m)
        }
        FilterKind::Basic { values, exclude } => values.is_empty() || values.iter().any(|x| x.key() == v.key()) != *exclude,
        _ => true,
    }
}

fn cond_holds(v: &Scalar, c: &Condition) -> bool {
    let x = v.as_f64();
    let y = c.value.as_f64();
    let t = v.text().to_lowercase();
    let u = c.value.text().to_lowercase();
    match c.op.as_str() {
        "is" => v.key() == c.value.key() || x.is_some() && x == y,
        "is not" => !(v.key() == c.value.key() || x.is_some() && x == y),
        "is less than" => matches!((x, y), (Some(a), Some(b)) if a < b),
        "is less than or equal to" => matches!((x, y), (Some(a), Some(b)) if a <= b),
        "is greater than" => matches!((x, y), (Some(a), Some(b)) if a > b),
        "is greater than or equal to" => matches!((x, y), (Some(a), Some(b)) if a >= b),
        "contains" => t.contains(&u),
        "does not contain" => !t.contains(&u),
        "starts with" => t.starts_with(&u),
        "does not start with" => !t.starts_with(&u),
        "is blank" | "is empty" => v.is_null(),
        "is not blank" | "is not empty" => !v.is_null(),
        _ => true,
    }
}

fn sort(d: &mut VisualData, spec: &VisualSpec, _p: &Project) {
    let by = spec.sort.as_ref();
    let col = match by {
        Some(s) => d.cols.iter().position(|c| c.label == s.by),
        None => {
            // Default: bar/column/pie/treemap/funnel by first value descending when the axis is text.
            use VisualKind::*;
            let text_axis = d.first("axis").or_else(|| d.first("legend")).and_then(|i| d.cols.get(i)).is_some_and(|c| !c.col_type.is_numeric() && !c.col_type.is_temporal() && !matches!(c.field, FieldRef::Level { .. }));
            let has_legend = matches!(spec.kind, StackedBar | StackedColumn | StackedBar100 | StackedColumn100) && !spec.well("legend").is_empty();
            let auto = matches!(spec.kind, StackedBar | ClusteredBar | StackedColumn | ClusteredColumn | Pie | Donut | Treemap | Funnel | PackedBubbles | WordCloud | Bullet) && text_axis && !has_legend;
            let is_month = d.first("axis").and_then(|i| d.cols.get(i)).is_some_and(|c| matches!(&c.field, FieldRef::Level { level, .. } if level == "Month" || level == "Quarter"));
            if auto && !is_month { d.values().first().copied() } else { None }
        }
    };
    let Some(ci) = col else { return };
    let desc = by.map(|s| s.descending).unwrap_or(true);
    let is_value = d.cols.get(ci).is_some_and(|c| c.is_value);
    if is_value && d.categories().len() > 1 {
        // Sort categories by their total, keep rows of a category together.
        let Some(cat) = d.categories().first().copied() else { return };
        let mut totals: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        for r in &d.rows {
            *totals.entry(r.get(cat).map(Scalar::key).unwrap_or_default()).or_default() += r.get(ci).and_then(|v| v.as_f64()).unwrap_or(0.0);
        }
        d.rows.sort_by(|a, b| {
            let ta = totals.get(&a.get(cat).map(Scalar::key).unwrap_or_default()).copied().unwrap_or(0.0);
            let tb = totals.get(&b.get(cat).map(Scalar::key).unwrap_or_default()).copied().unwrap_or(0.0);
            let o = ta.partial_cmp(&tb).unwrap_or(std::cmp::Ordering::Equal);
            if desc { o.reverse() } else { o }
        });
    } else {
        d.rows.sort_by(|a, b| {
            let o = compare(a.get(ci).unwrap_or(&Scalar::Null), b.get(ci).unwrap_or(&Scalar::Null));
            if desc { o.reverse() } else { o }
        });
    }
}

fn show_as(d: &mut VisualData) {
    let vals = d.values();
    for vi in vals {
        let Some(mode) = d.cols.get(vi).map(|c| c.show_as) else { continue };
        if mode == ShowAs::Value {
            continue;
        }
        let total = d.totals.get(vi).and_then(|v| v.as_f64()).unwrap_or_else(|| d.rows.iter().filter_map(|r| r.get(vi)?.as_f64()).sum());
        let mut running = 0.0;
        let mut prev: Option<f64> = None;
        let mut ranked: Vec<(usize, f64)> = d.rows.iter().enumerate().map(|(i, r)| (i, r.get(vi).and_then(|v| v.as_f64()).unwrap_or(f64::NEG_INFINITY))).collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let rank: std::collections::HashMap<usize, usize> = ranked.iter().enumerate().map(|(r, (i, _))| (*i, r + 1)).collect();
        for (ri, r) in d.rows.iter_mut().enumerate() {
            let Some(slot) = r.get_mut(vi) else { continue };
            let x = slot.as_f64();
            *slot = match (mode, x) {
                (ShowAs::PercentOfGrandTotal | ShowAs::PercentOfColumnTotal, Some(x)) if total != 0.0 => Scalar::Num(x / total),
                (ShowAs::RunningTotal, Some(x)) => {
                    running += x;
                    Scalar::Num(running)
                }
                (ShowAs::DifferenceFromPrevious, Some(x)) => {
                    let out = prev.map(|p| Scalar::Num(x - p)).unwrap_or(Scalar::Null);
                    prev = Some(x);
                    out
                }
                (ShowAs::PercentDifferenceFromPrevious, Some(x)) => {
                    let out = prev.filter(|p| *p != 0.0).map(|p| Scalar::Num((x - p) / p.abs())).unwrap_or(Scalar::Null);
                    prev = Some(x);
                    out
                }
                (ShowAs::Rank, Some(_)) => Scalar::Num(rank.get(&ri).copied().unwrap_or(0) as f64),
                _ => slot.clone(),
            };
        }
        if let Some(c) = d.cols.get_mut(vi) {
            match mode {
                ShowAs::PercentOfGrandTotal | ShowAs::PercentOfColumnTotal | ShowAs::PercentDifferenceFromPrevious => c.format = "0.0%".into(),
                ShowAs::Rank => c.format = "0".into(),
                _ => {}
            }
        }
    }
}

/// The values a slicer offers (respecting other filters on the page, like Power BI's
/// "show items with no data" off), with counts.
fn slicer_values(p: &Project, built: &Built, spec: &VisualSpec, cx: &Context) -> VisualData {
    let Some(fu) = spec.well("field").first() else { return VisualData::error("Add a field to the slicer.") };
    let fu = resolve_use(p, fu);
    let cats = vec![("field".to_string(), FieldUse::new(fu.field.clone()))];
    match query(p, built, &cats, &[], &cx.filters) {
        Ok(d) => d,
        Err(e) => VisualData::error(e),
    }
}

/// Distinct values of a column (for filter cards), unfiltered.
pub fn distinct_values(p: &Project, built: &Built, f: &FieldRef, limit: usize) -> Vec<Scalar> {
    let cats = vec![("field".to_string(), FieldUse::new(f.clone()))];
    match query(p, built, &cats, &[], &[]) {
        Ok(d) => d.rows.into_iter().filter_map(|r| r.into_iter().next()).take(limit).collect(),
        Err(_) => vec![],
    }
}
