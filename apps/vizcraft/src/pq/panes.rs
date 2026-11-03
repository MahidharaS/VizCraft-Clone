//! The editor's panes: Queries (left), Query Settings and Applied Steps (right), the formula bar
//! and the status bar.

use egui::{Align2, FontId, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use vizcraft_query::steps::{self, LoadTo, Step};

use super::dialogs::Dialog;
use super::forms;
use super::open_editor;
use crate::app::VizApp;
use crate::theme::Tokens;

pub fn empty(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    ui.add_space(70.0);
    ui.vertical_centered(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
        super::icons::paint(ui.painter(), r, "new-source", t.accent, t.dark);
        ui.add_space(8.0);
        ui.label(RichText::new("No queries yet").size(20.0).strong());
        ui.add_space(4.0);
        ui.label(RichText::new("Get data from a file, a folder, a database or the web, or type it in.").color(t.muted));
        ui.add_space(14.0);
    });
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - 610.0) / 2.0).max(0.0));
        for (kind, label) in [("csv", "Text/CSV"), ("excel", "Excel workbook"), ("folder", "Folder"), ("web", "Web"), ("sqlite", "SQLite"), ("enter", "Enter data"), ("more", "More…")] {
            if ui.add(egui::Button::new(label).min_size(vec2(80.0, 30.0))).clicked() {
                super::get_data(app, kind);
            }
        }
    });
}

fn row_button(ui: &mut Ui, t: &Tokens, icon: &str, label: &str, active: bool, italic: bool, width: f32, indent: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    let p = ui.painter();
    if active {
        p.rect_filled(rect, 3.0, t.accent_soft);
        p.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 1.0, t.accent);
    } else if resp.hovered() {
        p.rect_filled(rect, 3.0, t.panel_alt);
    }
    super::icons::paint(p, Rect::from_min_size(rect.min + vec2(6.0 + indent, 4.0), vec2(16.0, 16.0)), icon, if active { t.accent } else { t.muted }, t.dark);
    let mut rt = RichText::new(label).size(12.0).color(if active { t.accent } else { t.text });
    if italic {
        rt = rt.italics();
    }
    let galley = egui::WidgetText::from(rt).into_galley(ui, Some(egui::TextWrapMode::Truncate), (width - 30.0 - indent).max(20.0), egui::TextStyle::Body);
    ui.painter().galley(pos2(rect.min.x + 28.0 + indent, rect.center().y - galley.size().y / 2.0), galley, t.text);
    resp
}

