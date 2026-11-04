//! Table view, Model view (diagram), DAX query view, TMDL view, and the modeling dialogs
//! (relationships, measures, calculated columns/tables, parameters, roles).

mod dax_view;
mod diagram;
mod dialogs;
pub mod editor;
mod grid;
mod table_view;
mod tmdl_view;

use std::collections::BTreeSet;

use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};
use vizardcraft_core::build::{Built, ColType};
use vizardcraft_core::doc::*;
use vizardcraft_core::tmdl::TmdlObject;

use crate::app::{View, VizApp};
use crate::theme::Tokens;

/// What is selected in the Table / Model views and the Data pane.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Sel {
    #[default]
    None,
    Table(String),
    Column(String, String),
    Measure(String),
    Relationship(u64),
}

#[derive(Default)]
pub struct ModelUi {
    pub sel: Sel,
    /// Data pane search.
    pub search: String,
    /// Data pane: tables shown collapsed (by name); `expanded_all` flips the default.
    pub collapsed: BTreeSet<String>,
    pub table: table_view::TableViewState,
    pub diagram: diagram::DiagramState,
    pub dax: dax_view::DaxState,
    pub tmdl: tmdl_view::TmdlState,
    pub dialog: Option<dialogs::Dialog>,
    /// The Edit relationship dialog (opened from Manage relationships or the diagram).
    pub rel_edit: Option<dialogs::RelForm>,
    hook_done: bool,
}

// ------------------------------------------------------------------------------------- model info

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableKind {
    Fact,
    Dimension,
    Calculated,
    Parameter,
    Date,
    Other,
}

impl TableKind {
    pub fn color(self, t: &Tokens) -> Color32 {
        match self {
            TableKind::Fact => t.accent,
            TableKind::Dimension => Color32::from_rgb(0x0E, 0x9F, 0x8E),
            TableKind::Calculated => Color32::from_rgb(0x8B, 0x5C, 0xF6),
            TableKind::Parameter => Color32::from_rgb(0xD9, 0x8A, 0x0B),
            TableKind::Date => Color32::from_rgb(0xE1, 0x4D, 0x76),
            TableKind::Other => Color32::from_rgb(0x64, 0x74, 0x8B),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            TableKind::Fact => "Fact table",
            TableKind::Dimension => "Dimension",
            TableKind::Calculated => "Calculated table",
            TableKind::Parameter => "Parameter",
            TableKind::Date => "Date table",
            TableKind::Other => "Table",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ColMeta {
    pub name: String,
    pub ty: ColType,
    pub hidden: bool,
    pub calc: bool,
    pub summarize: bool,
}

#[derive(Clone, Debug)]
pub struct TableMeta {
    pub name: String,
    pub rows: usize,
    pub kind: TableKind,
    pub hidden: bool,
    pub columns: Vec<ColMeta>,
    pub measures: Vec<(String, bool)>,
    pub hierarchies: Vec<Hierarchy>,
}

fn is_date_level(name: &str) -> bool {
    vizardcraft_core::build::DATE_LEVELS.iter().chain(["MonthNo"].iter()).any(|l| name.ends_with(&format!(".{l}")))
}

/// A column's type: declared, loaded, or from the model's data (calculated columns).
pub fn column_type(p: &Project, built: &Built, table: &str, column: &str) -> ColType {
    if let Some(cp) = p.model.tables.iter().find(|t| t.name.eq_ignore_ascii_case(table)).and_then(|t| t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(column)))
        && !cp.data_type.is_empty()
    {
        let ty = ColType::from_label(&cp.data_type);
        if ty != ColType::Any {
            return ty;
        }
    }
    let ty = built.col_type(table, column);
    if ty != ColType::Any {
        return ty;
    }
    let col = built.model.table(table).and_then(|t| t.column_index(column).and_then(|i| t.column(i)));
    match col.map(|c| c.data_type()) {
        Some(vizardcraft_datamodel::store::DataType::Number) => {
            let int = col.is_some_and(|c| (0..c.cardinality().min(500) as u32).all(|id| c.dict_value(id).as_number().is_none_or(|x| x.fract() == 0.0)));
            if int { ColType::Integer } else { ColType::Number }
        }
        Some(vizardcraft_datamodel::store::DataType::Boolean) => ColType::Bool,
        _ => ColType::Text,
    }
}

pub fn column_props<'a>(p: &'a Project, table: &str, column: &str) -> Option<&'a ColumnProps> {
    p.model.tables.iter().find(|t| t.name.eq_ignore_ascii_case(table)).and_then(|t| t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(column)))
}

