//! The copilot's tools: read and edit the open project. Each tool takes JSON arguments and
//! returns a JSON-ish text for the model plus a one-line summary for the trace.
//!
//! Tools run where the project lives (the app's UI thread, via a channel; tests call
//! [`execute`] directly). Mutating tools are flagged so the app can ask before applying them.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use vizcraft_core::build::{self, Built, ColType};
use vizcraft_core::doc::*;
use vizcraft_core::scalar::Scalar;
use vizcraft_core::state::{self, Runtime};

use crate::provider::{ToolCall, ToolSpec};

/// The project as the tools see it.
pub struct Workspace<'a> {
    pub project: &'a mut Project,
    pub built: &'a mut Built,
    /// The page the user is looking at (create_visual puts visuals there).
    pub page: &'a mut u64,
    /// The selected element, if any ("selected" in set_format / explain_visual).
    pub selected: Option<u64>,
    pub max_rows: usize,
    /// Send cell values (off: only schema and aggregates leave the machine).
    pub send_rows: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolOutcome {
    /// What the model reads.
    pub text: String,
    /// One line for the trace / chat.
    pub summary: String,
    pub ok: bool,
    /// The project changed (the app re-renders, records undo, tracks it in the brain).
    pub mutated: bool,
    /// Elements created or changed.
    pub elements: Vec<u64>,
    /// Pages created.
    pub pages: Vec<u64>,
}

fn ok(text: Value, summary: impl Into<String>) -> ToolOutcome {
    ToolOutcome { text: text.to_string(), summary: summary.into(), ok: true, ..ToolOutcome::default() }
}
fn fail(msg: impl Into<String>) -> ToolOutcome {
    let m = msg.into();
    ToolOutcome { text: json!({"error": m}).to_string(), summary: m, ok: false, ..ToolOutcome::default() }
}

pub const MUTATING: &[&str] = &["create_visual", "set_format", "add_measure", "add_filter", "create_page", "create_query"];

pub fn is_mutating(name: &str) -> bool {
    MUTATING.contains(&name)
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required})
}

/// Tool definitions (JSON schema) sent to the model.
pub fn specs() -> Vec<ToolSpec> {
    let s = |name: &str, d: &str, p: Value| ToolSpec { name: name.into(), description: d.into(), parameters: p };
    let field_list = json!({"type": "array", "items": {"type": "string"}, "description": "Fields as 'Table[Column]' or '[Measure]'. Prefix with an aggregation and a colon to aggregate a column, e.g. 'Average:Sales[Price]'."});
    vec![
        s("describe_model", "List the model: tables with columns and types, row counts, measures with DAX, relationships, and report pages with their visuals.", obj(json!({}), &[])),
        s("run_dax", "Run a DAX query (EVALUATE ...) against the model and return the result rows.", obj(json!({"query": {"type": "string", "description": "A DAX query, e.g. EVALUATE SUMMARIZECOLUMNS('Store'[Region], \"Sales\", [Total Sales])"}}), &["query"])),
        s("run_m", "Evaluate a Power Query M expression (it can reference queries by name) and return the result.", obj(json!({"expression": {"type": "string"}}), &["expression"])),
        s("preview_table", "Return the first n rows of a table.", obj(json!({"name": {"type": "string"}, "n": {"type": "integer"}}), &["name"])),
        s(
            "create_visual",
            "Create a visual on the current report page. Wells by role: axis, values, legend, secondary, x, y, size, details, rows, columns, field (slicer), target. Kinds: clusteredColumn, clusteredBar, stackedColumn, line, area, scatter, pie, donut, treemap, card, kpi, table, matrix, slicer, gauge, waterfall, funnel, map.",
            obj(
                json!({
                    "kind": {"type": "string"},
                    "title": {"type": "string"},
                    "axis": field_list, "values": field_list, "legend": field_list, "secondary": field_list,
                    "x": field_list, "y": field_list, "size": field_list, "details": field_list,
                    "rows": field_list, "columns": field_list, "field": field_list, "target": field_list
                }),
                &["kind"],
            ),
        ),
        s(
            "set_format",
            "Set a formatting property of an element (id, or 'selected', or 'last'). Keys: title.text, title.show, title.size, title.bold, title.color, labels.show, legend.show, legend.position, background.color, border.show, shadow.show, lines.width, markers.show, grid.horizontal. Use element 'report' with key 'theme' to change the report theme (VizCraft, Classic, Midnight, Ocean, Sunset, Forest, Monochrome, High contrast, Pastel, Executive).",
            obj(json!({"element": {"type": "string"}, "key": {"type": "string"}, "value": {"description": "bool, number, text or #RRGGBB colour"}}), &["element", "key", "value"]),
        ),
        s("add_measure", "Add (or replace) a DAX measure. It is validated before it is kept.", obj(json!({"name": {"type": "string"}, "dax": {"type": "string"}, "table": {"type": "string"}, "format": {"type": "string", "description": "e.g. $#,0 or 0.0%"}}), &["name", "dax"])),
        s(
            "add_filter",
            "Add a basic filter keeping (or excluding) values of a column, on the current page, the whole report, or one visual (scope = 'page' | 'report' | element id).",
            obj(json!({"field": {"type": "string"}, "values": {"type": "array", "items": {}}, "exclude": {"type": "boolean"}, "scope": {"type": "string"}}), &["field", "values"]),
        ),
        s("create_page", "Create a new report page and make it current.", obj(json!({"name": {"type": "string"}}), &["name"])),
        s("explain_visual", "Return a visual's definition, its data and summary statistics so you can explain it (element id or 'selected').", obj(json!({"element": {"type": "string"}}), &["element"])),
        s("create_query", "Create a Power Query query from M code, load it to the model and refresh.", obj(json!({"name": {"type": "string"}, "m": {"type": "string"}}), &["name", "m"])),
        s("suggest_relationships", "Suggest relationships between tables from matching column names and key uniqueness.", obj(json!({}), &[])),
    ]
}