pub fn queries(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let doc = app.project.queries.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Queries [{}]", doc.queries.len())).strong().size(13.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button(RichText::new("+").strong(), |ui| {
                super::ops::new_source_items(app, ui);
                ui.separator();
                if ui.button("New Group…").clicked() {
                    app.pq.dialog = Some(Dialog::new_group(None));
                    ui.close();
                }
                if ui.button("New Parameter…").clicked() {
                    app.pq.dialog = Some(Dialog::parameters(app, Some(String::new())));
                    ui.close();
                }
            })
            .response
            .on_hover_text("New query, group or parameter");
            if ui.small_button("‹").on_hover_text("Hide the Queries pane (View › Queries Pane)").clicked() {
                app.pq.view.queries_pane = false;
            }
        });
    });
    ui.add_space(4.0);
    let mut groups: Vec<Option<String>> = Vec::new();
    for q in &doc.queries {
        if !groups.contains(&q.group) {
            groups.push(q.group.clone());
        }
    }
    groups.sort_by_key(|g| g.is_none());
    let row_w = (ui.available_width() - 4.0).max(60.0);
    let mut action: Option<Box<dyn FnOnce(&mut VizApp)>> = None;
    egui::ScrollArea::vertical().id_salt("pq_queries_scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        let grouped = groups.iter().any(Option::is_some);
        for g in &groups {
            let indent = if grouped { 12.0 } else { 0.0 };
            if grouped {
                let name = g.clone().unwrap_or_else(|| "Other Queries".into());
                let folded = app.pq.folded.contains(&name);
                let count = doc.queries.iter().filter(|q| &q.group == g).count();
                let (rect, resp) = ui.allocate_exact_size(vec2(row_w, 24.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 3.0, t.panel_alt);
                }
                forms::caret(ui.painter(), rect.min + vec2(7.0, 12.0), 3.5, t.muted, if folded { 'r' } else { 'd' });
                super::icons::paint(ui.painter(), Rect::from_min_size(rect.min + vec2(14.0, 4.0), vec2(16.0, 16.0)), "group-folder", t.muted, t.dark);
                ui.painter().text(rect.min + vec2(34.0, 12.0), Align2::LEFT_CENTER, format!("{name} [{count}]"), FontId::proportional(12.0), t.text);
                if resp.clicked() {
                    if folded {
                        app.pq.folded.retain(|x| *x != name);
                    } else {
                        app.pq.folded.push(name.clone());
                    }
                }
                if g.is_some() {
                    let gname = name.clone();
                    resp.context_menu(|ui| {
                        if ui.button("Rename Group…").clicked() {
                            app.pq.dialog = Some(Dialog::new_group(Some(gname.clone())));
                            ui.close();
                        }
                        if ui.button("Ungroup").clicked() {
                            let gn = gname.clone();
                            action = Some(Box::new(move |app: &mut VizApp| {
                                app.checkpoint("Ungroup");
                                for q in app.project.queries.queries.iter_mut() {
                                    if q.group.as_deref() == Some(gn.as_str()) {
                                        q.group = None;
                                    }
                                }
                            }));
                            ui.close();
                        }
                    });
                }
                if folded {
                    continue;
                }
            }
            for q in doc.queries.iter().filter(|q| &q.group == g) {
                let active = q.name == app.pq.query;
                let connection_only = q.load == LoadTo::None;
                let icon = if q.m.contains("=>") && !q.m.contains("Table.") { "invoke-function" } else { "query" };
                let r = row_button(ui, t, icon, &q.name, active, connection_only, row_w, indent);
                let r = r.on_hover_text(format!(
                    "{}{}",
                    if connection_only { "Connection only (doesn't load into the model)" } else { "Loads into the model" },
                    if q.description.is_empty() { String::new() } else { format!("\n{}", q.description) }
                ));
                if r.clicked() && !active {
                    app.pq.query = q.name.clone();
                    app.pq.step = None;
                    app.pq.selected.clear();
                    app.pq.examples = None;
                }
                let name = q.name.clone();
                let load = q.load;
                let groups_list: Vec<String> = groups.iter().flatten().cloned().collect();
                r.context_menu(|ui| query_menu(app, ui, &name, load, &groups_list));
            }
        }
        if !doc.parameters.is_empty() {
            ui.add_space(10.0);
            forms::caption(ui, t, "Parameters");
            for p in &doc.parameters {
                let r = row_button(ui, t, "parameter", &format!("{} = {}", p.name, p.value), false, false, row_w, 0.0).on_hover_text(if p.description.is_empty() { "Edit in Manage Parameters" } else { &p.description });
                if r.clicked() {
                    app.pq.dialog = Some(Dialog::parameters(app, Some(p.name.clone())));
                }
            }
        }
    });
    if let Some(a) = action {
        a(app);
    }
}

fn query_menu(app: &mut VizApp, ui: &mut Ui, name: &str, load: LoadTo, groups: &[String]) {
    if ui.button("Copy M").clicked() {
        if let Some(q) = app.project.queries.query(name) {
            ui.ctx().copy_text(q.m.clone());
        }
        ui.close();
    }
    ui.separator();
    if ui.button("Rename…").clicked() {
        app.pq.dialog = Some(Dialog::rename_query(name));
        ui.close();
    }
    if ui.button("Duplicate").clicked() {
        if let Some(n) = super::duplicate_query(app, name) {
            open_editor(app, Some(&n));
        }
        ui.close();
    }
    if ui.button("Reference").clicked() {
        if let Some(n) = super::reference_query(app, name) {
            open_editor(app, Some(&n));
        }
        ui.close();
    }
    if ui.button("Delete").clicked() {
        super::delete_query(app, name);
        ui.close();
    }
    ui.separator();
    let mut enabled = load != LoadTo::None;
    if ui.checkbox(&mut enabled, "Enable load").on_hover_text("Off: connection only (other queries can still use it)").clicked() {
        super::set_load(app, name, enabled);
        ui.close();
    }
    ui.menu_button("Move To Group", |ui| {
        for g in groups {
            if ui.button(g).clicked() {
                super::set_group(app, name, Some(g.clone()));
                ui.close();
            }
        }
        if ui.button("(No group)").clicked() {
            super::set_group(app, name, None);
            ui.close();
        }
        ui.separator();
        if ui.button("New Group…").clicked() {
            app.pq.dialog = Some(Dialog::new_group_for(name));
            ui.close();
        }
    });
    ui.separator();
    if ui.button("Advanced Editor").clicked() {
        app.pq.query = name.to_string();
        app.pq.step = None;
        app.pq.dialog = Some(Dialog::advanced(app));
        ui.close();
    }
    if ui.button("Properties…").clicked() {
        app.pq.query = name.to_string();
        app.pq.dialog = Some(Dialog::properties(app));
        ui.close();
    }
}