/// Every model table with its columns, measures and kind.
pub fn tables_meta(p: &Project, built: &Built) -> Vec<TableMeta> {
    let mut out = Vec::new();
    for t in &p.model.tables {
        let mut cols: Vec<String> = match built.table(&t.name) {
            Some(d) => d.columns.clone(),
            None => built.model.table(&t.name).map(|mt| mt.columns.iter().filter(|c| c.expression.is_none() && !is_date_level(&c.name)).map(|c| c.name.clone()).collect()).unwrap_or_default(),
        };
        for c in p.model.calculated_columns.iter().filter(|c| c.table.eq_ignore_ascii_case(&t.name)) {
            if !cols.iter().any(|x| x.eq_ignore_ascii_case(&c.name)) {
                cols.push(c.name.clone());
            }
        }
        let columns = cols
            .into_iter()
            .map(|c| {
                let cp = column_props(p, &t.name, &c);
                let ty = column_type(p, built, &t.name, &c);
                let calc = p.model.calculated_columns.iter().any(|cc| cc.table.eq_ignore_ascii_case(&t.name) && cc.name.eq_ignore_ascii_case(&c));
                let summarize = match cp.map(|x| x.summarize_by.as_str()) {
                    Some(s) if !s.is_empty() => s != "None",
                    _ => ty.is_numeric() && !c.to_ascii_lowercase().ends_with("id"),
                };
                ColMeta { hidden: cp.is_some_and(|x| x.hidden), name: c, ty, calc, summarize }
            })
            .collect();
        let kind = match &t.source {
            TableSource::Calculated(_) => TableKind::Calculated,
            TableSource::Parameter(_) => TableKind::Parameter,
            _ if t.is_date_table => TableKind::Date,
            _ if p.model.relationships.iter().any(|r| (r.from_table.eq_ignore_ascii_case(&t.name) && r.cardinality == Cardinality::ManyToOne) || (r.to_table.eq_ignore_ascii_case(&t.name) && r.cardinality == Cardinality::OneToMany)) => {
                TableKind::Fact
            }
            _ if p.model.relationships.iter().any(|r| r.to_table.eq_ignore_ascii_case(&t.name) || r.from_table.eq_ignore_ascii_case(&t.name)) => TableKind::Dimension,
            _ => TableKind::Other,
        };
        let mut measures: Vec<(String, bool)> = p.model.measures.iter().filter(|m| m.table.eq_ignore_ascii_case(&t.name)).map(|m| (m.name.clone(), m.hidden)).collect();
        // Parameter measures the build adds.
        if let TableSource::Parameter(n) = &t.source
            && matches!(p.parameter(n).map(|x| &x.kind), Some(ParameterKind::WhatIf { .. }))
        {
            measures.push((format!("{n} Value"), false));
        }
        out.push(TableMeta {
            name: t.name.clone(),
            rows: built.model.table(&t.name).map(|x| x.row_count()).unwrap_or(0),
            kind,
            hidden: t.hidden,
            columns,
            measures,
            hierarchies: t.hierarchies.clone(),
        });
    }
    out
}

/// The table new things go in by default: the selection's, else the first fact table.
pub fn current_table(app: &VizApp) -> Option<String> {
    let from_sel = match &app.model_ui.sel {
        Sel::Table(t) | Sel::Column(t, _) => Some(t.clone()),
        Sel::Measure(m) => app.project.model.measures.iter().find(|x| &x.name == m).map(|x| x.table.clone()),
        _ => None,
    };
    from_sel.filter(|t| app.project.model.tables.iter().any(|x| &x.name == t)).or_else(|| {
        let metas = tables_meta(&app.project, &app.built);
        metas.iter().find(|m| m.kind == TableKind::Fact).or(metas.first()).map(|m| m.name.clone())
    })
}

pub fn qt(t: &str) -> String {
    vizardcraft_datamodel::model::quote_table(t)
}

pub fn col_ref(t: &str, c: &str) -> String {
    format!("{}[{}]", qt(t), c.replace(']', "]]"))
}

/// A small type glyph for a column.
pub fn glyph(ty: ColType, calc: bool, summarize: bool) -> &'static str {
    if calc {
        return "ƒx";
    }
    match ty {
        ColType::Text | ColType::Any => "abc",
        ColType::Integer if summarize => "∑",
        ColType::Number if summarize => "∑",
        ColType::Integer => "123",
        ColType::Number => "1.2",
        ColType::Date | ColType::DateTime => "dt",
        ColType::Time => "tm",
        ColType::Bool => "t/f",
    }
}

