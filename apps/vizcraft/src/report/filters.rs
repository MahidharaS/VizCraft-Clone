//! The Filters pane: filters on this visual, on this page and on all pages, with basic,
//! advanced, Top N, relative date and range filtering.

use egui::{RichText, Ui};
use vizcraft_core::doc::*;
use vizcraft_core::scalar::Scalar;

use super::panes::DragField;
use crate::app::VizApp;
use crate::theme::Tokens;
use crate::widgets;

#[derive(Clone, Copy, PartialEq)]
enum Level {
    Visual(u64),
    Page,
    Report,
}

pub fn add_filter(app: &mut VizApp, level: &str, f: FieldRef) {
    let id = app.project.next_id();
    let kind = default_kind(app, &f);
    let fd = FilterDef { id, field: FieldUse::new(f), kind, locked: false, hidden: false, require_single: false, context: false };
    app.checkpoint("Add filter");
    match level {
        "report" => app.project.report.filters.push(fd),
        "page" => {
            if let Some(pg) = app.current_page_mut() {
                pg.filters.push(fd);
            }
        }
        _ => {
            if let Some(eid) = app.selected.first().copied()
                && let Some(v) = app.project.element_mut(eid).and_then(|e| e.visual_mut())
            {
                v.filters.push(fd);
            }
        }
    }
    app.report.render_cache.clear();
}

fn default_kind(app: &VizApp, f: &FieldRef) -> FilterKind {
    if f.is_measure() {
        return FilterKind::Advanced { conditions: vec![Condition { op: "is greater than".into(), value: Scalar::Num(0.0) }], and: true };
    }
    if let FieldRef::Column { table, column } = f
        && app.built.col_type(table, column).is_temporal()
    {
        return FilterKind::Range { min: None, max: None };
    }
    FilterKind::Basic { values: vec![], exclude: false }
}

pub fn pane(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    if widgets::pane_header(ui, t, "Filters") {
        app.panes.filters = false;
    }
    egui::ScrollArea::vertical().id_salt("filters-scroll").auto_shrink([false, false]).show(ui, |ui| {
        if let Some(eid) = app.selected.first().copied()
            && let Some((_, el)) = app.project.element(eid).map(|(p, e)| (p, e.clone()))
            && let Some(v) = el.visual()
        {
            section(app, ui, t, Level::Visual(eid), "Filters on this visual", v.filters.clone());
            // The visual's own fields, as Power BI lists them (filter on demand).
            let fields: Vec<FieldUse> = v.all_fields().filter(|f| !v.filters.iter().any(|x| x.field.field == f.field)).cloned().collect();
            for f in fields {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  is (All)", f.label())).size(11.0).color(t.muted));
                    if ui.small_button("+").on_hover_text("Filter this field").clicked() {
                        let id = app.project.next_id();
                        let mut kind = default_kind(app, &f.field);
                        if f.is_value() {
                            kind = FilterKind::Advanced { conditions: vec![Condition { op: "is greater than".into(), value: Scalar::Num(0.0) }], and: true };
                        }
                        app.checkpoint("Add filter");
                        if let Some(vv) = app.project.element_mut(eid).and_then(|e| e.visual_mut()) {
                            vv.filters.push(FilterDef { id, field: f.clone(), kind, locked: false, hidden: false, require_single: false, context: false });
                        }
                    }
                });
            }
        }
        let page_filters = app.current_page().map(|p| p.filters.clone()).unwrap_or_default();
        section(app, ui, t, Level::Page, "Filters on this page", page_filters);
        let rep = app.project.report.filters.clone();
        section(app, ui, t, Level::Report, "Filters on all pages", rep);
    });
}

