//! The editor's dialogs. Each OK adds an M step (or edits the query); an error keeps the dialog
//! open with the message.

use egui::{RichText, Ui, vec2};
use vizcraft_query::steps::{self, field, text_list};
use vizcraft_query::value::quote_text;
use vizcraft_query::{Table, Value};

use super::forms::{combo, error_line, fit, index_combo, ok_cancel, window};
use super::{add_step, open_editor, ops, preview_table};
use crate::app::VizApp;
use crate::theme::Tokens;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeepRemove {
    KeepTop,
    KeepBottom,
    KeepRange,
    RemoveTop,
    RemoveBottom,
    Alternate,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SplitMode {
    Delimiter,
    Chars,
    Positions,
    LowerUpper,
    UpperLower,
    DigitNon,
    NonDigit,
}

pub struct Dialog {
    pub kind: Kind,
    pub error: Option<String>,
}

pub enum Kind {
    Param { op: usize, add: bool, value: String },
    Rows { kind: KeepRemove, a: String, b: String, c: String },
    Split { col: String, mode: SplitMode, delim: String, custom: String, at: usize, n: String, parts: usize },
    GroupBy { advanced: bool, groups: Vec<String>, aggs: Vec<(String, usize, String)> },
    Replace { cols: Vec<String>, find: String, replace: String, entire: bool },
    ReplaceErrors { cols: Vec<String>, value: String },
    Merge { as_new: bool, other: String, left: Vec<String>, right: Vec<String>, kind: usize },
    Append { as_new: bool, many: bool, picked: Vec<String>, second: String, avail_sel: Option<String> },
    Parameters { selected: Option<String>, name: String, kind: String, value: String, description: String, suggestions: String },
    Advanced { m: String },
    ChooseColumns { keep: Vec<(String, bool)>, search: String },
    RenameColumn { old: String, new: String },
    RenameQuery { old: String, new: String },
    RenameStep { old: String, new: String },
    Properties { name: String, description: String, group: String, load: bool },
    Custom { name: String, formula: String, ty: usize },
    Conditional { name: String, rules: Vec<(String, usize, String, String)>, otherwise: String },
    Pivot { col: String, values: String, agg: usize },
    Index { start: String, step: String },
    MergeColumns { cols: Vec<String>, sep: usize, custom: String, name: String, add: bool },
    Expand { col: String, names: Vec<(String, bool)>, prefix: bool, record: bool, list: bool, aggregate: bool, aggs: Vec<(String, usize)> },
    FilterRows { col: String, rules: Vec<(usize, String)>, and: bool },
    Python { script: String, output: String },
    Invoke { name: String, function: String, args: Vec<String> },
    StepSettings { step: String, expr: String, path: Option<(String, String)> },
    NewGroup { old: Option<String>, name: String, query: Option<String> },
    Reference { search: String },
    Shortcuts,
}

fn d(kind: Kind) -> Dialog {
    Dialog { kind, error: None }
}

pub const SEPARATORS: &[(&str, &str)] = &[("None", ""), ("Colon", ":"), ("Comma", ","), ("Equals Sign", "="), ("Semicolon", ";"), ("Space", " "), ("Tab", "\t"), ("Custom", "")];
pub const AGG_OPS: &[&str] = &["Count Rows", "Count Distinct Rows", "Sum", "Average", "Median", "Min", "Max", "All Rows"];
pub const COND_OPS: &[&str] = &[
    "equals",
    "does not equal",
    "is greater than",
    "is greater than or equal to",
    "is less than",
    "is less than or equal to",
    "begins with",
    "does not begin with",
    "ends with",
    "does not end with",
    "contains",
    "does not contain",
];
pub const JOIN_KINDS: &[(&str, &str)] = &[
    ("Left Outer (all from first, matching from second)", "LeftOuter"),
    ("Right Outer (all from second, matching from first)", "RightOuter"),
    ("Full Outer (all rows from both)", "FullOuter"),
    ("Inner (only matching rows)", "Inner"),
    ("Left Anti (rows only in first)", "LeftAnti"),
    ("Right Anti (rows only in second)", "RightAnti"),
];
const PIVOT_AGGS: &[(&str, &str)] =
    &[("Count (All)", "List.Count"), ("Count (Not Blank)", "List.NonNullCount"), ("Minimum", "List.Min"), ("Maximum", "List.Max"), ("Median", "List.Median"), ("Sum", "List.Sum"), ("Average", "List.Average"), ("Don't Aggregate", "")];
const CUSTOM_TYPES: &[(&str, &str)] = &[("Any", ""), ("Text", "type text"), ("Decimal Number", "type number"), ("Whole Number", "Int64.Type"), ("Date", "type date"), ("True/False", "type logical")];
const EXPAND_AGGS: &[(&str, &str)] = &[("Sum", "List.Sum"), ("Average", "List.Average"), ("Minimum", "List.Min"), ("Maximum", "List.Max"), ("Count (All)", "List.Count"), ("Count (Not Blank)", "List.NonNullCount"), ("Median", "List.Median")];

impl Dialog {
    pub fn param(op: usize, add: bool) -> Dialog {
        let default = ops::OPS.get(op).and_then(|o| o.param).map(|p| p.1.to_string()).unwrap_or_default();
        d(Kind::Param { op, add, value: default })
    }
    pub fn rows(kind: KeepRemove) -> Dialog {
        d(Kind::Rows { kind, a: "1".into(), b: "1".into(), c: "1".into() })
    }
    pub fn split(col: &str, mode: SplitMode, _table: &Table) -> Dialog {
        d(Kind::Split { col: col.into(), mode, delim: "Comma".into(), custom: String::new(), at: 2, n: if mode == SplitMode::Positions { "0, 3".into() } else { "1".into() }, parts: 0 })
    }
    pub fn group_by(app: &mut VizApp) -> Dialog {
        let g = app.pq.selected.clone();
        d(Kind::GroupBy { advanced: g.len() > 1, groups: if g.is_empty() { vec![String::new()] } else { g }, aggs: vec![("Count".into(), 0, String::new())] })
    }
    pub fn replace(cols: Vec<String>) -> Dialog {
        d(Kind::Replace { cols, find: String::new(), replace: String::new(), entire: false })
    }
    pub fn replace_errors(cols: Vec<String>) -> Dialog {
        d(Kind::ReplaceErrors { cols, value: "null".into() })
    }
    pub fn merge(app: &mut VizApp, as_new: bool) -> Dialog {
        d(Kind::Merge { as_new, other: String::new(), left: app.pq.selected.clone(), right: vec![], kind: 0 })
    }
    pub fn append(as_new: bool) -> Dialog {
        d(Kind::Append { as_new, many: false, picked: vec![], second: String::new(), avail_sel: None })
    }
    pub fn parameters(app: &mut VizApp, new: Option<String>) -> Dialog {
        let doc = &app.project.queries;
        let sel = match &new {
            Some(n) if !n.is_empty() => Some(n.clone()),
            Some(_) => None,
            None => doc.parameters.first().map(|p| p.name.clone()),
        };
        let p = sel.as_ref().and_then(|n| doc.parameters.iter().find(|p| &p.name == n));
        d(Kind::Parameters {
            selected: sel.clone().filter(|_| p.is_some()),
            name: p.map(|p| p.name.clone()).unwrap_or_else(|| format!("Parameter{}", doc.parameters.len() + 1)),
            kind: p.map(|p| p.kind.clone()).unwrap_or_else(|| "Text".into()),
            value: p.map(|p| p.value.clone()).unwrap_or_else(|| "\"\"".into()),
            description: p.map(|p| p.description.clone()).unwrap_or_default(),
            suggestions: p.map(|p| p.suggestions.join(", ")).unwrap_or_default(),
        })
    }
    pub fn advanced(app: &mut VizApp) -> Dialog {
        d(Kind::Advanced { m: app.project.queries.query(&app.pq.query).map(|q| q.m.clone()).unwrap_or_default() })
    }
    pub fn choose_columns(app: &mut VizApp) -> Dialog {
        let t = preview_table(app);
        d(Kind::ChooseColumns { keep: t.columns.iter().map(|c| (c.clone(), true)).collect(), search: String::new() })
    }
    pub fn rename_column(c: &str) -> Dialog {
        d(Kind::RenameColumn { old: c.into(), new: c.into() })
    }
    pub fn rename_query(q: &str) -> Dialog {
        d(Kind::RenameQuery { old: q.into(), new: q.into() })
    }
    pub fn rename_step(s: &str) -> Dialog {
        d(Kind::RenameStep { old: s.into(), new: s.into() })
    }
    pub fn properties(app: &mut VizApp) -> Dialog {
        let q = app.project.queries.query(&app.pq.query);
        d(Kind::Properties {
            name: app.pq.query.clone(),
            description: q.map(|q| q.description.clone()).unwrap_or_default(),
            group: q.and_then(|q| q.group.clone()).unwrap_or_default(),
            load: q.is_some_and(|q| q.load != steps::LoadTo::None),
        })
    }
    pub fn custom_column(_app: &mut VizApp) -> Dialog {
        d(Kind::Custom { name: "Custom".into(), formula: String::new(), ty: 0 })
    }
    pub fn conditional(app: &mut VizApp) -> Dialog {
        let first = app.pq.selected.first().cloned().or_else(|| preview_table(app).columns.first().cloned()).unwrap_or_default();
        d(Kind::Conditional { name: "Custom".into(), rules: vec![(first, 0, String::new(), String::new())], otherwise: String::new() })
    }
    pub fn pivot(app: &mut VizApp, col: &str) -> Dialog {
        let t = preview_table(app);
        let values = t.columns.iter().find(|c| *c != col).cloned().unwrap_or_default();
        d(Kind::Pivot { col: col.into(), values, agg: 5 })
    }
    pub fn index() -> Dialog {
        d(Kind::Index { start: "1".into(), step: "1".into() })
    }
    pub fn merge_columns(cols: Vec<String>, add: bool) -> Dialog {
        d(Kind::MergeColumns { cols, sep: 0, custom: String::new(), name: "Merged".into(), add })
    }
    pub fn expand(app: &mut VizApp, col: &str, aggregate: bool) -> Dialog {
        let t = preview_table(app);
        let mut names: Vec<String> = Vec::new();
        let (mut record, mut list) = (false, false);
        if let Some(c) = t.column_index(col) {
            for r in t.rows.iter().take(300) {
                match r.get(c) {
                    Some(Value::Table(nt)) => {
                        for n in &nt.columns {
                            if !names.contains(n) {
                                names.push(n.clone());
                            }
                        }
                    }
                    Some(Value::Record(rec)) => {
                        record = true;
                        for n in &rec.names {
                            if !names.contains(n) {
                                names.push(n.clone());
                            }
                        }
                    }
                    Some(Value::List(_)) => list = true,
                    _ => {}
                }
            }
        }
        let aggs = names.iter().map(|n| (n.clone(), 0usize)).collect();
        d(Kind::Expand { col: col.into(), names: names.into_iter().map(|n| (n, !aggregate)).collect(), prefix: true, record, list, aggregate, aggs })
    }
    /// Filter Rows with a condition (`op` is 1-based into [`COND_OPS`]).
    pub fn filter_rows(_t: &Table, col: &str, op: usize) -> Dialog {
        d(Kind::FilterRows { col: col.into(), rules: vec![(op.saturating_sub(1), String::new()), (0, String::new())], and: true })
    }
    pub fn python() -> Dialog {
        d(Kind::Python { script: "# 'dataset' holds the input data for this script\n# e.g. dataset['Total'] = dataset['Units'] * dataset['Price']\n".into(), output: "dataset".into() })
    }
    pub fn invoke(app: &mut VizApp) -> Dialog {
        let f = app.project.queries.queries.iter().find(|q| is_function(&q.m)).map(|q| q.name.clone()).unwrap_or_default();
        d(Kind::Invoke { name: "Invoked".into(), function: f, args: app.pq.selected.clone() })
    }
    pub fn step_settings(app: &mut VizApp, step: &str) -> Dialog {
        let expr = super::steps(app).into_iter().find(|s| s.name == step).map(|s| s.expr).unwrap_or_default();
        let path = ["File.Contents(\"", "Folder.Files(\"", "Web.Contents(\"", "Sqlite.Database(\"", "OData.Feed(\""].iter().find_map(|f| {
            let i = expr.find(f)?;
            let rest = expr.get(i + f.len()..)?;
            let end = rest.find('"')?;
            Some((f.trim_end_matches("(\"").to_string(), rest.get(..end)?.to_string()))
        });
        d(Kind::StepSettings { step: step.into(), expr, path })
    }
    pub fn new_group(old: Option<String>) -> Dialog {
        d(Kind::NewGroup { name: old.clone().unwrap_or_else(|| "New Group".into()), old, query: None })
    }
    pub fn new_group_for(query: &str) -> Dialog {
        d(Kind::NewGroup { name: "New Group".into(), old: None, query: Some(query.into()) })
    }
    pub fn reference() -> Dialog {
        d(Kind::Reference { search: String::new() })
    }
    pub fn shortcuts() -> Dialog {
        d(Kind::Shortcuts)
    }
}

fn is_function(m: &str) -> bool {
    let t = m.trim_start();
    t.starts_with('(') && t.contains("=>")
}

/// An M literal for typed text: numbers stay numbers, `null` is null, the rest is text.
pub fn literal(s: &str) -> String {
    let t = s.trim();
    if t == "null" {
        return "null".into();
    }
    if t == "true" || t == "false" {
        return t.into();
    }
    if t.parse::<f64>().is_ok() && (!t.starts_with('0') || t == "0" || t.starts_with("0.")) {
        return t.to_string();
    }
    if t.len() == 10
        && let Ok(d) = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d")
    {
        use chrono::Datelike;
        return format!("#date({}, {}, {})", d.year(), d.month(), d.day());
    }
    quote_text(s)
}

pub fn cond_expr(col: &str, op: usize, value: &str) -> String {
    let f = field(col);
    let v = literal(value);
    let tv = quote_text(value);
    match op {
        0 => format!("{f} = {v}"),
        1 => format!("{f} <> {v}"),
        2 => format!("{f} > {v}"),
        3 => format!("{f} >= {v}"),
        4 => format!("{f} < {v}"),
        5 => format!("{f} <= {v}"),
        6 => format!("Text.StartsWith(Text.From({f}), {tv})"),
        7 => format!("not Text.StartsWith(Text.From({f}), {tv})"),
        8 => format!("Text.EndsWith(Text.From({f}), {tv})"),
        9 => format!("not Text.EndsWith(Text.From({f}), {tv})"),
        10 => format!("Text.Contains(Text.From({f}), {tv})"),
        _ => format!("not Text.Contains(Text.From({f}), {tv})"),
    }
}

/// The condition of a value checklist (`None`: everything kept).
pub fn filter_condition(col: &str, values: &[(Value, bool)], op: usize, value: &str, truncated: bool) -> Option<String> {
    let f = field(col);
    if op > 0 {
        return Some(cond_expr(col, op - 1, value));
    }
    if values.iter().all(|(_, k)| *k) {
        return None;
    }
    let kept: Vec<String> = values.iter().filter(|(_, k)| *k).map(|(v, _)| v.m_literal()).collect();
    let dropped: Vec<String> = values.iter().filter(|(_, k)| !*k).map(|(v, _)| v.m_literal()).collect();
    // With a truncated list, or when fewer values are unticked, say what goes.
    if truncated || dropped.len() < kept.len() {
        return Some(if dropped.len() <= 3 { dropped.iter().map(|v| format!("{f} <> {v}")).collect::<Vec<_>>().join(" and ") } else { format!("not List.Contains({{{}}}, {f})", dropped.join(", ")) });
    }
    Some(if kept.len() <= 4 {
        if kept.is_empty() { "false".to_string() } else { kept.iter().map(|v| format!("{f} = {v}")).collect::<Vec<_>>().join(" or ") }
    } else {
        format!("List.Contains({{{}}}, {f})", kept.join(", "))
    })
}

pub fn show(app: &mut VizApp, ctx: &egui::Context) {
    let Some(mut dlg) = app.pq.dialog.take() else { return };
    let t = app.tokens();
    let keep = edit_dialog(app, ctx, &t, &mut dlg);
    if keep && app.pq.dialog.is_none() {
        app.pq.dialog = Some(dlg);
    }
}

type Commit = Box<dyn FnOnce(&mut VizApp, &mut Dialog) -> bool>;

fn edit_dialog(app: &mut VizApp, ctx: &egui::Context, t: &Tokens, dlg: &mut Dialog) -> bool {
    let table = preview_table(app);
    let cols: Vec<String> = table.columns.clone();
    let queries: Vec<String> = app.project.queries.queries.iter().map(|q| q.name.clone()).collect();
    let err = dlg.error.clone();
    let mut keep = true;
    let mut action: Option<Commit> = None;
    let (title, size) = title_of(&dlg.kind);
    let mut open = true;
    fit(window(title), ctx, size).open(&mut open).show(ctx, |ui| {
        body(app, ui, t, &mut dlg.kind, &cols, &queries, &table);
        error_line(ui, t, &err);
        let ok_label = match &dlg.kind {
            Kind::Advanced { .. } => "Done",
            Kind::Reference { .. } | Kind::Shortcuts => "Close",
            _ => "OK",
        };
        let (ok, cancel) = ok_cancel(ui, t, ok_label);
        if cancel {
            keep = false;
        }
        if ok {
            action = Some(commit(&dlg.kind, &table));
        }
    });
    if !open {
        keep = false;
    }
    if let Some(a) = action
        && a(app, dlg)
    {
        keep = false;
    }
    keep
}

fn title_of(k: &Kind) -> (&'static str, egui::Vec2) {
    let s = vec2(480.0, 300.0);
    match k {
        Kind::Param { .. } => ("Enter a Value", s),
        Kind::Rows { kind, .. } => (
            match kind {
                KeepRemove::KeepTop => "Keep Top Rows",
                KeepRemove::KeepBottom => "Keep Bottom Rows",
                KeepRemove::KeepRange => "Keep Range of Rows",
                KeepRemove::RemoveTop => "Remove Top Rows",
                KeepRemove::RemoveBottom => "Remove Bottom Rows",
                KeepRemove::Alternate => "Remove Alternate Rows",
            },
            s,
        ),
        Kind::Split { .. } => ("Split Column", s),
        Kind::GroupBy { .. } => ("Group By", vec2(620.0, 360.0)),
        Kind::Replace { .. } => ("Replace Values", s),
        Kind::ReplaceErrors { .. } => ("Replace Errors", s),
        Kind::Merge { .. } => ("Merge", vec2(860.0, 620.0)),
        Kind::Append { .. } => ("Append", vec2(620.0, 380.0)),
        Kind::Parameters { .. } => ("Manage Parameters", vec2(680.0, 380.0)),
        Kind::Advanced { .. } => ("Advanced Editor", vec2(820.0, 560.0)),
        Kind::ChooseColumns { .. } => ("Choose Columns", vec2(380.0, 440.0)),
        Kind::RenameColumn { .. } => ("Rename Column", s),
        Kind::RenameQuery { .. } => ("Rename Query", s),
        Kind::RenameStep { .. } => ("Rename Step", s),
        Kind::Properties { .. } => ("Query Properties", s),
        Kind::Custom { .. } => ("Custom Column", vec2(700.0, 420.0)),
        Kind::Conditional { .. } => ("Add Conditional Column", vec2(820.0, 360.0)),
        Kind::Pivot { .. } => ("Pivot Column", s),
        Kind::Index { .. } => ("Add Index Column", s),
        Kind::MergeColumns { .. } => ("Merge Columns", s),
        Kind::Expand { aggregate, .. } => (if *aggregate { "Aggregate" } else { "Expand" }, vec2(420.0, 420.0)),
        Kind::FilterRows { .. } => ("Filter Rows", vec2(560.0, 300.0)),
        Kind::Python { .. } => ("Run Python script", vec2(640.0, 420.0)),
        Kind::Invoke { .. } => ("Invoke Custom Function", s),
        Kind::StepSettings { .. } => ("Step Settings", vec2(640.0, 340.0)),
        Kind::NewGroup { .. } => ("Group", s),
        Kind::Reference { .. } => ("M Function Reference", vec2(520.0, 520.0)),
        Kind::Shortcuts => ("Keyboard Shortcuts", vec2(460.0, 360.0)),
    }
}

fn labeled_row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(150.0, 20.0), egui::Label::new(label));
        add(ui);
    });
}