/// Paints a column type icon in `r`.
pub fn paint_type_icon(p: &egui::Painter, r: Rect, ty: ColType, calc: bool, summarize: bool, color: Color32) {
    if !calc && ty.is_temporal() {
        // A little calendar.
        let c = Rect::from_center_size(r.center(), vec2(11.0, 10.0));
        p.rect_stroke(c, 1.5, egui::Stroke::new(1.2, color), egui::StrokeKind::Inside);
        p.rect_filled(Rect::from_min_size(c.min, vec2(c.width(), 3.0)), 1.0, color);
        for i in 0..3 {
            p.circle_filled(pos2(c.left() + 2.8 + i as f32 * 2.7, c.bottom() - 2.6), 0.8, color);
        }
        return;
    }
    let g = glyph(ty, calc, summarize);
    let size = if g.chars().count() >= 3 { 8.5 } else { 11.0 };
    p.text(r.center(), Align2::CENTER_CENTER, g, FontId::monospace(size), color);
}

// ------------------------------------------------------------------------------------- actions

/// Things the panes ask for, carried out after drawing.
#[derive(Clone, Debug)]
pub enum Act {
    Select(Sel),
    Insert(String),
    NewMeasure(String),
    NewColumn(String),
    EditMeasure(String),
    EditColumn(String, String),
    MarkDate(String),
    ToggleHidden(Sel),
    Delete(Sel),
    ScriptTmdl(TmdlObject),
    TopRows(String),
    DefineEvaluate(String),
    ColumnValues(String, String),
    ShowData(String),
    EditRelationship(u64),
}

pub fn run_act(app: &mut VizApp, act: Act) {
    match act {
        Act::Select(s) => app.model_ui.sel = s,
        Act::Insert(_) => {}
        Act::NewMeasure(t) => new_measure(app, Some(&t)),
        Act::NewColumn(t) => new_column(app, Some(&t)),
        Act::EditMeasure(m) => dialogs::edit_measure(app, &m),
        Act::EditColumn(t, c) => dialogs::edit_column(app, &t, &c),
        Act::MarkDate(t) => {
            app.model_ui.sel = Sel::Table(t);
            mark_date_table(app);
        }
        Act::ToggleHidden(s) => toggle_hidden(app, &s),
        Act::Delete(s) => delete(app, &s),
        Act::ScriptTmdl(obj) => {
            let text = vizardcraft_core::tmdl::write_object(&app.project, &app.built, &obj);
            let name = match &obj {
                TmdlObject::Table(n) | TmdlObject::Measure(n) | TmdlObject::Role(n) => n.clone(),
                TmdlObject::Relationship(id) => format!("Relationship {id}"),
                TmdlObject::Expressions => "Expressions".into(),
            };
            app.model_ui.tmdl.open_tab(&name, text);
            app.view = View::Tmdl;
        }
        Act::TopRows(t) => {
            app.model_ui.dax.open_query(&format!("Top 100 {t}"), format!("EVALUATE\n\tTOPN(100, {})", qt(&t)));
            app.view = View::Dax;
            app.model_ui.dax.run_pending = true;
        }
        Act::DefineEvaluate(m) => {
            let Some(ms) = app.project.model.measures.iter().find(|x| x.name == m).cloned() else { return };
            let body = ms.expression.lines().collect::<Vec<_>>().join("\n\t\t");
            let q = format!(
                "DEFINE\n\tMEASURE {}[{}] =\n\t\t{}\n\nEVALUATE\n\tSUMMARIZECOLUMNS(\n\t\t\"{}\", [{}]\n\t)",
                qt(&ms.table),
                ms.name,
                body,
                ms.name.replace('"', "\"\""),
                ms.name
            );
            app.model_ui.dax.open_query(&ms.name, q);
            app.view = View::Dax;
            app.model_ui.dax.run_pending = true;
        }
        Act::ColumnValues(t, c) => {
            app.model_ui.dax.open_query(&format!("{c} values"), format!("EVALUATE\n\tTOPN(100, DISTINCT({}))\nORDER BY {} ASC", col_ref(&t, &c), col_ref(&t, &c)));
            app.view = View::Dax;
            app.model_ui.dax.run_pending = true;
        }
        Act::ShowData(t) => {
            app.model_ui.sel = Sel::Table(t);
            app.view = View::Table;
        }
        Act::EditRelationship(id) => dialogs::edit_relationship(app, id),
    }
}

fn toggle_hidden(app: &mut VizApp, s: &Sel) {
    app.checkpoint("Hide in report view");
    match s {
        Sel::Table(t) => {
            if let Some(mt) = app.project.model.tables.iter_mut().find(|x| &x.name == t) {
                mt.hidden = !mt.hidden;
            }
        }
        Sel::Column(t, c) => {
            if let Some(mt) = app.project.model.tables.iter_mut().find(|x| &x.name == t) {
                match mt.columns.iter_mut().find(|x| &x.name == c) {
                    Some(cp) => cp.hidden = !cp.hidden,
                    None => mt.columns.push(ColumnProps { name: c.clone(), hidden: true, ..ColumnProps::default() }),
                }
            }
        }
        Sel::Measure(m) => {
            if let Some(ms) = app.project.model.measures.iter_mut().find(|x| &x.name == m) {
                ms.hidden = !ms.hidden;
            }
        }
        _ => {}
    }
    app.mark_model_dirty();
}

