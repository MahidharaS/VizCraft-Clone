//! Modeling dialogs: the measure / calculated column / calculated table editor (a formula-bar
//! style window with live validation), quick measures, Tableau calculated fields, manage and edit
//! relationships, parameters (what-if, fields, Tableau value parameters), roles, View as, and Mark
//! as date table.

use egui::{Align2, Color32, RichText, Sense, Ui, vec2};
use vizcraft_core::build::{Built, ColType};
use vizcraft_core::doc::*;
use vizcraft_core::scalar::Scalar;

use super::editor::{self, Lang};
use super::{Sel, col_ref, current_table, glyph_icon, qt, tables_meta};
use crate::app::VizApp;
use crate::theme::Tokens;

pub enum Dialog {
    Formula(FormulaForm),
    Quick(QuickForm),
    Calc(CalcForm),
    Relationships(RelListForm),
    Parameter(ParamForm),
    Roles(RolesForm),
    ViewAs(Option<String>),
    DateTable(DateTableForm),
}

/// Shows whichever dialogs are open.
pub fn show(app: &mut VizApp, ctx: &egui::Context) {
    if let Some(mut d) = app.model_ui.dialog.take() {
        let keep = match &mut d {
            Dialog::Formula(f) => formula_dialog(app, ctx, f),
            Dialog::Quick(f) => quick_dialog(app, ctx, f),
            Dialog::Calc(f) => calc_dialog(app, ctx, f),
            Dialog::Relationships(f) => relationships_dialog(app, ctx, f),
            Dialog::Parameter(f) => parameter_dialog(app, ctx, f),
            Dialog::Roles(f) => roles_dialog(app, ctx, f),
            Dialog::ViewAs(r) => view_as_dialog(app, ctx, r),
            Dialog::DateTable(f) => date_table_dialog(app, ctx, f),
        };
        if keep && app.model_ui.dialog.is_none() {
            app.model_ui.dialog = Some(d);
        }
    }
    if let Some(mut f) = app.model_ui.rel_edit.take() {
        let keep = rel_edit_dialog(app, ctx, &mut f);
        if keep && app.model_ui.rel_edit.is_none() {
            app.model_ui.rel_edit = Some(f);
        }
    }
}

// ------------------------------------------------------------------------------------- helpers

fn status_line(ui: &mut Ui, t: &Tokens, ok: bool, text: &str) {
    ui.horizontal_wrapped(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        glyph_icon(ui.painter(), r, if ok { "check" } else { "warn" }, if ok { t.good } else { t.danger });
        ui.label(RichText::new(text).size(12.0).color(if ok { t.text } else { t.danger }));
    });
}

/// A row of buttons aligned right (a bare right-to-left layout would take the whole height).
pub fn buttons_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
    });
}

/// A vertical divider of a fixed height (a plain separator would stretch the window).
fn vsep(ui: &mut Ui, t: &Tokens, h: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(13.0, h), Sense::hover());
    ui.painter().line_segment([r.center_top(), r.center_bottom()], egui::Stroke::new(1.0, t.border));
}

fn info_line(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.horizontal_wrapped(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        glyph_icon(ui.painter(), r, "info", t.accent);
        ui.label(RichText::new(text).size(11.5).color(t.muted));
    });
}

fn primary(ui: &mut Ui, t: &Tokens, label: &str, enabled: bool) -> bool {
    ui.add_enabled(enabled, egui::Button::new(RichText::new(label).color(Color32::WHITE).strong()).fill(t.accent).min_size(vec2(84.0, 26.0))).clicked()
}

fn secondary(ui: &mut Ui, label: &str) -> bool {
    ui.add(egui::Button::new(label).min_size(vec2(76.0, 26.0))).clicked()
}

fn window(title: &str, width: f32) -> egui::Window<'_> {
    egui::Window::new(title).collapsible(false).resizable(true).default_width(width).pivot(Align2::CENTER_TOP).default_pos(egui::pos2(800.0, 110.0))
}

/// Measure names, for highlighting.
fn measure_names(app: &VizApp) -> Vec<String> {
    app.built.model.measures.iter().map(|m| m.name.clone()).collect()
}

/// A free name: `base`, `base 2`, …
fn free_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

fn name_taken(p: &Project, built: &Built, name: &str, except: Option<&str>) -> bool {
    let n = name.trim();
    if except.is_some_and(|e| e.eq_ignore_ascii_case(n)) {
        return false;
    }
    p.model.measures.iter().any(|m| m.name.eq_ignore_ascii_case(n))
        || built.model.measures.iter().any(|m| m.name.eq_ignore_ascii_case(n))
        || p.model.tables.iter().any(|t| t.name.eq_ignore_ascii_case(n) && false)
}

/// The value of a measure expression with no filters, formatted.
fn try_measure(app: &VizApp, expr: &str, fmt: &str) -> Result<String, String> {
    let e = vizcraft_core::build::substitute_params(expr, &app.project, &app.params());
    let v = app.built.model.evaluate(&e).map_err(|e| e.to_string())?;
    Ok(match v {
        vizcraft_datamodel::Value::Blank => "(Blank)".into(),
        vizcraft_datamodel::Value::Number(n) => vizcraft_core::numfmt::format(n, if fmt.is_empty() { "#,0.##" } else { fmt }),
        other => Scalar::from_dm(&other).text(),
    })
}

fn try_column(app: &VizApp, table: &str, expr: &str) -> Result<String, String> {
    let e = vizcraft_core::build::substitute_params(expr, &app.project, &app.params());
    let mut m = (*app.built.model).clone();
    m.add_calculated_column(table, "__vizcraft_check__", &e).map_err(|e| e.to_string().replace("__vizcraft_check__", "this column"))?;
    let t = m.table(table).ok_or("no such table")?;
    let c = t.column_index("__vizcraft_check__").and_then(|i| t.column(i)).ok_or("no column")?;
    let sample: Vec<String> = (0..t.row_count().min(4)).map(|r| Scalar::from_dm(c.value(r)).text()).collect();
    Ok(format!("{} rows · {} distinct values · first: {}", super::fmt_count(t.row_count()), super::fmt_count(c.cardinality()), sample.join(", ")))
}

fn try_table(app: &VizApp, expr: &str) -> Result<(String, vizcraft_datamodel::QueryResult), String> {
    let e = vizcraft_core::build::substitute_params(expr, &app.project, &app.params());
    let r = app.built.model.query(&e).map_err(|e| e.to_string())?;
    Ok((format!("{} rows · {} columns", super::fmt_count(r.rows.len()), r.columns.len()), r))
}

/// Every model field as (label, DAX reference), for pickers.
fn field_refs(app: &VizApp) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for m in tables_meta(&app.project, &app.built) {
        for c in &m.columns {
            out.push((format!("{}[{}]", m.name, c.name), col_ref(&m.name, &c.name)));
        }
        for (ms, _) in &m.measures {
            out.push((format!("[{ms}]  (measure)"), format!("[{ms}]")));
        }
    }
    out
}

fn table_combo(ui: &mut Ui, id: &str, value: &mut String, tables: &[String], width: f32) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id).selected_text(value.as_str()).width(width).show_ui(ui, |ui| {
        for tn in tables {
            if ui.selectable_label(value == tn, tn).clicked() && value != tn {
                *value = tn.clone();
                changed = true;
            }
        }
    });
    changed
}

// ------------------------------------------------------------------------------------- formula editor

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FormulaKind {
    Measure,
    Column,
    Table,
}

pub struct FormulaForm {
    kind: FormulaKind,
    /// The object being edited (None: new).
    original: Option<String>,
    name: String,
    table: String,
    expr: String,
    format: String,
    folder: String,
    description: String,
    checked: Option<String>,
    check: Result<String, String>,
    preview: Option<vizcraft_datamodel::QueryResult>,
}

impl FormulaForm {
    pub fn new(kind: FormulaKind, table: &str, p: &Project) -> FormulaForm {
        let base = match kind {
            FormulaKind::Measure => "Measure",
            FormulaKind::Column => "Column",
            FormulaKind::Table => "Table",
        };
        let name = match kind {
            FormulaKind::Measure => free_name(base, |n| p.model.measures.iter().any(|m| m.name.eq_ignore_ascii_case(n))),
            FormulaKind::Column => free_name(base, |n| p.model.calculated_columns.iter().any(|c| c.table == table && c.name.eq_ignore_ascii_case(n))),
            FormulaKind::Table => free_name(base, |n| p.model.tables.iter().any(|t| t.name.eq_ignore_ascii_case(n))),
        };
        FormulaForm {
            kind,
            original: None,
            name,
            table: table.to_string(),
            expr: String::new(),
            format: String::new(),
            folder: String::new(),
            description: String::new(),
            checked: None,
            check: Err(String::new()),
            preview: None,
        }
    }
}

pub fn edit_measure(app: &mut VizApp, name: &str) {
    let Some(m) = app.project.model.measures.iter().find(|m| m.name == name).cloned() else { return };
    let mut f = FormulaForm::new(FormulaKind::Measure, &m.table, &app.project);
    f.original = Some(m.name.clone());
    f.name = m.name;
    f.expr = m.expression;
    f.format = m.format;
    f.folder = m.display_folder;
    f.description = m.description;
    app.model_ui.dialog = Some(Dialog::Formula(f));
}

pub fn edit_column(app: &mut VizApp, table: &str, name: &str) {
    let Some(c) = app.project.model.calculated_columns.iter().find(|c| c.table == table && c.name == name).cloned() else { return };
    let mut f = FormulaForm::new(FormulaKind::Column, table, &app.project);
    f.original = Some(c.name.clone());
    f.name = c.name;
    f.expr = c.expression;
    app.model_ui.dialog = Some(Dialog::Formula(f));
}

const TABLE_TEMPLATES: &[(&str, &str)] = &[
    ("Calendar", "ADDCOLUMNS(\n    CALENDAR(DATE(2023, 1, 1), DATE(2024, 12, 31)),\n    \"Year\", YEAR([Date]),\n    \"Quarter\", \"Q\" & QUARTER([Date]),\n    \"Month\", FORMAT([Date], \"mmm\"),\n    \"MonthNo\", MONTH([Date])\n)"),
    ("Calendar (auto)", "CALENDARAUTO()"),
    ("Number series", "GENERATESERIES(1, 10, 1)"),
    ("Distinct values", "DISTINCT(Product[Category])"),
    ("Summary", "SUMMARIZE(Sales, Product[Category], \"Units\", SUM(Sales[Quantity]))"),
    ("Filtered rows", "FILTER(Product, Product[List Price] > 100)"),
    ("Top 5", "TOPN(5, Product, Product[List Price], DESC)"),
];

