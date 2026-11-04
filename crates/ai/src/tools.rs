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
    /// Requests for the app (navigate to a view, save, undo, refresh…), as JSON objects with an
    /// `action` key. Headless runs (CLI) ignore the ones that need a window.
    pub actions: Vec<Value>,
}

fn ok(text: Value, summary: impl Into<String>) -> ToolOutcome {
    ToolOutcome { text: text.to_string(), summary: summary.into(), ok: true, ..ToolOutcome::default() }
}
fn fail(msg: impl Into<String>) -> ToolOutcome {
    let m = msg.into();
    ToolOutcome { text: json!({"error": m}).to_string(), summary: m, ok: false, ..ToolOutcome::default() }
}

pub const MUTATING: &[&str] = &[
    "create_visual", "set_format", "add_measure", "add_filter", "create_page", "create_query", "update_query", "edit_project", "apply_tmdl", "add_relationship",
    "add_calculated_column", "delete", "move_element", "add_text", "add_shape", "create_sheet", "set_cells", "create_workflow", "add_widget", "refresh",
];

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
        // ---- whole-app control
        s("get_project", "Read any part of the project document as JSON by JSON pointer (e.g. '' for an overview, '/report/pages/0', '/model/measures', '/queries/queries', '/sheets/0', '/studio/0', '/report/theme'). Use this to discover structure before edit_project.", obj(json!({"path": {"type": "string"}}), &[])),
        s(
            "edit_project",
            "Edit ANY part of the project document (full control: report, pages, visuals, shapes, themes, bookmarks, stories, model, queries, sheets, flows, notebooks, studio workflows, publish settings). op = set (replace the value at path), append (push value onto the array at path) or remove. The edit is validated; invalid edits are rejected. New elements need unique ids above /idSeed (and raise idSeed).",
            obj(json!({"op": {"type": "string", "enum": ["set", "append", "remove"]}, "path": {"type": "string"}, "value": {}}), &["op", "path"]),
        ),
        s("get_tmdl", "Return the semantic model as TMDL code (tables, columns, measures, relationships).", obj(json!({}), &[])),
        s("apply_tmdl", "Apply a TMDL script to the model (create or alter measures, columns, tables, relationships), like Power BI's TMDL view.", obj(json!({"script": {"type": "string"}}), &["script"])),
        s("add_relationship", "Create a relationship 'from' (many side) → 'to' (one side), fields as Table[Column].", obj(json!({"from": {"type": "string"}, "to": {"type": "string"}, "cardinality": {"type": "string", "enum": ["manyToOne", "oneToOne", "oneToMany", "manyToMany"]}, "bothDirections": {"type": "boolean"}}), &["from", "to"])),
        s("add_calculated_column", "Add (or replace) a DAX calculated column on a table.", obj(json!({"table": {"type": "string"}, "name": {"type": "string"}, "dax": {"type": "string"}}), &["table", "name", "dax"])),
        s("update_query", "Replace the M code of an existing Power Query query and refresh.", obj(json!({"name": {"type": "string"}, "m": {"type": "string"}}), &["name", "m"])),
        s("delete", "Delete something: kind = element | page | measure | column | query | relationship | sheet | flow | workflow | bookmark; target = id or name.", obj(json!({"kind": {"type": "string"}, "target": {"type": "string"}}), &["kind", "target"])),
        s("move_element", "Move / resize an element (id, 'selected' or 'last').", obj(json!({"element": {"type": "string"}, "x": {"type": "number"}, "y": {"type": "number"}, "w": {"type": "number"}, "h": {"type": "number"}}), &["element"])),
        s("add_text", "Add a text box to the current page. Text can contain live values: {[Measure]}, {[Measure]:$#,0}, {Selected:Region}.", obj(json!({"text": {"type": "string"}, "x": {"type": "number"}, "y": {"type": "number"}, "w": {"type": "number"}, "h": {"type": "number"}, "size": {"type": "number"}, "bold": {"type": "boolean"}, "color": {"type": "string"}}), &["text"])),
        s("add_shape", "Add a Figma-style shape to the current page: rectangle, roundedRectangle, ellipse, line, arrow, triangle, star…", obj(json!({"shape": {"type": "string"}, "x": {"type": "number"}, "y": {"type": "number"}, "w": {"type": "number"}, "h": {"type": "number"}, "fill": {"type": "string"}, "text": {"type": "string"}}), &["shape"])),
        s("create_sheet", "Create an Excel-like data sheet (row 1 = headers) that loads into the model as a table.", obj(json!({"name": {"type": "string"}, "rows": {"type": "array", "items": {"type": "array", "items": {}}}}), &["name", "rows"])),
        s("set_cells", "Write values or formulas (=SUM(B2:B9)) into a data sheet starting at a cell like 'B2'; rows is a 2-D array.", obj(json!({"sheet": {"type": "string"}, "start": {"type": "string"}, "rows": {"type": "array", "items": {"type": "array", "items": {}}}}), &["sheet", "start", "rows"])),
        s("create_workflow", "Create a Data Science Studio workflow, empty or from an example: high_margin (classification), ols (regression and tests), arima (forecasting), segments (clustering).", obj(json!({"name": {"type": "string"}, "example": {"type": "string"}}), &[])),
        s("add_widget", "Add a widget to a Studio workflow (kinds like data.table, model.forest, eval.test_score, stats.ols, unsup.kmeans, deploy.register) with settings, optionally connected from another widget id.", obj(json!({"workflow": {"type": "string"}, "kind": {"type": "string"}, "settings": {"type": "object"}, "connect_from": {"type": "integer"}}), &["workflow", "kind"])),
        s("refresh", "Re-run all queries and rebuild the model.", obj(json!({}), &[])),
        s("navigate", "Show a view to the user: report, table-view, model, dax, tmdl, sheet, flow, story, sql, studio, formulator, ai, power-query; optionally a page (name or id).", obj(json!({"view": {"type": "string"}, "page": {"type": "string"}}), &["view"])),
        s("app_command", "Run an app command: save, undo, redo, publish, reading-view.", obj(json!({"command": {"type": "string"}}), &["command"])),
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
        "get_project" => get_project(ws, &s("path")),
        "edit_project" => edit_project(ws, &s("op"), &s("path"), a.get("value").cloned()),
        "get_tmdl" => ok(json!({"tmdl": vizcraft_core::tmdl::write(ws.project, ws.built)}), "Read the model as TMDL"),
        "apply_tmdl" => apply_tmdl(ws, &s("script")),
        "add_relationship" => add_relationship(ws, &s("from"), &s("to"), &s("cardinality"), a.get("bothDirections").and_then(Value::as_bool).unwrap_or(false)),
        "add_calculated_column" => add_calc_column(ws, &s("table"), &s("name"), &s("dax")),
        "update_query" => update_query(ws, &s("name"), &s("m")),
        "delete" => delete(ws, &s("kind"), &s("target")),
        "move_element" => move_element(ws, &s("element"), a),
        "add_text" => add_text(ws, a),
        "add_shape" => add_shape(ws, a),
        "create_sheet" => create_sheet(ws, &s("name"), a.get("rows").cloned().unwrap_or(json!([]))),
        "set_cells" => set_cells(ws, &s("sheet"), &s("start"), a.get("rows").cloned().unwrap_or(json!([]))),
        "create_workflow" => create_workflow(ws, &s("name"), &s("example")),
        "add_widget" => add_widget(ws, &s("workflow"), &s("kind"), a.get("settings").cloned(), a.get("connect_from").and_then(Value::as_u64)),
        "refresh" => {
            refresh(ws);
            let errs: Vec<String> = ws.built.errors.iter().map(|(o, e)| format!("{o}: {e}")).collect();
            let mut o = ok(json!({"tables": ws.built.tables.len(), "errors": errs}), format!("Refreshed {} tables", ws.built.tables.len()));
            o.mutated = true;
            o
        }
        "navigate" => {
            let mut o = ok(json!({"shown": s("view")}), format!("Opened {}", s("view")));
            o.actions.push(json!({"action": "navigate", "view": s("view"), "page": s("page")}));
            o
        }
        "app_command" => {
            let mut o = ok(json!({"command": s("command")}), format!("Ran {}", s("command")));
            o.actions.push(json!({"action": s("command")}));
            o
        }
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
// ============================================================== whole-app control