fn syntax_line(ui: &mut Ui, t: &Tokens, src: &str) {
    match vizcraft_query::m::parse(src) {
        Ok(_) => {
            ui.colored_label(t.good, "No syntax errors have been detected.");
        }
        Err(e) => {
            ui.colored_label(t.danger, e.message());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn body(app: &mut VizApp, ui: &mut Ui, t: &Tokens, k: &mut Kind, cols: &[String], queries: &[String], table: &Table) {
    match k {
        Kind::Param { op, value, .. } => {
            let prompt = ops::OPS.get(*op).and_then(|o| o.param).map(|p| p.0).unwrap_or("Value");
            ui.label(prompt);
            ui.text_edit_singleline(value).request_focus();
        }
        Kind::Rows { kind, a, b, c } => match kind {
            KeepRemove::KeepRange => {
                ui.label("Specify the range of rows to keep.");
                labeled_row(ui, "First row", |ui| {
                    ui.text_edit_singleline(a);
                });
                labeled_row(ui, "Number of rows", |ui| {
                    ui.text_edit_singleline(b);
                });
            }
            KeepRemove::Alternate => {
                ui.label("Specify the pattern of rows to remove and keep.");
                labeled_row(ui, "First row to remove", |ui| {
                    ui.text_edit_singleline(a);
                });
                labeled_row(ui, "Number of rows to remove", |ui| {
                    ui.text_edit_singleline(b);
                });
                labeled_row(ui, "Number of rows to keep", |ui| {
                    ui.text_edit_singleline(c);
                });
            }
            _ => {
                ui.label(match kind {
                    KeepRemove::KeepTop => "Specify how many rows to keep from the top.",
                    KeepRemove::KeepBottom => "Specify how many rows to keep from the bottom.",
                    KeepRemove::RemoveTop => "Specify how many rows to remove from the top.",
                    _ => "Specify how many rows to remove from the bottom.",
                });
                labeled_row(ui, "Number of rows", |ui| {
                    ui.text_edit_singleline(a).request_focus();
                });
            }
        },
        Kind::Split { col, mode, delim, custom, at, n, .. } => {
            ui.label(RichText::new(format!("Split column {col}")).strong());
            match mode {
                SplitMode::Delimiter => {
                    let opts: Vec<String> = ["Comma", "Colon", "Equals Sign", "Semicolon", "Space", "Tab", "Custom"].iter().map(|s| s.to_string()).collect();
                    labeled_row(ui, "Select or enter delimiter", |ui| {
                        combo(ui, "pq_split_delim", delim, &opts, 160.0);
                        if delim == "Custom" {
                            ui.text_edit_singleline(custom);
                        }
                    });
                    ui.label("Split at");
                    ui.radio_value(at, 0, "Left-most delimiter");
                    ui.radio_value(at, 1, "Right-most delimiter");
                    ui.radio_value(at, 2, "Each occurrence of the delimiter");
                }
                SplitMode::Chars => {
                    labeled_row(ui, "Number of characters", |ui| {
                        ui.text_edit_singleline(n);
                    });
                    ui.radio_value(at, 0, "Once, as far left as possible");
                    ui.radio_value(at, 2, "Repeatedly");
                }
                SplitMode::Positions => {
                    labeled_row(ui, "Positions", |ui| {
                        ui.text_edit_singleline(n);
                    });
                    ui.label(RichText::new("Zero-based character positions, e.g. 0, 3, 7").small().color(t.muted));
                }
                _ => {
                    ui.label("Splits where the kind of character changes.");
                }
            }
        }
        Kind::GroupBy { advanced, groups, aggs } => {
            ui.label("Specify the columns to group by and the outputs.");
            ui.horizontal(|ui| {
                ui.radio_value(advanced, false, "Basic");
                ui.radio_value(advanced, true, "Advanced");
            });
            ui.add_space(4.0);
            let mut remove_g = None;
            let shown = if *advanced { groups.len() } else { 1 };
            for (i, g) in groups.iter_mut().enumerate().take(shown) {
                ui.horizontal(|ui| {
                    combo(ui, ("pq_gb_g", i), g, cols, 240.0);
                    if i > 0 && super::forms::glyph_button(ui, t, 'x', t.muted, "Remove").clicked() {
                        remove_g = Some(i);
                    }
                });
            }
            if let Some(i) = remove_g {
                groups.remove(i);
            }
            if *advanced && ui.small_button("Add grouping").clicked() {
                groups.push(String::new());
            }
            ui.separator();
            egui::Grid::new("pq_gb_aggs").num_columns(4).spacing(vec2(8.0, 4.0)).show(ui, |ui| {
                ui.label(RichText::new("New column name").color(t.muted));
                ui.label(RichText::new("Operation").color(t.muted));
                ui.label(RichText::new("Column").color(t.muted));
                ui.label("");
                ui.end_row();
                let mut remove_a = None;
                let shown = if *advanced { aggs.len() } else { 1 };
                for (i, (name, op, col)) in aggs.iter_mut().enumerate().take(shown) {
                    ui.add_sized(vec2(150.0, 22.0), egui::TextEdit::singleline(name));
                    index_combo(ui, ("pq_gb_op", i), op, AGG_OPS, 150.0);
                    if (2..=6).contains(op) {
                        combo(ui, ("pq_gb_c", i), col, cols, 170.0);
                    } else {
                        ui.label("");
                    }
                    if i > 0 && super::forms::glyph_button(ui, t, 'x', t.muted, "Remove").clicked() {
                        remove_a = Some(i);
                    }
                    ui.end_row();
                }
                if let Some(i) = remove_a {
                    aggs.remove(i);
                }
            });
            if *advanced && ui.small_button("Add aggregation").clicked() {
                aggs.push((format!("Aggregation{}", aggs.len()), 2, String::new()));
            }
        }
        Kind::Replace { cols: c, find, replace, entire } => {
            ui.label(format!("Replace one value with another in {}.", c.join(", ")));
            labeled_row(ui, "Value To Find", |ui| {
                ui.text_edit_singleline(find).request_focus();
            });
            labeled_row(ui, "Replace With", |ui| {
                ui.text_edit_singleline(replace);
            });
            ui.checkbox(entire, "Match entire cell contents");
        }
        Kind::ReplaceErrors { value, .. } => {
            labeled_row(ui, "Value", |ui| {
                ui.text_edit_singleline(value);
            });
        }
        Kind::Merge { as_new, other, left, right, kind } => merge_body(app, ui, t, *as_new, other, left, right, kind, queries, table),
        Kind::Append { many, picked, second, avail_sel, .. } => {
            let cur = app.pq.query.clone();
            ui.label("Concatenate rows from two or more tables into one.");
            ui.horizontal(|ui| {
                ui.radio_value(many, false, "Two tables");
                ui.radio_value(many, true, "Three or more tables");
            });
            ui.add_space(4.0);
            if !*many {
                labeled_row(ui, "Primary table", |ui| {
                    ui.label(RichText::new(&cur).strong());
                });
                let others: Vec<String> = queries.iter().filter(|q| **q != cur).cloned().collect();
                labeled_row(ui, "Table to append", |ui| {
                    combo(ui, "pq_append_second", second, &others, 260.0);
                });
            } else {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(220.0);
                        ui.label(RichText::new("Available tables").color(t.muted));
                        egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).inner_margin(4).show(ui, |ui| {
                            egui::ScrollArea::vertical().id_salt("pq_app_avail").max_height(200.0).show(ui, |ui| {
                                ui.set_min_width(210.0);
                                for q in queries.iter() {
                                    let r = ui.selectable_label(avail_sel.as_ref() == Some(q), q).on_hover_text("Double-click to add");
                                    if r.clicked() {
                                        *avail_sel = Some(q.clone());
                                    }
                                    if r.double_clicked() {
                                        picked.push(q.clone());
                                    }
                                }
                            });
                        });
                    });
                    ui.vertical(|ui| {
                        ui.add_space(60.0);
                        if ui.add_enabled(avail_sel.is_some(), egui::Button::new("Add  ›")).clicked()
                            && let Some(s) = avail_sel.clone()
                        {
                            picked.push(s);
                        }
                    });
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Tables to append (after the current query)").color(t.muted));
                        let mut up = None;
                        let mut remove = None;
                        for (i, q) in picked.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(q);
                                if i > 0 && super::forms::glyph_button(ui, t, 'u', t.muted, "Move up").clicked() {
                                    up = Some(i);
                                }
                                if super::forms::glyph_button(ui, t, 'x', t.muted, "Remove").clicked() {
                                    remove = Some(i);
                                }
                            });
                        }
                        if let Some(i) = up {
                            picked.swap(i, i - 1);
                        }
                        if let Some(i) = remove {
                            picked.remove(i);
                        }
                    });
                });
            }
        }
        Kind::Parameters { selected, name, kind, value, description, suggestions } => {
            let params = app.project.queries.parameters.clone();
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(170.0);
                    for p in &params {
                        if ui.selectable_label(selected.as_ref() == Some(&p.name), &p.name).clicked() {
                            *selected = Some(p.name.clone());
                            *name = p.name.clone();
                            *kind = p.kind.clone();
                            *value = p.value.clone();
                            *description = p.description.clone();
                            *suggestions = p.suggestions.join(", ");
                        }
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("New").clicked() {
                            *selected = None;
                            *name = format!("Parameter{}", params.len() + 1);
                            *value = "\"\"".into();
                            description.clear();
                            suggestions.clear();
                        }
                        if let Some(s) = selected.clone()
                            && ui.button("Delete").clicked()
                        {
                            app.checkpoint("Delete parameter");
                            app.project.queries.parameters.retain(|p| p.name != s);
                            app.pq.pending = true;
                            *selected = None;
                        }
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    egui::Grid::new("pq_params").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(name);
                        ui.end_row();
                        ui.label("Description");
                        ui.text_edit_singleline(description);
                        ui.end_row();
                        ui.label("Type");
                        let opts: Vec<String> = ["Text", "Decimal Number", "Whole Number", "Date", "Date/Time", "True/False", "Any"].iter().map(|s| s.to_string()).collect();
                        combo(ui, "pq_param_type", kind, &opts, 160.0);
                        ui.end_row();
                        ui.label("Suggested values");
                        ui.add(egui::TextEdit::singleline(suggestions).hint_text("Any value, or a list: \"East\", \"West\""));
                        ui.end_row();
                        ui.label("Current value (M)");
                        ui.text_edit_singleline(value);
                        ui.end_row();
                    });
                    ui.label(RichText::new("Text in quotes (\"East\"), numbers as is (100), dates as #date(2024, 1, 31). Queries use a parameter by its name.").small().color(t.muted));
                    syntax_line(ui, t, value);
                });
            });
        }
        Kind::Advanced { m } => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&app.pq.query).strong().size(14.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("Ctrl+Z undoes the change after Done").small().color(t.muted));
                });
            });
            ui.add_space(4.0);
            let h = (ui.ctx().content_rect().height() - 260.0).clamp(200.0, 560.0);
            egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(h).show(ui, |ui| {
                    let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                        let mut job = highlight(text.as_str(), t);
                        job.wrap.max_width = wrap;
                        ui.ctx().fonts_mut(|f| f.layout_job(job))
                    };
                    ui.add(egui::TextEdit::multiline(m).code_editor().desired_rows(22).desired_width(f32::INFINITY).layouter(&mut layouter));
                });
            });
            syntax_line(ui, t, m);
        }
        Kind::ChooseColumns { keep, search } => {
            ui.add(egui::TextEdit::singleline(search).hint_text("Search Columns").desired_width(f32::INFINITY));
            let mut a = keep.iter().all(|(_, k)| *k);
            if ui.checkbox(&mut a, "(Select All Columns)").changed() {
                for (_, k) in keep.iter_mut() {
                    *k = a;
                }
            }
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                let q = search.to_lowercase();
                for (c, k) in keep.iter_mut().filter(|(c, _)| q.is_empty() || c.to_lowercase().contains(&q)) {
                    ui.checkbox(k, c.as_str());
                }
            });
        }
        Kind::RenameColumn { new, .. } | Kind::RenameQuery { new, .. } | Kind::RenameStep { new, .. } => {
            ui.label("New name");
            ui.add(egui::TextEdit::singleline(new).desired_width(f32::INFINITY)).request_focus();
        }
        Kind::Properties { name, description, group, load } => {
            egui::Grid::new("pq_props").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(name).desired_width(300.0));
                ui.end_row();
                ui.label("Description");
                ui.add(egui::TextEdit::multiline(description).desired_rows(3).desired_width(300.0));
                ui.end_row();
                ui.label("Group");
                ui.add(egui::TextEdit::singleline(group).desired_width(300.0).hint_text("(none)"));
                ui.end_row();
                ui.label("");
                ui.checkbox(load, "Enable load to the model");
                ui.end_row();
            });
        }
        Kind::Custom { name, formula, ty } => {
            ui.label("Add a column that is computed from the other columns.");
            ui.horizontal(|ui| {
                ui.label("New column name");
                ui.text_edit_singleline(name);
                ui.label("Data type");
                let labels: Vec<&str> = CUSTOM_TYPES.iter().map(|x| x.0).collect();
                index_combo(ui, "pq_custom_ty", ty, &labels, 140.0);
            });
            ui.add_space(4.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.label("Custom column formula");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("=").monospace().strong());
                        ui.add(egui::TextEdit::multiline(formula).code_editor().desired_rows(10).desired_width(420.0).hint_text("[Units] * [Price]"));
                    });
                });
                ui.vertical(|ui| {
                    ui.label("Available columns");
                    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).inner_margin(4).show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                            ui.set_min_width(170.0);
                            for c in cols {
                                if ui.selectable_label(false, c).on_hover_text("Insert (double-click)").double_clicked() {
                                    formula.push_str(&field(c));
                                }
                            }
                        });
                    });
                    ui.label(RichText::new("Double-click to insert").small().color(t.muted));
                });
            });
            if !formula.trim().is_empty() {
                syntax_line(ui, t, &format!("each {formula}"));
            }
            ui.hyperlink_to(RichText::new("Learn about M formulas (Help › M Reference)").small(), "https://learn.microsoft.com/powerquery-m/");
        }
        Kind::Conditional { name, rules, otherwise } => {
            ui.label("Add a conditional column that is computed from the other columns or values.");
            ui.horizontal(|ui| {
                ui.label("New column name");
                ui.text_edit_singleline(name);
            });
            ui.add_space(4.0);
            let mut remove = None;
            let mut up = None;
            egui::Grid::new("pq_cond").num_columns(8).spacing(vec2(6.0, 4.0)).show(ui, |ui| {
                ui.label("");
                ui.label(RichText::new("Column Name").color(t.muted));
                ui.label(RichText::new("Operator").color(t.muted));
                ui.label(RichText::new("Value").color(t.muted));
                ui.label("");
                ui.label(RichText::new("Output").color(t.muted));
                ui.label("");
                ui.label("");
                ui.end_row();
                for (i, (col, op, value, out)) in rules.iter_mut().enumerate() {
                    ui.label(if i == 0 { "If" } else { "Else If" });
                    combo(ui, ("pq_cond_c", i), col, cols, 140.0);
                    index_combo(ui, ("pq_cond_o", i), op, COND_OPS, 160.0);
                    ui.add_sized(vec2(110.0, 22.0), egui::TextEdit::singleline(value).hint_text("Value"));
                    ui.label("Then");
                    ui.add_sized(vec2(110.0, 22.0), egui::TextEdit::singleline(out).hint_text("Output"));
                    if i > 0 && super::forms::glyph_button(ui, t, 'u', t.muted, "Move up").clicked() {
                        up = Some(i);
                    } else if i == 0 {
                        ui.label("");
                    }
                    if rules_len_gt1(i) && super::forms::glyph_button(ui, t, 'x', t.muted, "Remove this clause").clicked() {
                        remove = Some(i);
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                }
            });
            if let Some(i) = up {
                rules.swap(i, i - 1);
            }
            if let Some(i) = remove
                && rules.len() > 1
            {
                rules.remove(i);
            }
            if ui.small_button("Add Clause").clicked() {
                let c = rules.last().map(|r| r.0.clone()).unwrap_or_default();
                rules.push((c, 0, String::new(), String::new()));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Else");
                ui.add(egui::TextEdit::singleline(otherwise).desired_width(140.0).hint_text("null"));
            });
            ui.label(RichText::new("Values: numbers stay numbers, null is null, everything else is text.").small().color(t.muted));
        }
        Kind::Pivot { col, values, agg } => {
            ui.label(format!("Use the names in column \"{col}\" to create new columns."));
            let others: Vec<String> = cols.iter().filter(|c| *c != col).cloned().collect();
            labeled_row(ui, "Values Column", |ui| {
                combo(ui, "pq_pivot_v", values, &others, 200.0);
            });
            labeled_row(ui, "Aggregate Value Function", |ui| {
                let labels: Vec<&str> = PIVOT_AGGS.iter().map(|x| x.0).collect();
                index_combo(ui, "pq_pivot_a", agg, &labels, 180.0);
            });
        }
        Kind::Index { start, step } => {
            labeled_row(ui, "Starting Index", |ui| {
                ui.text_edit_singleline(start);
            });
            labeled_row(ui, "Increment", |ui| {
                ui.text_edit_singleline(step);
            });
        }
        Kind::MergeColumns { cols: c, sep, custom, name, .. } => {
            ui.label(format!("Merge {} in the selected order.", c.join(", ")));
            labeled_row(ui, "Separator", |ui| {
                let labels: Vec<&str> = SEPARATORS.iter().map(|x| x.0).collect();
                index_combo(ui, "pq_mc_sep", sep, &labels, 140.0);
                if *sep == SEPARATORS.len() - 1 {
                    ui.text_edit_singleline(custom);
                }
            });
            labeled_row(ui, "New column name", |ui| {
                ui.text_edit_singleline(name);
            });
        }
        Kind::Expand { names, prefix, aggregate, aggs, list, .. } => {
            if *list && names.is_empty() {
                ui.label("This column holds lists: expanding makes one row per item.");
            } else if names.is_empty() {
                ui.label("This column holds no tables or records to expand.");
            }
            if !names.is_empty() {
                ui.horizontal(|ui| {
                    ui.radio_value(aggregate, false, "Expand");
                    ui.radio_value(aggregate, true, "Aggregate");
                });
            }
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                if *aggregate {
                    let labels: Vec<&str> = EXPAND_AGGS.iter().map(|x| x.0).collect();
                    for (i, (n, on)) in names.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.checkbox(on, n.as_str());
                            if let Some((_, a)) = aggs.get_mut(i) {
                                index_combo(ui, ("pq_exp_agg", i), a, &labels, 140.0);
                            }
                        });
                    }
                } else {
                    let mut a = names.iter().all(|(_, k)| *k);
                    if !names.is_empty() && ui.checkbox(&mut a, "(Select All Columns)").changed() {
                        for (_, k) in names.iter_mut() {
                            *k = a;
                        }
                    }
                    for (n, on) in names.iter_mut() {
                        ui.checkbox(on, n.as_str());
                    }
                }
            });
            if !*aggregate && !names.is_empty() {
                ui.checkbox(prefix, "Use original column name as prefix");
            }
        }
        Kind::FilterRows { col, rules, and } => {
            ui.label(format!("Keep rows where \"{col}\""));
            ui.add_space(4.0);
            for (i, (op, value)) in rules.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    index_combo(ui, ("pq_fr_op", i), op, COND_OPS, 200.0);
                    ui.add(egui::TextEdit::singleline(value).desired_width(220.0).hint_text("Enter or select a value"));
                });
                if i == 0 {
                    ui.horizontal(|ui| {
                        ui.radio_value(and, true, "And");
                        ui.radio_value(and, false, "Or");
                    });
                }
            }
            ui.label(RichText::new("Leave the second value empty to use only the first condition.").small().color(t.muted));
        }
        Kind::Python { script, output } => {
            ui.label("Script");
            ui.label(RichText::new("Enter Python scripts into the editor to transform and shape your data. The table is in the variable 'dataset' (a pandas DataFrame when pandas is installed).").small().color(t.muted));
            ui.add(egui::TextEdit::multiline(script).code_editor().desired_rows(14).desired_width(f32::INFINITY));
            labeled_row(ui, "Result table (variable)", |ui| {
                ui.text_edit_singleline(output);
            });
        }
        Kind::Invoke { name, function, args } => {
            let fns: Vec<String> = app.project.queries.queries.iter().filter(|q| is_function(&q.m)).map(|q| q.name.clone()).collect();
            if fns.is_empty() {
                ui.label("There are no function queries yet. Create one with a blank query: (x) => x * 2");
            }
            labeled_row(ui, "New column name", |ui| {
                ui.text_edit_singleline(name);
            });
            labeled_row(ui, "Function query", |ui| {
                combo(ui, "pq_inv_fn", function, &fns, 220.0);
            });
            let nparams = app.project.queries.query(function).map(|q| q.m.split("=>").next().unwrap_or_default().split(',').filter(|s| !s.trim().trim_matches(['(', ')']).is_empty()).count()).unwrap_or(1).max(1);
            args.resize(nparams, String::new());
            for (i, a) in args.iter_mut().enumerate() {
                labeled_row(ui, &format!("Argument {} (column)", i + 1), |ui| {
                    combo(ui, ("pq_inv_arg", i), a, cols, 220.0);
                });
            }
        }
        Kind::StepSettings { step, expr, path } => {
            ui.label(RichText::new(step.as_str()).strong());
            if let Some((func, p)) = path {
                ui.label(format!("{func} source"));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(p).desired_width(420.0));
                    if func != "Web.Contents" && func != "OData.Feed" && ui.button("Browse…").clicked() {
                        let picked = if func == "Folder.Files" { rfd::FileDialog::new().pick_folder() } else { rfd::FileDialog::new().pick_file() };
                        if let Some(x) = picked {
                            *p = x.to_string_lossy().to_string();
                        }
                    }
                });
                ui.add_space(4.0);
            }
            ui.label("Step formula (M)");
            ui.add(egui::TextEdit::multiline(expr).code_editor().desired_rows(8).desired_width(f32::INFINITY));
            syntax_line(ui, t, expr);
        }
        Kind::NewGroup { name, .. } => {
            labeled_row(ui, "Group name", |ui| {
                ui.text_edit_singleline(name).request_focus();
            });
        }
        Kind::Reference { search } => {
            ui.add(egui::TextEdit::singleline(search).hint_text("Search functions").desired_width(f32::INFINITY));
            let q = search.to_lowercase();
            let list: Vec<&str> = vizcraft_query::funcs::NAMES.iter().copied().filter(|n| q.is_empty() || n.to_lowercase().contains(&q)).collect();
            ui.label(RichText::new(format!("{} functions", list.len())).small().color(t.muted));
            egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                for n in list {
                    if ui.selectable_label(false, RichText::new(n).monospace()).on_hover_text("Copy the name").clicked() {
                        ui.ctx().copy_text(n.to_string());
                    }
                }
            });
        }
        Kind::Shortcuts => {
            egui::Grid::new("pq_keys").num_columns(2).spacing(vec2(24.0, 6.0)).show(ui, |ui| {
                for (k, v) in [
                    ("Ctrl+Z / Ctrl+Y", "Undo / redo the last query edit"),
                    ("F5", "Refresh"),
                    ("Esc", "Close the editor (or the open menu)"),
                    ("Click a header", "Select a column"),
                    ("Ctrl/Shift+Click", "Select more columns"),
                    ("Double-click a header", "Rename the column"),
                    ("Right-click a header", "Column menu"),
                    ("Double-click a step", "Rename the step"),
                    ("Enter (formula bar)", "Apply the step formula"),
                ] {
                    ui.label(RichText::new(k).strong());
                    ui.label(v);
                    ui.end_row();
                }
            });
        }
    }
}