fn formula_dialog(app: &mut VizApp, ctx: &egui::Context, f: &mut FormulaForm) -> bool {
    let t = app.tokens();
    let title = match (f.kind, f.original.is_some()) {
        (FormulaKind::Measure, false) => "New measure",
        (FormulaKind::Measure, true) => "Edit measure",
        (FormulaKind::Column, false) => "New column",
        (FormulaKind::Column, true) => "Edit column",
        (FormulaKind::Table, _) => "New table",
    };
    // Validate when the formula changed.
    let key = format!("{}|{}|{}", f.table, f.expr, f.format);
    if f.checked.as_deref() != Some(key.as_str()) {
        f.checked = Some(key);
        f.preview = None;
        f.check = if f.expr.trim().is_empty() {
            Err(String::new())
        } else {
            match f.kind {
                FormulaKind::Measure => try_measure(app, &f.expr, &f.format).map(|v| format!("Valid · value with no filters: {v}")),
                FormulaKind::Column => try_column(app, &f.table, &f.expr).map(|v| format!("Valid · {v}")),
                FormulaKind::Table => try_table(app, &f.expr).map(|(s, r)| {
                    f.preview = Some(r);
                    format!("Valid · {s}")
                }),
            }
        };
    }
    let tables: Vec<String> = app.project.model.tables.iter().map(|t| t.name.clone()).collect();
    let measures = measure_names(app);
    let fields = field_refs(app);
    let mut open = true;
    let mut commit = false;
    let mut cancel = false;
    window(title, 780.0).open(&mut open).show(ctx, |ui| {
        super::style_fields(ui, &t);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Name").color(t.muted));
            ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(220.0));
            if f.kind != FormulaKind::Table {
                ui.add_space(10.0);
                ui.label(RichText::new(if f.kind == FormulaKind::Measure { "Home table" } else { "Table" }).color(t.muted));
                ui.add_enabled_ui(f.kind == FormulaKind::Measure || f.original.is_none(), |ui| {
                    table_combo(ui, "fd_table", &mut f.table, &tables, 160.0);
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut ins = None;
                egui::ComboBox::from_id_salt("fd_insert_fn").selected_text("ƒ Function").width(110.0).show_ui(ui, |ui| {
                    for (_, name, sig, d) in editor::DAX_FUNCTIONS {
                        if ui.selectable_label(false, *name).on_hover_text(format!("{sig}\n{d}")).clicked() {
                            ins = Some(format!("{name}("));
                        }
                    }
                });
                egui::ComboBox::from_id_salt("fd_insert").selected_text("Insert field").width(130.0).show_ui(ui, |ui| {
                    for (label, r) in &fields {
                        if ui.selectable_label(false, label).clicked() {
                            ins = Some(r.clone());
                        }
                    }
                });
                if let Some(s) = ins {
                    editor::insert_at_cursor(ui.ctx(), egui::Id::unique("fd_editor"), &mut f.expr, &s);
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} =", if f.name.is_empty() { "…" } else { f.name.as_str() })).monospace().strong());
        });
        let hint = match f.kind {
            FormulaKind::Measure => "SUM(Sales[Quantity])",
            FormulaKind::Column => "Sales[Quantity] * Sales[Unit Price]",
            FormulaKind::Table => "CALENDAR(DATE(2024, 1, 1), DATE(2024, 12, 31))",
        };
        let out = editor::code_editor(ui, egui::Id::unique("fd_editor"), &mut f.expr, Lang::Dax, &t, &measures, 150.0, hint);
        if out.run {
            commit = true;
        }
        ui.add_space(4.0);
        match &f.check {
            Ok(s) => status_line(ui, &t, true, s),
            Err(e) if !e.is_empty() => status_line(ui, &t, false, e),
            Err(_) => info_line(ui, &t, "Type a DAX formula. Ctrl+Enter saves."),
        }
        if f.kind == FormulaKind::Table {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Start from:").size(11.5).color(t.muted));
                for (l, x) in TABLE_TEMPLATES {
                    if ui.small_button(*l).clicked() {
                        f.expr = x.to_string();
                        if f.name.starts_with("Table") {
                            f.name = free_name(l.split(' ').next().unwrap_or("Table"), |n| app.project.model.tables.iter().any(|t| t.name.eq_ignore_ascii_case(n)));
                        }
                    }
                }
            });
            if let Some(r) = &f.preview {
                ui.add_space(4.0);
                preview_grid(ui, &t, r, 6);
            }
        }
        if f.kind == FormulaKind::Measure {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Format").color(t.muted));
                let label = super::table_view::NUMBER_FORMATS.iter().find(|(_, x)| *x == f.format).map(|(l, _)| l.to_string()).unwrap_or_else(|| if f.format.is_empty() { "General".into() } else { f.format.clone() });
                egui::ComboBox::from_id_salt("fd_fmt").selected_text(label).width(150.0).show_ui(ui, |ui| {
                    for (l, x) in super::table_view::NUMBER_FORMATS {
                        if ui.selectable_label(f.format == *x, format!("{l}   {}", vizcraft_core::numfmt::format(1234.5, x))).clicked() {
                            f.format = x.to_string();
                        }
                    }
                });
                ui.add(egui::TextEdit::singleline(&mut f.format).desired_width(90.0).hint_text("custom"));
                ui.add_space(10.0);
                ui.label(RichText::new("Display folder").color(t.muted));
                ui.add(egui::TextEdit::singleline(&mut f.folder).desired_width(140.0));
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Description").color(t.muted));
                ui.add(egui::TextEdit::singleline(&mut f.description).desired_width(f32::INFINITY));
            });
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = !f.name.trim().is_empty() && !f.expr.trim().is_empty() && (f.check.is_ok() || f.kind == FormulaKind::Measure);
                if primary(ui, &t, if f.original.is_some() { "Save" } else { "Create" }, ok) {
                    commit = true;
                }
                if secondary(ui, "Cancel") {
                    cancel = true;
                }
            });
        });
    });
    if commit {
        return !commit_formula(app, f);
    }
    open && !cancel
}

fn commit_formula(app: &mut VizApp, f: &FormulaForm) -> bool {
    let name = f.name.trim().to_string();
    let expr = f.expr.trim().to_string();
    if name.is_empty() || name.contains(['[', ']']) {
        app.error("Give it a valid name (no square brackets).");
        return false;
    }
    if let Err(e) = vizcraft_datamodel::dax::parse(&expr) {
        app.error(format!("The formula has an error: {e}"));
        return false;
    }
    match f.kind {
        FormulaKind::Measure => {
            if name_taken(&app.project, &app.built, &name, f.original.as_deref()) || app.built.model.tables.iter().any(|t| t.column_index(&name).is_some()) {
                app.error(format!("The name {name} is already used by a measure or column."));
                return false;
            }
            app.checkpoint(if f.original.is_some() { "Edit measure" } else { "New measure" });
            let m = Measure { table: f.table.clone(), name: name.clone(), expression: expr, format: f.format.trim().to_string(), description: f.description.clone(), display_folder: f.folder.clone(), ..Measure::default() };
            match &f.original {
                Some(old) => {
                    if let Some(x) = app.project.model.measures.iter_mut().find(|x| &x.name == old) {
                        *x = Measure { hidden: x.hidden, origin: x.origin.clone(), ..m };
                    }
                    if *old != name {
                        super::table_view::replace_refs(&mut app.project, &format!("[{old}]"), &format!("[{name}]"));
                    }
                }
                None => app.project.model.measures.push(m),
            }
            app.model_ui.sel = Sel::Measure(name.clone());
            app.toast(format!("Measure [{name}] saved"));
        }
        FormulaKind::Column => {
            let exists = app.built.model.table(&f.table).is_some_and(|t| t.column_index(&name).is_some());
            if exists && f.original.as_deref().is_none_or(|o| !o.eq_ignore_ascii_case(&name)) {
                app.error(format!("{} already has a column named {name}.", f.table));
                return false;
            }
            app.checkpoint(if f.original.is_some() { "Edit column" } else { "New column" });
            match &f.original {
                Some(old) => {
                    if let Some(x) = app.project.model.calculated_columns.iter_mut().find(|x| x.table == f.table && &x.name == old) {
                        x.name = name.clone();
                        x.expression = expr;
                    }
                    if *old != name {
                        super::table_view::replace_refs(&mut app.project, &format!("[{old}]"), &format!("[{name}]"));
                    }
                }
                None => app.project.model.calculated_columns.push(CalcColumn { table: f.table.clone(), name: name.clone(), expression: expr, origin: String::new() }),
            }
            app.model_ui.sel = Sel::Column(f.table.clone(), name.clone());
            app.toast(format!("Column {}[{name}] saved", f.table));
        }
        FormulaKind::Table => {
            if app.project.model.tables.iter().any(|t| t.name.eq_ignore_ascii_case(&name)) || app.project.queries.queries.iter().any(|q| q.name.eq_ignore_ascii_case(&name)) {
                app.error(format!("There's already a table named {name}."));
                return false;
            }
            app.checkpoint("New table");
            app.project.model.tables.push(ModelTable { name: name.clone(), source: TableSource::Calculated(expr), storage_mode: "Import".into(), ..ModelTable::default() });
            app.model_ui.sel = Sel::Table(name.clone());
            app.toast(format!("Table {name} created"));
        }
    }
    app.mark_model_dirty();
    true
}

/// A small read-only grid of the first rows of a result.
fn preview_grid(ui: &mut Ui, t: &Tokens, r: &vizcraft_datamodel::QueryResult, rows: usize) {
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4.0).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        egui::ScrollArea::horizontal().id_salt("fd_preview").show(ui, |ui| {
            egui::Grid::new("fd_preview_grid").striped(true).spacing(vec2(16.0, 4.0)).show(ui, |ui| {
                for c in &r.columns {
                    let short = c.rsplit_once('[').map(|(_, x)| x.trim_end_matches(']').to_string()).unwrap_or(c.clone());
                    ui.label(RichText::new(short).strong().size(11.5));
                }
                ui.end_row();
                for row in r.rows.iter().take(rows) {
                    for v in row {
                        ui.label(RichText::new(Scalar::from_dm(v).text()).size(11.5));
                    }
                    ui.end_row();
                }
            });
        });
    });
}

// ------------------------------------------------------------------------------------- quick measures

const QUICK: &[(&str, &str, &str)] = &[
    // (title, group, what the fields are)
    ("Average per category", "Aggregate per category", "base,category"),
    ("Variance from baseline", "Filters", "base,field,value"),
    ("Filtered value", "Filters", "base,field,value"),
    ("Percentage difference from filtered value", "Filters", "base,field,value"),
    ("Year-to-date total", "Time intelligence", "base,date"),
    ("Month-over-month change", "Time intelligence", "base,date"),
    ("Year-over-year change", "Time intelligence", "base,date"),
    ("Running total", "Totals", "base,field"),
    ("Total for category (filters applied)", "Totals", "base,category"),
    ("Total for category (filters not applied)", "Totals", "base,category"),
    ("Percentage of grand total", "Totals", "base"),
    ("Weighted average", "Aggregate per category", "base,weight,category"),
    ("Star rating", "Text", "base,category,n"),
    ("Concatenated list of values", "Text", "field,n"),
];