/// Shortens big JSON for the model: long arrays keep their first items and a count.
fn trim_json(v: &Value, depth: usize) -> Value {
    match v {
        Value::Array(a) if a.len() > 40 => {
            let mut out: Vec<Value> = a.iter().take(40).map(|x| trim_json(x, depth + 1)).collect();
            out.push(json!(format!("… {} more items", a.len() - 40)));
            Value::Array(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| trim_json(x, depth + 1)).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), trim_json(x, depth + 1))).collect()),
        Value::String(s) if s.len() > 4000 => json!(format!("{}… ({} chars)", &s[..s.char_indices().nth(4000).map(|(i, _)| i).unwrap_or(s.len())], s.len())),
        x => x.clone(),
    }
}

fn get_project(ws: &mut Workspace, path: &str) -> ToolOutcome {
    let doc = serde_json::to_value(&*ws.project).unwrap_or(Value::Null);
    if path.is_empty() || path == "/" {
        // An overview: top-level keys with sizes, page and element names.
        let pages: Vec<Value> = ws.project.report.pages.iter().enumerate().map(|(i, p)| json!({"index": i, "id": p.id, "name": p.name, "elements": p.elements.iter().map(|e| json!({"id": e.id, "name": e.name, "kind": e.kind_label()})).collect::<Vec<_>>()})).collect();
        let keys: Vec<Value> = doc.as_object().map(|m| m.iter().map(|(k, v)| json!({"key": k, "size": v.to_string().len()})).collect()).unwrap_or_default();
        return ok(json!({"keys": keys, "pages": pages, "currentPage": *ws.page, "idSeed": ws.project.id_seed}), "Read the project overview");
    }
    match doc.pointer(path) {
        Some(v) => ok(trim_json(v, 0), format!("Read {path}")),
        None => fail(format!("Nothing at `{path}`. Call get_project with path '' for an overview.")),
    }
}