/// The icon of a step, from what it does.
fn step_icon(expr: &str) -> &'static str {
    let e = expr.trim_start();
    let f = e.split('(').next().unwrap_or_default();
    match f {
        "Csv.Document" | "File.Contents" => "file-csv",
        "Excel.Workbook" => "file-excel",
        "Json.Document" => "file-json",
        "Folder.Files" | "Folder.Contents" => "folder",
        "Web.Contents" | "Web.Page" => "web",
        "Sqlite.Database" | "PostgreSQL.Database" | "MySQL.Database" | "Sql.Database" | "Odbc.Query" => "database",
        "Table.TransformColumnTypes" => "data-type",
        "Table.PromoteHeaders" | "Table.DemoteHeaders" => "headers",
        "Table.SelectRows" => "keep-rows",
        "Table.RemoveColumns" => "remove-columns",
        "Table.SelectColumns" => "choose-columns",
        "Table.Sort" => "sort-asc",
        "Table.Group" => "group-by",
        "Table.AddColumn" => "custom-column",
        "Table.NestedJoin" | "Table.Join" => "merge-queries",
        "Table.Combine" => "append-queries",
        "Table.ExpandTableColumn" | "Table.ExpandRecordColumn" | "Table.ExpandListColumn" => "expand-column",
        "Table.RenameColumns" => "rename",
        "Table.ReplaceValue" | "Table.ReplaceErrorValues" => "replace",
        "Table.SplitColumn" => "split-column",
        "Table.Pivot" => "pivot-column",
        "Table.Unpivot" | "Table.UnpivotOtherColumns" => "unpivot",
        "Table.AddIndexColumn" => "index-column",
        "Python.Execute" => "python",
        _ if e.starts_with("#table") => "enter-data",
        _ => "settings",
    }
}

/// Steps whose settings open a dialog (the gear icon).
fn has_settings(expr: &str) -> bool {
    let f = expr.trim_start().split('(').next().unwrap_or_default();
    !matches!(f, "Table.TransformColumnTypes" | "Table.RemoveColumns" | "Table.Distinct" | "Table.Transpose" | "Table.ReverseRows")
}