/// Deletes a measure, calculated column, calculated table or relationship.
pub fn delete(app: &mut VizApp, s: &Sel) {
    match s {
        Sel::Measure(m) => {
            app.checkpoint(&format!("Delete measure {m}"));
            app.project.model.measures.retain(|x| &x.name != m);
            app.toast(format!("Deleted measure [{m}]"));
        }
        Sel::Column(t, c) => {
            if !app.project.model.calculated_columns.iter().any(|x| &x.table == t && &x.name == c) {
                app.error("Only calculated columns can be deleted here. Remove data columns in Power Query.");
                return;
            }
            app.checkpoint(&format!("Delete column {c}"));
            app.project.model.calculated_columns.retain(|x| !(&x.table == t && &x.name == c));
            app.toast(format!("Deleted column {t}[{c}]"));
        }
        Sel::Table(t) => {
            let calc = app.project.model.tables.iter().any(|x| &x.name == t && matches!(x.source, TableSource::Calculated(_)));
            if !calc {
                app.error("Only calculated tables can be deleted from the model. To drop a query's table, turn off its load in Power Query.");
                return;
            }
            app.checkpoint(&format!("Delete table {t}"));
            app.project.model.tables.retain(|x| &x.name != t);
            app.project.model.measures.retain(|x| &x.table != t);
            app.project.model.calculated_columns.retain(|x| &x.table != t);
            app.project.model.relationships.retain(|r| &r.from_table != t && &r.to_table != t);
            app.toast(format!("Deleted table {t}"));
        }
        Sel::Relationship(id) => {
            app.checkpoint("Delete relationship");
            app.project.model.relationships.retain(|r| r.id != *id);
            app.toast("Deleted relationship");
        }
        Sel::None => return,
    }
    app.model_ui.sel = Sel::None;
    app.mark_model_dirty();
}

// ------------------------------------------------------------------------------------- data pane

/// What clicking an item does in the pane's view.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PaneMode {
    Table,
    Model,
    Dax,
    Tmdl,
}

fn pane_row(ui: &mut Ui, t: &Tokens, indent: f32, selected: bool, dim: bool, label: &str, right: &str, icon: &dyn Fn(&egui::Painter, Rect, Color32)) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click_and_drag());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 4.0, t.accent_soft);
    } else if resp.hovered() {
        p.rect_filled(rect, 4.0, t.panel_alt);
    }
    let ir = Rect::from_center_size(pos2(rect.left() + indent + 9.0, rect.center().y), vec2(16.0, 16.0));
    icon(p, ir, if dim { t.muted } else { t.accent });
    let rw = if right.is_empty() { 0.0 } else { right.len() as f32 * 6.5 + 10.0 };
    let shown = vizardcraft_core::scene::ellipsize(label, 12.5, w - indent - 30.0 - rw);
    p.text(pos2(ir.right() + 6.0, rect.center().y), Align2::LEFT_CENTER, shown, FontId::proportional(12.5), if dim { t.muted } else { t.text });
    if !right.is_empty() {
        p.text(pos2(rect.right() - 6.0, rect.center().y), Align2::RIGHT_CENTER, right, FontId::proportional(10.5), t.muted);
    }
    resp
}

pub fn fmt_count(n: usize) -> String {
    vizardcraft_core::numfmt::format(n as f64, "#,0")
}