fn edit_project(ws: &mut Workspace, op: &str, path: &str, value: Option<Value>) -> ToolOutcome {
    if path.is_empty() || !path.starts_with('/') {
        return fail("path must be a JSON pointer like /report/pages/0/name");
    }
    let mut doc = serde_json::to_value(&*ws.project).unwrap_or(Value::Null);
    let (parent_path, last) = path.rsplit_once('/').unwrap_or(("", path));
    let last = last.replace("~1", "/").replace("~0", "~");
    let res: Result<(), String> = (|| {
        match op {
            "set" => {
                let v = value.clone().ok_or("set needs a value")?;
                let target = doc.pointer_mut(path).ok_or(format!("Nothing at `{path}`"))?;
                *target = v;
            }
            "append" => {
                let v = value.clone().ok_or("append needs a value")?;
                match doc.pointer_mut(path) {
                    Some(Value::Array(a)) => a.push(v),
                    _ => return Err(format!("`{path}` is not an array")),
                }
            }
            "remove" => match doc.pointer_mut(parent_path) {
                Some(Value::Array(a)) => {
                    let i: usize = last.parse().map_err(|_| format!("`{last}` is not an index"))?;
                    if i >= a.len() {
                        return Err(format!("index {i} out of range"));
                    }
                    a.remove(i);
                }
                Some(Value::Object(m)) => {
                    m.remove(&last).ok_or(format!("No key `{last}`"))?;
                }
                _ => return Err(format!("Nothing at `{parent_path}`")),
            },
            other => return Err(format!("Unknown op `{other}` (set, append, remove)")),
        }
        Ok(())
    })();
    if let Err(e) = res {
        return fail(e);
    }
    let new: Project = match serde_json::from_value(doc) {
        Ok(p) => p,
        Err(e) => return fail(format!("That edit doesn't fit the project schema and was not applied: {e}. Read the surrounding JSON with get_project first.")),
    };
    // Keep ids unique: raise the seed above every id now in use.
    let max_id = serde_json::to_string(&new).ok().map(|s| max_numeric_id(&s)).unwrap_or(0);
    *ws.project = new;
    if ws.project.id_seed < max_id {
        ws.project.id_seed = max_id;
    }
    if ws.project.page(*ws.page).is_none() {
        *ws.page = ws.project.report.pages.first().map(|p| p.id).unwrap_or(0);
    }
    if path.starts_with("/queries") || path.starts_with("/sheets") || path.starts_with("/flows") {
        refresh(ws);
    } else {
        rebuild(ws);
    }
    let errs: Vec<String> = ws.built.errors.iter().take(5).map(|(o, e)| format!("{o}: {e}")).collect();
    let mut o = ok(json!({"applied": op, "path": path, "modelErrors": errs}), format!("Edited {path}"));
    o.mutated = true;
    o
}

/// The largest `"id": N` in a JSON text.
fn max_numeric_id(s: &str) -> u64 {
    let mut best = 0;
    for (i, _) in s.match_indices("\"id\":") {
        let rest = s[i + 5..].trim_start();
        let n: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(v) = n.parse::<u64>() {
            best = best.max(v);
        }
    }
    best
}