fn section(app: &mut VizApp, ui: &mut Ui, t: &Tokens, level: Level, title: &str, filters: Vec<FilterDef>) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).strong().size(12.0));
    for f in &filters {
        card(app, ui, t, level, f);
    }
    let frame = egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4.0).inner_margin(egui::Margin::same(6));
    let (_, payload) = ui.dnd_drop_zone::<DragField, ()>(frame, |ui| {
        ui.set_min_width(ui.available_width());
        ui.label(RichText::new("Add data fields here").size(11.0).color(t.muted));
    });
    if let Some(p) = payload {
        let lv = match level {
            Level::Visual(_) => "visual",
            Level::Page => "page",
            Level::Report => "report",
        };
        add_filter(app, lv, p.0.clone());
    }
}

fn store(app: &mut VizApp, level: Level, f: FilterDef, remove: bool) {
    let list: Option<&mut Vec<FilterDef>> = match level {
        Level::Report => Some(&mut app.project.report.filters),
        Level::Page => {
            let id = app.page;
            app.project.page_mut(id).map(|p| &mut p.filters)
        }
        Level::Visual(eid) => app.project.element_mut(eid).and_then(|e| e.visual_mut()).map(|v| &mut v.filters),
    };
    if let Some(list) = list {
        if remove {
            list.retain(|x| x.id != f.id);
        } else if let Some(slot) = list.iter_mut().find(|x| x.id == f.id) {
            *slot = f;
        }
    }
    app.dirty = true;
    app.report.render_cache.clear();
}

fn summary(f: &FilterDef) -> String {
    match &f.kind {
        FilterKind::Basic { values, exclude } if !values.is_empty() => {
            let v: Vec<String> = values.iter().take(3).map(Scalar::text).collect();
            format!("{} {}{}", if *exclude { "is not" } else { "is" }, v.join(", "), if values.len() > 3 { "…" } else { "" })
        }
        FilterKind::Advanced { conditions, .. } if !conditions.is_empty() => conditions.iter().map(|c| format!("{} {}", c.op, c.value.text())).collect::<Vec<_>>().join(" / "),
        FilterKind::TopN { n, top, .. } => format!("{} {n}", if *top { "Top" } else { "Bottom" }),
        FilterKind::Range { min, max } if min.is_some() || max.is_some() => format!("{} – {}", min.map(fmt_num).unwrap_or_default(), max.map(fmt_num).unwrap_or_default()),
        FilterKind::RelativeDate { mode, n, unit, .. } => format!("{mode} {n} {unit}"),
        FilterKind::Dax { expression } if !expression.is_empty() => "DAX".into(),
        _ => "is (All)".into(),
    }
}

fn fmt_num(x: f64) -> String {
    if x > 20000.0 && x < 80000.0 { vizcraft_core::numfmt::format_date(x, "yyyy-mm-dd") } else { vizcraft_core::numfmt::general(x) }
}