/// The Data pane: tables with their measures, columns and hierarchies; search; context menus.
pub fn data_pane(app: &mut VizApp, ui: &mut Ui, mode: PaneMode) -> Vec<Act> {
    let t = app.tokens();
    let mut acts = Vec::new();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Data").strong().size(14.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_btn(ui, &t, "minus", "Collapse all", Some(t.muted)).clicked() {
                for m in &app.project.model.tables {
                    app.model_ui.collapsed.insert(m.name.clone());
                }
            }
            if icon_btn(ui, &t, "plus", "Expand all", Some(t.muted)).clicked() {
                app.model_ui.collapsed.clear();
            }
        });
    });
    ui.add(egui::TextEdit::singleline(&mut app.model_ui.search).hint_text("Search").desired_width(f32::INFINITY));
    ui.add_space(4.0);
    let metas = tables_meta(&app.project, &app.built);
    let q = app.model_ui.search.to_lowercase();
    let sel = app.model_ui.sel.clone();
    egui::ScrollArea::vertical().id_salt(("datapane", mode as u8)).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        for m in &metas {
            let name_hit = q.is_empty() || m.name.to_lowercase().contains(&q);
            let cols: Vec<&ColMeta> = m.columns.iter().filter(|c| name_hit || c.name.to_lowercase().contains(&q)).collect();
            let meas: Vec<&(String, bool)> = m.measures.iter().filter(|(n, _)| name_hit || n.to_lowercase().contains(&q)).collect();
            if !name_hit && cols.is_empty() && meas.is_empty() {
                continue;
            }
            let open = !app.model_ui.collapsed.contains(&m.name) || !q.is_empty();
            let kc = m.kind.color(&t);
            let resp = pane_row(ui, &t, 0.0, sel == Sel::Table(m.name.clone()), m.hidden, &m.name, &fmt_count(m.rows), &|p: &egui::Painter, r: Rect, _c: Color32| {
                tri(p, pos2(r.left() + 1.0, r.center().y), if open { Dir::Down } else { Dir::Right }, 3.5, Color32::GRAY);
                let tr = Rect::from_center_size(r.center() + vec2(7.0, 0.0), vec2(12.0, 11.0));
                p.rect_filled(tr, 2.0, kc.gamma_multiply(0.2));
                p.rect_stroke(tr, 2.0, egui::Stroke::new(1.0, kc), egui::StrokeKind::Inside);
                p.line_segment([pos2(tr.left(), tr.top() + 3.5), pos2(tr.right(), tr.top() + 3.5)], egui::Stroke::new(1.0, kc));
            });
            let resp = resp.on_hover_text(format!("{} · {} rows · {} columns · {} measures", m.kind.label(), fmt_count(m.rows), m.columns.len(), m.measures.len()));
            if resp.clicked() {
                if resp.interact_pointer_pos().is_some_and(|p| p.x < resp.rect.left() + 14.0) || sel == Sel::Table(m.name.clone()) {
                    if open {
                        app.model_ui.collapsed.insert(m.name.clone());
                    } else {
                        app.model_ui.collapsed.remove(&m.name);
                    }
                }
                acts.push(Act::Select(Sel::Table(m.name.clone())));
            }
            if resp.double_clicked() {
                acts.push(match mode {
                    PaneMode::Dax => Act::Insert(qt(&m.name)),
                    PaneMode::Tmdl => Act::ScriptTmdl(TmdlObject::Table(m.name.clone())),
                    _ => Act::ShowData(m.name.clone()),
                });
            }
            let tname = m.name.clone();
            let kind = m.kind;
            let hidden = m.hidden;
            resp.context_menu(|ui| {
                if ui.button("New measure").clicked() {
                    acts.push(Act::NewMeasure(tname.clone()));
                    ui.close();
                }
                if ui.button("New column").clicked() {
                    acts.push(Act::NewColumn(tname.clone()));
                    ui.close();
                }
                ui.separator();
                if ui.button("Show top 100 rows (DAX query)").clicked() {
                    acts.push(Act::TopRows(tname.clone()));
                    ui.close();
                }
                if ui.button("Script TMDL to a new tab").clicked() {
                    acts.push(Act::ScriptTmdl(TmdlObject::Table(tname.clone())));
                    ui.close();
                }
                if ui.button("View data").clicked() {
                    acts.push(Act::ShowData(tname.clone()));
                    ui.close();
                }
                ui.separator();
                if ui.button("Mark as date table…").clicked() {
                    acts.push(Act::MarkDate(tname.clone()));
                    ui.close();
                }
                if ui.button(if hidden { "Unhide in report view" } else { "Hide in report view" }).clicked() {
                    acts.push(Act::ToggleHidden(Sel::Table(tname.clone())));
                    ui.close();
                }
                if kind == TableKind::Calculated && ui.button("Delete table").clicked() {
                    acts.push(Act::Delete(Sel::Table(tname.clone())));
                    ui.close();
                }
            });
            if !open {
                continue;
            }
            for (mn, mh) in meas {
                let s = Sel::Measure(mn.clone());
                let r = pane_row(ui, &t, 16.0, sel == s, *mh, mn, "", &|p: &egui::Painter, r: Rect, c: Color32| crate::icons::paint(p, r.shrink(1.0), "measure", c));
                let r = r.on_hover_text(app.project.model.measures.iter().find(|x| &x.name == mn).map(|x| x.expression.clone()).unwrap_or_default());
                if r.clicked() {
                    acts.push(Act::Select(s.clone()));
                }
                let user = app.project.model.measures.iter().any(|x| &x.name == mn);
                if r.double_clicked() {
                    acts.push(match mode {
                        PaneMode::Dax => Act::Insert(format!("[{mn}]")),
                        PaneMode::Tmdl if user => Act::ScriptTmdl(TmdlObject::Measure(mn.clone())),
                        _ if user => Act::EditMeasure(mn.clone()),
                        _ => Act::Select(s.clone()),
                    });
                }
                let mn2 = mn.clone();
                r.context_menu(|ui| {
                    if user && ui.button("Edit measure…").clicked() {
                        acts.push(Act::EditMeasure(mn2.clone()));
                        ui.close();
                    }
                    if user && ui.button("Define and evaluate (DAX query)").clicked() {
                        acts.push(Act::DefineEvaluate(mn2.clone()));
                        ui.close();
                    }
                    if user && ui.button("Script TMDL to a new tab").clicked() {
                        acts.push(Act::ScriptTmdl(TmdlObject::Measure(mn2.clone())));
                        ui.close();
                    }
                    if user && ui.button(if *mh { "Unhide in report view" } else { "Hide in report view" }).clicked() {
                        acts.push(Act::ToggleHidden(Sel::Measure(mn2.clone())));
                        ui.close();
                    }
                    if user {
                        ui.separator();
                        if ui.button("Delete measure").clicked() {
                            acts.push(Act::Delete(Sel::Measure(mn2.clone())));
                            ui.close();
                        }
                    } else {
                        ui.label(egui::RichText::new("Created by a parameter").color(Color32::GRAY));
                    }
                });
            }
            for c in cols {
                let s = Sel::Column(m.name.clone(), c.name.clone());
                let (ty, calc, sum) = (c.ty, c.calc, c.summarize);
                let r = pane_row(ui, &t, 16.0, sel == s, c.hidden, &c.name, "", &|p: &egui::Painter, r: Rect, col: Color32| paint_type_icon(p, r, ty, calc, sum, col));
                if r.clicked() {
                    acts.push(Act::Select(s.clone()));
                }
                if r.double_clicked() {
                    acts.push(match mode {
                        PaneMode::Dax => Act::Insert(col_ref(&m.name, &c.name)),
                        PaneMode::Tmdl => Act::ScriptTmdl(TmdlObject::Table(m.name.clone())),
                        _ if calc => Act::EditColumn(m.name.clone(), c.name.clone()),
                        _ => Act::Select(s.clone()),
                    });
                }
                let (tn, cn, hid) = (m.name.clone(), c.name.clone(), c.hidden);
                r.context_menu(|ui| {
                    if calc && ui.button("Edit column formula…").clicked() {
                        acts.push(Act::EditColumn(tn.clone(), cn.clone()));
                        ui.close();
                    }
                    if ui.button("Show values (DAX query)").clicked() {
                        acts.push(Act::ColumnValues(tn.clone(), cn.clone()));
                        ui.close();
                    }
                    if ui.button(if hid { "Unhide in report view" } else { "Hide in report view" }).clicked() {
                        acts.push(Act::ToggleHidden(Sel::Column(tn.clone(), cn.clone())));
                        ui.close();
                    }
                    if calc {
                        ui.separator();
                        if ui.button("Delete column").clicked() {
                            acts.push(Act::Delete(Sel::Column(tn.clone(), cn.clone())));
                            ui.close();
                        }
                    }
                });
            }
            for h in &m.hierarchies {
                if !q.is_empty() && !name_hit && !h.name.to_lowercase().contains(&q) {
                    continue;
                }
                pane_row(ui, &t, 16.0, false, false, &h.name, &format!("{} levels", h.levels.len()), &|p: &egui::Painter, r: Rect, c: Color32| {
                    for i in 0..3 {
                        let y = r.top() + 3.0 + i as f32 * 4.5;
                        let x = r.left() + 2.0 + i as f32 * 3.0;
                        p.rect_filled(Rect::from_min_size(pos2(x, y), vec2(6.0, 3.0)), 1.0, c);
                    }
                })
                .on_hover_text(h.levels.join(" › "));
            }
            ui.add_space(3.0);
        }
        if metas.is_empty() {
            ui.add_space(20.0);
            ui.label(egui::RichText::new("No tables yet. Get data with Power Query or a data sheet, then refresh.").color(t.muted));
        }
    });
    acts
}