pub fn settings(app: &mut VizApp, ui: &mut Ui, t: &Tokens, steps: &[Step]) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Query Settings").strong().size(13.5));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if forms::glyph_button(ui, t, 'x', t.muted, "Hide Query Settings (View › Query Settings)").clicked() {
                app.pq.view.settings_pane = false;
            }
        });
    });
    ui.add_space(6.0);
    forms::caption(ui, t, "Properties");
    ui.label(RichText::new("Name").size(11.0).color(t.muted));
    let q = app.pq.query.clone();
    let id = ui.scope_id().with("pq_name_edit");
    let mut buf: String = ui.data_mut(|d| d.get_temp::<(String, String)>(id)).filter(|(k, _)| *k == q).map(|(_, v)| v).unwrap_or_else(|| q.clone());
    let r = ui.add(egui::TextEdit::singleline(&mut buf).desired_width(f32::INFINITY));
    if r.lost_focus() && buf.trim() != q && !q.is_empty() {
        if let Err(e) = super::rename_query(app, &q, &buf) {
            app.error(e);
        }
        ui.data_mut(|d| d.remove::<(String, String)>(id));
    } else {
        ui.data_mut(|d| d.insert_temp(id, (q.clone(), buf)));
    }
    let desc = app.project.queries.query(&q).map(|x| x.description.clone()).unwrap_or_default();
    if !desc.is_empty() {
        ui.label(RichText::new(desc).size(11.0).color(t.muted));
    }
    if ui.link("All Properties").clicked() {
        app.pq.dialog = Some(Dialog::properties(app));
    }
    ui.add_space(10.0);
    forms::caption(ui, t, "Applied Steps");
    ui.add_space(2.0);
    let selected = app.pq.step.clone().or_else(|| steps.last().map(|s| s.name.clone()));
    let last = steps.len().saturating_sub(1);
    let row_w = (ui.available_width() - 2.0).max(80.0);
    egui::ScrollArea::vertical().id_salt("pq_steps").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        for (i, s) in steps.iter().enumerate() {
            let active = selected.as_deref() == Some(s.name.as_str());
            let (rect, r) = ui.allocate_exact_size(vec2(row_w, 24.0), Sense::click());
            let p = ui.painter();
            if active {
                p.rect_filled(rect, 3.0, t.accent_soft);
                p.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 1.0, t.accent);
            } else if r.hovered() {
                p.rect_filled(rect, 3.0, t.panel_alt);
            }
            super::icons::paint(p, Rect::from_min_size(rect.min + vec2(7.0, 4.0), vec2(16.0, 16.0)), step_icon(&s.expr), if active { t.accent } else { t.muted }, t.dark);
            let gear_w = if has_settings(&s.expr) { 20.0 } else { 0.0 };
            let del = Rect::from_min_size(pos2(rect.max.x - 20.0, rect.min.y + 4.0), vec2(16.0, 16.0));
            let gear = Rect::from_min_size(pos2(del.min.x - gear_w, rect.min.y + 4.0), vec2(16.0, 16.0));
            let text_w = (gear.min.x - rect.min.x - 30.0).max(20.0);
            let galley = egui::WidgetText::from(RichText::new(&s.name).size(12.0).color(if active { t.accent } else { t.text })).into_galley(ui, Some(egui::TextWrapMode::Truncate), text_w, egui::TextStyle::Body);
            ui.painter().galley(pos2(rect.min.x + 29.0, rect.center().y - galley.size().y / 2.0), galley, t.text);
            let show_tools = r.hovered() || active || r.contains_pointer();
            let mut consumed = false;
            if gear_w > 0.0 {
                let gr = ui.interact(gear, ui.scope_id().with(("pq_gear", i)), Sense::click()).on_hover_text("Edit this step's settings");
                super::icons::paint(ui.painter(), gear, "gear", if gr.hovered() { t.accent } else { t.muted.gamma_multiply(if show_tools { 1.0 } else { 0.6 }) }, t.dark);
                if gr.clicked() {
                    app.pq.step = if i == last { None } else { Some(s.name.clone()) };
                    app.pq.dialog = Some(Dialog::step_settings(app, &s.name));
                    consumed = true;
                }
            }
            if show_tools {
                let dr = ui.interact(del, ui.scope_id().with(("pq_del", i)), Sense::click()).on_hover_text("Delete this step");
                forms::cross(ui.painter(), del.center(), 3.5, if dr.hovered() { t.danger } else { t.muted });
                if dr.clicked() {
                    let name = s.name.clone();
                    super::edit_steps(app, "Delete step", |m| steps::delete_step(m, &name));
                    app.pq.step = None;
                    consumed = true;
                }
            }
            let r = r.on_hover_text(&s.expr);
            if r.clicked() && !consumed {
                app.pq.step = if i == last { None } else { Some(s.name.clone()) };
                app.pq.selected.clear();
            }
            if r.double_clicked() {
                app.pq.dialog = Some(Dialog::rename_step(&s.name));
            }
            let name = s.name.clone();
            r.context_menu(|ui| step_menu(app, ui, &name, i, last));
        }
    });
}

fn step_menu(app: &mut VizApp, ui: &mut Ui, name: &str, i: usize, last: usize) {
    if ui.button("Edit Settings").clicked() {
        app.pq.dialog = Some(Dialog::step_settings(app, name));
        ui.close();
    }
    if ui.button("Rename").clicked() {
        app.pq.dialog = Some(Dialog::rename_step(name));
        ui.close();
    }
    if ui.button("Delete").clicked() {
        super::edit_steps(app, "Delete step", |m| steps::delete_step(m, name));
        app.pq.step = None;
        ui.close();
    }
    if i > 0 && ui.button("Delete Until End").clicked() {
        super::edit_steps(app, "Delete until end", |m| steps::delete_until_end(m, name));
        app.pq.step = None;
        ui.close();
    }
    if ui.button("Insert Step After").clicked() {
        app.pq.step = if i == last { None } else { Some(name.to_string()) };
        super::add_step(app, "Custom", "{prev}");
        ui.close();
    }
    if i > 1 && ui.button("Move Before").clicked() {
        super::edit_steps(app, "Move step", |m| steps::move_step(m, name, -1));
        ui.close();
    }
    if i > 0 && i < last && ui.button("Move After").clicked() {
        super::edit_steps(app, "Move step", |m| steps::move_step(m, name, 1));
        ui.close();
    }
    if i > 0 && ui.button("Extract Previous").on_hover_text("Move the steps up to this one into a new query, and refer to it").clicked() {
        extract_previous(app, name);
        ui.close();
    }
}