fn apply_tmdl(ws: &mut Workspace, script: &str) -> ToolOutcome {
    let before = ws.project.model.clone();
    match vizcraft_core::tmdl::apply_with(ws.project, Some(ws.built), script) {
        Ok(changes) => {
            rebuild(ws);
            let errs: Vec<String> = ws.built.errors.iter().take(5).map(|(o, e)| format!("{o}: {e}")).collect();
            if !errs.is_empty() && ws.built.errors.len() > 0 && changes.is_empty() {
                ws.project.model = before;
                rebuild(ws);
                return fail(format!("TMDL applied nothing: {}", errs.join("; ")));
            }
            let mut o = ok(json!({"changes": changes, "modelErrors": errs}), format!("Applied TMDL ({} change(s))", changes.len()));
            o.mutated = true;
            o
        }
        Err(e) => fail(format!("TMDL error: {e}")),
    }
}

fn split_field(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    let (t, c) = s.split_once('[')?;
    Some((t.trim().trim_matches('\'').to_string(), c.trim_end_matches(']').to_string()))
}

fn add_relationship(ws: &mut Workspace, from: &str, to: &str, card: &str, both: bool) -> ToolOutcome {
    let (Some((ft, fc)), Some((tt, tc))) = (split_field(from), split_field(to)) else { return fail("Fields must look like Table[Column].") };
    for (t, c) in [(&ft, &fc), (&tt, &tc)] {
        if ws.built.table(t).and_then(|d| d.column_index(c)).is_none() {
            return fail(format!("No column {t}[{c}]."));
        }
    }
    let id = ws.project.next_id();
    let cardinality = match card {
        "oneToOne" => Cardinality::OneToOne,
        "oneToMany" => Cardinality::OneToMany,
        "manyToMany" => Cardinality::ManyToMany,
        _ => Cardinality::ManyToOne,
    };
    let active = !ws.project.model.relationships.iter().any(|r| r.active && ((r.from_table == ft && r.to_table == tt) || (r.from_table == tt && r.to_table == ft)));
    ws.project.model.relationships.push(Relationship { id, from_table: ft.clone(), from_column: fc.clone(), to_table: tt.clone(), to_column: tc.clone(), cardinality, cross_filter: if both { CrossFilterDir::Both } else { CrossFilterDir::Single }, active, ..Default::default() });
    rebuild(ws);
    let mut o = ok(json!({"relationship": id, "active": active}), format!("Related {ft}[{fc}] → {tt}[{tc}]{}", if active { "" } else { " (inactive: another path exists)" }));
    o.mutated = true;
    o
}

fn add_calc_column(ws: &mut Workspace, table: &str, name: &str, dax: &str) -> ToolOutcome {
    let Some(t) = ws.project.model.tables.iter().find(|t| t.name.eq_ignore_ascii_case(table)).map(|t| t.name.clone()) else { return fail(format!("No table `{table}`.")) };
    let before = ws.project.model.calculated_columns.clone();
    ws.project.model.calculated_columns.retain(|c| !(c.table == t && c.name.eq_ignore_ascii_case(name)));
    ws.project.model.calculated_columns.push(CalcColumn { table: t.clone(), name: name.to_string(), expression: dax.trim().trim_start_matches('=').trim().to_string(), origin: "Copilot".into() });
    rebuild(ws);
    if let Some((_, e)) = ws.built.errors.iter().find(|(o, _)| o.contains(name)) {
        let e = e.clone();
        ws.project.model.calculated_columns = before;
        rebuild(ws);
        return fail(format!("The column didn't compute and was not added: {e}"));
    }
    let mut o = ok(json!({"table": t, "column": name}), format!("Added column {t}[{name}]"));
    o.mutated = true;
    o
}

fn update_query(ws: &mut Workspace, name: &str, m: &str) -> ToolOutcome {
    let Some(q) = ws.project.queries.queries.iter_mut().find(|q| q.name.eq_ignore_ascii_case(name)) else { return fail(format!("No query `{name}`.")) };
    let old = std::mem::replace(&mut q.m, m.to_string());
    let qname = q.name.clone();
    if let Err(e) = eval_m(ws.project, &format!("#\"{}\"", qname.replace('"', "\"\""))) {
        if let Some(q) = ws.project.queries.query_mut(&qname) {
            q.m = old;
        }
        return fail(format!("The new M didn't evaluate and was not kept: {e}"));
    }
    refresh(ws);
    let mut o = ok(json!({"query": qname, "rows": ws.built.table(&qname).map(|d| d.len())}), format!("Updated query {qname}"));
    o.mutated = true;
    o
}