pub struct QuickForm {
    template: usize,
    base: String,
    agg: String,
    weight: String,
    field: String,
    value: String,
    n: usize,
    name: String,
    named: bool,
    checked: String,
    check: Result<String, String>,
}

impl QuickForm {
    pub fn new(app: &VizApp) -> QuickForm {
        let base = app.project.model.measures.first().map(|m| format!("M:{}", m.name)).unwrap_or_default();
        // A text column of a dimension table makes a good first category.
        let metas = tables_meta(&app.project, &app.built);
        let field = metas
            .iter()
            .filter(|m| m.kind == super::TableKind::Dimension)
            .chain(metas.iter())
            .find_map(|m| m.columns.iter().find(|c| c.ty == ColType::Text && !c.name.to_ascii_lowercase().ends_with("id")).map(|c| format!("{}|{}", m.name, c.name)))
            .unwrap_or_default();
        QuickForm { template: 0, base, agg: "Sum".into(), weight: String::new(), field, value: String::new(), n: 5, name: String::new(), named: false, checked: String::new(), check: Err(String::new()) }
    }
}

/// (key, label) of every measure and numeric column: base values.
fn value_fields(app: &VizApp) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for m in tables_meta(&app.project, &app.built) {
        for (ms, _) in &m.measures {
            out.push((format!("M:{ms}"), format!("[{ms}]")));
        }
    }
    for m in tables_meta(&app.project, &app.built) {
        for c in m.columns.iter().filter(|c| c.ty.is_numeric()) {
            out.push((format!("C:{}|{}", m.name, c.name), format!("{}[{}]", m.name, c.name)));
        }
    }
    out
}

fn category_fields(app: &VizApp, temporal: Option<bool>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for m in tables_meta(&app.project, &app.built) {
        for c in &m.columns {
            if temporal.is_some_and(|want| want != c.ty.is_temporal()) {
                continue;
            }
            out.push((format!("{}|{}", m.name, c.name), format!("{}[{}]", m.name, c.name)));
        }
    }
    out
}

fn split_col(key: &str) -> Option<(String, String)> {
    let k = key.strip_prefix("C:").unwrap_or(key);
    k.split_once('|').map(|(a, b)| (a.to_string(), b.to_string()))
}

fn base_dax(key: &str, agg: &str) -> String {
    if let Some(m) = key.strip_prefix("M:") {
        return format!("[{m}]");
    }
    match split_col(key) {
        Some((t, c)) => {
            let f = match agg {
                "Average" => "AVERAGE",
                "Min" => "MIN",
                "Max" => "MAX",
                "Count" => "COUNT",
                "Distinct count" => "DISTINCTCOUNT",
                _ => "SUM",
            };
            format!("{f}({})", col_ref(&t, &c))
        }
        None => "BLANK()".into(),
    }
}

fn base_label(key: &str, agg: &str) -> String {
    if let Some(m) = key.strip_prefix("M:") {
        return m.to_string();
    }
    split_col(key).map(|(_, c)| if agg == "Sum" { c } else { format!("{agg} of {c}") }).unwrap_or_default()
}

fn dax_literal(v: &str) -> String {
    if v.trim().parse::<f64>().is_ok() { v.trim().to_string() } else { format!("\"{}\"", v.replace('"', "\"\"")) }
}

/// The quick measure's DAX and default name.
fn quick_dax(f: &QuickForm) -> Result<(String, String), String> {
    let (title, _, needs) = QUICK.get(f.template).copied().ok_or("Pick a calculation.")?;
    let needs: Vec<&str> = needs.split(',').collect();
    if needs.contains(&"base") && f.base.is_empty() {
        return Err("Pick a base value.".into());
    }
    let b = base_dax(&f.base, &f.agg);
    let bl = base_label(&f.base, &f.agg);
    let field = split_col(&f.field);
    if (needs.contains(&"category") || needs.contains(&"field") || needs.contains(&"date")) && field.is_none() {
        return Err(format!("Pick the {} field.", if needs.contains(&"date") { "date" } else if needs.contains(&"category") { "category" } else { "" }).replace("the  field", "a field"));
    }
    let (ft, fc) = field.clone().unwrap_or_default();
    let fr = col_ref(&ft, &fc);
    if needs.contains(&"value") && f.value.trim().is_empty() {
        return Err("Type the value to filter on.".into());
    }
    let v = dax_literal(&f.value);
    let n = f.n.max(1);
    Ok(match title {
        "Average per category" => (format!("AVERAGEX(\n    KEEPFILTERS(VALUES({fr})),\n    CALCULATE({b})\n)"), format!("{bl} average per {fc}")),
        "Variance from baseline" => (format!("VAR __BASELINE = CALCULATE({b}, {fr} = {v})\nRETURN\n    {b} - __BASELINE"), format!("{bl} difference from {}", f.value)),
        "Filtered value" => (format!("CALCULATE({b}, {fr} = {v})"), format!("{bl} for {}", f.value)),
        "Percentage difference from filtered value" => {
            (format!("VAR __BASELINE = CALCULATE({b}, {fr} = {v})\nRETURN\n    DIVIDE({b} - __BASELINE, __BASELINE)"), format!("{bl} % difference from {}", f.value))
        }
        "Year-to-date total" => (format!("TOTALYTD({b}, {fr})"), format!("{bl} YTD")),
        "Month-over-month change" => (
            format!("VAR __PREV_MONTH = CALCULATE({b}, DATEADD({fr}, -1, MONTH))\nRETURN\n    DIVIDE({b} - __PREV_MONTH, __PREV_MONTH)"),
            format!("{bl} MoM%"),
        ),
        "Year-over-year change" => (
            format!("VAR __PREV_YEAR = CALCULATE({b}, SAMEPERIODLASTYEAR({fr}))\nRETURN\n    DIVIDE({b} - __PREV_YEAR, __PREV_YEAR)"),
            format!("{bl} YoY%"),
        ),
        "Running total" => (format!("CALCULATE(\n    {b},\n    FILTER(ALLSELECTED({fr}), ISONORAFTER({fr}, MAX({fr}), DESC))\n)"), format!("{bl} running total in {fc}")),
        "Total for category (filters applied)" => (format!("CALCULATE({b}, ALLSELECTED({fr}))"), format!("{bl} total for {fc}")),
        "Total for category (filters not applied)" => (format!("CALCULATE({b}, ALL({fr}))"), format!("{bl} total for {fc} (all)")),
        "Percentage of grand total" => (format!("DIVIDE({b}, CALCULATE({b}, ALLSELECTED()))"), format!("{bl} % of total")),
        "Weighted average" => {
            if f.weight.is_empty() {
                return Err("Pick the weight.".into());
            }
            let w = base_dax(&f.weight, "Sum");
            (
                format!("VAR __CATEGORIES = KEEPFILTERS(VALUES({fr}))\nRETURN\n    DIVIDE(\n        SUMX(__CATEGORIES, CALCULATE({b}) * CALCULATE({w})),\n        SUMX(__CATEGORIES, CALCULATE({w}))\n    )"),
                format!("{bl} weighted by {}", base_label(&f.weight, "Sum")),
            )
        }
        "Star rating" => (
            format!(
                "VAR __MAX_STARS = {n}\nVAR __BASE = {b}\nVAR __MIN = MINX(ALLSELECTED({fr}), CALCULATE({b}))\nVAR __MAX = MAXX(ALLSELECTED({fr}), CALCULATE({b}))\nVAR __STARS =\n    IF(ISBLANK(__BASE), 0, INT(DIVIDE(__BASE - __MIN, __MAX - __MIN, 1) * (__MAX_STARS - 1)) + 1)\nRETURN\n    REPT(UNICHAR(9733), __STARS) & REPT(UNICHAR(9734), __MAX_STARS - __STARS)"
            ),
            format!("{bl} star rating"),
        ),
        "Concatenated list of values" => (
            format!(
                "VAR __DISTINCT = VALUES({fr})\nVAR __COUNT = COUNTROWS(__DISTINCT)\nRETURN\n    IF(\n        __COUNT > {n},\n        CONCATENATEX(TOPN({n}, __DISTINCT, {fr}, ASC), {fr}, \", \") & \", etc.\",\n        CONCATENATEX(__DISTINCT, {fr}, \", \")\n    )"
            ),
            format!("List of {fc} values"),
        ),
        _ => return Err("Unknown calculation.".into()),
    })
}

fn field_combo(ui: &mut Ui, id: &str, value: &mut String, items: &[(String, String)], width: f32) {
    let label = items.iter().find(|(k, _)| k == value).map(|(_, l)| l.clone()).unwrap_or_else(|| "Choose…".into());
    egui::ComboBox::from_id_salt(id).selected_text(label).width(width).height(360.0).show_ui(ui, |ui| {
        for (k, l) in items {
            if ui.selectable_label(value == k, l).clicked() {
                *value = k.clone();
            }
        }
    });
}

