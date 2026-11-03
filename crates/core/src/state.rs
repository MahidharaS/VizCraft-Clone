//! Runtime interaction state (cross-filter selection, navigation, drill-through, "view as") and
//! the filter context it produces for each element: report, page and visual filters, slicers
//! (including synced slicers), cross-filtering/highlighting from other visuals (respecting
//! "Edit interactions"), drill-through filters and row-level security.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::build::{Built, ColType, date_level_column};
use crate::doc::*;
use crate::scalar::Scalar;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Runtime {
    pub page: u64,
    pub selection: Option<Selection>,
    /// Pages visited (for the Back action).
    pub back_stack: Vec<u64>,
    /// Drill-through filters applied to a page (page id → field = value).
    pub drillthrough: BTreeMap<u64, Vec<(FieldRef, Scalar)>>,
    /// "View as" role.
    pub role: Option<String>,
    /// Focus mode / spotlight.
    pub focus: Option<u64>,
    pub spotlight: Option<u64>,
    /// Hovered element (for hover actions).
    pub hover: Option<u64>,
}

/// The filter context of one element.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Context {
    /// DAX filter arguments (as CALCULATE takes them).
    pub filters: Vec<String>,
    /// Filters on values (measures/aggregates), applied to the visual's result rows.
    pub value_filters: Vec<(FieldUse, FilterKind)>,
    /// When set, the visual shows these extra filters as a highlight over the full values.
    pub highlight: Option<Vec<String>>,
    /// Human-readable list of what filters the element ("Region is East").
    pub description: Vec<String>,
}

/// `'Table'[Column]`.
pub fn col_ref(table: &str, column: &str) -> String {
    format!("{}[{}]", vizcraft_datamodel::model::quote_table(table), column.replace(']', "]]"))
}

/// The model column a categorical field groups by.
pub fn field_column(p: &Project, built: &Built, f: &FieldRef) -> Option<(String, String)> {
    match f {
        FieldRef::Column { table, column } => Some((table.clone(), column.clone())),
        FieldRef::Level { table, hierarchy, level } => {
            if built.col_type(table, hierarchy).is_temporal() || crate::build::DATE_LEVELS.contains(&level.as_str()) && built.table(table).is_some_and(|t| t.column_index(hierarchy).is_some()) {
                Some((table.clone(), date_level_column(hierarchy, level)))
            } else {
                let _ = p;
                Some((table.clone(), level.clone()))
            }
        }
        FieldRef::Measure { .. } => None,
        FieldRef::FieldParam { name } => resolve_field_param(p, name).and_then(|r| field_column(p, built, &r)),
    }
}

/// The field a field parameter currently stands for.
pub fn resolve_field_param(p: &Project, name: &str) -> Option<FieldRef> {
    let prm = p.parameter(name)?;
    let ParameterKind::Fields { items } = &prm.kind else { return None };
    // A slicer on the parameter table picks the item.
    let selected = p.report.pages.iter().flat_map(|pg| pg.elements.iter()).find_map(|e| {
        let v = e.visual()?;
        if v.kind != VisualKind::Slicer {
            return None;
        }
        let f = v.well("field").first()?;
        match &f.field {
            FieldRef::Column { table, .. } if table.eq_ignore_ascii_case(name) => v.selection.first().cloned(),
            _ => None,
        }
    });
    let idx = match selected {
        Some(Scalar::Text(t)) => items.iter().position(|i| i.label() == t),
        _ => prm.current.as_f64().map(|x| x as usize),
    };
    items.get(idx.unwrap_or(0)).or(items.first()).cloned()
}

/// Replaces field parameters in a field use with the field they stand for.
pub fn resolve_use(p: &Project, fu: &FieldUse) -> FieldUse {
    if let FieldRef::FieldParam { name } = &fu.field
        && let Some(f) = resolve_field_param(p, name)
    {
        let mut out = fu.clone();
        out.field = f;
        if out.field.is_measure() {
            out.agg = Agg::None;
        } else if out.agg == Agg::None && fu.agg != Agg::None {
            out.agg = fu.agg;
        }
        return out;
    }
    fu.clone()
}