fn delete(ws: &mut Workspace, kind: &str, target: &str) -> ToolOutcome {
    let t = target.trim();
    let id: Option<u64> = t.parse().ok();
    let eq = |a: &str| a.eq_ignore_ascii_case(t);
    let removed = match kind {
        "element" | "visual" => {
            let eid = find_element(ws, t);
            let mut n = 0;
            for p in &mut ws.project.report.pages {
                let before = p.elements.len();
                p.elements.retain(|e| eid.is_none() || (Some(e.id) != eid && e.parent != eid));
                n += before - p.elements.len();
            }
            n
        }
        "page" => {
            let before = ws.project.report.pages.len();
            if before <= 1 {
                return fail("A report keeps at least one page.");
            }
            ws.project.report.pages.retain(|p| !(Some(p.id) == id || eq(&p.name)));
            if ws.project.page(*ws.page).is_none() {
                *ws.page = ws.project.report.pages[0].id;
            }
            before - ws.project.report.pages.len()
        }
        "measure" => {
            let b = ws.project.model.measures.len();
            ws.project.model.measures.retain(|m| !eq(&m.name) && !eq(&format!("[{}]", m.name)));
            b - ws.project.model.measures.len()
        }
        "column" => {
            let b = ws.project.model.calculated_columns.len();
            let (tt, cc) = split_field(t).unwrap_or_default();
            ws.project.model.calculated_columns.retain(|c| !(c.name.eq_ignore_ascii_case(if cc.is_empty() { t } else { &cc }) && (tt.is_empty() || c.table.eq_ignore_ascii_case(&tt))));
            b - ws.project.model.calculated_columns.len()
        }
        "query" => {
            let b = ws.project.queries.queries.len();
            ws.project.queries.queries.retain(|q| !eq(&q.name));
            b - ws.project.queries.queries.len()
        }
        "relationship" => {
            let b = ws.project.model.relationships.len();
            ws.project.model.relationships.retain(|r| Some(r.id) != id);
            b - ws.project.model.relationships.len()
        }
        "sheet" => {
            let b = ws.project.sheets.len();
            ws.project.sheets.retain(|s| !eq(&s.name));
            b - ws.project.sheets.len()
        }
        "flow" => {
            let b = ws.project.flows.len();
            ws.project.flows.retain(|s| !eq(&s.name));
            b - ws.project.flows.len()
        }
        "workflow" => {
            let b = ws.project.studio.len();
            ws.project.studio.retain(|s| !eq(&s.name));
            b - ws.project.studio.len()
        }
        "bookmark" => {
            let b = ws.project.report.bookmarks.len();
            ws.project.report.bookmarks.retain(|s| !(Some(s.id) == id || eq(&s.name)));
            b - ws.project.report.bookmarks.len()
        }
        other => return fail(format!("Can't delete a `{other}`.")),
    };
    if removed == 0 {
        return fail(format!("No {kind} `{t}`."));
    }
    if matches!(kind, "query" | "sheet" | "flow") {
        refresh(ws);
    } else {
        rebuild(ws);
    }
    let mut o = ok(json!({"deleted": removed}), format!("Deleted {kind} {t}"));
    o.mutated = true;
    o
}

fn move_element(ws: &mut Workspace, which: &str, a: &Value) -> ToolOutcome {
    let Some(id) = find_element(ws, which) else { return fail(format!("No element `{which}`.")) };
    let Some(e) = ws.project.element_mut(id) else { return fail("No element.") };
    let f = |k: &str| a.get(k).and_then(Value::as_f64).map(|v| v as f32);
    if let Some(v) = f("x") {
        e.x = v;
    }
    if let Some(v) = f("y") {
        e.y = v;
    }
    if let Some(v) = f("w") {
        e.w = v.max(8.0);
    }
    if let Some(v) = f("h") {
        e.h = v.max(8.0);
    }
    let mut o = ok(json!({"element": id, "x": e.x, "y": e.y, "w": e.w, "h": e.h}), format!("Moved #{id}"));
    o.mutated = true;
    o.elements = vec![id];
    o
}