fn quick_dialog(app: &mut VizApp, ctx: &egui::Context, f: &mut QuickForm) -> bool {
    let t = app.tokens();
    let values = value_fields(app);
    let mut open = true;
    let mut done = false;
    let mut cancel = false;
    let dax = quick_dax(f);
    if let Ok((d, n)) = &dax {
        if !f.named {
            f.name = n.clone();
        }
        if &f.checked != d {
            f.checked = d.clone();
            f.check = try_measure(app, d, "");
        }
    }
    let measures = measure_names(app);
    window("Quick measures", 880.0).open(&mut open).show(ctx, |ui| {
        super::style_fields(ui, &t);
        ui.horizontal_top(|ui| {
            // Calculations.
            ui.vertical(|ui| {
                ui.set_width(250.0);
                ui.label(RichText::new("Calculation").strong());
                ui.add_space(4.0);
                let mut group = "";
                egui::ScrollArea::vertical().id_salt("qm_list").max_height(380.0).show(ui, |ui| {
                    for (i, (title, g, _)) in QUICK.iter().enumerate() {
                        if *g != group {
                            group = g;
                            ui.add_space(4.0);
                            ui.label(RichText::new(g.to_uppercase()).size(10.0).color(t.muted));
                        }
                        if ui.selectable_label(f.template == i, *title).clicked() {
                            f.template = i;
                            f.named = false;
                        }
                    }
                });
            });
            vsep(ui, &t, 400.0);
            // Fields.
            ui.vertical(|ui| {
                let needs = QUICK.get(f.template).map(|q| q.2).unwrap_or("");
                ui.label(RichText::new(QUICK.get(f.template).map(|q| q.0).unwrap_or("")).strong().size(14.0));
                ui.add_space(6.0);
                egui::Grid::new("qm_fields").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                    if needs.contains("base") {
                        ui.label(RichText::new("Base value").color(t.muted));
                        ui.horizontal(|ui| {
                            field_combo(ui, "qm_base", &mut f.base, &values, 220.0);
                            if f.base.starts_with("C:") {
                                egui::ComboBox::from_id_salt("qm_agg").selected_text(&f.agg).width(110.0).show_ui(ui, |ui| {
                                    for a in ["Sum", "Average", "Min", "Max", "Count", "Distinct count"] {
                                        ui.selectable_value(&mut f.agg, a.to_string(), a);
                                    }
                                });
                            }
                        });
                        ui.end_row();
                    }
                    if needs.contains("weight") {
                        ui.label(RichText::new("Weight").color(t.muted));
                        field_combo(ui, "qm_weight", &mut f.weight, &values, 220.0);
                        ui.end_row();
                    }
                    if needs.contains("category") || needs.contains("field") || needs.contains("date") {
                        ui.label(RichText::new(if needs.contains("date") { "Date" } else if needs.contains("category") { "Category" } else { "Field" }).color(t.muted));
                        let cats = category_fields(app, if needs.contains("date") { Some(true) } else { None });
                        field_combo(ui, "qm_field", &mut f.field, &cats, 260.0);
                        ui.end_row();
                    }
                    if needs.contains("value") {
                        ui.label(RichText::new("Filter value").color(t.muted));
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut f.value).desired_width(160.0));
                            if let Some((tn, cn)) = split_col(&f.field)
                                && let Some(col) = app.built.model.table(&tn).and_then(|x| x.column_index(&cn).and_then(|i| x.column(i)))
                            {
                                egui::ComboBox::from_id_salt("qm_vals").selected_text("Pick").width(80.0).show_ui(ui, |ui| {
                                    for id in 0..col.cardinality().min(200) as u32 {
                                        let v = col.dict_value(id).to_text();
                                        if ui.selectable_label(false, &v).clicked() {
                                            f.value = v;
                                        }
                                    }
                                });
                            }
                        });
                        ui.end_row();
                    }
                    if needs.contains(",n") {
                        ui.label(RichText::new(if f.template == 12 { "Stars" } else { "Values shown" }).color(t.muted));
                        ui.add(egui::DragValue::new(&mut f.n).range(1..=20));
                        ui.end_row();
                    }
                    ui.label(RichText::new("Name").color(t.muted));
                    if ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(300.0)).changed() {
                        f.named = true;
                    }
                    ui.end_row();
                });
                ui.add_space(8.0);
                ui.label(RichText::new("DAX").size(11.0).color(t.muted));
                match &dax {
                    Ok((d, _)) => {
                        let job = editor::highlight(d, Lang::Dax, t.dark, &measures, 12.5);
                        egui::Frame::new().fill(t.panel_alt).corner_radius(4.0).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                            ui.set_min_width(520.0);
                            ui.label(job);
                        });
                        ui.add_space(4.0);
                        match &f.check {
                            Ok(v) => status_line(ui, &t, true, &format!("Valid · value with no filters: {v}")),
                            Err(e) => status_line(ui, &t, false, e),
                        }
                    }
                    Err(e) => info_line(ui, &t, e),
                }
            });
        });
        ui.add_space(8.0);
        buttons_row(ui, |ui| {
            if primary(ui, &t, "Add measure", dax.is_ok() && !f.name.trim().is_empty()) {
                done = true;
            }
            if secondary(ui, "Cancel") {
                cancel = true;
            }
        });
    });
    if done && let Ok((d, _)) = &dax {
        let name = f.name.trim().to_string();
        if name_taken(&app.project, &app.built, &name, None) {
            app.error(format!("There's already a measure named {name}."));
            return true;
        }
        let table = split_col(&f.base)
            .map(|(t, _)| t)
            .or_else(|| f.base.strip_prefix("M:").and_then(|m| app.project.model.measures.iter().find(|x| x.name == m).map(|x| x.table.clone())))
            .or_else(|| current_table(app))
            .unwrap_or_default();
        app.checkpoint("Quick measure");
        let fmt = if name.ends_with('%') || name.contains("% ") { "0.0%".to_string() } else { String::new() };
        app.project.model.measures.push(Measure { table, name: name.clone(), expression: d.clone(), format: fmt, origin: "Quick measure".into(), ..Measure::default() });
        app.model_ui.sel = Sel::Measure(name.clone());
        app.mark_model_dirty();
        app.toast(format!("Added quick measure [{name}]"));
        return false;
    }
    open && !cancel
}

// ------------------------------------------------------------------------------------- Tableau calculated field

pub struct CalcForm {
    name: String,
    table: String,
    formula: String,
    checked: String,
    result: Result<vizcraft_core::tableau::Translation, String>,
    check: Result<String, String>,
    search: String,
}

impl CalcForm {
    pub fn set_formula(&mut self, f: &str) {
        self.formula = f.to_string();
    }

    pub fn new(table: &str, p: &Project) -> CalcForm {
        let name = free_name("Calculation", |n| p.model.measures.iter().any(|m| m.name.eq_ignore_ascii_case(n)) || p.model.calculated_columns.iter().any(|c| c.name.eq_ignore_ascii_case(n)));
        CalcForm { name, table: table.to_string(), formula: String::new(), checked: String::new(), result: Err(String::new()), check: Err(String::new()), search: String::new() }
    }
}

const TABLEAU_SNIPPETS: &[(&str, &str)] = &[
    ("IF", "IF [Field] > 0 THEN 'Yes' ELSE 'No' END"),
    ("CASE", "CASE [Field] WHEN 'A' THEN 1 ELSE 0 END"),
    ("FIXED", "{FIXED [Dimension] : SUM([Measure])}"),
    ("ZN", "ZN([Field])"),
    ("DATETRUNC", "DATETRUNC('month', [Date])"),
    ("RUNNING_SUM", "RUNNING_SUM(SUM([Field]))"),
    ("COUNTD", "COUNTD([Field])"),
];

fn calc_dialog(app: &mut VizApp, ctx: &egui::Context, f: &mut CalcForm) -> bool {
    let t = app.tokens();
    let key = format!("{}|{}", f.table, f.formula);
    if f.checked != key {
        f.checked = key;
        f.result = if f.formula.trim().is_empty() { Err(String::new()) } else { vizcraft_core::tableau::translate_in(&app.project, &app.built, &f.formula, &f.table) };
        f.check = match &f.result {
            Ok(tr) if tr.aggregated => try_measure(app, &tr.dax, "").map(|v| format!("Valid · value with no filters: {v}")),
            Ok(tr) => try_column(app, &tr.home_table, &tr.dax).map(|v| format!("Valid · {v}")),
            Err(e) => Err(e.clone()),
        };
    }
    let tables: Vec<String> = app.project.model.tables.iter().map(|t| t.name.clone()).collect();
    let metas = tables_meta(&app.project, &app.built);
    let measures = measure_names(app);
    let mut open = true;
    let mut done = false;
    let mut cancel = false;
    window("Calculated field (Tableau syntax)", 920.0).open(&mut open).show(ctx, |ui| {
        super::style_fields(ui, &t);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Name").color(t.muted));
            ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(240.0));
            ui.add_space(10.0);
            ui.label(RichText::new("Data source table").color(t.muted));
            table_combo(ui, "cf_table", &mut f.table, &tables, 160.0);
        });
        ui.add_space(6.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(600.0);
                let out = editor::code_editor(ui, egui::Id::unique("cf_editor"), &mut f.formula, Lang::Tableau, &t, &measures, 170.0, "IF SUM([Profit]) > 0 THEN 'Profitable' ELSE 'Loss' END");
                if out.run {
                    done = true;
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Insert:").size(11.5).color(t.muted));
                    for (l, s) in TABLEAU_SNIPPETS {
                        if ui.small_button(*l).on_hover_text(*s).clicked() {
                            editor::insert_at_cursor(ui.ctx(), egui::Id::unique("cf_editor"), &mut f.formula, s);
                        }
                    }
                });
                ui.add_space(6.0);
                match &f.result {
                    Ok(tr) => {
                        ui.horizontal(|ui| {
                            let (kind, color) = if tr.aggregated { ("Measure (aggregated)".to_string(), t.good) } else { (format!("Calculated column in {}", tr.home_table), t.accent) };
                            egui::Frame::new().fill(color.gamma_multiply(0.15)).corner_radius(10.0).inner_margin(egui::Margin::symmetric(8, 2)).show(ui, |ui| {
                                ui.label(RichText::new(kind).size(11.5).color(color).strong());
                            });
                            ui.label(RichText::new("translated to DAX:").size(11.5).color(t.muted));
                        });
                        let job = editor::highlight(&tr.dax, Lang::Dax, t.dark, &measures, 12.5);
                        egui::Frame::new().fill(t.panel_alt).corner_radius(4.0).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                            ui.set_min_width(580.0);
                            ui.add(egui::Label::new(job).wrap());
                        });
                        for w in &tr.warnings {
                            info_line(ui, &t, w);
                        }
                        match &f.check {
                            Ok(s) => status_line(ui, &t, true, s),
                            Err(e) => status_line(ui, &t, false, e),
                        }
                    }
                    Err(e) if !e.is_empty() => status_line(ui, &t, false, e),
                    Err(_) => info_line(ui, &t, "Write a Tableau calculation: fields in [brackets], IF/THEN/ELSE/END, CASE, IIF, ZN, LOD expressions {FIXED …}, table calculations. Parameters: [Parameters].[Name]."),
                }
            });
            vsep(ui, &t, 330.0);
            ui.vertical(|ui| {
                ui.set_width(220.0);
                ui.label(RichText::new("Fields").strong());
                ui.add(egui::TextEdit::singleline(&mut f.search).hint_text("Search").desired_width(f32::INFINITY));
                let q = f.search.to_lowercase();
                egui::ScrollArea::vertical().id_salt("cf_fields").max_height(300.0).show(ui, |ui| {
                    for m in &metas {
                        let cols: Vec<&super::ColMeta> = m.columns.iter().filter(|c| q.is_empty() || c.name.to_lowercase().contains(&q)).collect();
                        let ms: Vec<&(String, bool)> = m.measures.iter().filter(|(n, _)| q.is_empty() || n.to_lowercase().contains(&q)).collect();
                        if cols.is_empty() && ms.is_empty() {
                            continue;
                        }
                        ui.label(RichText::new(&m.name).size(11.0).color(t.muted).strong());
                        for c in cols {
                            if ui.add(egui::Button::new(RichText::new(&c.name).size(12.0)).frame(false)).clicked() {
                                editor::insert_at_cursor(ui.ctx(), egui::Id::unique("cf_editor"), &mut f.formula, &format!("[{}]", c.name));
                            }
                        }
                        for (n, _) in ms {
                            if ui.add(egui::Button::new(RichText::new(n).size(12.0).color(t.good)).frame(false)).clicked() {
                                editor::insert_at_cursor(ui.ctx(), egui::Id::unique("cf_editor"), &mut f.formula, &format!("[{n}]"));
                            }
                        }
                    }
                    let params: Vec<String> = app.project.model.parameters.iter().map(|p| p.name.clone()).collect();
                    if !params.is_empty() {
                        ui.label(RichText::new("Parameters").size(11.0).color(t.muted).strong());
                        for p in params {
                            if ui.add(egui::Button::new(RichText::new(&p).size(12.0).color(Color32::from_rgb(0xD9, 0x8A, 0x0B))).frame(false)).clicked() {
                                editor::insert_at_cursor(ui.ctx(), egui::Id::unique("cf_editor"), &mut f.formula, &format!("[Parameters].[{p}]"));
                            }
                        }
                    }
                });
            });
        });
        ui.add_space(8.0);
        buttons_row(ui, |ui| {
            if primary(ui, &t, "OK", f.result.is_ok() && !f.name.trim().is_empty()) {
                done = true;
            }
            if secondary(ui, "Cancel") {
                cancel = true;
            }
        });
    });
    if done && let Ok(tr) = &f.result {
        let name = f.name.trim().to_string();
        if name_taken(&app.project, &app.built, &name, None) || app.built.model.tables.iter().any(|x| x.column_index(&name).is_some()) {
            app.error(format!("The name {name} is already used."));
            return true;
        }
        app.checkpoint("Calculated field");
        if tr.aggregated {
            let table = if f.table.is_empty() { tr.home_table.clone() } else { f.table.clone() };
            app.project.model.measures.push(Measure { table, name: name.clone(), expression: tr.dax.clone(), origin: "Tableau calculated field".into(), description: f.formula.clone(), ..Measure::default() });
            app.model_ui.sel = Sel::Measure(name.clone());
            app.toast(format!("Created measure [{name}]"));
        } else {
            app.project.model.calculated_columns.push(CalcColumn { table: tr.home_table.clone(), name: name.clone(), expression: tr.dax.clone(), origin: "Tableau calculated field".into() });
            app.model_ui.sel = Sel::Column(tr.home_table.clone(), name.clone());
            app.toast(format!("Created column {}[{name}]", tr.home_table));
        }
        app.mark_model_dirty();
        return false;
    }
    open && !cancel
}