// ------------------------------------------------------------------------------------- views

pub fn table_view(app: &mut VizApp, ui: &mut egui::Ui) {
    table_view::show(app, ui);
}

pub fn model_view(app: &mut VizApp, ui: &mut egui::Ui) {
    diagram::show(app, ui);
}

pub fn dax_view(app: &mut VizApp, ui: &mut egui::Ui) {
    dax_view::show(app, ui);
}

pub fn tmdl_view(app: &mut VizApp, ui: &mut egui::Ui) {
    tmdl_view::show(app, ui);
}

/// The modeling dialogs. `VIZARDCRAFT_OPEN=<dialog>` opens one at start (demos, screenshots):
/// measure, quick, column, table, calc, relationships, edit-relationship, whatif, fields, value,
/// roles, viewas, datetable.
pub fn dialogs(app: &mut VizApp, ctx: &egui::Context) {
    if !app.model_ui.hook_done && !app.built.model.tables.is_empty() {
        app.model_ui.hook_done = true;
        if let Ok(which) = std::env::var("VIZARDCRAFT_OPEN") {
            match which.as_str() {
                "measure" => new_measure(app, None),
                "edit-measure" => dialogs::edit_measure(app, "Margin %"),
                "quick" => quick_measure(app),
                "column" => new_column(app, None),
                "table" => new_table(app),
                "calc" => {
                    calculated_field(app);
                    if let Some(dialogs::Dialog::Calc(f)) = &mut app.model_ui.dialog {
                        f.set_formula("IF AVG({FIXED [Region] : SUM([Quantity])}) > 300 THEN 'Strong' ELSE 'Weak' END");
                    }
                }
                "relationships" => manage_relationships(app),
                "edit-relationship" => {
                    if let Some(id) = app.project.model.relationships.first().map(|r| r.id) {
                        dialogs::edit_relationship(app, id);
                    }
                }
                "whatif" | "fields" | "value" => new_parameter(app, &which),
                "roles" => manage_roles(app),
                "viewas" => view_as(app),
                "datetable" => mark_date_table(app),
                "sel-measure" => app.model_ui.sel = Sel::Measure("Margin %".into()),
                "tmdl-apply" => tmdl_view::demo_apply(app),
                "sel-column" => app.model_ui.sel = Sel::Column("Sales".into(), "Unit Price".into()),
                "sel-relationship" => app.model_ui.sel = app.project.model.relationships.first().map(|r| Sel::Relationship(r.id)).unwrap_or_default(),
                _ => {}
            }
        }
    }
    dialogs::show(app, ctx);
}