fn place(ws: &mut Workspace, a: &Value, w: f32, h: f32) -> (f32, f32, f32, f32) {
    let w = a.get("w").and_then(Value::as_f64).map(|v| v as f32).unwrap_or(w);
    let h = a.get("h").and_then(Value::as_f64).map(|v| v as f32).unwrap_or(h);
    let (fx, fy) = ws.project.page(*ws.page).map(|p| free_spot(p, w, h)).unwrap_or((40.0, 40.0));
    (a.get("x").and_then(Value::as_f64).map(|v| v as f32).unwrap_or(fx), a.get("y").and_then(Value::as_f64).map(|v| v as f32).unwrap_or(fy), w, h)
}

fn push_element(ws: &mut Workspace, mut e: Element, summary: String) -> ToolOutcome {
    let id = ws.project.next_id();
    e.id = id;
    let Some(p) = ws.project.page_mut(*ws.page) else { return fail("No current page.") };
    p.elements.push(e);
    let mut o = ok(json!({"element": id}), summary);
    o.mutated = true;
    o.elements = vec![id];
    o
}

fn add_text(ws: &mut Workspace, a: &Value) -> ToolOutcome {
    let text = a.get("text").and_then(Value::as_str).unwrap_or("").to_string();
    let (x, y, w, h) = place(ws, a, 360.0, 60.0);
    let mut rt = RichText::plain(&text);
    for r in &mut rt.runs {
        if let Some(sz) = a.get("size").and_then(Value::as_f64) {
            r.size = sz as f32;
        }
        if let Some(b) = a.get("bold").and_then(Value::as_bool) {
            r.bold = b;
        }
        if let Some(c) = a.get("color").and_then(Value::as_str).and_then(vizcraft_core::color::Color::parse) {
            r.color = Some(Prop::Color(c));
        }
    }
    let e = Element { name: "Text".into(), x, y, w, h, kind: ElementKind::Text(TextSpec { text: rt, smart_narrative: false }), ..Element::default() };
    push_element(ws, e, format!("Added text \"{}\"", text.chars().take(40).collect::<String>()))
}

fn add_shape(ws: &mut Workspace, a: &Value) -> ToolOutcome {
    let name = a.get("shape").and_then(Value::as_str).unwrap_or("rectangle");
    let shape: ShapeKind = serde_json::from_value(json!(name)).or_else(|_| serde_json::from_value(json!(cap(name)))).unwrap_or_default();
    let (x, y, w, h) = place(ws, a, 200.0, 120.0);
    let mut spec = ShapeSpec { shape, ..ShapeSpec::default() };
    if let Some(c) = a.get("fill").and_then(Value::as_str).and_then(vizcraft_core::color::Color::parse) {
        spec.fill = Fill::Solid(c);
    }
    if let Some(t) = a.get("text").and_then(Value::as_str) {
        spec.text = RichText::plain(t);
    }
    let e = Element { name: format!("{shape:?}"), x, y, w, h, kind: ElementKind::Shape(spec), ..Element::default() };
    push_element(ws, e, format!("Added a {name}"))
}