fn rules_len_gt1(_i: usize) -> bool {
    true
}

/// Syntax colouring of M for the Advanced Editor.
fn highlight(src: &str, t: &Tokens) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob::default();
    let font = egui::FontId::monospace(12.5);
    let kw = egui::Color32::from_rgb(0x2B, 0x6C, 0xE0);
    let kw = if t.dark { egui::Color32::from_rgb(0x7C, 0xB3, 0xFF) } else { kw };
    let string = if t.dark { egui::Color32::from_rgb(0xF0, 0x9A, 0x6B) } else { egui::Color32::from_rgb(0xA3, 0x15, 0x15) };
    let num = if t.dark { egui::Color32::from_rgb(0x9C, 0xDC, 0xA5) } else { egui::Color32::from_rgb(0x09, 0x86, 0x58) };
    let comment = t.muted;
    let func = if t.dark { egui::Color32::from_rgb(0xDC, 0xDC, 0xAA) } else { egui::Color32::from_rgb(0x79, 0x5E, 0x26) };
    let push = |job: &mut LayoutJob, s: &str, c: egui::Color32| job.append(s, 0.0, TextFormat::simple(font.clone(), c));
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let mut i = 0;
    let slice = |a: usize, b: usize| src.get(chars.get(a).map(|x| x.0).unwrap_or(src.len())..chars.get(b).map(|x| x.0).unwrap_or(src.len())).unwrap_or_default();
    while i < chars.len() {
        let c = chars.get(i).map(|x| x.1).unwrap_or(' ');
        let start = i;
        if c == '/' && chars.get(i + 1).map(|x| x.1) == Some('/') {
            while i < chars.len() && chars.get(i).map(|x| x.1) != Some('\n') {
                i += 1;
            }
            push(&mut job, slice(start, i), comment);
        } else if c == '"' || (c == '#' && chars.get(i + 1).map(|x| x.1) == Some('"')) {
            i += if c == '#' { 2 } else { 1 };
            while i < chars.len() {
                if chars.get(i).map(|x| x.1) == Some('"') {
                    if chars.get(i + 1).map(|x| x.1) == Some('"') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            push(&mut job, slice(start, i), if c == '#' { t.text } else { string });
        } else if c.is_ascii_digit() {
            while i < chars.len() && chars.get(i).is_some_and(|x| x.1.is_ascii_digit() || x.1 == '.') {
                i += 1;
            }
            push(&mut job, slice(start, i), num);
        } else if c.is_alphabetic() || c == '_' || c == '#' {
            i += 1;
            while i < chars.len() && chars.get(i).is_some_and(|x| x.1.is_alphanumeric() || x.1 == '_' || x.1 == '.') {
                i += 1;
            }
            let w = slice(start, i);
            let color = match w {
                "let" | "in" | "each" | "if" | "then" | "else" | "and" | "or" | "not" | "try" | "otherwise" | "error" | "type" | "as" | "is" | "meta" | "true" | "false" | "null" => kw,
                _ if w.starts_with('#') => kw,
                _ if w.contains('.') => func,
                _ => t.text,
            };
            push(&mut job, w, color);
        } else {
            i += 1;
            push(&mut job, slice(start, i), t.text);
        }
    }
    job
}

#[allow(clippy::too_many_arguments)]
fn merge_body(app: &mut VizApp, ui: &mut Ui, t: &Tokens, as_new: bool, other: &mut String, left: &mut Vec<String>, right: &mut Vec<String>, kind: &mut usize, queries: &[String], table: &Table) {
    ui.label(format!("Select a table and matching columns to create a merged {}.", if as_new { "query" } else { "table" }));
    let pick = |ui: &mut Ui, id: &str, tb: &Table, keys: &mut Vec<String>| {
        egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).inner_margin(2).show(ui, |ui| {
            egui::ScrollArea::horizontal().id_salt(id).show(ui, |ui| {
                egui::Grid::new(id).striped(true).min_col_width(70.0).spacing(vec2(10.0, 3.0)).show(ui, |ui| {
                    for c in &tb.columns {
                        let i = keys.iter().position(|k| k == c);
                        let label = match i {
                            Some(i) => RichText::new(format!("{c}  [{}]", i + 1)).strong().color(t.accent),
                            None => RichText::new(c).strong(),
                        };
                        if ui.selectable_label(i.is_some(), label).on_hover_text("Click to use as a key (Ctrl-click for more, in order)").clicked() {
                            let multi = ui.input(|i| i.modifiers.command || i.modifiers.shift);
                            match i {
                                Some(i) => {
                                    keys.remove(i);
                                }
                                None if multi => keys.push(c.clone()),
                                None => *keys = vec![c.clone()],
                            }
                        }
                    }
                    ui.end_row();
                    for r in tb.rows.iter().take(5) {
                        for v in r {
                            ui.label(RichText::new(v.display()).size(11.0));
                        }
                        ui.end_row();
                    }
                });
            });
        });
    };
    ui.add_space(4.0);
    ui.label(RichText::new(&app.pq.query).strong());
    pick(ui, "pq_merge_l", table, left);
    ui.add_space(8.0);
    let others: Vec<String> = queries.iter().filter(|q| **q != app.pq.query).cloned().chain(app.project.sheets.iter().map(|s| s.name.clone())).collect();
    combo(ui, "pq_merge_other", other, &others, 300.0);
    let other_t = if other.is_empty() { None } else { super::table_of(app, other).ok() };
    if let Some(ot) = &other_t {
        if right.is_empty()
            && let Some(l) = left.first()
            && ot.column_index(l).is_some()
        {
            right.push(l.clone());
        }
        pick(ui, "pq_merge_r", ot, right);
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Join Kind");
        let labels: Vec<&str> = JOIN_KINDS.iter().map(|x| x.0).collect();
        index_combo(ui, "pq_merge_kind", kind, &labels, 380.0);
    });
    if let Some(ot) = &other_t {
        let lk: Vec<usize> = left.iter().filter_map(|c| table.column_index(c)).collect();
        let rk: Vec<usize> = right.iter().filter_map(|c| ot.column_index(c)).collect();
        if !lk.is_empty() && lk.len() == rk.len() {
            let key = |t: &Table, r: usize, ks: &[usize]| vizcraft_query::funcs::table::key_of(&ks.iter().map(|c| t.cell(r, *c)).collect::<Vec<_>>());
            let rkeys: std::collections::HashSet<String> = (0..ot.rows.len()).map(|r| key(ot, r, &rk)).collect();
            let lkeys: std::collections::HashSet<String> = (0..table.rows.len()).map(|r| key(table, r, &lk)).collect();
            let lhits = (0..table.rows.len()).filter(|r| rkeys.contains(&key(table, *r, &lk))).count();
            let rhits = (0..ot.rows.len()).filter(|r| lkeys.contains(&key(ot, *r, &rk))).count();
            let msg = match *kind {
                1 => format!("The selection matches {rhits} of {} rows from the second table.", ot.rows.len()),
                3 => format!("The selection matches {lhits} of {} rows from the first table, and {rhits} of {} from the second.", table.rows.len(), ot.rows.len()),
                4 => format!("{} of {} rows from the first table have no match.", table.rows.len() - lhits, table.rows.len()),
                5 => format!("{} of {} rows from the second table have no match.", ot.rows.len() - rhits, ot.rows.len()),
                _ => format!("The selection matches {lhits} of {} rows from the first table.", table.rows.len()),
            };
            let good = lhits > 0 || rhits > 0;
            ui.add_space(4.0);
            ui.colored_label(if good { t.good } else { egui::Color32::from_rgb(0xB4, 0x53, 0x09) }, msg);
        } else if !lk.is_empty() {
            ui.colored_label(t.muted, "Select the same number of columns in both tables.");
        }
    }
}