/// The DAX expression that computes a value field.
pub fn value_dax(p: &Project, built: &Built, fu: &FieldUse) -> Option<String> {
    let fu = resolve_use(p, fu);
    match &fu.field {
        FieldRef::Measure { name, .. } => Some(format!("[{}]", name.replace(']', "]]"))),
        other => {
            let (t, c) = field_column(p, built, other)?;
            let r = col_ref(&t, &c);
            let numeric = built.col_type(&t, &c).is_numeric() || built.col_type(&t, &c) == ColType::Any;
            Some(match fu.agg {
                Agg::None => return None,
                Agg::Sum => format!("SUM({r})"),
                Agg::Average => format!("AVERAGE({r})"),
                Agg::Min => format!("MIN({r})"),
                Agg::Max => format!("MAX({r})"),
                Agg::Count => format!("COUNTA({r})"),
                Agg::DistinctCount => format!("DISTINCTCOUNT({r})"),
                Agg::Median => format!("MEDIAN({r})"),
                Agg::StdDev => format!("STDEV.P({r})"),
                Agg::Variance => format!("VAR.P({r})"),
                Agg::First => {
                    if numeric {
                        format!("MIN({r})")
                    } else {
                        format!("FIRSTNONBLANK({r})")
                    }
                }
                Agg::Last => {
                    if numeric {
                        format!("MAX({r})")
                    } else {
                        format!("LASTNONBLANK({r})")
                    }
                }
            })
        }
    }
}

/// The default aggregation for a column dragged into a value well.
pub fn default_agg(p: &Project, built: &Built, table: &str, column: &str) -> Agg {
    if let Some(cp) = p.model.tables.iter().find(|t| t.name == table).and_then(|t| t.columns.iter().find(|c| c.name == column))
        && !cp.summarize_by.is_empty()
    {
        return Agg::from_label(&cp.summarize_by);
    }
    match built.col_type(table, column) {
        ColType::Number | ColType::Integer => Agg::Sum,
        ColType::Date | ColType::DateTime => Agg::Min,
        _ => Agg::Count,
    }
}

fn list(vals: &[Scalar]) -> String {
    vals.iter().map(Scalar::dax).collect::<Vec<_>>().join(", ")
}

fn cond_dax(r: &str, c: &Condition) -> String {
    let v = c.value.dax();
    match c.op.as_str() {
        "is" => format!("{r} = {v}"),
        "is not" => format!("{r} <> {v}"),
        "is less than" => format!("{r} < {v}"),
        "is less than or equal to" => format!("{r} <= {v}"),
        "is greater than" => format!("{r} > {v}"),
        "is greater than or equal to" => format!("{r} >= {v}"),
        "contains" => format!("CONTAINSSTRING({r}, {v})"),
        "does not contain" => format!("NOT(CONTAINSSTRING({r}, {v}))"),
        "starts with" => format!("LEFT({r}, LEN({v})) = {v}"),
        "does not start with" => format!("LEFT({r}, LEN({v})) <> {v}"),
        "is blank" | "is empty" => format!("ISBLANK({r})"),
        "is not blank" | "is not empty" => format!("NOT(ISBLANK({r}))"),
        _ => "TRUE()".into(),
    }
}

/// Overrides "today" (serial date bits; 0 = not set). The web viewer sets it from JavaScript:
/// `wasm32-unknown-unknown` has no clock.
static TODAY_OVERRIDE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Sets the date [`today_serial`] returns (`None` = the system clock).
pub fn set_today_serial(serial: Option<f64>) {
    TODAY_OVERRIDE.store(serial.map(f64::to_bits).unwrap_or(0), std::sync::atomic::Ordering::Relaxed);
}

/// Today as a serial date.
pub fn today_serial() -> f64 {
    let bits = TODAY_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed);
    if bits != 0 {
        return f64::from_bits(bits);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let today = chrono::Local::now().date_naive();
        vizcraft_datamodel::dates::to_serial(today).unwrap_or(45000.0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        45000.0
    }
}

fn relative_range(mode: &str, n: i64, unit: &str, include_today: bool) -> (f64, f64) {
    let t = today_serial();
    let days = |u: &str| -> f64 {
        match u.to_ascii_lowercase().trim_end_matches('s') {
            "week" => 7.0,
            "month" | "calendar month" => 30.4375,
            "quarter" => 91.3125,
            "year" | "calendar year" => 365.25,
            _ => 1.0,
        }
    };
    let span = days(unit) * n.max(0) as f64;
    let end_inc = if include_today { 1.0 } else { 0.0 };
    match mode {
        "Next" => (t + 1.0 - end_inc, t + 1.0 + span),
        "This" => {
            let d = days(unit);
            (t - (t % d.max(1.0)), t - (t % d.max(1.0)) + d)
        }
        _ => (t - span + 1.0 - (1.0 - end_inc), t + end_inc),
    }
}