/// New measure (in `table`, or the first table).
pub fn new_measure(app: &mut VizApp, table: Option<&str>) {
    let table = table.map(str::to_string).or_else(|| current_table(app));
    match table {
        Some(t) => app.model_ui.dialog = Some(dialogs::Dialog::Formula(dialogs::FormulaForm::new(dialogs::FormulaKind::Measure, &t, &app.project))),
        None => app.error("Load a table first: a measure lives in a table."),
    }
}
pub fn quick_measure(app: &mut VizApp) {
    if app.project.model.tables.is_empty() {
        app.error("Load a table first.");
        return;
    }
    app.model_ui.dialog = Some(dialogs::Dialog::Quick(dialogs::QuickForm::new(app)));
}
pub fn new_column(app: &mut VizApp, table: Option<&str>) {
    let table = table.map(str::to_string).or_else(|| current_table(app));
    match table {
        Some(t) => app.model_ui.dialog = Some(dialogs::Dialog::Formula(dialogs::FormulaForm::new(dialogs::FormulaKind::Column, &t, &app.project))),
        None => app.error("Load a table first."),
    }
}
pub fn new_table(app: &mut VizApp) {
    let t = current_table(app).unwrap_or_default();
    app.model_ui.dialog = Some(dialogs::Dialog::Formula(dialogs::FormulaForm::new(dialogs::FormulaKind::Table, &t, &app.project)));
}
/// A Tableau-style calculated field (Tableau syntax, translated to DAX).
pub fn calculated_field(app: &mut VizApp) {
    let t = current_table(app).unwrap_or_default();
    app.model_ui.dialog = Some(dialogs::Dialog::Calc(dialogs::CalcForm::new(&t, &app.project)));
}
pub fn manage_relationships(app: &mut VizApp) {
    app.model_ui.dialog = Some(dialogs::Dialog::Relationships(dialogs::RelListForm::default()));
}
/// New parameter: "whatif", "fields" or "value".
pub fn new_parameter(app: &mut VizApp, kind: &str) {
    app.model_ui.dialog = Some(dialogs::Dialog::Parameter(dialogs::ParamForm::new(kind, &app.project)));
}
pub fn manage_roles(app: &mut VizApp) {
    app.model_ui.dialog = Some(dialogs::Dialog::Roles(dialogs::RolesForm::new(&app.project)));
}
pub fn view_as(app: &mut VizApp) {
    app.model_ui.dialog = Some(dialogs::Dialog::ViewAs(app.rt.role.clone()));
}
pub fn mark_date_table(app: &mut VizApp) {
    let t = current_table(app).unwrap_or_default();
    app.model_ui.dialog = Some(dialogs::Dialog::DateTable(dialogs::DateTableForm::new(app, &t)));
}