/// Moves the steps before `name` into a new query that this one refers to.
fn extract_previous(app: &mut VizApp, name: &str) {
    let Some(q) = app.project.queries.query(&app.pq.query).cloned() else { return };
    let Ok(all) = steps::steps_of(&q.m) else { return };
    let Some(i) = all.iter().position(|s| s.name == name) else { return };
    if i == 0 {
        return;
    }
    let before: Vec<Step> = all.get(..i).unwrap_or_default().to_vec();
    let prev = before.last().map(|s| s.name.clone()).unwrap_or_default();
    let new_name = app.project.free_table_name(&format!("{} Base", q.name));
    let base = super::new_query(app, &new_name, steps::build_m(&before), false, q.group.clone());
    let mut rest: Vec<Step> = all.get(i..).unwrap_or_default().to_vec();
    rest.insert(0, Step { name: "Source".into(), expr: steps::ident(&base) });
    if let Some(s) = rest.get_mut(1) {
        s.expr = steps::rename_refs(&s.expr, &prev, "Source");
    }
    let cur = app.pq.query.clone();
    if let Some(qq) = app.project.queries.query_mut(&cur) {
        qq.m = steps::build_m(&rest);
    }
    app.pq.step = None;
}

pub fn formula_bar(app: &mut VizApp, ui: &mut Ui, t: &Tokens, steps: &[Step]) {
    let sel = app.pq.step.clone().or_else(|| steps.last().map(|s| s.name.clone())).unwrap_or_default();
    let key = Some((app.pq.query.clone(), sel.clone()));
    let expr = steps.iter().find(|s| s.name == sel).map(|s| s.expr.clone()).unwrap_or_default();
    if app.pq.formula_for != key {
        app.pq.formula = format!("= {expr}");
        app.pq.formula_for = key;
    }
    egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::symmetric(8, 5)).stroke(Stroke::new(1.0, t.border)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let cancel = forms::glyph_button(ui, t, 'x', t.muted, "Cancel (Esc)").clicked();
            let ok = forms::glyph_button(ui, t, 'v', t.good, "Apply (Enter)").clicked();
            if ui.add(egui::Button::new(RichText::new("fx").italics().color(t.accent)).frame(false)).on_hover_text("Add a custom step after this one").clicked() {
                super::add_step(app, "Custom", "{prev}");
            }
            let lines = app.pq.formula.lines().count().clamp(1, 6);
            let r = ui.add(egui::TextEdit::multiline(&mut app.pq.formula).desired_rows(lines).desired_width(f32::INFINITY).font(FontId::monospace(12.5)).return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)));
            let enter = r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
            if cancel || (r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape))) {
                app.pq.formula_for = None;
            }
            if ok || enter {
                let text = app.pq.formula.trim().trim_start_matches('=').trim().to_string();
                if text != expr && !sel.is_empty() {
                    let s = sel.clone();
                    if !super::edit_steps(app, "Edit step", |m| steps::set_step_expr(m, &s, &text)) {
                        app.pq.formula_for = Some((String::new(), String::new()));
                    }
                }
                r.surrender_focus();
            }
        });
    });
}

pub fn status_bar(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    ui.horizontal(|ui| {
        let Some(p) = super::preview(app) else {
            ui.label(RichText::new("Ready").size(11.0).color(t.muted));
            return;
        };
        let text = match (&p.error, &p.scalar) {
            (Some(_), _) => "Error".to_string(),
            (_, Some(_)) => "1 value".to_string(),
            _ => format!("{} COLUMNS, {} ROWS", p.table.columns.len(), p.table.rows.len()),
        };
        ui.label(RichText::new(text).size(11.0).color(t.text));
        ui.separator();
        let basis = if app.pq.view.all_rows { "Column profiling based on entire data set" } else { "Column profiling based on top 1000 rows" };
        let r = ui.add(egui::Label::new(RichText::new(basis).size(11.0).color(t.muted)).sense(Sense::click())).on_hover_text("Click to switch");
        if r.clicked() {
            app.pq.view.all_rows = !app.pq.view.all_rows;
            app.pq.profiles.clear();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("Preview evaluated in {:.0} ms", p.ms)).size(11.0).color(t.muted));
        });
    });
}