/// A column filter as a DAX filter argument; `None` when it doesn't filter.
pub fn filter_dax(p: &Project, built: &Built, f: &FilterDef) -> Option<String> {
    let (t, c) = field_column(p, built, &resolve_use(p, &f.field).field)?;
    let r = col_ref(&t, &c);
    let all = format!("ALL({r})");
    let body = match &f.kind {
        FilterKind::Basic { values, exclude } => {
            if values.is_empty() {
                return None;
            }
            if *exclude { format!("NOT({r} IN {{{}}})", list(values)) } else { format!("{r} IN {{{}}}", list(values)) }
        }
        FilterKind::Advanced { conditions, and } => {
            let parts: Vec<String> = conditions.iter().filter(|c| !c.op.is_empty()).map(|c| cond_dax(&r, c)).collect();
            if parts.is_empty() {
                return None;
            }
            parts.join(if *and { " && " } else { " || " })
        }
        FilterKind::Range { min, max } => match (min, max) {
            (None, None) => return None,
            (Some(a), None) => format!("{r} >= {a}"),
            (None, Some(b)) => format!("{r} <= {b}"),
            (Some(a), Some(b)) => format!("{r} >= {a} && {r} <= {b}"),
        },
        FilterKind::RelativeDate { mode, n, unit, include_today } => {
            let (a, b) = relative_range(mode, *n, unit, *include_today);
            format!("{r} >= {a} && {r} < {b}")
        }
        FilterKind::TopN { .. } => return None,
        FilterKind::Dax { expression } => {
            if expression.trim().is_empty() {
                return None;
            }
            return Some(expression.clone());
        }
    };
    Some(format!("FILTER({all}, {body})"))
}

fn describe(p: &Project, f: &FilterDef) -> Option<String> {
    let name = resolve_use(p, &f.field).label();
    match &f.kind {
        FilterKind::Basic { values, exclude } if !values.is_empty() => {
            let vs: Vec<String> = values.iter().take(4).map(Scalar::text).collect();
            let more = if values.len() > 4 { format!(" and {} more", values.len() - 4) } else { String::new() };
            Some(format!("{name} {} {}{more}", if *exclude { "is not" } else { "is" }, vs.join(", ")))
        }
        FilterKind::Advanced { conditions, .. } if !conditions.is_empty() => {
            Some(format!("{name} {}", conditions.iter().map(|c| format!("{} {}", c.op, c.value.text())).collect::<Vec<_>>().join(" / ")))
        }
        FilterKind::TopN { n, top, .. } => Some(format!("{} {n} {name}", if *top { "Top" } else { "Bottom" })),
        FilterKind::Range { min, max } if min.is_some() || max.is_some() => {
            Some(format!("{name} between {} and {}", min.map(crate::numfmt::general).unwrap_or_default(), max.map(crate::numfmt::general).unwrap_or_default()))
        }
        FilterKind::RelativeDate { mode, n, unit, .. } => Some(format!("{name} in the {} {n} {unit}", mode.to_lowercase())),
        FilterKind::Dax { expression } if !expression.is_empty() => Some(expression.clone()),
        _ => None,
    }
}

/// Visual kinds that cross-highlight (others are cross-filtered).
pub fn supports_highlight(k: VisualKind) -> bool {
    use VisualKind::*;
    matches!(k, StackedBar | StackedColumn | ClusteredBar | ClusteredColumn | StackedBar100 | StackedColumn100 | Pie | Donut | Treemap | Funnel | Waterfall | Ribbon | LineStackedColumn | LineClusteredColumn)
}

fn selection_filters(p: &Project, built: &Built, sel: &Selection) -> Vec<String> {
    // Group the selected values per column; each column gets an IN filter.
    let mut per: BTreeMap<(String, String), Vec<Scalar>> = BTreeMap::new();
    for point in &sel.points {
        for (f, v) in point {
            if let Some(tc) = field_column(p, built, f) {
                let e = per.entry(tc).or_default();
                if !e.contains(v) {
                    e.push(v.clone());
                }
            }
        }
    }
    per.into_iter().map(|((t, c), vals)| format!("FILTER(ALL({}), {} IN {{{}}})", col_ref(&t, &c), col_ref(&t, &c), list(&vals))).collect()
}

/// The filter a slicer applies (from its selection or range).
pub fn slicer_filter(p: &Project, built: &Built, v: &VisualSpec) -> Option<(FilterDef, String)> {
    let fu = v.well("field").first()?;
    let kind = if let Some((a, b)) = v.range {
        FilterKind::Range { min: Some(a), max: Some(b) }
    } else if !v.selection.is_empty() {
        FilterKind::Basic { values: v.selection.clone(), exclude: false }
    } else {
        return None;
    };
    let f = FilterDef { id: 0, field: fu.clone(), kind, locked: false, hidden: false, require_single: false, context: false };
    // Slicers on a field parameter's table pick fields, they don't filter data.
    if let FieldRef::Column { table, .. } = &fu.field
        && p.parameter(table).is_some_and(|prm| matches!(prm.kind, ParameterKind::Fields { .. }))
    {
        return None;
    }
    let d = filter_dax(p, built, &f)?;
    Some((f, d))
}

fn interaction(target: &Element, source: u64) -> Interaction {
    target.interactions.get(&source).copied().unwrap_or_default()
}