// ------------------------------------------------------------------------------------- relationships

/// The column's values are all different and none is blank.
pub fn is_unique(built: &Built, table: &str, column: &str) -> bool {
    let Some(t) = built.model.table(table) else { return false };
    let Some(c) = t.column_index(column).and_then(|i| t.column(i)) else { return false };
    let blanks = (0..c.cardinality() as u32).any(|id| c.dict_value(id).is_blank());
    c.cardinality() == t.row_count() && !blanks
}

fn key_match(built: &Built, many: (&str, &str), one: (&str, &str)) -> Option<f64> {
    let mt = built.model.table(many.0)?;
    let mc = mt.column_index(many.1).and_then(|i| mt.column(i))?;
    let ot = built.model.table(one.0)?;
    let oc = ot.column_index(one.1).and_then(|i| ot.column(i))?;
    let keys: Vec<_> = (0..mc.cardinality() as u32).map(|id| mc.dict_value(id)).filter(|v| !v.is_blank()).collect();
    if keys.is_empty() {
        return None;
    }
    let hits = keys.iter().filter(|v| oc.id_of(&v.key()).is_some()).count();
    Some(hits as f64 / keys.len() as f64)
}

/// Checks a relationship against the data: (fine, message) lines.
pub fn validate_relationship(p: &Project, built: &Built, r: &Relationship) -> Vec<(bool, String)> {
    let mut out = Vec::new();
    let from = (r.from_table.as_str(), r.from_column.as_str());
    let to = (r.to_table.as_str(), r.to_column.as_str());
    let uf = is_unique(built, from.0, from.1);
    let ut = is_unique(built, to.0, to.1);
    let name = |x: (&str, &str)| format!("{}[{}]", x.0, x.1);
    match r.cardinality {
        Cardinality::ManyToOne => out.push(if ut { (true, format!("{} has unique values: it can be the one side.", name(to))) } else { (false, format!("{} has duplicate or blank values: use Many to many.", name(to))) }),
        Cardinality::OneToMany => out.push(if uf { (true, format!("{} has unique values.", name(from))) } else { (false, format!("{} has duplicate or blank values: use Many to many.", name(from))) }),
        Cardinality::OneToOne => out.push(if uf && ut { (true, "Both columns have unique values.".into()) } else { (false, "One to one needs unique values on both sides.".into()) }),
        Cardinality::ManyToMany => out.push((true, "Many to many: neither side has to be unique (VizCraft keeps it but doesn't filter through it yet).".into())),
    }
    let (many, one) = if r.cardinality == Cardinality::OneToMany { (to, from) } else { (from, to) };
    if let Some(m) = key_match(built, many, one) {
        let pct = (m * 100.0).round();
        out.push((m >= 0.5, format!("{pct}% of {} values are found in {}.", name(many), name(one))));
    }
    let tf = super::column_type(p, built, from.0, from.1);
    let tt = super::column_type(p, built, to.0, to.1);
    if (tf == ColType::Text) != (tt == ColType::Text) {
        out.push((false, format!("The columns have different data types ({} and {}).", tf.label(), tt.label())));
    }
    if r.active
        && p.model.relationships.iter().any(|x| x.id != r.id && x.active && ((x.from_table == r.from_table && x.to_table == r.to_table) || (x.from_table == r.to_table && x.to_table == r.from_table)))
    {
        out.push((false, "Another active relationship already connects these tables: only one can be active.".into()));
    }
    out
}

#[derive(Default)]
pub struct RelListForm {
    selected: Option<u64>,
}

pub struct RelForm {
    id: Option<u64>,
    r: Relationship,
}

pub fn edit_relationship(app: &mut VizApp, id: u64) {
    if let Some(r) = app.project.model.relationships.iter().find(|r| r.id == id).cloned() {
        app.model_ui.rel_edit = Some(RelForm { id: Some(id), r });
    }
}

fn new_relationship_form(app: &VizApp) -> RelForm {
    let metas = tables_meta(&app.project, &app.built);
    let a = metas.iter().find(|m| m.kind == super::TableKind::Fact).or(metas.first()).map(|m| m.name.clone()).unwrap_or_default();
    let b = metas.iter().find(|m| m.name != a).map(|m| m.name.clone()).unwrap_or_default();
    RelForm { id: None, r: Relationship { from_table: a, to_table: b, active: true, ..Relationship::default() } }
}

/// Relationships the data suggests: same-named key columns, and `XxxID` → table `Xxx`.
pub fn autodetect(app: &mut VizApp) -> usize {
    let mut found: Vec<((String, String), (String, String))> = app.built.model.suggest_relationships();
    let metas = tables_meta(&app.project, &app.built);
    for a in &metas {
        for b in &metas {
            if a.name == b.name {
                continue;
            }
            for c in &a.columns {
                let base = c.name.trim_end_matches("ID").trim_end_matches("Id").trim_end_matches("Key").trim_end_matches([' ', '_']);
                if base.len() == c.name.len() || !(base.eq_ignore_ascii_case(&b.name) || format!("{base}s").eq_ignore_ascii_case(&b.name)) {
                    continue;
                }
                for key in ["ID", "Id", "Key", &c.name] {
                    if b.columns.iter().any(|x| x.name == key) && is_unique(&app.built, &b.name, key) && !is_unique(&app.built, &a.name, &c.name) {
                        found.push(((a.name.clone(), c.name.clone()), (b.name.clone(), key.to_string())));
                        break;
                    }
                }
            }
        }
    }
    let mut added = 0;
    for ((mt, mc), (ot, oc)) in found {
        let related = app.project.model.relationships.iter().any(|r| (r.from_table == mt && r.to_table == ot) || (r.from_table == ot && r.to_table == mt));
        if related {
            continue;
        }
        if added == 0 {
            app.checkpoint("Autodetect relationships");
        }
        let id = app.project.next_id();
        app.project.model.relationships.push(Relationship { id, from_table: mt, from_column: mc, to_table: ot, to_column: oc, active: true, ..Relationship::default() });
        added += 1;
    }
    if added > 0 {
        app.mark_model_dirty();
    }
    added
}

fn relationships_dialog(app: &mut VizApp, ctx: &egui::Context, f: &mut RelListForm) -> bool {
    let t = app.tokens();
    let mut open = true;
    let mut close = false;
    let rels = app.project.model.relationships.clone();
    window("Manage relationships", 860.0).open(&mut open).show(ctx, |ui| {
        super::style_fields(ui, &t);
        ui.label(RichText::new(format!("{} relationships", rels.len())).color(t.muted));
        ui.add_space(4.0);
        egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4.0).show(ui, |ui| {
            ui.set_min_height(260.0);
            egui::ScrollArea::vertical().id_salt("mr_list").max_height(320.0).show(ui, |ui| {
                egui::Grid::new("mr_grid").num_columns(5).striped(true).spacing(vec2(18.0, 6.0)).min_col_width(40.0).show(ui, |ui| {
                    for h in ["Active", "From: Table (Column)", "To: Table (Column)", "Cardinality", "Cross-filter"] {
                        ui.label(RichText::new(h).strong().size(12.0));
                    }
                    ui.end_row();
                    for r in &rels {
                        let mut active = r.active;
                        if ui.checkbox(&mut active, "").changed() {
                            app.checkpoint("Relationship active");
                            if active {
                                // Only one active path between two tables.
                                for x in app.project.model.relationships.iter_mut() {
                                    if x.id != r.id && ((x.from_table == r.from_table && x.to_table == r.to_table) || (x.from_table == r.to_table && x.to_table == r.from_table)) {
                                        x.active = false;
                                    }
                                }
                            }
                            if let Some(x) = app.project.model.relationships.iter_mut().find(|x| x.id == r.id) {
                                x.active = active;
                            }
                            app.mark_model_dirty();
                        }
                        let sel = f.selected == Some(r.id);
                        let a = ui.selectable_label(sel, format!("{} ({})", r.from_table, r.from_column));
                        let b = ui.selectable_label(sel, format!("{} ({})", r.to_table, r.to_column));
                        ui.label(r.cardinality.label());
                        ui.label(if r.cross_filter == CrossFilterDir::Both { "Both" } else { "Single" });
                        ui.end_row();
                        if a.clicked() || b.clicked() {
                            f.selected = Some(r.id);
                        }
                        if a.double_clicked() || b.double_clicked() {
                            app.model_ui.rel_edit = Some(RelForm { id: Some(r.id), r: r.clone() });
                        }
                    }
                });
                if rels.is_empty() {
                    ui.add_space(20.0);
                    ui.label(RichText::new("No relationships yet. Click New… or Autodetect.").color(t.muted));
                }
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if secondary(ui, "New…") {
                app.model_ui.rel_edit = Some(new_relationship_form(app));
            }
            if secondary(ui, "Autodetect…") {
                let n = autodetect(app);
                if n == 0 {
                    app.toast("Autodetect found no new relationships.");
                } else {
                    app.toast(format!("Autodetect found {n} new relationship{}.", if n == 1 { "" } else { "s" }));
                }
            }
            if ui.add_enabled(f.selected.is_some(), egui::Button::new("Edit…").min_size(vec2(76.0, 26.0))).clicked()
                && let Some(id) = f.selected
            {
                edit_relationship(app, id);
            }
            if ui.add_enabled(f.selected.is_some(), egui::Button::new("Delete").min_size(vec2(76.0, 26.0))).clicked()
                && let Some(id) = f.selected.take()
            {
                super::delete(app, &Sel::Relationship(id));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if primary(ui, &t, "Close", true) {
                    close = true;
                }
            });
        });
    });
    open && !close
}