/// What OK does: returns true when the dialog can close.
fn commit(k: &Kind, table: &Table) -> Commit {
    match k {
        Kind::Param { op, add, value } => {
            let (op, add, value) = (*op, *add, value.clone());
            Box::new(move |app, _| {
                ops::apply_op(app, op, add, Some(value));
                true
            })
        }
        Kind::Rows { kind, a, b, c } => {
            let n = |s: &str| s.trim().parse::<i64>().unwrap_or(1).max(0);
            let (a, b, c) = (n(a), n(b), n(c));
            let (name, e) = match kind {
                KeepRemove::KeepTop => ("Kept First Rows", format!("Table.FirstN({{prev}}, {a})")),
                KeepRemove::KeepBottom => ("Kept Last Rows", format!("Table.LastN({{prev}}, {a})")),
                KeepRemove::KeepRange => ("Kept Range of Rows", format!("Table.Range({{prev}}, {}, {b})", (a - 1).max(0))),
                KeepRemove::RemoveTop => ("Removed Top Rows", format!("Table.Skip({{prev}}, {a})")),
                KeepRemove::RemoveBottom => ("Removed Bottom Rows", format!("Table.RemoveLastN({{prev}}, {a})")),
                KeepRemove::Alternate => ("Removed Alternate Rows", format!("Table.AlternateRows({{prev}}, {}, {b}, {c})", (a - 1).max(0))),
            };
            Box::new(move |app, _| {
                add_step(app, name, &e);
                true
            })
        }
        Kind::Split { col, mode, delim, custom, at, n, .. } => {
            let dl = match delim.as_str() {
                "Comma" => ",",
                "Colon" => ":",
                "Equals Sign" => "=",
                "Semicolon" => ";",
                "Space" => " ",
                "Tab" => "\t",
                _ => custom.as_str(),
            };
            let splitter = match mode {
                SplitMode::Delimiter => match at {
                    0 => format!("Splitter.SplitTextByEachDelimiter({{{}}}, QuoteStyle.Csv, false)", quote_text(dl)),
                    1 => format!("Splitter.SplitTextByEachDelimiter({{{}}}, QuoteStyle.Csv, true)", quote_text(dl)),
                    _ => format!("Splitter.SplitTextByDelimiter({}, QuoteStyle.Csv)", quote_text(dl)),
                },
                SplitMode::Chars => {
                    let k = n.trim().parse::<usize>().unwrap_or(1).max(1);
                    if *at == 0 {
                        format!("Splitter.SplitTextByPositions({{0, {k}}})")
                    } else {
                        let longest = table.column_index(col).map(|c| table.rows.iter().take(1000).map(|r| r.get(c).map(|v| v.display().chars().count()).unwrap_or(0)).max().unwrap_or(0)).unwrap_or(0);
                        let parts = longest.div_ceil(k).clamp(1, 200);
                        format!("Splitter.SplitTextByLengths({{{}}})", vec![k.to_string(); parts].join(", "))
                    }
                }
                SplitMode::Positions => format!("Splitter.SplitTextByPositions({{{n}}})"),
                SplitMode::LowerUpper => "Splitter.SplitTextByCharacterTransition({\"a\"..\"z\"}, {\"A\"..\"Z\"})".into(),
                SplitMode::UpperLower => "Splitter.SplitTextByCharacterTransition({\"A\"..\"Z\"}, {\"a\"..\"z\"})".into(),
                SplitMode::DigitNon => "Splitter.SplitTextByCharacterTransition({\"0\"..\"9\"}, (c) => not List.Contains({\"0\"..\"9\"}, c))".into(),
                SplitMode::NonDigit => "Splitter.SplitTextByCharacterTransition((c) => not List.Contains({\"0\"..\"9\"}, c), {\"0\"..\"9\"})".into(),
            };
            let parts = split_count(table, col, &splitter);
            let names: Vec<String> = (1..=parts).map(|i| format!("{col}.{i}")).collect();
            let e = format!("Table.SplitColumn({{prev}}, {}, {splitter}, {})", quote_text(col), text_list(&names));
            let step = match mode {
                SplitMode::Delimiter => "Split Column by Delimiter",
                SplitMode::Chars => "Split Column by Character Count",
                SplitMode::Positions => "Split Column by Positions",
                _ => "Split Column by Character Transition",
            };
            Box::new(move |app, _| {
                add_step(app, step, &e);
                true
            })
        }
        Kind::GroupBy { advanced, groups, aggs } => {
            let g: Vec<String> = groups.iter().take(if *advanced { usize::MAX } else { 1 }).filter(|g| !g.is_empty()).cloned().collect();
            let specs: Vec<String> = aggs
                .iter()
                .take(if *advanced { usize::MAX } else { 1 })
                .map(|(name, op, col)| {
                    let f = field(col);
                    let (e, ty) = match op {
                        0 => ("Table.RowCount(_)".to_string(), "Int64.Type"),
                        1 => ("Table.RowCount(Table.Distinct(_))".to_string(), "Int64.Type"),
                        2 => (format!("List.Sum({f})"), "type number"),
                        3 => (format!("List.Average({f})"), "type number"),
                        4 => (format!("List.Median({f})"), "type number"),
                        5 => (format!("List.Min({f})"), "type any"),
                        6 => (format!("List.Max({f})"), "type any"),
                        _ => ("_".to_string(), "type table"),
                    };
                    format!("{{{}, each {e}, {ty}}}", quote_text(name))
                })
                .collect();
            let bad = aggs.iter().any(|(_, op, col)| (2..=6).contains(op) && col.is_empty());
            let e = format!("Table.Group({{prev}}, {}, {{{}}})", text_list(&g), specs.join(", "));
            Box::new(move |app, dlg| {
                if g.is_empty() {
                    dlg.error = Some("Choose a column to group by.".into());
                    return false;
                }
                if bad {
                    dlg.error = Some("Choose the column each aggregation uses.".into());
                    return false;
                }
                add_step(app, "Grouped Rows", &e);
                true
            })
        }
        Kind::Replace { cols, find, replace, entire } => {
            let e = if *entire {
                format!("Table.ReplaceValue({{prev}}, {}, {}, Replacer.ReplaceValue, {})", literal(find), literal(replace), text_list(cols))
            } else {
                format!("Table.ReplaceValue({{prev}}, {}, {}, Replacer.ReplaceText, {})", quote_text(find), quote_text(replace), text_list(cols))
            };
            Box::new(move |app, _| {
                add_step(app, "Replaced Value", &e);
                true
            })
        }
        Kind::ReplaceErrors { cols, value } => {
            let pairs: Vec<String> = cols.iter().map(|c| format!("{{{}, {}}}", quote_text(c), literal(value))).collect();
            let e = format!("Table.ReplaceErrorValues({{prev}}, {{{}}})", pairs.join(", "));
            Box::new(move |app, _| {
                add_step(app, "Replaced Errors", &e);
                true
            })
        }
        Kind::Merge { as_new, other, left, right, kind } => {
            let (as_new, other, left, right) = (*as_new, other.clone(), left.clone(), right.clone());
            let jk = JOIN_KINDS.get(*kind).map(|x| x.1).unwrap_or("LeftOuter");
            Box::new(move |app, dlg| {
                if other.is_empty() || left.is_empty() || left.len() != right.len() {
                    dlg.error = Some("Pick the second table and the same number of key columns on both sides.".into());
                    return false;
                }
                let other_id = steps::ident(&other);
                let col = other.clone();
                if as_new {
                    let cur = steps::ident(&app.pq.query);
                    let m = format!("let\n    Source = Table.NestedJoin({cur}, {}, {other_id}, {}, {}, JoinKind.{jk})\nin\n    Source", text_list(&left), text_list(&right), quote_text(&col));
                    let n = super::new_query(app, "Merge1", m, true, None);
                    open_editor(app, Some(&n));
                } else {
                    add_step(app, "Merged Queries", &format!("Table.NestedJoin({{prev}}, {}, {other_id}, {}, {}, JoinKind.{jk})", text_list(&left), text_list(&right), quote_text(&col)));
                }
                if !matches!(jk, "LeftAnti" | "RightAnti") {
                    app.pq.dialog = Some(Dialog::expand(app, &col, false));
                }
                true
            })
        }
        Kind::Append { as_new, many, picked, second, .. } => {
            let list: Vec<String> = if *many { picked.clone() } else if second.is_empty() { vec![] } else { vec![second.clone()] };
            let as_new = *as_new;
            Box::new(move |app, dlg| {
                if list.is_empty() {
                    dlg.error = Some("Choose at least one table to append.".into());
                    return false;
                }
                let ids: Vec<String> = list.iter().map(|q| steps::ident(q)).collect();
                if as_new {
                    let cur = steps::ident(&app.pq.query);
                    let m = format!("let\n    Source = Table.Combine({{{cur}, {}}})\nin\n    Source", ids.join(", "));
                    let n = super::new_query(app, "Append1", m, true, None);
                    open_editor(app, Some(&n));
                } else {
                    add_step(app, "Appended Query", &format!("Table.Combine({{{{prev}}, {}}})", ids.join(", ")));
                }
                true
            })
        }
        Kind::Parameters { selected, name, kind, value, description, suggestions } => {
            let (sel, name, kind, value, description) = (selected.clone(), name.trim().to_string(), kind.clone(), value.clone(), description.clone());
            let sugg: Vec<String> = suggestions.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            Box::new(move |app, dlg| {
                if name.is_empty() {
                    dlg.error = Some("A parameter needs a name.".into());
                    return false;
                }
                if let Err(e) = vizcraft_query::m::parse(&value) {
                    dlg.error = Some(e.message());
                    return false;
                }
                let clash = sel.as_deref() != Some(name.as_str()) && (app.project.queries.parameters.iter().any(|p| p.name == name) || app.project.queries.query(&name).is_some());
                if clash {
                    dlg.error = Some(format!("{name} is already used."));
                    return false;
                }
                app.checkpoint("Parameter");
                let p = vizcraft_query::steps::Parameter { name: name.clone(), kind, value, description, suggestions: sugg };
                match sel.and_then(|s| app.project.queries.parameters.iter().position(|x| x.name == s).map(|i| (s, i))) {
                    Some((old, i)) => {
                        if let Some(slot) = app.project.queries.parameters.get_mut(i) {
                            *slot = p;
                        }
                        if old != name {
                            for q in app.project.queries.queries.iter_mut() {
                                q.m = steps::rename_refs(&q.m, &old, &name);
                            }
                        }
                    }
                    None => app.project.queries.parameters.push(p),
                }
                app.pq.pending = true;
                true
            })
        }
        Kind::Advanced { m } => {
            let m = m.clone();
            Box::new(move |app, dlg| {
                if let Err(e) = vizcraft_query::m::parse(&m) {
                    dlg.error = Some(e.message());
                    return false;
                }
                super::set_m(app, "Advanced Editor", m);
                app.pq.step = None;
                true
            })
        }
        Kind::ChooseColumns { keep, .. } => {
            let cols: Vec<String> = keep.iter().filter(|(_, k)| *k).map(|(c, _)| c.clone()).collect();
            let all = keep.iter().all(|(_, k)| *k);
            Box::new(move |app, dlg| {
                if cols.is_empty() {
                    dlg.error = Some("Keep at least one column.".into());
                    return false;
                }
                if !all {
                    add_step(app, "Removed Other Columns", &format!("Table.SelectColumns({{prev}}, {})", text_list(&cols)));
                }
                true
            })
        }
        Kind::RenameColumn { old, new } => {
            let e = format!("Table.RenameColumns({{prev}}, {{{{{}, {}}}}})", quote_text(old), quote_text(new.trim()));
            let (old, new) = (old.clone(), new.trim().to_string());
            Box::new(move |app, _| {
                if old != new && !new.is_empty() {
                    add_step(app, "Renamed Columns", &e);
                    app.pq.selected = vec![new];
                }
                true
            })
        }
        Kind::RenameQuery { old, new } => {
            let (old, new) = (old.clone(), new.clone());
            Box::new(move |app, dlg| match super::rename_query(app, &old, &new) {
                Ok(()) => true,
                Err(e) => {
                    dlg.error = Some(e);
                    false
                }
            })
        }
        Kind::RenameStep { old, new } => {
            let (old, new) = (old.clone(), new.trim().to_string());
            Box::new(move |app, _| {
                let ok = super::edit_steps(app, "Rename step", |m| steps::rename_step(m, &old, &new));
                if ok && app.pq.step.as_deref() == Some(old.as_str()) {
                    app.pq.step = Some(new);
                }
                ok
            })
        }
        Kind::Properties { name, description, group, load } => {
            let (name, description, group, load) = (name.clone(), description.clone(), group.clone(), *load);
            Box::new(move |app, dlg| {
                let q = app.pq.query.clone();
                if let Err(e) = super::rename_query(app, &q, &name) {
                    dlg.error = Some(e);
                    return false;
                }
                let n = app.pq.query.clone();
                app.checkpoint("Query properties");
                if let Some(qd) = app.project.queries.query_mut(&n) {
                    qd.description = description;
                    qd.group = Some(group.trim().to_string()).filter(|g| !g.is_empty());
                    qd.load = if load { steps::LoadTo::Model } else { steps::LoadTo::None };
                }
                app.pq.pending = true;
                true
            })
        }
        Kind::Custom { name, formula, ty } => {
            let ty = CUSTOM_TYPES.get(*ty).map(|x| x.1).unwrap_or("");
            let e = format!("Table.AddColumn({{prev}}, {}, each {}{})", quote_text(name), formula.trim(), if ty.is_empty() { String::new() } else { format!(", {ty}") });
            let f = formula.trim().to_string();
            Box::new(move |app, dlg| {
                if f.is_empty() {
                    dlg.error = Some("Write a formula, for example [Units] * [Price].".into());
                    return false;
                }
                if let Err(err) = vizcraft_query::m::parse(&format!("each {f}")) {
                    dlg.error = Some(err.message());
                    return false;
                }
                add_step(app, "Added Custom", &e);
                true
            })
        }
        Kind::Conditional { name, rules, otherwise } => {
            let mut e = String::new();
            for (col, op, value, out) in rules.iter().filter(|r| !r.0.is_empty()) {
                e.push_str(&format!("if {} then {} else ", cond_expr(col, *op, value), literal(out)));
            }
            e.push_str(&if otherwise.trim().is_empty() { "null".to_string() } else { literal(otherwise) });
            let step = format!("Table.AddColumn({{prev}}, {}, each {e})", quote_text(name));
            Box::new(move |app, _| {
                add_step(app, "Added Conditional Column", &step);
                true
            })
        }
        Kind::Pivot { col, values, agg } => {
            let f = PIVOT_AGGS.get(*agg).map(|x| x.1).unwrap_or("List.Sum");
            let e = format!(
                "Table.Pivot({{prev}}, List.Distinct(List.Transform({{prev}}{}, Text.From)), {}, {}{})",
                field(col),
                quote_text(col),
                quote_text(values),
                if f.is_empty() { String::new() } else { format!(", {f}") }
            );
            Box::new(move |app, _| {
                add_step(app, "Pivoted Column", &e);
                true
            })
        }
        Kind::Index { start, step } => {
            let e = format!("Table.AddIndexColumn({{prev}}, \"Index\", {}, {}, Int64.Type)", start.trim().parse::<f64>().unwrap_or(1.0), step.trim().parse::<f64>().unwrap_or(1.0));
            Box::new(move |app, _| {
                add_step(app, "Added Index", &e);
                true
            })
        }
        Kind::MergeColumns { cols, sep, custom, name, add } => {
            let s = if *sep == SEPARATORS.len() - 1 { custom.clone() } else { SEPARATORS.get(*sep).map(|x| x.1.to_string()).unwrap_or_default() };
            let name = if name.trim().is_empty() { "Merged".to_string() } else { name.trim().to_string() };
            let e = if *add {
                let parts: Vec<String> = cols.iter().map(|c| format!("Text.From({})", field(c))).collect();
                format!("Table.AddColumn({{prev}}, {}, each Text.Combine({{{}}}, {}), type text)", quote_text(&name), parts.join(", "), quote_text(&s))
            } else {
                let types: Vec<String> = cols.iter().map(|c| format!("{{{}, type text}}", quote_text(c))).collect();
                format!("Table.CombineColumns(Table.TransformColumnTypes({{prev}}, {{{}}}), {}, Combiner.CombineTextByDelimiter({}, QuoteStyle.None), {})", types.join(", "), text_list(cols), quote_text(&s), quote_text(&name))
            };
            let step = if *add { "Inserted Merged Column" } else { "Merged Columns" };
            Box::new(move |app, _| {
                add_step(app, step, &e);
                true
            })
        }
        Kind::Expand { col, names, prefix, record, list, aggregate, aggs } => {
            let picked: Vec<String> = names.iter().filter(|(_, on)| *on).map(|(n, _)| n.clone()).collect();
            let name = format!("{} {col}", if *aggregate { "Aggregated" } else { "Expanded" });
            let e = if *list && names.is_empty() {
                format!("Table.ExpandListColumn({{prev}}, {})", quote_text(col))
            } else if *aggregate {
                let specs: Vec<String> = names
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, on))| *on)
                    .map(|(i, (n, _))| {
                        let (label, f) = EXPAND_AGGS.get(aggs.get(i).map(|a| a.1).unwrap_or(0)).copied().unwrap_or(("Sum", "List.Sum"));
                        format!("{{{}, {f}, {}}}", quote_text(n), quote_text(&format!("{label} of {n}")))
                    })
                    .collect();
                format!("Table.AggregateTableColumn({{prev}}, {}, {{{}}})", quote_text(col), specs.join(", "))
            } else {
                let new: Vec<String> = picked.iter().map(|n| if *prefix { format!("{col}.{n}") } else { n.clone() }).collect();
                let f = if *record { "ExpandRecordColumn" } else { "ExpandTableColumn" };
                format!("Table.{f}({{prev}}, {}, {}, {})", quote_text(col), text_list(&picked), text_list(&new))
            };
            let none = picked.is_empty() && !(*list && names.is_empty());
            Box::new(move |app, dlg| {
                if none {
                    dlg.error = Some("Choose at least one column.".into());
                    return false;
                }
                add_step(app, &name, &e);
                true
            })
        }
        Kind::FilterRows { col, rules, and } => {
            let conds: Vec<String> = rules.iter().enumerate().filter(|(i, (_, v))| *i == 0 || !v.trim().is_empty()).map(|(_, (op, v))| cond_expr(col, *op, v)).collect();
            let joined = conds.join(if *and { " and " } else { " or " });
            Box::new(move |app, _| {
                add_step(app, "Filtered Rows", &format!("Table.SelectRows({{prev}}, each {joined})"));
                true
            })
        }
        Kind::Python { script, output } => {
            let (script, output) = (script.clone(), output.trim().to_string());
            Box::new(move |app, _| {
                add_step(app, "Run Python script", &format!("Python.Execute({}, [dataset={{prev}}])", quote_text(&script)));
                if !output.is_empty() {
                    add_step(app, &output, &format!("{{prev}}{{[Name={}]}}[Value]", quote_text(&output)));
                }
                true
            })
        }
        Kind::Invoke { name, function, args } => {
            let (name, function) = (name.clone(), function.clone());
            let a: Vec<String> = args.iter().map(|c| field(c)).collect();
            let missing = args.iter().any(|c| c.is_empty());
            Box::new(move |app, dlg| {
                if function.is_empty() || missing {
                    dlg.error = Some("Choose the function and a column for each argument.".into());
                    return false;
                }
                add_step(app, "Invoked Custom Function", &format!("Table.AddColumn({{prev}}, {}, each {}({}))", quote_text(&name), steps::ident(&function), a.join(", ")));
                true
            })
        }
        Kind::StepSettings { step, expr, path } => {
            let (step, mut expr) = (step.clone(), expr.clone());
            if let Some((f, p)) = path
                && let Some(i) = expr.find(&format!("{f}(\""))
            {
                let start = i + f.len() + 2;
                if let Some(end) = expr.get(start..).and_then(|r| r.find('"')) {
                    expr.replace_range(start..start + end, &p.replace('"', "\"\""));
                }
            }
            Box::new(move |app, dlg| {
                if let Err(e) = vizcraft_query::m::parse(&expr) {
                    dlg.error = Some(e.message());
                    return false;
                }
                super::edit_steps(app, "Step settings", |m| steps::set_step_expr(m, &step, &expr))
            })
        }
        Kind::NewGroup { old, name, query } => {
            let (old, name, query) = (old.clone(), name.trim().to_string(), query.clone());
            Box::new(move |app, dlg| {
                if name.is_empty() {
                    dlg.error = Some("A group needs a name.".into());
                    return false;
                }
                match (&old, &query) {
                    (Some(o), _) => {
                        app.checkpoint("Rename group");
                        for q in app.project.queries.queries.iter_mut() {
                            if q.group.as_deref() == Some(o.as_str()) {
                                q.group = Some(name.clone());
                            }
                        }
                    }
                    (None, Some(q)) => super::set_group(app, q, Some(name.clone())),
                    (None, None) => {
                        let q = app.pq.query.clone();
                        super::set_group(app, &q, Some(name.clone()));
                    }
                }
                true
            })
        }
        Kind::Reference { .. } | Kind::Shortcuts => Box::new(|_, _| true),
    }
}

/// The number of parts the splitter makes, at most, over the preview rows.
fn split_count(t: &Table, col: &str, splitter: &str) -> usize {
    let Some(c) = t.column_index(col) else { return 2 };
    let host = vizcraft_query::NoHost;
    let ev = vizcraft_query::Evaluator::new(&host);
    let Ok(e) = vizcraft_query::m::parse(splitter) else { return 2 };
    let Ok(f) = ev.eval(&e, &Default::default()) else { return 2 };
    let mut n = 1;
    for r in t.rows.iter().take(1000) {
        let s = r.get(c).map(|v| v.display()).unwrap_or_default();
        if let Ok(Value::List(l)) = ev.apply1(&f, Value::text(s)) {
            n = n.max(l.len());
        }
    }
    n.clamp(1, 500)
}