fn card(app: &mut VizApp, ui: &mut Ui, t: &Tokens, level: Level, f: &FilterDef) {
    let mut fd = f.clone();
    let mut changed = false;
    let mut remove = false;
    let active = !summary(f).ends_with("(All)");
    egui::Frame::new().fill(if active { t.accent_soft } else { t.panel_alt }).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4.0).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        let open_id = ui.scope_id().with(("fopen", f.id));
        let mut open = ui.ctx().data_mut(|d| *d.get_temp_mut_or(open_id, false));
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(if open { "▾" } else { "▸" }).frame(false)).clicked() {
                open = !open;
            }
            ui.vertical(|ui| {
                ui.label(RichText::new(f.field.label()).strong().size(12.0));
                ui.label(RichText::new(summary(f)).size(10.5).color(t.muted));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("✕").on_hover_text("Remove filter").clicked() {
                    remove = true;
                }
                if ui.small_button(if fd.locked { "🔒" } else { "🔓" }).on_hover_text("Lock filter (viewers can't change it)").clicked() {
                    fd.locked = !fd.locked;
                    changed = true;
                }
                if ui.small_button(if fd.hidden { "◌" } else { "◉" }).on_hover_text("Hide filter from viewers").clicked() {
                    fd.hidden = !fd.hidden;
                    changed = true;
                }
                if ui.small_button("↺").on_hover_text("Clear filter").clicked() {
                    fd.kind = default_kind(app, &fd.field.field);
                    changed = true;
                }
            });
        });
        ui.ctx().data_mut(|d| d.insert_temp(open_id, open));
        if !open {
            return;
        }
        // Filter type.
        let types = ["Basic filtering", "Advanced filtering", "Top N", "Relative date", "Range", "DAX"];
        let cur = match fd.kind {
            FilterKind::Basic { .. } => 0,
            FilterKind::Advanced { .. } => 1,
            FilterKind::TopN { .. } => 2,
            FilterKind::RelativeDate { .. } => 3,
            FilterKind::Range { .. } => 4,
            FilterKind::Dax { .. } => 5,
        };
        let mut sel = cur;
        egui::ComboBox::from_id_salt(("ftype", f.id)).selected_text(*types.get(cur).unwrap_or(&"")).show_ui(ui, |ui| {
            for (i, ty) in types.iter().enumerate() {
                ui.selectable_value(&mut sel, i, *ty);
            }
        });
        if sel != cur {
            fd.kind = match sel {
                1 => FilterKind::Advanced { conditions: vec![Condition { op: "is".into(), value: Scalar::Null }], and: true },
                2 => FilterKind::TopN { n: 5, top: true, by: None },
                3 => FilterKind::RelativeDate { mode: "Last".into(), n: 30, unit: "days".into(), include_today: true },
                4 => FilterKind::Range { min: None, max: None },
                5 => FilterKind::Dax { expression: String::new() },
                _ => FilterKind::Basic { values: vec![], exclude: false },
            };
            changed = true;
        }
        match &mut fd.kind {
            FilterKind::Basic { values, exclude } => {
                let search_id = ui.scope_id().with(("fs", f.id));
                let mut search: String = ui.ctx().data_mut(|d| d.get_temp::<String>(search_id).unwrap_or_default());
                ui.add(egui::TextEdit::singleline(&mut search).hint_text("Search"));
                ui.ctx().data_mut(|d| d.insert_temp(search_id, search.clone()));
                let all = vizcraft_core::vquery::distinct_values(&app.project, &app.built, &fd.field.field, 2000);
                ui.horizontal(|ui| {
                    let mut all_on = values.is_empty();
                    if ui.checkbox(&mut all_on, "Select all").changed() {
                        values.clear();
                        changed = true;
                    }
                    changed |= ui.checkbox(exclude, "Exclude").changed();
                });
                egui::ScrollArea::vertical().id_salt(("fvals", f.id)).max_height(180.0).show(ui, |ui| {
                    for v in all.iter().filter(|v| search.is_empty() || v.text().to_lowercase().contains(&search.to_lowercase())).take(500) {
                        let mut on = values.iter().any(|x| x.key() == v.key());
                        if ui.checkbox(&mut on, v.text()).changed() {
                            if on {
                                if fd.require_single {
                                    values.clear();
                                }
                                values.push(v.clone());
                            } else {
                                values.retain(|x| x.key() != v.key());
                            }
                            changed = true;
                        }
                    }
                });
                changed |= ui.checkbox(&mut fd.require_single, "Require single selection").changed();
            }
            FilterKind::Advanced { conditions, and } => {
                ui.label(RichText::new("Show items when the value").size(11.0));
                let ops = ["is", "is not", "is less than", "is less than or equal to", "is greater than", "is greater than or equal to", "contains", "does not contain", "starts with", "does not start with", "is blank", "is not blank"];
                while conditions.len() < 2 {
                    conditions.push(Condition { op: String::new(), value: Scalar::Null });
                }
                for (i, c) in conditions.iter_mut().enumerate() {
                    if i == 1 {
                        ui.horizontal(|ui| {
                            changed |= ui.radio_value(and, true, "And").changed();
                            changed |= ui.radio_value(and, false, "Or").changed();
                        });
                    }
                    egui::ComboBox::from_id_salt(("cop", f.id, i)).selected_text(if c.op.is_empty() { "(none)" } else { c.op.as_str() }).width(ui.available_width()).show_ui(ui, |ui| {
                        if ui.selectable_label(c.op.is_empty(), "(none)").clicked() {
                            c.op.clear();
                            changed = true;
                        }
                        for o in ops {
                            if ui.selectable_label(c.op == o, o).clicked() {
                                c.op = o.into();
                                changed = true;
                            }
                        }
                    });
                    let mut s = if c.value.is_null() { String::new() } else { c.value.text() };
                    if ui.text_edit_singleline(&mut s).changed() {
                        c.value = vizcraft_core::sheet::parse_input(&s);
                        changed = true;
                    }
                }
            }
            FilterKind::TopN { n, top, by } => {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("topn", f.id)).selected_text(if *top { "Top" } else { "Bottom" }).show_ui(ui, |ui| {
                        changed |= ui.selectable_value(top, true, "Top").changed();
                        changed |= ui.selectable_value(top, false, "Bottom").changed();
                    });
                    changed |= ui.add(egui::DragValue::new(n).range(1..=10000)).changed();
                });
                ui.label(RichText::new("By value").size(11.0));
                let frame = egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4.0).inner_margin(egui::Margin::same(4));
                let (_, payload) = ui.dnd_drop_zone::<DragField, ()>(frame, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(by.as_ref().map(|b| b.label()).unwrap_or_else(|| "Drag a measure here".into()));
                });
                if let Some(p) = payload {
                    let mut fu = FieldUse::new(p.0.clone());
                    if let FieldRef::Column { table, column } = &p.0 {
                        fu.agg = vizcraft_core::state::default_agg(&app.project, &app.built, table, column);
                    }
                    *by = Some(fu);
                    changed = true;
                }
            }
            FilterKind::RelativeDate { mode, n, unit, include_today } => {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("rdm", f.id)).selected_text(mode.as_str()).show_ui(ui, |ui| {
                        for m in ["Last", "Next", "This"] {
                            changed |= ui.selectable_value(mode, m.to_string(), m).changed();
                        }
                    });
                    changed |= ui.add(egui::DragValue::new(n).range(0..=10000)).changed();
                    egui::ComboBox::from_id_salt(("rdu", f.id)).selected_text(unit.as_str()).show_ui(ui, |ui| {
                        for u in ["days", "weeks", "months", "quarters", "years"] {
                            changed |= ui.selectable_value(unit, u.to_string(), u).changed();
                        }
                    });
                });
                changed |= ui.checkbox(include_today, "Include today").changed();
            }
            FilterKind::Range { min, max } => {
                let date = matches!(&fd.field.field, FieldRef::Column { table, column } if app.built.col_type(table, column).is_temporal());
                for (label, slot) in [("From", min), ("To", max)] {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        let mut s = slot.map(|x| if date { vizcraft_core::numfmt::format_date(x, "yyyy-mm-dd") } else { vizcraft_core::numfmt::general(x) }).unwrap_or_default();
                        if ui.text_edit_singleline(&mut s).changed() {
                            *slot = if s.trim().is_empty() { None } else { s.trim().parse::<f64>().ok().or_else(|| vizcraft_core::build::parse_date(&s)) };
                            changed = true;
                        }
                    });
                }
            }
            FilterKind::Dax { expression } => {
                changed |= ui.add(egui::TextEdit::multiline(expression).code_editor().hint_text("'Sales'[Quantity] > 2").desired_rows(2)).changed();
            }
        }
        changed |= ui.checkbox(&mut fd.context, "Context filter (applied before Top N)").changed();
    });
    if remove {
        app.checkpoint("Remove filter");
        store(app, level, fd, true);
    } else if changed {
        app.checkpoint("Filter");
        store(app, level, fd, false);
    }
}