fn column_picker(ui: &mut Ui, t: &Tokens, app: &VizApp, id: &str, table: &str, column: &mut String) {
    let Some(tb) = app.built.model.table(table) else {
        ui.label(RichText::new("No data loaded for this table.").color(t.muted));
        return;
    };
    let cols: Vec<String> = tables_meta(&app.project, &app.built).into_iter().find(|m| m.name == table).map(|m| m.columns.into_iter().map(|c| c.name).collect()).unwrap_or_default();
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4.0).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
        egui::ScrollArea::horizontal().id_salt(id).show(ui, |ui| {
            egui::Grid::new((id, "g")).striped(true).spacing(vec2(2.0, 3.0)).show(ui, |ui| {
                for c in &cols {
                    let sel = column == c;
                    let txt = RichText::new(c).strong().size(12.0).color(if sel { Color32::WHITE } else { t.text });
                    if ui.add(egui::Button::new(txt).fill(if sel { t.accent } else { t.panel_alt }).min_size(vec2(80.0, 22.0))).on_hover_text("Click to use this column").clicked() {
                        *column = c.clone();
                    }
                }
                ui.end_row();
                for r in 0..tb.row_count().min(5) {
                    for c in &cols {
                        let v = tb.column_index(c).and_then(|i| tb.column(i)).map(|x| Scalar::from_dm(x.value(r)).text()).unwrap_or_default();
                        let sel = column == c;
                        ui.label(RichText::new(vizcraft_core::scene::ellipsize(&v, 11.5, 120.0)).size(11.5).color(if sel { t.accent } else { t.muted }));
                    }
                    ui.end_row();
                }
            });
        });
    });
}

fn rel_edit_dialog(app: &mut VizApp, ctx: &egui::Context, f: &mut RelForm) -> bool {
    let t = app.tokens();
    let tables: Vec<String> = app.project.model.tables.iter().map(|t| t.name.clone()).collect();
    let mut open = true;
    let mut done = false;
    let mut cancel = false;
    let title = if f.id.is_some() { "Edit relationship" } else { "New relationship" };
    window(title, 840.0).open(&mut open).show(ctx, |ui| {
        super::style_fields(ui, &t);
        ui.label(RichText::new("Select tables and columns that are related.").color(t.muted));
        ui.add_space(6.0);
        ui.label(RichText::new("From table").strong());
        if table_combo(ui, "re_from", &mut f.r.from_table, &tables, 260.0) {
            f.r.from_column.clear();
        }
        column_picker(ui, &t, app, "re_from_cols", &f.r.from_table.clone(), &mut f.r.from_column);
        ui.add_space(8.0);
        ui.label(RichText::new("To table").strong());
        if table_combo(ui, "re_to", &mut f.r.to_table, &tables, 260.0) {
            f.r.to_column.clear();
        }
        column_picker(ui, &t, app, "re_to_cols", &f.r.to_table.clone(), &mut f.r.to_column);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Cardinality").size(11.5).color(t.muted));
                egui::ComboBox::from_id_salt("re_card").selected_text(f.r.cardinality.label()).width(200.0).show_ui(ui, |ui| {
                    for c in Cardinality::ALL {
                        ui.selectable_value(&mut f.r.cardinality, c, c.label());
                    }
                });
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("Cross-filter direction").size(11.5).color(t.muted));
                egui::ComboBox::from_id_salt("re_cross").selected_text(if f.r.cross_filter == CrossFilterDir::Both { "Both" } else { "Single" }).width(200.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut f.r.cross_filter, CrossFilterDir::Single, "Single");
                    ui.selectable_value(&mut f.r.cross_filter, CrossFilterDir::Both, "Both");
                });
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.checkbox(&mut f.r.active, "Make this relationship active");
                ui.add_enabled(f.r.cross_filter == CrossFilterDir::Both, egui::Checkbox::new(&mut f.r.security_both, "Apply security filter in both directions"));
                ui.checkbox(&mut f.r.assume_integrity, "Assume referential integrity");
            });
        });
        ui.add_space(8.0);
        let ready = !f.r.from_column.is_empty() && !f.r.to_column.is_empty() && !(f.r.from_table == f.r.to_table);
        if ready {
            if ui.small_button("Suggest cardinality from the data").clicked() {
                let (uf, ut) = (is_unique(&app.built, &f.r.from_table, &f.r.from_column), is_unique(&app.built, &f.r.to_table, &f.r.to_column));
                f.r.cardinality = match (uf, ut) {
                    (true, true) => Cardinality::OneToOne,
                    (false, true) => Cardinality::ManyToOne,
                    (true, false) => Cardinality::OneToMany,
                    _ => Cardinality::ManyToMany,
                };
            }
            for (ok, msg) in validate_relationship(&app.project, &app.built, &Relationship { id: f.id.unwrap_or(0), ..f.r.clone() }) {
                status_line(ui, &t, ok, &msg);
            }
        } else if f.r.from_table == f.r.to_table {
            status_line(ui, &t, false, "Pick two different tables.");
        } else {
            info_line(ui, &t, "Click a column header in each table to choose the columns to relate.");
        }
        ui.add_space(8.0);
        buttons_row(ui, |ui| {
            if primary(ui, &t, "OK", ready) {
                done = true;
            }
            if secondary(ui, "Cancel") {
                cancel = true;
            }
        });
    });
    if done {
        app.checkpoint(title);
        if f.r.active {
            for x in app.project.model.relationships.iter_mut() {
                if Some(x.id) != f.id && ((x.from_table == f.r.from_table && x.to_table == f.r.to_table) || (x.from_table == f.r.to_table && x.to_table == f.r.from_table)) {
                    x.active = false;
                }
            }
        }
        match f.id {
            Some(id) => {
                if let Some(x) = app.project.model.relationships.iter_mut().find(|x| x.id == id) {
                    *x = Relationship { id, ..f.r.clone() };
                }
            }
            None => {
                let id = app.project.next_id();
                app.project.model.relationships.push(Relationship { id, ..f.r.clone() });
                app.model_ui.sel = Sel::Relationship(id);
            }
        }
        app.mark_model_dirty();
        return false;
    }
    open && !cancel
}

// ------------------------------------------------------------------------------------- parameters

pub struct ParamForm {
    kind: String,
    name: String,
    data_type: String,
    min: f64,
    max: f64,
    step: f64,
    default: f64,
    add_slicer: bool,
    items: Vec<FieldRef>,
    search: String,
    vtype: String,
    allow: usize,
    list: Vec<String>,
    new_item: String,
    current: String,
    rmin: f64,
    rmax: f64,
    rstep: f64,
    from_field: String,
}

impl ParamForm {
    pub fn new(kind: &str, p: &Project) -> ParamForm {
        let base = match kind {
            "fields" => "Parameter",
            "value" => "Parameter",
            _ => "Parameter",
        };
        let name = free_name(base, |n| p.model.parameters.iter().any(|x| x.name.eq_ignore_ascii_case(n)) || p.model.tables.iter().any(|t| t.name.eq_ignore_ascii_case(n)));
        ParamForm {
            kind: kind.to_string(),
            name,
            data_type: "Decimal number".into(),
            min: 0.0,
            max: 20.0,
            step: 1.0,
            default: 0.0,
            add_slicer: true,
            items: vec![],
            search: String::new(),
            vtype: "Float".into(),
            allow: 0,
            list: vec![],
            new_item: String::new(),
            current: "0".into(),
            rmin: 0.0,
            rmax: 100.0,
            rstep: 1.0,
            from_field: String::new(),
        }
    }
}

fn add_slicer(app: &mut VizApp, name: &str, mode: &str) {
    let id = app.project.next_id();
    let Some(page) = app.current_page_mut() else { return };
    let mut v = VisualSpec::new(VisualKind::Slicer);
    v.slicer_mode = mode.to_string();
    v.set("field", vec![FieldUse::new(FieldRef::col(name, name))]);
    let w = 260.0;
    let x = (page.width - w - 24.0).max(24.0);
    let y = 24.0 + page.elements.iter().filter(|e| e.x >= x - 10.0 && matches!(e.kind, ElementKind::Visual(ref s) if s.kind == VisualKind::Slicer)).count() as f32 * 130.0;
    page.elements.push(Element { id, name: name.to_string(), x, y, w, h: 110.0, kind: ElementKind::Visual(v), ..Element::default() });
}