fn cell_rows(v: &Value) -> Vec<Vec<String>> {
    v.as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| {
                    r.as_array()
                        .map(|c| {
                            c.iter()
                                .map(|x| match x {
                                    Value::String(s) => s.clone(),
                                    Value::Null => String::new(),
                                    o => o.to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn create_sheet(ws: &mut Workspace, name: &str, rows: Value) -> ToolOutcome {
    if name.is_empty() {
        return fail("A sheet needs a name.");
    }
    if ws.project.sheets.iter().any(|s| s.name.eq_ignore_ascii_case(name)) || ws.project.model.tables.iter().any(|t| t.name.eq_ignore_ascii_case(name)) {
        return fail(format!("`{name}` already exists."));
    }
    let cells = cell_rows(&rows);
    let n = cells.len().saturating_sub(1);
    ws.project.sheets.push(DataSheet { name: name.to_string(), cells, load: true, ..Default::default() });
    refresh(ws);
    let mut o = ok(json!({"sheet": name, "rows": n}), format!("Created sheet {name} ({n} rows)"));
    o.mutated = true;
    o
}

/// "B12" → (row 11, col 1).
fn parse_cell(s: &str) -> Option<(usize, usize)> {
    let s = s.trim().to_uppercase();
    let letters: String = s.chars().take_while(char::is_ascii_alphabetic).collect();
    let digits: String = s.chars().skip(letters.len()).collect();
    let mut col = 0usize;
    for c in letters.chars() {
        col = col * 26 + (c as usize - 'A' as usize + 1);
    }
    let row: usize = digits.parse().ok()?;
    if col == 0 || row == 0 {
        return None;
    }
    Some((row - 1, col - 1))
}

fn set_cells(ws: &mut Workspace, sheet: &str, start: &str, rows: Value) -> ToolOutcome {
    let Some((r0, c0)) = parse_cell(start) else { return fail(format!("`{start}` is not a cell like B2.")) };
    let Some(sh) = ws.project.sheets.iter_mut().find(|s| s.name.eq_ignore_ascii_case(sheet)) else { return fail(format!("No sheet `{sheet}`.")) };
    let data = cell_rows(&rows);
    for (i, row) in data.iter().enumerate() {
        let r = r0 + i;
        while sh.cells.len() <= r {
            sh.cells.push(vec![]);
        }
        for (j, v) in row.iter().enumerate() {
            let c = c0 + j;
            let line = &mut sh.cells[r];
            while line.len() <= c {
                line.push(String::new());
            }
            line[c] = v.clone();
        }
    }
    let name = sh.name.clone();
    refresh(ws);
    let mut o = ok(json!({"sheet": name, "written": data.iter().map(Vec::len).sum::<usize>()}), format!("Wrote cells in {name} from {start}"));
    o.mutated = true;
    o
}

fn create_workflow(ws: &mut Workspace, name: &str, example: &str) -> ToolOutcome {
    let mut wf = if example.is_empty() { vizcraft_core::studio::StudioWorkflow::default() } else {
        match vizcraft_core::studio::examples::build(example) {
            Some(w) => w,
            None => return fail(format!("No example `{example}` (high_margin, ols, arima, segments).")),
        }
    };
    if !name.is_empty() {
        wf.name = name.to_string();
    } else if wf.name.is_empty() {
        wf.name = format!("Workflow {}", ws.project.studio.len() + 1);
    }
    let widgets: Vec<Value> = wf.widgets.iter().map(|w| json!({"id": w.id, "kind": w.kind, "label": w.label})).collect();
    let n = wf.name.clone();
    ws.project.studio.push(wf);
    let mut o = ok(json!({"workflow": n, "index": ws.project.studio.len() - 1, "widgets": widgets}), format!("Created workflow {n}"));
    o.mutated = true;
    o.actions.push(json!({"action": "navigate", "view": "studio"}));
    o
}

fn add_widget(ws: &mut Workspace, workflow: &str, kind: &str, settings: Option<Value>, from: Option<u64>) -> ToolOutcome {
    if vizcraft_core::studio::widget_type(kind).is_none() {
        let kinds: Vec<&str> = vizcraft_core::studio::catalog().iter().map(|w| w.id).collect();
        return fail(format!("Unknown widget `{kind}`. Kinds: {}", kinds.join(", ")));
    }
    let Some(wf) = ws.project.studio.iter_mut().find(|w| w.name.eq_ignore_ascii_case(workflow) || workflow.parse::<usize>().ok().is_some_and(|_| false)) else { return fail(format!("No workflow `{workflow}`.")) };
    let (x, y) = from.and_then(|f| wf.widget(f)).map(|w| (w.x + 200.0, w.y)).unwrap_or((40.0 + wf.widgets.len() as f32 * 30.0, 60.0 + wf.widgets.len() as f32 * 40.0));
    let id = wf.add(kind, x, y);
    if let (Some(Value::Object(m)), Some(w)) = (settings, wf.widget_mut(id)) {
        for (k, v) in m {
            w.settings.insert(k, v);
        }
    }
    let mut linked = Value::Null;
    if let Some(f) = from {
        let pair = (wf.widget(f).and_then(|w| vizcraft_core::studio::widget_type(&w.kind)), vizcraft_core::studio::widget_type(kind));
        if let (Some(a), Some(b)) = pair
            && let Some((op, ip)) = a.outputs.iter().enumerate().find_map(|(oi, o)| b.inputs.iter().position(|i| o.chan.fits(i.chan)).map(|ii| (oi, ii)))
        {
            linked = match wf.connect(f, op, id, ip) {
                Ok(()) => json!(true),
                Err(e) => json!(e),
            };
        }
    }
    let mut o = ok(json!({"widget": id, "connected": linked}), format!("Added {kind} to {workflow}"));
    o.mutated = true;
    o
}

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

    #[test]
    fn whole_app_tools_edit_everything() {
        let (mut p, mut b, mut page) = sample();
        // Read the overview and a pointer.
        let ov = ws_run(&mut p, &mut b, &mut page, "get_project", json!({"path": ""}));
        assert!(ov.ok && ov.text.contains("pages"), "{}", ov.text);
        assert!(ws_run(&mut p, &mut b, &mut page, "get_project", json!({"path": "/report/theme"})).ok);
        // edit_project: rename a page, reject a schema-breaking edit.
        let r = ws_run(&mut p, &mut b, &mut page, "edit_project", json!({"op": "set", "path": "/report/pages/0/name", "value": "Overview!"}));
        assert!(r.ok, "{}", r.text);
        assert_eq!(p.report.pages[0].name, "Overview!");
        let bad = ws_run(&mut p, &mut b, &mut page, "edit_project", json!({"op": "set", "path": "/report/pages/0/width", "value": "wide"}));
        assert!(!bad.ok);
        // Text and shapes, moving, deleting.
        let t = ws_run(&mut p, &mut b, &mut page, "add_text", json!({"text": "Sales {[Total Sales]:$#,0}", "size": 20, "bold": true, "color": "#2563EB"}));
        assert!(t.ok, "{}", t.text);
        let tid = t.elements[0];
        let sh = ws_run(&mut p, &mut b, &mut page, "add_shape", json!({"shape": "ellipse", "fill": "#ff0000"}));
        assert!(sh.ok, "{}", sh.text);
        assert!(ws_run(&mut p, &mut b, &mut page, "move_element", json!({"element": tid.to_string(), "x": 10, "y": 12})).ok);
        assert_eq!(p.element(tid).map(|(_, e)| e.x), Some(10.0));
        assert!(ws_run(&mut p, &mut b, &mut page, "delete", json!({"kind": "element", "target": sh.elements[0].to_string()})).ok);
        // Model: calculated column, relationship, TMDL round trip.
        let cc = ws_run(&mut p, &mut b, &mut page, "add_calculated_column", json!({"table": "Sales", "name": "Big", "dax": "IF(Sales[Quantity] > 2, \"Yes\", \"No\")"}));
        assert!(cc.ok, "{}", cc.text);
        let tm = ws_run(&mut p, &mut b, &mut page, "get_tmdl", json!({}));
        assert!(tm.ok && tm.text.contains("measure"), "{}", &tm.text[..200.min(tm.text.len())]);
        // Sheets and cells feed the model.
        let sh = ws_run(&mut p, &mut b, &mut page, "create_sheet", json!({"name": "Goals", "rows": [["Region", "Goal"], ["South", 100], ["West", 80]]}));
        assert!(sh.ok, "{}", sh.text);
        assert!(ws_run(&mut p, &mut b, &mut page, "set_cells", json!({"sheet": "Goals", "start": "B3", "rows": [[90]]})).ok);
        assert_eq!(p.sheets.iter().find(|s| s.name == "Goals").map(|s| s.cells[2][1].clone()), Some("90".into()));
        assert!(b.table("Goals").is_some());
        let rel = ws_run(&mut p, &mut b, &mut page, "add_relationship", json!({"from": "Goals[Region]", "to": "Targets[Region]", "cardinality": "oneToOne"}));
        assert!(rel.ok, "{}", rel.text);
        // Queries.
        let q = p.queries.queries[0].name.clone();
        let m = p.queries.queries[0].m.clone();
        assert!(!ws_run(&mut p, &mut b, &mut page, "update_query", json!({"name": q, "m": "let x = in"})).ok);
        assert_eq!(p.queries.queries[0].m, m, "a broken query is not kept");
        // Studio.
        let wf = ws_run(&mut p, &mut b, &mut page, "create_workflow", json!({"name": "Churn", "example": "high_margin"}));
        assert!(wf.ok && wf.actions.iter().any(|a| a["view"] == "studio"));
        let first = p.studio[0].widgets[0].id;
        let w = ws_run(&mut p, &mut b, &mut page, "add_widget", json!({"workflow": "Churn", "kind": "data.datatable", "connect_from": first}));
        assert!(w.ok && w.text.contains("true"), "{}", w.text);
        // Navigation is an app action.
        let n = ws_run(&mut p, &mut b, &mut page, "navigate", json!({"view": "model"}));
        assert_eq!(n.actions[0]["view"], "model");
        assert!(specs().iter().all(|s| !s.description.is_empty()));
    }
}