// ------------------------------------------------------------------------------------- painted glyphs

/// Directions for [`tri`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Right,
}

/// A small filled triangle (chevrons, sort marks, arrows).
pub fn tri(p: &egui::Painter, c: egui::Pos2, dir: Dir, s: f32, color: Color32) {
    let pts = match dir {
        Dir::Down => vec![pos2(c.x - s, c.y - s * 0.55), pos2(c.x + s, c.y - s * 0.55), pos2(c.x, c.y + s * 0.6)],
        Dir::Up => vec![pos2(c.x - s, c.y + s * 0.55), pos2(c.x + s, c.y + s * 0.55), pos2(c.x, c.y - s * 0.6)],
        Dir::Right => vec![pos2(c.x - s * 0.55, c.y - s), pos2(c.x - s * 0.55, c.y + s), pos2(c.x + s * 0.6, c.y)],
    };
    p.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
}

/// A painted icon: "x", "check", "plus", "minus", "warn", "play", "fit", "grid", "info".
pub fn glyph_icon(p: &egui::Painter, r: Rect, name: &str, color: Color32) {
    let c = r.center();
    let s = r.width().min(r.height()) * 0.32;
    let st = egui::Stroke::new(1.6, color);
    match name {
        "x" => {
            p.line_segment([c + vec2(-s, -s), c + vec2(s, s)], st);
            p.line_segment([c + vec2(-s, s), c + vec2(s, -s)], st);
        }
        "check" => {
            p.line_segment([c + vec2(-s, 0.0), c + vec2(-s * 0.3, s * 0.75)], st);
            p.line_segment([c + vec2(-s * 0.3, s * 0.75), c + vec2(s, -s * 0.8)], st);
        }
        "plus" => {
            p.line_segment([c + vec2(-s, 0.0), c + vec2(s, 0.0)], st);
            p.line_segment([c + vec2(0.0, -s), c + vec2(0.0, s)], st);
        }
        "minus" => {
            p.line_segment([c + vec2(-s, 0.0), c + vec2(s, 0.0)], st);
        }
        "warn" => {
            let pts = vec![c + vec2(0.0, -s * 1.3), c + vec2(s * 1.3, s * 1.0), c + vec2(-s * 1.3, s * 1.0)];
            p.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
            p.text(c + vec2(0.0, 0.5), Align2::CENTER_CENTER, "!", FontId::proportional(r.height() * 0.55), Color32::WHITE);
        }
        "info" => {
            p.circle_filled(c, s * 1.3, color);
            p.text(c, Align2::CENTER_CENTER, "i", FontId::proportional(r.height() * 0.55), Color32::WHITE);
        }
        "play" => tri(p, c, Dir::Right, s * 1.1, color),
        "fit" => {
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let corner = c + vec2(dx * s, dy * s);
                p.line_segment([corner, corner - vec2(dx * s * 0.6, 0.0)], st);
                p.line_segment([corner, corner - vec2(0.0, dy * s * 0.6)], st);
            }
        }
        "grid" => {
            for i in 0..2 {
                for j in 0..2 {
                    p.rect_stroke(Rect::from_min_size(c + vec2(-s + i as f32 * s * 1.1, -s + j as f32 * s * 1.1), vec2(s * 0.9, s * 0.9)), 1.0, st, egui::StrokeKind::Inside);
                }
            }
        }
        _ => {}
    }
}

/// A square button with a painted icon and a tooltip.
pub fn icon_btn(ui: &mut Ui, t: &Tokens, name: &str, tip: &str, color: Option<Color32>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.accent_soft);
    }
    glyph_icon(ui.painter(), rect, name, color.unwrap_or(t.text));
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// A ribbon-style group for the views' tool strips: content, caption below, separator after.
pub fn strip_group(ui: &mut Ui, t: &Tokens, caption: &str, add: impl FnOnce(&mut Ui)) {
    let r = ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.set_min_height(62.0);
            add(ui);
        });
        ui.label(egui::RichText::new(caption).size(10.0).color(t.muted));
    });
    let rr = r.response.rect;
    ui.add_space(6.0);
    let x = rr.right() + 3.0;
    ui.painter().line_segment([pos2(x, rr.top() + 2.0), pos2(x, rr.bottom() - 2.0)], egui::Stroke::new(1.0, t.border));
    ui.add_space(6.0);
}

/// Text fields with a visible border and a slightly tinted fill (the panels are white too).
pub fn style_fields(ui: &mut Ui, t: &Tokens) {
    let v = ui.visuals_mut();
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, t.border);
    v.extreme_bg_color = if t.dark { Color32::from_rgb(0x16, 0x19, 0x20) } else { Color32::from_rgb(0xFB, 0xFC, 0xFD) };
}