fn parameter_dialog(app: &mut VizApp, ctx: &egui::Context, f: &mut ParamForm) -> bool {
    let t = app.tokens();
    let mut open = true;
    let mut done = false;
    let mut cancel = false;
    let title = match f.kind.as_str() {
        "fields" => "Parameters: Fields",
        "value" => "Create parameter (Tableau)",
        _ => "Parameters: Numeric range (what-if)",
    };
    let metas = tables_meta(&app.project, &app.built);
    window(title, 620.0).open(&mut open).show(ctx, |ui| {
        super::style_fields(ui, &t);
        ui.horizontal(|ui| {
            ui.add_sized([110.0, 20.0], egui::Label::new(RichText::new("Name").color(t.muted)));
            ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(260.0));
        });
        ui.add_space(6.0);
        match f.kind.as_str() {
            "fields" => {
                ui.label(RichText::new("Add and reorder fields: the slicer switches visuals between them.").color(t.muted));
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(260.0);
                        ui.add(egui::TextEdit::singleline(&mut f.search).hint_text("Search fields").desired_width(f32::INFINITY));
                        let q = f.search.to_lowercase();
                        egui::ScrollArea::vertical().id_salt("pf_fields").max_height(260.0).show(ui, |ui| {
                            for m in &metas {
                                ui.label(RichText::new(&m.name).size(11.0).strong().color(t.muted));
                                for (ms, _) in m.measures.iter().filter(|(n, _)| q.is_empty() || n.to_lowercase().contains(&q)) {
                                    let fr = FieldRef::measure(&m.name, ms);
                                    let has = f.items.contains(&fr);
                                    if ui.selectable_label(has, RichText::new(ms).color(t.good)).clicked() && !has {
                                        f.items.push(fr);
                                    }
                                }
                                for c in m.columns.iter().filter(|c| q.is_empty() || c.name.to_lowercase().contains(&q)) {
                                    let fr = FieldRef::col(&m.name, &c.name);
                                    let has = f.items.contains(&fr);
                                    if ui.selectable_label(has, &c.name).clicked() && !has {
                                        f.items.push(fr);
                                    }
                                }
                            }
                        });
                    });
                    vsep(ui, &t, 280.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Fields in the parameter").strong());
                        let mut action = None;
                        for (i, it) in f.items.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("{}.", i + 1)).color(t.muted));
                                ui.label(format!("{} ({})", it.label(), it.table()));
                                if super::icon_btn(ui, &t, "minus", "Remove", Some(t.danger)).clicked() {
                                    action = Some((i, 0));
                                }
                                if i > 0 && ui.small_button("Up").clicked() {
                                    action = Some((i, 1));
                                }
                            });
                        }
                        if let Some((i, a)) = action {
                            if a == 0 {
                                f.items.remove(i);
                            } else {
                                f.items.swap(i, i - 1);
                            }
                        }
                        if f.items.is_empty() {
                            ui.label(RichText::new("Click fields on the left to add them.").color(t.muted));
                        }
                    });
                });
                ui.checkbox(&mut f.add_slicer, "Add slicer to this page");
            }
            "value" => {
                egui::Grid::new("pv_grid").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                    ui.label(RichText::new("Data type").color(t.muted));
                    egui::ComboBox::from_id_salt("pv_type").selected_text(&f.vtype).width(160.0).show_ui(ui, |ui| {
                        for ty in ["Float", "Integer", "String", "Boolean", "Date"] {
                            ui.selectable_value(&mut f.vtype, ty.to_string(), ty);
                        }
                    });
                    ui.end_row();
                    ui.label(RichText::new("Current value").color(t.muted));
                    ui.add(egui::TextEdit::singleline(&mut f.current).desired_width(160.0));
                    ui.end_row();
                    ui.label(RichText::new("Allowable values").color(t.muted));
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut f.allow, 0, "All");
                        ui.radio_value(&mut f.allow, 1, "List");
                        ui.add_enabled_ui(matches!(f.vtype.as_str(), "Float" | "Integer" | "Date"), |ui| {
                            ui.radio_value(&mut f.allow, 2, "Range");
                        });
                    });
                    ui.end_row();
                });
                ui.add_space(4.0);
                if f.allow == 1 {
                    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4.0).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                        let mut remove = None;
                        for (i, v) in f.list.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(v);
                                if super::icon_btn(ui, &t, "minus", "Remove", Some(t.danger)).clicked() {
                                    remove = Some(i);
                                }
                            });
                        }
                        if let Some(i) = remove {
                            f.list.remove(i);
                        }
                        ui.horizontal(|ui| {
                            let r = ui.add(egui::TextEdit::singleline(&mut f.new_item).hint_text("Add a value").desired_width(160.0));
                            if (ui.button("Add").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))) && !f.new_item.trim().is_empty() {
                                f.list.push(f.new_item.trim().to_string());
                                f.new_item.clear();
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Fill from field when the data refreshes").size(11.5).color(t.muted));
                            let cats = category_fields(app, None);
                            field_combo(ui, "pv_from", &mut f.from_field, &cats, 200.0);
                            if !f.from_field.is_empty() && ui.small_button("Clear").clicked() {
                                f.from_field.clear();
                            }
                        });
                    });
                }
                if f.allow == 2 {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Minimum").color(t.muted));
                        ui.add(egui::DragValue::new(&mut f.rmin).speed(1.0));
                        ui.label(RichText::new("Maximum").color(t.muted));
                        ui.add(egui::DragValue::new(&mut f.rmax).speed(1.0));
                        ui.label(RichText::new("Step size").color(t.muted));
                        ui.add(egui::DragValue::new(&mut f.rstep).speed(0.1).range(0.0..=f64::MAX));
                    });
                }
                ui.add_space(6.0);
                let r = if f.name.chars().all(|c| c.is_alphanumeric() || c == '_') { format!("@{}", f.name) } else { format!("@[{}]", f.name) };
                info_line(ui, &t, &format!("Use it in DAX and fx formulas as {r} (e.g. [Total Sales] * (1 + {r})), and in calculated fields as [Parameters].[{}].", f.name));
            }
            _ => {
                egui::Grid::new("pw_grid").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                    ui.label(RichText::new("Data type").color(t.muted));
                    egui::ComboBox::from_id_salt("pw_type").selected_text(&f.data_type).width(160.0).show_ui(ui, |ui| {
                        for ty in ["Whole number", "Decimal number", "Fixed decimal number"] {
                            ui.selectable_value(&mut f.data_type, ty.to_string(), ty);
                        }
                    });
                    ui.end_row();
                    let whole = f.data_type == "Whole number";
                    for (label, v) in [("Minimum", &mut f.min), ("Maximum", &mut f.max), ("Increment", &mut f.step), ("Default", &mut f.default)] {
                        ui.label(RichText::new(label).color(t.muted));
                        let mut dv = egui::DragValue::new(v).speed(if whole { 1.0 } else { 0.01 });
                        if whole {
                            dv = dv.fixed_decimals(0);
                        }
                        ui.add(dv);
                        ui.end_row();
                    }
                });
                ui.checkbox(&mut f.add_slicer, "Add slicer to this page");
                let n = if f.step > 0.0 { ((f.max - f.min) / f.step).floor() + 1.0 } else { 0.0 };
                if f.step <= 0.0 || f.max < f.min {
                    status_line(ui, &t, false, "The increment must be above 0 and the maximum at least the minimum.");
                } else if n > 1000.0 {
                    status_line(ui, &t, false, &format!("That makes {n} values; keep it under 1,000."));
                } else {
                    info_line(ui, &t, &format!("Creates a table '{}' of {n} values (GENERATESERIES) and a measure [{} Value] that returns the selected value.", f.name, f.name));
                }
            }
        }
        ui.add_space(8.0);
        buttons_row(ui, |ui| {
            if primary(ui, &t, "Create", !f.name.trim().is_empty()) {
                done = true;
            }
            if secondary(ui, "Cancel") {
                cancel = true;
            }
        });
    });
    if done {
        let name = f.name.trim().to_string();
        if app.project.model.parameters.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) || app.project.model.tables.iter().any(|x| x.name.eq_ignore_ascii_case(&name)) {
            app.error(format!("The name {name} is already used."));
            return true;
        }
        let kind = match f.kind.as_str() {
            "fields" => {
                if f.items.is_empty() {
                    app.error("Add at least one field.");
                    return true;
                }
                ParameterKind::Fields { items: f.items.clone() }
            }
            "value" => ParameterKind::Value {
                data_type: match f.vtype.as_str() {
                    "Integer" => "Whole Number",
                    "String" => "Text",
                    "Boolean" => "True/False",
                    "Date" => "Date",
                    _ => "Decimal Number",
                }
                .into(),
                list: if f.allow == 1 { f.list.iter().map(|v| vizcraft_core::sheet::parse_input(v)).collect() } else { vec![] },
                min: (f.allow == 2).then_some(f.rmin),
                max: (f.allow == 2).then_some(f.rmax),
                step: (f.allow == 2).then_some(f.rstep),
                from_field: Some(f.from_field.replacen('|', "[", 1) + "]").filter(|_| !f.from_field.is_empty()),
            },
            _ => {
                if f.step <= 0.0 || f.max < f.min || (f.max - f.min) / f.step > 1000.0 {
                    app.error("Check the range: the increment must be above 0, with at most 1,000 values.");
                    return true;
                }
                ParameterKind::WhatIf { min: f.min, max: f.max, step: f.step, default: f.default }
            }
        };
        let current = match &kind {
            ParameterKind::WhatIf { default, .. } => Scalar::Num(*default),
            ParameterKind::Fields { .. } => Scalar::Num(0.0),
            ParameterKind::Value { data_type, .. } => match data_type.as_str() {
                "Text" => Scalar::Text(f.current.clone()),
                "Date" => vizcraft_core::build::parse_date(&f.current).map(Scalar::Num).unwrap_or(Scalar::Text(f.current.clone())),
                _ => vizcraft_core::sheet::parse_input(&f.current),
            },
        };
        app.checkpoint("New parameter");
        let slicer = f.add_slicer && f.kind != "value";
        app.project.model.parameters.push(Parameter { name: name.clone(), kind, current, description: String::new() });
        if slicer {
            add_slicer(app, &name, if f.kind == "fields" { "Tile" } else { "Dropdown" });
        }
        app.mark_model_dirty();
        app.toast(if slicer { format!("Created parameter {name} and added a slicer to the page") } else { format!("Created parameter {name}") });
        return false;
    }
    open && !cancel
}

// ------------------------------------------------------------------------------------- roles

pub struct RolesForm {
    roles: Vec<Role>,
    role: usize,
    table: String,
    check: Option<Result<String, String>>,
    members: String,
}

impl RolesForm {
    pub fn new(p: &Project) -> RolesForm {
        let roles = p.model.roles.clone();
        let members = roles.first().map(|r| r.members.join(", ")).unwrap_or_default();
        RolesForm { roles, role: 0, table: p.model.tables.first().map(|t| t.name.clone()).unwrap_or_default(), check: None, members }
    }
}