/// A short human description of a call, for the "Apply?" confirmation.
pub fn describe_call(call: &ToolCall) -> String {
    let a = &call.arguments;
    let g = |k: &str| a.get(k).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default();
    match call.name.as_str() {
        "create_visual" => {
            let wells: Vec<String> = a
                .as_object()
                .map(|o| o.iter().filter(|(k, v)| v.is_array() && *k != "kind").map(|(k, v)| format!("{k}: {}", v.as_array().map(|x| x.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default())).collect())
                .unwrap_or_default();
            format!("Create a {} visual ({})", g("kind"), wells.join("; "))
        }
        "set_format" => format!("Set {} = {} on {}", g("key"), g("value"), g("element")),
        "add_measure" => format!("Add measure [{}] = {}", g("name"), g("dax")),
        "add_filter" => format!("Filter {} to {}", g("field"), g("values")),
        "create_page" => format!("Create page \"{}\"", g("name")),
        "create_query" => format!("Create query \"{}\"", g("name")),
        other => other.to_string(),
    }
}

pub fn params(p: &Project) -> BTreeMap<String, Scalar> {
    p.model.parameters.iter().map(|x| (x.name.clone(), x.current.clone())).collect()
}

fn rebuild(ws: &mut Workspace) {
    build::sync_model_tables(ws.project);
    *ws.built = build::rebuild(ws.project, ws.built, &params(ws.project));
}

fn refresh(ws: &mut Workspace) {
    build::sync_model_tables(ws.project);
    let host = vizcraft_connectors::DesktopHost::default();
    *ws.built = build::build(ws.project, &host, &params(ws.project));
}

pub fn execute(ws: &mut Workspace, call: &ToolCall) -> ToolOutcome {
    let a = &call.arguments;
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    match call.name.as_str() {
        "describe_model" => describe_model(ws),
        "run_dax" => run_dax(ws, &s("query")),
        "run_m" => run_m(ws, &s("expression")),
        "preview_table" => preview(ws, &s("name"), a.get("n").and_then(Value::as_u64).unwrap_or(10) as usize),
        "create_visual" => create_visual(ws, a),
        "set_format" => set_format(ws, &s("element"), &s("key"), a.get("value").cloned().unwrap_or(Value::Null)),
        "add_measure" => add_measure(ws, &s("name"), &s("dax"), &s("table"), &s("format")),
        "add_filter" => add_filter(ws, &s("field"), a.get("values").cloned().unwrap_or(json!([])), a.get("exclude").and_then(Value::as_bool).unwrap_or(false), &s("scope")),
        "create_page" => create_page(ws, &s("name")),
        "explain_visual" => explain_visual(ws, &s("element")),
        "create_query" => create_query(ws, &s("name"), &s("m")),
        "suggest_relationships" => suggest_relationships(ws),
        other => fail(format!("Unknown tool `{other}`.")),
    }
}

// ============================================================== reading

pub fn describe_model_json(p: &Project, built: &Built) -> Value {
    let tables: Vec<Value> = p
        .model
        .tables
        .iter()
        .filter(|t| !t.hidden)
        .map(|t| {
            let cols: Vec<Value> = build::visible_columns(p, built, &t.name).iter().map(|(c, ty)| json!(format!("{c} ({})", ty.label()))).collect();
            json!({"name": t.name, "rows": built.table(&t.name).map(|d| d.len()).unwrap_or(0), "columns": cols})
        })
        .collect();
    let measures: Vec<Value> = p.model.measures.iter().map(|m| json!({"name": m.name, "table": m.table, "dax": m.expression, "format": m.format})).collect();
    let rels: Vec<Value> = p.model.relationships.iter().map(|r| json!(format!("{}[{}] -> {}[{}]{}", r.from_table, r.from_column, r.to_table, r.to_column, if r.active { "" } else { " (inactive)" }))).collect();
    let pages: Vec<Value> = p
        .report
        .pages
        .iter()
        .map(|pg| {
            let visuals: Vec<Value> = pg
                .elements
                .iter()
                .filter_map(|e| {
                    e.visual().map(|v| json!({"id": e.id, "kind": v.kind, "title": e.name, "fields": v.wells.iter().map(|(r, f)| format!("{r}: {}", f.iter().map(FieldUse::label).collect::<Vec<_>>().join(", "))).collect::<Vec<_>>()}))
                })
                .collect();
            json!({"id": pg.id, "name": pg.name, "visuals": visuals})
        })
        .collect();
    json!({"project": p.name, "tables": tables, "measures": measures, "relationships": rels, "pages": pages})
}

fn describe_model(ws: &mut Workspace) -> ToolOutcome {
    let v = describe_model_json(ws.project, ws.built);
    let n = v["tables"].as_array().map(Vec::len).unwrap_or(0);
    ok(v, format!("Described the model ({n} tables, {} measures)", ws.project.model.measures.len()))
}

pub fn run_dax_query(built: &Built, p: &Project, query: &str) -> Result<(Vec<String>, Vec<Vec<Scalar>>), String> {
    if query.is_empty() {
        return Err("Empty query.".into());
    }
    let q = build::substitute_params(query, p, &BTreeMap::new());
    let r = built.model.query(&q).map_err(|e| e.to_string())?;
    Ok((r.columns, r.rows.iter().map(|row| row.iter().map(Scalar::from_dm).collect()).collect()))
}

fn rows_json(cols: &[String], rows: &[Vec<Scalar>], max: usize, send: bool) -> Value {
    if !send {
        return json!({"columns": cols, "rowCount": rows.len(), "note": "Row values are withheld by the user's privacy settings."});
    }
    let shown: Vec<Value> = rows.iter().take(max).map(|r| Value::Array(r.iter().map(|v| serde_json::to_value(v).unwrap_or(Value::Null)).collect())).collect();
    json!({"columns": cols, "rows": shown, "rowCount": rows.len(), "truncated": rows.len() > max})
}

fn run_dax(ws: &mut Workspace, query: &str) -> ToolOutcome {
    match run_dax_query(ws.built, ws.project, query) {
        Ok((cols, rows)) => ok(rows_json(&cols, &rows, ws.max_rows, ws.send_rows), format!("DAX returned {} rows × {} columns", rows.len(), cols.len())),
        Err(e) => fail(format!("DAX error: {e}")),
    }
}

pub fn eval_m(p: &Project, expression: &str) -> Result<build::TableData, String> {
    let host = vizcraft_connectors::DesktopHost::default();
    let ph = build::ProjectHost::new(p, &host);
    let v = vizcraft_query::evaluate(expression, &ph).map_err(|e| e.message())?;
    let t = match v {
        vizcraft_query::Value::Table(t) => (*t).clone(),
        vizcraft_query::Value::List(l) => vizcraft_query::Table::new(vec!["Value".into()], l.iter().map(|v| vec![v.clone()]).collect()),
        vizcraft_query::Value::Record(r) => vizcraft_query::Table::new(r.names.clone(), vec![r.values.clone()]),
        other => vizcraft_query::Table::new(vec!["Value".into()], vec![vec![other]]),
    };
    let mut td = build::TableData::from_m("Result", &t);
    td.infer_types();
    Ok(td)
}

fn run_m(ws: &mut Workspace, expression: &str) -> ToolOutcome {
    match eval_m(ws.project, expression) {
        Ok(t) => ok(rows_json(&t.columns, &t.rows, ws.max_rows, ws.send_rows), format!("M returned {} rows × {} columns", t.len(), t.columns.len())),
        Err(e) => fail(format!("M error: {e}")),
    }
}

fn preview(ws: &mut Workspace, name: &str, n: usize) -> ToolOutcome {
    let Some(t) = ws.built.table(name) else { return fail(format!("There's no table named `{name}`.")) };
    let cols: Vec<String> = t.columns.iter().zip(t.types.iter()).map(|(c, ty)| format!("{c} ({})", ty.label())).collect();
    ok(rows_json(&cols, &t.rows, n.clamp(1, ws.max_rows.max(1)), ws.send_rows), format!("Previewed {} ({} rows)", t.name, t.len()))
}

// ============================================================== fields

/// Resolves "Table[Column]", "'Table'[Column]", "[Measure]", a bare measure or column name.
pub fn resolve_field(p: &Project, built: &Built, s: &str) -> Option<FieldRef> {
    let s = s.trim();
    let measure = |name: &str| p.model.measures.iter().find(|m| m.name.eq_ignore_ascii_case(name.trim())).map(|m| FieldRef::measure(&m.table, &m.name));
    if let Some(open) = s.find('[')
        && s.ends_with(']')
    {
        let table = s[..open].trim().trim_matches('\'');
        let inner = &s[open + 1..s.len() - 1];
        if table.is_empty() {
            if let Some(m) = measure(inner) {
                return Some(m);
            }
            return column_anywhere(p, built, inner);
        }
        let tname = p.model.tables.iter().find(|t| t.name.eq_ignore_ascii_case(table)).map(|t| t.name.clone())?;
        if let Some((c, _)) = build::visible_columns(p, built, &tname).into_iter().find(|(c, _)| c.eq_ignore_ascii_case(inner)) {
            return Some(FieldRef::col(&tname, &c));
        }
        if let Some(m) = p.model.measures.iter().find(|m| m.name.eq_ignore_ascii_case(inner)) {
            return Some(FieldRef::measure(&m.table, &m.name));
        }
        return None;
    }
    measure(s).or_else(|| column_anywhere(p, built, s))
}

fn column_anywhere(p: &Project, built: &Built, name: &str) -> Option<FieldRef> {
    p.model.tables.iter().filter(|t| !t.hidden).find_map(|t| build::visible_columns(p, built, &t.name).into_iter().find(|(c, _)| c.eq_ignore_ascii_case(name.trim())).map(|(c, _)| FieldRef::col(&t.name, &c)))
}

pub fn value_role(role: &str) -> bool {
    matches!(role, "values" | "secondary" | "x" | "y" | "size" | "target" | "min" | "max" | "tooltips")
}

/// A field use for a well: "Average:Sales[Price]" aggregates; plain columns in value wells get
/// the column's default aggregation.
pub fn field_use(p: &Project, built: &Built, role: &str, s: &str) -> Option<FieldUse> {
    let (agg, field) = match s.split_once(':') {
        Some((a, f)) if !a.contains('[') => (Some(Agg::from_label(&cap(a.trim()))), f),
        _ => (None, s),
    };
    let f = resolve_field(p, built, field)?;
    let mut fu = FieldUse::new(f.clone());
    if let FieldRef::Column { table, column } = &f {
        if let Some(a) = agg.filter(|a| *a != Agg::None) {
            fu.agg = a;
        } else if value_role(role) {
            fu.agg = state::default_agg(p, built, table, column);
        }
    }
    Some(fu)
}

fn cap(s: &str) -> String {
    let l = s.to_lowercase();
    match l.as_str() {
        "avg" | "mean" | "average" => "Average".into(),
        "count" => "Count".into(),
        "distinct" | "distinctcount" | "distinct count" | "countd" => "Distinct count".into(),
        "sum" => "Sum".into(),
        "min" => "Min".into(),
        "max" => "Max".into(),
        "median" => "Median".into(),
        _ => s.to_string(),
    }
}

/// Visual kind from a name the model wrote ("line", "Line chart", "clusteredColumn", "bar").
pub fn parse_kind(s: &str) -> Option<VisualKind> {
    let norm = |x: &str| x.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    let n = norm(s);
    let alias = match n.as_str() {
        "bar" | "barchart" | "hbar" => Some(VisualKind::ClusteredBar),
        "column" | "columnchart" | "vbar" => Some(VisualKind::ClusteredColumn),
        "linechart" | "line" => Some(VisualKind::Line),
        "point" | "scatterplot" | "bubble" => Some(VisualKind::Scatter),
        "number" | "bignumber" | "kpicard" => Some(VisualKind::Card),
        "heatmap" => Some(VisualKind::HeatMap),
        "pivot" | "crosstab" => Some(VisualKind::Matrix),
        _ => None,
    };
    alias.or_else(|| {
        VisualKind::ALL.iter().copied().find(|k| {
            let serde_name = serde_json::to_value(k).ok().and_then(|v| v.as_str().map(norm)).unwrap_or_default();
            serde_name == n || norm(k.label()) == n || norm(k.label()).trim_end_matches("chart") == n
        })
    })
}

// ============================================================== editing

fn create_visual(ws: &mut Workspace, a: &Value) -> ToolOutcome {
    let kind_s = a.get("kind").and_then(Value::as_str).unwrap_or("");
    let Some(kind) = parse_kind(kind_s) else { return fail(format!("Unknown visual kind `{kind_s}`.")) };
    let mut spec = VisualSpec::new(kind);
    let mut missing = Vec::new();
    for (role, _) in kind.wells() {
        let Some(list) = a.get(*role) else { continue };
        let items: Vec<String> = match list {
            Value::Array(x) => x.iter().filter_map(|v| v.as_str().map(str::to_string).or_else(|| v.get("field").and_then(Value::as_str).map(str::to_string))).collect(),
            Value::String(s) => vec![s.clone()],
            _ => vec![],
        };
        for it in items {
            match field_use(ws.project, ws.built, role, &it) {
                Some(fu) => spec.well_mut(role).push(fu),
                None => missing.push(it),
            }
        }
    }
    // Wells the kind doesn't have: map axis/values onto the kind's own roles.
    if spec.wells.values().all(Vec::is_empty) {
        for (src, list) in a.as_object().into_iter().flatten() {
            if src == "kind" || src == "title" {
                continue;
            }
            for it in list.as_array().into_iter().flatten().filter_map(Value::as_str) {
                if let Some(fu) = field_use(ws.project, ws.built, if value_role(src) { "values" } else { "axis" }, it) {
                    let role = if fu.is_value() { kind.wells().iter().map(|(r, _)| *r).find(|r| value_role(r) || *r == "columns") } else { kind.wells().first().map(|(r, _)| *r) };
                    if let Some(r) = role {
                        spec.well_mut(r).push(fu);
                    }
                }
            }
        }
    }
    if !missing.is_empty() && spec.wells.values().all(Vec::is_empty) {
        return fail(format!("None of the fields exist: {}. Call describe_model to see the field names.", missing.join(", ")));
    }
    let (w, h) = match kind {
        VisualKind::Card | VisualKind::Kpi => (260.0, 130.0),
        VisualKind::Slicer => (220.0, 220.0),
        VisualKind::Table | VisualKind::Matrix => (520.0, 300.0),
        _ => (460.0, 300.0),
    };
    let title = a.get("title").and_then(Value::as_str).unwrap_or("").to_string();
    let id = ws.project.next_id();
    let mut el = Element { id, name: if title.is_empty() { kind.label().to_string() } else { title.clone() }, w, h, kind: ElementKind::Visual(spec), ..Element::default() };
    if !title.is_empty() {
        el.props.insert("title.text".into(), Prop::Text(title));
    }
    if ws.project.page(*ws.page).is_none()
        && let Some(p) = ws.project.report.pages.first()
    {
        *ws.page = p.id;
    }
    let Some(page) = ws.project.page_mut(*ws.page) else { return fail("The report has no page.") };
    let (x, y) = free_spot(page, w, h);
    el.x = x;
    el.y = y;
    page.elements.push(el);
    let mut o = ok(json!({"created": id, "page": *ws.page, "missingFields": missing}), format!("Created a {} (#{id})", kind.label()));
    o.mutated = true;
    o.elements = vec![id];
    o
}

/// The first spot on a 20-pt grid where a w×h box doesn't overlap existing elements.
pub fn free_spot(page: &Page, w: f32, h: f32) -> (f32, f32) {
    let overlaps = |x: f32, y: f32| page.elements.iter().any(|e| e.parent.is_none() && x < e.x + e.w && x + w > e.x && y < e.y + e.h && y + h > e.y);
    let mut y = 20.0;
    while y + h <= page.height.max(h + 40.0) {
        let mut x = 20.0;
        while x + w <= page.width.max(w + 40.0) {
            if !overlaps(x, y) {
                return (x, y);
            }
            x += 20.0;
        }
        y += 20.0;
    }
    let n = page.elements.len() as f32;
    (40.0 + (n % 8.0) * 20.0, 40.0 + (n % 8.0) * 20.0)
}

fn find_element(ws: &Workspace, which: &str) -> Option<u64> {
    match which.trim().trim_start_matches('#') {
        "selected" | "" => ws.selected.or_else(|| last_visual(ws)),
        "last" => last_visual(ws),
        s => s.parse::<u64>().ok().filter(|id| ws.project.element(*id).is_some()).or_else(|| {
            ws.project.page(*ws.page).and_then(|p| p.elements.iter().find(|e| e.name.eq_ignore_ascii_case(s)).map(|e| e.id))
        }),
    }
}

fn last_visual(ws: &Workspace) -> Option<u64> {
    ws.project.page(*ws.page).and_then(|p| p.elements.iter().rev().find(|e| e.visual().is_some()).map(|e| e.id))
}

pub fn json_to_prop(v: &Value) -> Prop {
    match v {
        Value::Bool(b) => Prop::Bool(*b),
        Value::Number(n) => Prop::Num(n.as_f64().unwrap_or(0.0)),
        Value::String(s) => {
            let t = s.trim();
            if let Some(c) = vizcraft_core::color::Color::parse(t).filter(|_| t.starts_with('#')) {
                Prop::Color(c)
            } else if t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("on") {
                Prop::Bool(true)
            } else if t.eq_ignore_ascii_case("false") || t.eq_ignore_ascii_case("off") {
                Prop::Bool(false)
            } else if let Ok(n) = t.parse::<f64>() {
                Prop::Num(n)
            } else {
                Prop::Text(s.clone())
            }
        }
        other => Prop::Text(other.to_string()),
    }
}

fn set_format(ws: &mut Workspace, which: &str, key: &str, value: Value) -> ToolOutcome {
    if key.eq_ignore_ascii_case("theme") || which.eq_ignore_ascii_case("report") {
        let name = value.as_str().unwrap_or("").to_string();
        let Some(theme) = vizcraft_core::color::Theme::NAMES.iter().find(|n| n.eq_ignore_ascii_case(&name)).and_then(|n| vizcraft_core::color::Theme::by_name(n)) else {
            return fail(format!("Unknown theme `{name}`. Themes: {}", vizcraft_core::color::Theme::NAMES.join(", ")));
        };
        ws.project.report.theme = theme;
        let mut o = ok(json!({"theme": name}), format!("Applied the {name} theme"));
        o.mutated = true;
        return o;
    }
    let Some(id) = find_element(ws, which) else { return fail(format!("No element `{which}` on this page.")) };
    let Some(el) = ws.project.element_mut(id) else { return fail("Element not found.") };
    let schema = vizcraft_core::format::schema_key(el);
    let mut prop = json_to_prop(&value);
    if let Some(def) = vizcraft_core::format::prop_def(&schema, key) {
        // Coerce to the property's kind.
        prop = match (&def.kind, prop) {
            (vizcraft_core::format::PropKind::Choice(opts), Prop::Text(t)) => Prop::Choice(opts.iter().find(|o| o.eq_ignore_ascii_case(&t)).map(|o| o.to_string()).unwrap_or(t)),
            (vizcraft_core::format::PropKind::Text | vizcraft_core::format::PropKind::Template, Prop::Num(n)) => Prop::Text(n.to_string()),
            (_, p) => p,
        };
    } else if !key.contains('.') {
        return fail(format!("Unknown format key `{key}` (keys look like title.text, labels.show)."));
    }
    if key == "title.text"
        && let Prop::Text(t) = &prop
    {
        el.name = t.clone();
    }
    el.props.insert(key.to_string(), prop);
    let mut o = ok(json!({"element": id, "key": key}), format!("Set {key} on #{id}"));
    o.mutated = true;
    o.elements = vec![id];
    o
}

fn add_measure(ws: &mut Workspace, name: &str, dax: &str, table: &str, format: &str) -> ToolOutcome {
    if name.is_empty() || dax.is_empty() {
        return fail("A measure needs a name and a DAX expression.");
    }
    let dax = dax.trim().trim_start_matches(&format!("{name} =")).trim_start_matches('=').trim().to_string();
    let table = if table.is_empty() {
        ws.project.model.measures.first().map(|m| m.table.clone()).or_else(|| ws.project.model.tables.first().map(|t| t.name.clone())).unwrap_or_default()
    } else {
        ws.project.model.tables.iter().find(|t| t.name.eq_ignore_ascii_case(table)).map(|t| t.name.clone()).unwrap_or_else(|| table.to_string())
    };
    let before = ws.project.model.measures.clone();
    ws.project.model.measures.retain(|m| !m.name.eq_ignore_ascii_case(name));
    ws.project.model.measures.push(Measure { table: table.clone(), name: name.to_string(), expression: dax.clone(), format: format.to_string(), origin: "Copilot".into(), ..Measure::default() });
    rebuild(ws);
    let check = ws.built.model.evaluate(&format!("[{name}]"));
    let build_err = ws.built.errors.iter().find(|(o, _)| o.contains(name)).map(|(_, e)| e.clone());
    match (check, build_err) {
        (Ok(v), None) => {
            let mut o = ok(json!({"measure": name, "table": table, "grandTotal": Scalar::from_dm(&v)}), format!("Added measure [{name}] (total {})", Scalar::from_dm(&v).text()));
            o.mutated = true;
            o
        }
        (check, build_err) => {
            let e = build_err.or_else(|| check.err().map(|e| e.to_string())).unwrap_or_default();
            ws.project.model.measures = before;
            rebuild(ws);
            fail(format!("The measure didn't validate and was not added: {e}"))
        }
    }
}

fn add_filter(ws: &mut Workspace, field: &str, values: Value, exclude: bool, scope: &str) -> ToolOutcome {
    let Some(f) = resolve_field(ws.project, ws.built, field) else { return fail(format!("No field `{field}`.")) };
    if f.is_measure() {
        return fail("Basic filters work on columns; filter a measure with a visual-level filter.");
    }
    let vals: Vec<Scalar> = match values {
        Value::Array(a) => a.into_iter().map(|v| serde_json::from_value(v).unwrap_or(Scalar::Null)).collect(),
        v => vec![serde_json::from_value(v).unwrap_or(Scalar::Null)],
    };
    let id = ws.project.next_id();
    let def = FilterDef { id, field: FieldUse::new(f.clone()), kind: FilterKind::Basic { values: vals.clone(), exclude }, locked: false, hidden: false, require_single: false, context: false };
    let label = f.label();
    let summary = format!("Filtered {label} {} {}", if exclude { "excluding" } else { "to" }, vals.iter().map(Scalar::text).collect::<Vec<_>>().join(", "));
    match scope.trim() {
        "report" => ws.project.report.filters.push(def),
        "" | "page" => {
            let Some(p) = ws.project.page_mut(*ws.page) else { return fail("No current page.") };
            p.filters.push(def);
        }
        other => {
            let Some(eid) = find_element(ws, other) else { return fail(format!("No element `{other}`.")) };
            let Some(v) = ws.project.element_mut(eid).and_then(|e| e.visual_mut()) else { return fail("That element isn't a visual.") };
            v.filters.push(def);
        }
    }
    let mut o = ok(json!({"filter": id}), summary);
    o.mutated = true;
    o
}

fn create_page(ws: &mut Workspace, name: &str) -> ToolOutcome {
    let id = ws.project.next_id();
    let name = if name.is_empty() { format!("Page {}", ws.project.report.pages.len() + 1) } else { name.to_string() };
    ws.project.report.pages.push(Page::new(id, &name, PageKind::Report));
    *ws.page = id;
    let mut o = ok(json!({"page": id, "name": name}), format!("Created page \"{name}\""));
    o.mutated = true;
    o.pages = vec![id];
    o
}

fn explain_visual(ws: &mut Workspace, which: &str) -> ToolOutcome {
    let Some(id) = find_element(ws, which) else { return fail(format!("No visual `{which}`.")) };
    let Some((pid, el)) = ws.project.element(id) else { return fail("Not found.") };
    let Some(v) = el.visual() else { return fail("That element isn't a visual.") };
    let Some(page) = ws.project.page(pid) else { return fail("No page.") };
    let cx = state::context_for(ws.project, ws.built, &Runtime::default(), page, el);
    let data = vizcraft_core::vquery::run(ws.project, ws.built, v, &cx);
    if let Some(e) = &data.error {
        return fail(format!("The visual can't compute: {e}"));
    }
    let cols: Vec<String> = data.cols.iter().map(|c| c.label.clone()).collect();
    let mut stats = Vec::new();
    for vi in data.values() {
        let nums: Vec<f64> = (0..data.rows.len()).filter_map(|r| data.num(r, vi)).collect();
        if nums.is_empty() {
            continue;
        }
        let sum: f64 = nums.iter().sum();
        let (imax, max) = nums.iter().enumerate().fold((0, f64::MIN), |a, (i, x)| if *x > a.1 { (i, *x) } else { a });
        let (imin, min) = nums.iter().enumerate().fold((0, f64::MAX), |a, (i, x)| if *x < a.1 { (i, *x) } else { a });
        let cat = |r: usize| data.categories().iter().map(|c| data.text(r, *c)).collect::<Vec<_>>().join(" / ");
        stats.push(json!({"value": data.cols.get(vi).map(|c| c.label.clone()), "sum": sum, "mean": sum / nums.len() as f64, "max": max, "maxAt": cat(imax), "min": min, "minAt": cat(imin)}));
    }
    let rows: Vec<Vec<Scalar>> = (0..data.rows.len()).map(|r| (0..data.cols.len()).map(|c| Scalar::Text(data.text(r, c))).collect()).collect();
    let mut out = rows_json(&cols, &rows, ws.max_rows, ws.send_rows);
    out["kind"] = json!(v.kind.label());
    out["title"] = json!(el.name);
    out["filters"] = json!(cx.description);
    out["stats"] = Value::Array(stats);
    ok(out, format!("Read {} ({} rows)", el.name, data.rows.len()))
}

fn create_query(ws: &mut Workspace, name: &str, m: &str) -> ToolOutcome {
    if name.is_empty() || m.is_empty() {
        return fail("A query needs a name and M code.");
    }
    let t = match eval_m(ws.project, m) {
        Ok(t) => t,
        Err(e) => return fail(format!("The M code doesn't evaluate: {e}")),
    };
    let name = ws.project.free_table_name(name);
    ws.project.queries.queries.push(vizcraft_query::steps::QueryDef { name: name.clone(), m: m.to_string(), load: vizcraft_query::steps::LoadTo::Model, loaded_table: None, group: None, description: "Created by the copilot".into() });
    refresh(ws);
    let mut o = ok(json!({"query": name, "rows": t.len(), "columns": t.columns}), format!("Created query {name} ({} rows)", t.len()));
    o.mutated = true;
    o
}

/// Candidate relationships: same column name in two tables, unique on the "one" side.
pub fn relationship_candidates(p: &Project, built: &Built) -> Vec<(String, String, String, String, f64)> {
    let mut out = Vec::new();
    let tables: Vec<_> = built.tables.values().collect();
    for a in &tables {
        for b in &tables {
            if a.name == b.name {
                continue;
            }
            for (ci, c) in a.columns.iter().enumerate() {
                let Some(bi) = b.column_index(c) else { continue };
                let exists = p.model.relationships.iter().any(|r| r.from_table == a.name && r.to_table == b.name && r.from_column.eq_ignore_ascii_case(c));
                if exists {
                    continue;
                }
                let distinct_b: std::collections::HashSet<String> = b.rows.iter().map(|r| r.get(bi).map(Scalar::key).unwrap_or_default()).collect();
                if b.rows.is_empty() || distinct_b.len() != b.rows.len() {
                    continue;
                }
                // Both sides unique (1:1): suggest it once.
                let distinct_a: std::collections::HashSet<String> = a.rows.iter().map(|r| r.get(ci).map(Scalar::key).unwrap_or_default()).collect();
                if distinct_a.len() == a.rows.len() && a.name > b.name {
                    continue;
                }
                let hits = a.rows.iter().filter(|r| r.get(ci).is_some_and(|v| distinct_b.contains(&v.key()))).count();
                let coverage = if a.rows.is_empty() { 0.0 } else { hits as f64 / a.rows.len() as f64 };
                let typed = a.types.get(ci) == b.types.get(bi) || a.types.get(ci) == Some(&ColType::Any);
                if coverage > 0.5 && typed {
                    out.push((a.name.clone(), c.clone(), b.name.clone(), b.columns.get(bi).cloned().unwrap_or_default(), coverage));
                }
            }
        }
    }
    out
}

fn suggest_relationships(ws: &mut Workspace) -> ToolOutcome {
    let c = relationship_candidates(ws.project, ws.built);
    let list: Vec<Value> = c.iter().map(|(ft, fc, tt, tc, cov)| json!({"from": format!("{ft}[{fc}]"), "to": format!("{tt}[{tc}]"), "cardinality": "many-to-one", "matchRate": (cov * 100.0).round() / 100.0})).collect();
    ok(json!({"suggestions": list, "existing": ws.project.model.relationships.len()}), format!("Found {} relationship candidates", c.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws_run(p: &mut Project, b: &mut Built, page: &mut u64, name: &str, args: Value) -> ToolOutcome {
        let mut ws = Workspace { project: p, built: b, page, selected: None, max_rows: 20, send_rows: true };
        execute(&mut ws, &ToolCall { id: "1".into(), name: name.into(), arguments: args })
    }

    pub fn sample() -> (Project, Built, u64) {
        let mut p = vizcraft_core::sample::sales();
        build::sync_model_tables(&mut p);
        let host = vizcraft_connectors::DesktopHost::default();
        let b = build::build(&p, &host, &params(&p));
        let page = p.report.pages.first().map(|x| x.id).unwrap_or(0);
        (p, b, page)
    }

    #[test]
    fn tools_against_the_sample() {
        let (mut p, mut b, mut page) = sample();
        let d = ws_run(&mut p, &mut b, &mut page, "describe_model", json!({}));
        assert!(d.ok && d.text.contains("Total Sales"), "{}", d.text);
        let q = ws_run(&mut p, &mut b, &mut page, "run_dax", json!({"query": "EVALUATE SUMMARIZECOLUMNS(Store[Region], \"x\", [Total Sales])"}));
        assert!(q.ok, "{}", q.text);
        let bad = ws_run(&mut p, &mut b, &mut page, "run_dax", json!({"query": "EVALUATE NOPE("}));
        assert!(!bad.ok);
        let m = ws_run(&mut p, &mut b, &mut page, "add_measure", json!({"name": "Double Sales", "dax": "[Total Sales] * 2", "format": "$#,0"}));
        assert!(m.ok && m.mutated, "{}", m.text);
        assert!(p.model.measures.iter().any(|x| x.name == "Double Sales"));
        let badm = ws_run(&mut p, &mut b, &mut page, "add_measure", json!({"name": "Broken", "dax": "SUMX(NoTable, 1"}));
        assert!(!badm.ok);
        assert!(!p.model.measures.iter().any(|x| x.name == "Broken"));
        let pg = ws_run(&mut p, &mut b, &mut page, "create_page", json!({"name": "Copilot"}));
        assert!(pg.ok && p.page(page).is_some_and(|x| x.name == "Copilot"));
        let v = ws_run(&mut p, &mut b, &mut page, "create_visual", json!({"kind": "line", "axis": ["Sales[Date]"], "values": ["[Double Sales]"], "title": "Doubled"}));
        assert!(v.ok, "{}", v.text);
        let id = v.elements[0];
        let el = p.element(id).unwrap().1;
        assert_eq!(el.visual().unwrap().kind, VisualKind::Line);
        let f = ws_run(&mut p, &mut b, &mut page, "set_format", json!({"element": "last", "key": "labels.show", "value": true}));
        assert!(f.ok, "{}", f.text);
        assert_eq!(p.element(id).unwrap().1.props.get("labels.show"), Some(&Prop::Bool(true)));
        let e = ws_run(&mut p, &mut b, &mut page, "explain_visual", json!({"element": id.to_string()}));
        assert!(e.ok && e.text.contains("stats"), "{}", e.text);
        let th = ws_run(&mut p, &mut b, &mut page, "set_format", json!({"element": "report", "key": "theme", "value": "Midnight"}));
        assert!(th.ok && p.report.theme.name == "Midnight");
        let mq = ws_run(&mut p, &mut b, &mut page, "create_query", json!({"name": "Mini", "m": "#table({\"A\",\"B\"}, {{1, \"x\"}, {2, \"y\"}})"}));
        assert!(mq.ok, "{}", mq.text);
        assert!(b.table("Mini").is_some());
        let r = ws_run(&mut p, &mut b, &mut page, "suggest_relationships", json!({}));
        assert!(r.ok);
        let fl = ws_run(&mut p, &mut b, &mut page, "add_filter", json!({"field": "Mini[B]", "values": ["x"]}));
        assert!(fl.ok, "{}", fl.text);
        assert!(!ws_run(&mut p, &mut b, &mut page, "nope", json!({})).ok);
    }

    #[test]
    fn kinds_parse() {
        assert_eq!(parse_kind("line"), Some(VisualKind::Line));
        assert_eq!(parse_kind("Line chart"), Some(VisualKind::Line));
        assert_eq!(parse_kind("clusteredColumn"), Some(VisualKind::ClusteredColumn));
        assert_eq!(parse_kind("bar"), Some(VisualKind::ClusteredBar));
        assert_eq!(parse_kind("Donut chart"), Some(VisualKind::Donut));
        assert_eq!(parse_kind("zzz"), None);
    }
}