/// The filter context for `element` on `page`.
pub fn context_for(p: &Project, built: &Built, rt: &Runtime, page: &Page, element: &Element) -> Context {
    let mut cx = Context::default();
    let push = |cx: &mut Context, f: &FilterDef| {
        if let FilterKind::TopN { n, top, by } = &f.kind {
            cx.value_filters.push((f.field.clone(), FilterKind::TopN { n: *n, top: *top, by: by.clone() }));
            if let Some(d) = describe(p, f) {
                cx.description.push(d);
            }
            return;
        }
        if resolve_use(p, &f.field).is_value() {
            cx.value_filters.push((f.field.clone(), f.kind.clone()));
            if let Some(d) = describe(p, f) {
                cx.description.push(d);
            }
            return;
        }
        if let Some(d) = filter_dax(p, built, f) {
            cx.filters.push(d);
            if let Some(d) = describe(p, f) {
                cx.description.push(d);
            }
        }
    };
    for f in &p.report.filters {
        push(&mut cx, f);
    }
    for f in &page.filters {
        push(&mut cx, f);
    }
    if let Some(v) = element.visual() {
        for f in &v.filters {
            push(&mut cx, f);
        }
    }
    // Slicers on this page.
    for s in &page.elements {
        if s.id == element.id || s.hidden {
            continue;
        }
        let Some(sv) = s.visual() else { continue };
        if sv.kind != VisualKind::Slicer {
            continue;
        }
        if interaction(element, s.id) == Interaction::None {
            continue;
        }
        if let Some((f, d)) = slicer_filter(p, built, sv) {
            cx.filters.push(d);
            if let Some(t) = describe(p, &f) {
                cx.description.push(t);
            }
        }
    }
    // Synced slicers on other pages.
    for other in p.report.pages.iter().filter(|o| o.id != page.id) {
        for s in &other.elements {
            let (Some(group), Some(sv)) = (&s.sync_group, s.visual()) else { continue };
            if sv.kind != VisualKind::Slicer || page.elements.iter().any(|e| e.sync_group.as_ref() == Some(group)) {
                continue;
            }
            if let Some((_, d)) = slicer_filter(p, built, sv) {
                cx.filters.push(d);
            }
        }
    }
    // Drill-through.
    if let Some(dt) = rt.drillthrough.get(&page.id) {
        for (f, v) in dt {
            if let Some((t, c)) = field_column(p, built, f) {
                cx.filters.push(format!("FILTER(ALL({}), {} = {})", col_ref(&t, &c), col_ref(&t, &c), v.dax()));
                cx.description.push(format!("{} is {}", f.label(), v.text()));
            }
        }
    }
    // Row-level security.
    if let Some(role) = rt.role.as_ref().and_then(|r| p.model.roles.iter().find(|x| &x.name == r)) {
        for (t, dax) in &role.filters {
            if !dax.trim().is_empty() {
                cx.filters.push(format!("FILTER({}, {dax})", vizcraft_datamodel::model::quote_table(t)));
            }
        }
    }
    // Cross-filter / cross-highlight from another visual on the page.
    if let Some(sel) = &rt.selection
        && sel.source != element.id
        && page.element(sel.source).is_some()
    {
        let fs = selection_filters(p, built, sel);
        let kind = element.visual().map(|v| v.kind).unwrap_or_default();
        match interaction(element, sel.source) {
            Interaction::None => {}
            Interaction::Filter => cx.filters.extend(fs),
            Interaction::Highlight => cx.highlight = Some(fs),
            Interaction::Default => {
                if p.report.settings.cross_highlight && supports_highlight(kind) {
                    cx.highlight = Some(fs);
                } else {
                    cx.filters.extend(fs);
                }
            }
        }
    }
    cx
}

/// Clicking a data point: toggles it in the selection (ctrl adds to it).
pub fn toggle_selection(rt: &mut Runtime, source: u64, point: Vec<(FieldRef, Scalar)>, additive: bool) {
    match &mut rt.selection {
        Some(sel) if sel.source == source => {
            if let Some(i) = sel.points.iter().position(|p| *p == point) {
                sel.points.remove(i);
                if sel.points.is_empty() {
                    rt.selection = None;
                }
            } else if additive {
                sel.points.push(point);
            } else {
                sel.points = vec![point];
            }
        }
        _ => rt.selection = Some(Selection { source, points: vec![point] }),
    }
}

/// Whether a data point (by its identity) is in the current selection of `source`.
pub fn is_selected(rt: &Runtime, source: u64, point: &[(FieldRef, Scalar)]) -> Option<bool> {
    let sel = rt.selection.as_ref()?;
    if sel.source != source {
        return None;
    }
    Some(sel.points.iter().any(|p| p.as_slice() == point))
}