fn roles_dialog(app: &mut VizApp, ctx: &egui::Context, f: &mut RolesForm) -> bool {
    let t = app.tokens();
    let mut open = true;
    let mut done = false;
    let mut cancel = false;
    let mut delete_role = false;
    let tables: Vec<String> = app.project.model.tables.iter().map(|t| t.name.clone()).collect();
    let measures = measure_names(app);
    window("Manage security roles", 940.0).open(&mut open).show(ctx, |ui| {
        super::style_fields(ui, &t);
        ui.horizontal_top(|ui| {
            // Roles.
            ui.vertical(|ui| {
                ui.set_width(200.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Roles").strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if super::icon_btn(ui, &t, "plus", "New role", Some(t.accent)).clicked() {
                            let name = free_name("Role", |n| f.roles.iter().any(|r| r.name == n));
                            f.roles.push(Role { name, ..Role::default() });
                            f.role = f.roles.len() - 1;
                            f.members.clear();
                        }
                    });
                });
                let mut switch = None;
                for (i, r) in f.roles.iter().enumerate() {
                    let n = r.filters.iter().filter(|(_, d)| !d.trim().is_empty()).count();
                    if ui.selectable_label(f.role == i, format!("{}   ({n} filter{})", r.name, if n == 1 { "" } else { "s" })).clicked() {
                        switch = Some(i);
                    }
                }
                if let Some(i) = switch {
                    if let Some(r) = f.roles.get_mut(f.role) {
                        r.members = f.members.split(',').map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect();
                    }
                    f.role = i;
                    f.members = f.roles.get(i).map(|r| r.members.join(", ")).unwrap_or_default();
                    f.check = None;
                }
                if f.roles.is_empty() {
                    ui.label(RichText::new("No roles yet.").color(t.muted));
                }
            });
            vsep(ui, &t, 380.0);
            let Some(role) = f.roles.get_mut(f.role) else {
                ui.label(RichText::new("Create a role, then give it DAX filters on tables: people in the role only see the rows the filters keep.").color(t.muted));
                return;
            };
            // Tables.
            ui.vertical(|ui| {
                ui.set_width(200.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Name").color(t.muted));
                    ui.add(egui::TextEdit::singleline(&mut role.name).desired_width(140.0));
                });
                ui.add_space(6.0);
                ui.label(RichText::new("Tables").strong());
                for tn in &tables {
                    let has = role.filters.iter().any(|(x, d)| x == tn && !d.trim().is_empty());
                    let label = if has { RichText::new(format!("{tn}   (filtered)")).color(t.accent) } else { RichText::new(tn) };
                    if ui.selectable_label(f.table == *tn, label).clicked() {
                        f.table = tn.clone();
                        f.check = None;
                    }
                }
            });
            vsep(ui, &t, 380.0);
            // Filter.
            ui.vertical(|ui| {
                ui.set_width(460.0);
                ui.label(RichText::new(format!("Filter DAX expression for {}", f.table)).strong());
                ui.label(RichText::new("A TRUE/FALSE condition evaluated for each row, e.g. [Region] = \"West\"").size(11.5).color(t.muted));
                let idx = match role.filters.iter().position(|(x, _)| *x == f.table) {
                    Some(i) => i,
                    None => {
                        role.filters.push((f.table.clone(), String::new()));
                        role.filters.len() - 1
                    }
                };
                if let Some((_, dax)) = role.filters.get_mut(idx) {
                    let out = editor::code_editor(ui, egui::Id::unique(("role_editor", f.role, &f.table)), dax, Lang::Dax, &t, &measures, 120.0, "[Region] = \"West\"");
                    if out.changed {
                        f.check = None;
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Validate").clicked() {
                            f.check = Some(if dax.trim().is_empty() {
                                Ok("No filter: every row is visible.".into())
                            } else {
                                let q = format!("COUNTROWS(FILTER({}, {}))", qt(&f.table), dax);
                                match app.built.model.evaluate(&q) {
                                    Ok(v) => {
                                        let total = app.built.model.table(&f.table).map(|x| x.row_count()).unwrap_or(0);
                                        Ok(format!("{} of {} rows of {} are visible to this role.", super::fmt_count(v.as_number().unwrap_or(0.0) as usize), super::fmt_count(total), f.table))
                                    }
                                    Err(e) => Err(e.to_string()),
                                }
                            });
                        }
                        if ui.button("Clear filter").clicked() {
                            dax.clear();
                            f.check = None;
                        }
                    });
                }
                match &f.check {
                    Some(Ok(s)) => status_line(ui, &t, true, s),
                    Some(Err(e)) => status_line(ui, &t, false, e),
                    None => {}
                }
                ui.add_space(8.0);
                ui.label(RichText::new("Members (emails, comma-separated; used when publishing)").size(11.5).color(t.muted));
                ui.add(egui::TextEdit::singleline(&mut f.members).desired_width(f32::INFINITY));
                ui.add_space(4.0);
                if ui.button("Delete this role").clicked() {
                    delete_role = true;
                }
            });
        });
        ui.add_space(8.0);
        buttons_row(ui, |ui| {
            if primary(ui, &t, "Save", true) {
                done = true;
            }
            if secondary(ui, "Cancel") {
                cancel = true;
            }
        });
    });
    if delete_role && f.role < f.roles.len() {
        f.roles.remove(f.role);
        f.role = 0;
        f.members = f.roles.first().map(|r| r.members.join(", ")).unwrap_or_default();
    }
    if done {
        if let Some(r) = f.roles.get_mut(f.role) {
            r.members = f.members.split(',').map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect();
        }
        let mut roles = f.roles.clone();
        for r in &mut roles {
            r.filters.retain(|(_, d)| !d.trim().is_empty());
        }
        for r in &roles {
            for (tn, d) in &r.filters {
                if let Err(e) = vizcraft_datamodel::dax::parse(d) {
                    app.error(format!("Role {}, table {tn}: {e}", r.name));
                    return true;
                }
            }
        }
        app.checkpoint("Manage roles");
        app.project.model.roles = roles;
        if app.rt.role.as_ref().is_some_and(|r| !app.project.model.roles.iter().any(|x| &x.name == r)) {
            app.rt.role = None;
        }
        app.mark_model_dirty();
        app.toast("Roles saved");
        return false;
    }
    open && !cancel
}

fn view_as_dialog(app: &mut VizApp, ctx: &egui::Context, role: &mut Option<String>) -> bool {
    let t = app.tokens();
    let mut open = true;
    let mut done = false;
    let mut cancel = false;
    let roles: Vec<String> = app.project.model.roles.iter().map(|r| r.name.clone()).collect();
    window("View as roles", 380.0).open(&mut open).show(ctx, |ui| {
        super::style_fields(ui, &t);
        ui.radio_value(role, None, "None (see all the data)");
        for r in &roles {
            ui.radio_value(role, Some(r.clone()), r);
        }
        if roles.is_empty() {
            info_line(ui, &t, "There are no roles yet: create them in Manage roles.");
            if ui.button("Manage roles…").clicked() {
                super::manage_roles(app);
            }
        }
        ui.add_space(8.0);
        buttons_row(ui, |ui| {
            if primary(ui, &t, "OK", true) {
                done = true;
            }
            if secondary(ui, "Cancel") {
                cancel = true;
            }
        });
    });
    if done {
        app.rt.role = role.clone();
        app.cache.clear();
        match role {
            Some(r) => app.toast(format!("Now viewing as {r}")),
            None => app.toast("Stopped viewing as a role"),
        }
        return false;
    }
    open && !cancel && app.model_ui.dialog.is_none()
}

// ------------------------------------------------------------------------------------- date table

pub struct DateTableForm {
    table: String,
    column: String,
    checked: String,
    checks: Vec<(bool, String)>,
}

impl DateTableForm {
    pub fn new(app: &VizApp, table: &str) -> DateTableForm {
        let mt = app.project.model.tables.iter().find(|t| t.name == table);
        let column = mt.and_then(|m| m.date_column.clone()).or_else(|| date_columns(app, table).into_iter().next()).unwrap_or_default();
        DateTableForm { table: table.to_string(), column, checked: String::new(), checks: vec![] }
    }
}

fn date_columns(app: &VizApp, table: &str) -> Vec<String> {
    tables_meta(&app.project, &app.built).into_iter().find(|m| m.name == table).map(|m| m.columns.into_iter().filter(|c| c.ty.is_temporal()).map(|c| c.name).collect()).unwrap_or_default()
}

fn check_date_column(built: &Built, table: &str, column: &str) -> Vec<(bool, String)> {
    let Some(t) = built.model.table(table) else { return vec![(false, "The table has no data.".into())] };
    let Some(c) = t.column_index(column).and_then(|i| t.column(i)) else { return vec![(false, "Pick a date column.".into())] };
    let mut out = Vec::new();
    let mut days: Vec<f64> = Vec::new();
    let mut blanks = 0;
    let mut bad = 0;
    for r in 0..t.row_count() {
        match c.value(r) {
            vizcraft_datamodel::Value::Blank => blanks += 1,
            vizcraft_datamodel::Value::Number(n) => days.push(n.floor()),
            vizcraft_datamodel::Value::Text(s) => match vizcraft_core::build::parse_date(s) {
                Some(n) => days.push(n.floor()),
                None => bad += 1,
            },
            _ => bad += 1,
        }
    }
    out.push(if bad == 0 { (true, "Every value is a date.".into()) } else { (false, format!("{bad} values aren't dates.")) });
    out.push(if blanks == 0 { (true, "No blank values.".into()) } else { (false, format!("{blanks} blank values.")) });
    let n = days.len();
    days.sort_by(f64::total_cmp);
    days.dedup();
    out.push(if days.len() == n { (true, "Every date appears once.".into()) } else { (false, format!("{} dates appear more than once: a date table has one row per day.", n - days.len())) });
    if let (Some(a), Some(b)) = (days.first(), days.last()) {
        let span = (b - a) as usize + 1;
        out.push(if span == days.len() {
            (true, format!("The dates are contiguous: {} days from {} to {}.", super::fmt_count(span), vizcraft_core::numfmt::format_date(*a, "yyyy-mm-dd"), vizcraft_core::numfmt::format_date(*b, "yyyy-mm-dd")))
        } else {
            (false, format!("{} days are missing between the first and last date.", super::fmt_count(span - days.len())))
        });
    }
    out
}

fn date_table_dialog(app: &mut VizApp, ctx: &egui::Context, f: &mut DateTableForm) -> bool {
    let t = app.tokens();
    let key = format!("{}|{}|{}", f.table, f.column, app.generation);
    if f.checked != key {
        f.checked = key;
        f.checks = check_date_column(&app.built, &f.table, &f.column);
    }
    let tables: Vec<String> = app.project.model.tables.iter().map(|t| t.name.clone()).collect();
    let marked = app.project.model.tables.iter().any(|x| x.name == f.table && x.is_date_table);
    let mut open = true;
    let mut done = false;
    let mut unmark = false;
    let mut cancel = false;
    window("Mark as date table", 520.0).open(&mut open).show(ctx, |ui| {
        super::style_fields(ui, &t);
        ui.label(RichText::new("A date table has one row per day with no gaps. Time intelligence (TOTALYTD, SAMEPERIODLASTYEAR, …) works along it.").color(t.muted));
        ui.add_space(6.0);
        egui::Grid::new("dt_grid").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
            ui.label(RichText::new("Table").color(t.muted));
            if table_combo(ui, "dt_table", &mut f.table, &tables, 220.0) {
                f.column = date_columns(app, &f.table).into_iter().next().unwrap_or_default();
            }
            ui.end_row();
            ui.label(RichText::new("Date column").color(t.muted));
            let cols = date_columns(app, &f.table);
            egui::ComboBox::from_id_salt("dt_col").selected_text(if f.column.is_empty() { "Choose…" } else { f.column.as_str() }).width(220.0).show_ui(ui, |ui| {
                for c in &cols {
                    ui.selectable_value(&mut f.column, c.clone(), c);
                }
            });
            ui.end_row();
        });
        if date_columns(app, &f.table).is_empty() {
            status_line(ui, &t, false, "This table has no date columns. Create one with New table ▸ Calendar.");
        }
        ui.add_space(6.0);
        for (ok, m) in &f.checks {
            status_line(ui, &t, *ok, m);
        }
        ui.add_space(8.0);
        buttons_row(ui, |ui| {
            let ok = !f.column.is_empty() && f.checks.iter().all(|(ok, _)| *ok);
            if primary(ui, &t, "OK", ok) {
                done = true;
            }
            if secondary(ui, "Cancel") {
                cancel = true;
            }
            if marked && ui.button("Unmark").clicked() {
                unmark = true;
            }
        });
    });
    if done || unmark {
        app.checkpoint(if done { "Mark as date table" } else { "Unmark date table" });
        for x in app.project.model.tables.iter_mut() {
            if x.name == f.table {
                x.is_date_table = done;
                x.date_column = if done { Some(f.column.clone()) } else { None };
            } else if done {
                // One date table per model.
                x.is_date_table = false;
                x.date_column = None;
            }
        }
        app.mark_model_dirty();
        app.toast(if done { format!("{} is the date table ({})", f.table, f.column) } else { format!("{} is no longer a date table", f.table) });
        return false;
    }
    open && !cancel
}
